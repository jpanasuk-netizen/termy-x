use super::*;

impl KittyGraphicsState {
    pub(super) fn placement_by_key(
        &self,
        screen: KittyGraphicsScreen,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&Placement> {
        self.placements.iter().find(|placement| {
            placement.screen == screen
                && placement.image_id == image_id
                && (placement_id == 0 || placement.placement_id == placement_id)
        })
    }

    pub(super) fn validate_relative_parent(
        &self,
        screen: KittyGraphicsScreen,
        image_id: u32,
        placement_id: u32,
        parent_image_id: u32,
        parent_placement_id: u32,
    ) -> Result<(), String> {
        let mut parent = self
            .placement_by_key(screen, parent_image_id, parent_placement_id)
            .ok_or_else(|| "ENOPARENT:relative placement parent not found".to_string())?;
        for depth in 1..=MAX_RELATIVE_DEPTH {
            // Q=0 selects an existing placement. Compare its resolved identity,
            // not the wildcard, before accepting a replacement of that placement.
            if placement_id != 0
                && parent.image_id == image_id
                && parent.placement_id == placement_id
            {
                return Err("ECYCLE:relative placement cycle".into());
            }
            let PlacementLocation::Relative {
                parent_image_id,
                parent_placement_id,
                ..
            } = parent.location
            else {
                return Ok(());
            };
            parent = self
                .placement_by_key(screen, parent_image_id, parent_placement_id)
                .ok_or_else(|| "ENOPARENT:relative placement parent not found".to_string())?;
            if depth == MAX_RELATIVE_DEPTH {
                return Err("ETOODEEP:relative placement chain is too deep".into());
            }
        }
        unreachable!()
    }

    /// Remove relative placements whose parent is gone. Their images stay:
    /// named ones can be placed again, and id-less ones are released below.
    pub(super) fn remove_orphaned_relative_placements(&mut self) {
        loop {
            let existing: std::collections::HashSet<_> = self
                .placements
                .iter()
                .flat_map(|p| {
                    [
                        (p.screen, p.image_id, p.placement_id),
                        (p.screen, p.image_id, 0),
                    ]
                })
                .collect();
            let before = self.placements.len();
            self.retain_placements(|placement| {
                let PlacementLocation::Relative {
                    parent_image_id,
                    parent_placement_id,
                    ..
                } = placement.location
                else {
                    return true;
                };
                existing.contains(&(placement.screen, parent_image_id, parent_placement_id))
            });
            if before == self.placements.len() {
                break;
            }
        }
        self.release_unplaced_anonymous_images();
    }
}

/// Resolves placement origins for one command or snapshot. Parents are found
/// through an index built on the first relative hop, and each virtual
/// parent's placeholder scan runs once, instead of once per placement.
pub(super) struct OriginResolver<'a> {
    placements: &'a [Placement],
    placeholders: &'a [KittyGraphicsPlaceholder],
    by_key: Option<HashMap<(KittyGraphicsScreen, u32, u32), usize>>,
    virtual_origins: HashMap<(u64, bool), Option<(i64, usize)>>,
}

impl<'a> OriginResolver<'a> {
    pub(super) fn new(
        placements: &'a [Placement],
        placeholders: &'a [KittyGraphicsPlaceholder],
    ) -> Self {
        Self {
            placements,
            placeholders,
            by_key: None,
            virtual_origins: HashMap::new(),
        }
    }

    /// Same lookup as [`KittyGraphicsState::placement_by_key`]: `Q=0` selects
    /// the image's first placement.
    fn parent(
        &mut self,
        screen: KittyGraphicsScreen,
        image_id: u32,
        placement_id: u32,
    ) -> Option<&'a Placement> {
        let placements = self.placements;
        let by_key = self.by_key.get_or_insert_with(|| {
            let mut by_key = HashMap::with_capacity(placements.len() * 2);
            for (index, placement) in placements.iter().enumerate() {
                let (screen, image_id) = (placement.screen, placement.image_id);
                by_key
                    .entry((screen, image_id, placement.placement_id))
                    .or_insert(index);
                by_key.entry((screen, image_id, 0)).or_insert(index);
            }
            by_key
        });
        by_key
            .get(&(screen, image_id, placement_id))
            .map(|index| &placements[*index])
    }

    fn virtual_origin(&mut self, current: &Placement, relative: bool) -> Option<(i64, usize)> {
        if let Some(origin) = self
            .virtual_origins
            .get(&(current.placement_serial, relative))
        {
            return *origin;
        }
        #[cfg(test)]
        VIRTUAL_ORIGIN_SCANS.with(|scans| scans.set(scans.get() + 1));
        // Whether `current` is the image's prototype is the same for every
        // placeholder, so resolve it once.
        let prototype = self
            .placements
            .iter()
            .rev()
            .find(|candidate| {
                candidate.screen == current.screen
                    && candidate.image_id == current.image_id
                    && matches!(candidate.location, PlacementLocation::Virtual)
            })
            .is_some_and(|candidate| candidate.placement_serial == current.placement_serial);
        let matching = self.placeholders.iter().filter(|placeholder| {
            placeholder.image_id == current.image_id
                && (placeholder.placement_id == current.placement_id
                    || (placeholder.placement_id == 0 && prototype))
        });
        let origin = if relative {
            matching.fold(None, |origin, placeholder| {
                let candidate = (placeholder.viewport_row, placeholder.col);
                Some(origin.map_or(candidate, |(row, col): (i64, usize)| {
                    (row.min(candidate.0), col.min(candidate.1))
                }))
            })
        } else {
            matching.fold(None, |origin, placeholder| {
                let row = placeholder
                    .viewport_row
                    .saturating_sub(i64::from(placeholder.image_row));
                let col = placeholder
                    .col
                    .saturating_sub(placeholder.image_col as usize);
                Some(
                    origin.map_or((row, col), |(old_row, old_col): (i64, usize)| {
                        (old_row.min(row), old_col.min(col))
                    }),
                )
            })
        };
        self.virtual_origins
            .insert((current.placement_serial, relative), origin);
        origin
    }

    pub(super) fn resolve(&mut self, placement: &'a Placement) -> Option<ResolvedOrigin> {
        let mut current = placement;
        let mut horizontal_offset = 0i64;
        let mut vertical_offset = 0i64;
        let relative = matches!(placement.location, PlacementLocation::Relative { .. });
        for _ in 0..=MAX_RELATIVE_DEPTH {
            match current.location {
                PlacementLocation::Direct { anchor_line, col } => {
                    let col = i64::try_from(col)
                        .unwrap_or(i64::MAX)
                        .saturating_add(horizontal_offset);
                    return Some(ResolvedOrigin::Buffer {
                        anchor_line: anchor_line.saturating_add(vertical_offset),
                        col,
                    });
                }
                PlacementLocation::Virtual => {
                    let (row, col) = self.virtual_origin(current, relative)?;
                    let col = i64::try_from(col)
                        .unwrap_or(i64::MAX)
                        .saturating_add(horizontal_offset);
                    return Some(ResolvedOrigin::Viewport {
                        row: row.saturating_add(vertical_offset),
                        col,
                    });
                }
                PlacementLocation::Relative {
                    parent_image_id,
                    parent_placement_id,
                    horizontal_offset: horizontal,
                    vertical_offset: vertical,
                } => {
                    horizontal_offset = horizontal_offset.saturating_add(i64::from(horizontal));
                    vertical_offset = vertical_offset.saturating_add(i64::from(vertical));
                    current =
                        self.parent(placement.screen, parent_image_id, parent_placement_id)?;
                }
            }
        }
        None
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static VIRTUAL_ORIGIN_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
