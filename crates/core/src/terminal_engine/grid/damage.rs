//! Damage tracking: the dirty column span of each viewport row, ordered
//! row rotations for renderers that replay scrolls, and full invalidation.
use super::{Cursor, Damage, DirtySpan, Grid, MAX_SCROLL_DAMAGE, ViewportScroll};

impl Grid {
    pub(in crate::terminal_engine) fn mark_full_damage(&mut self) {
        self.visual_dirty = true;
        self.full_damage = true;
        self.damage_saturated = true;
        self.flood_exposed = false;
        self.pending_scrolls.clear();
    }

    /// Record a scroll of rows `top..bottom` by `lines` (up when positive).
    /// Inline for the case every line of a flood but the first hits: damage
    /// is saturated and the pending scroll already exposes the region.
    #[inline]
    pub(super) fn mark_scroll_damage(&mut self, top: usize, bottom: usize, lines: i32) {
        self.visual_dirty = true;
        if self.full_damage
            || (self.damage_saturated
                && self.display_offset() == 0
                && self.exposes_region(top, bottom, lines))
        {
            // As below: only the cursor would be marked, on a dirty row.
            return;
        }
        self.record_scroll_damage(top, bottom, lines);
    }

    /// [`Self::mark_scroll_damage`] for the whole screen scrolling up a
    /// line while the viewport shows the live screen, as a flood does.
    #[inline]
    pub(super) fn mark_live_scroll_damage(&mut self) {
        debug_assert_eq!(self.display_offset(), 0);
        debug_assert!(
            !self.flood_exposed
                || (self.damage_saturated && self.exposes_region(0, self.size.rows, 1)),
            "flood exposure outlived its pending scroll"
        );
        self.visual_dirty = true;
        if self.full_damage || self.flood_exposed {
            return;
        }
        let rows = self.size.rows;
        if self.damage_saturated && self.exposes_region(0, rows, 1) {
            self.flood_exposed = true;
            return;
        }
        self.record_scroll_damage(0, rows, 1);
    }

    /// Whether the last pending scroll moved `top..bottom` the same way as
    /// `lines`, by the whole region.
    #[inline]
    fn exposes_region(&self, top: usize, bottom: usize, lines: i32) -> bool {
        // Pending scrolls move a region by at most its height.
        let whole = (bottom - top) as i32;
        let lines = if lines > 0 { whole } else { -whole };
        self.pending_scrolls.last() == Some(&ViewportScroll { top, bottom, lines })
    }

    #[inline(never)]
    fn record_scroll_damage(&mut self, top: usize, bottom: usize, lines: i32) {
        // The last pending scroll may change below.
        self.flood_exposed = false;
        // While browsing history, scrolling can change both history and live
        // rows in the viewport. A full refresh also covers eviction at its top.
        if self.display_offset() != 0 {
            self.mark_full_damage();
            return;
        }
        let count = lines.unsigned_abs() as usize;
        debug_assert!(count != 0 && count <= bottom - top);
        // Once a pending scroll exposed the whole region, every row in it is
        // fully dirty, so rotating again (per line of a flood) changes nothing.
        if self.exposes_region(top, bottom, lines) {
            self.mark_cursor(self.cursor);
            return;
        }
        // The renderer may have painted the old cursor into a cached row. Its
        // old mark must travel with that row before marking its new position.
        self.mark_cursor(self.cursor);
        let exposed = Some((0, self.size.cols));
        // Rows shift by `count`, and the ones exposed are dirty throughout.
        if lines > 0 {
            self.dirty.copy_within(top + count..bottom, top);
            self.dirty[bottom - count..bottom].fill(exposed);
        } else {
            self.dirty.copy_within(top..bottom - count, top + count);
            self.dirty[top..top + count].fill(exposed);
        }
        self.mark_cursor(self.cursor);
        if let Some(previous) = self.pending_scrolls.last_mut()
            && previous.top == top
            && previous.bottom == bottom
            && previous.lines.signum() == lines.signum()
        {
            // Once the entire region was exposed, every cell is dirty, so a
            // full rotation plus those patches represents any further scroll.
            let count = (previous.lines.unsigned_abs() as usize + count).min(bottom - top);
            previous.lines = count as i32 * lines.signum();
        } else if self.pending_scrolls.len() < MAX_SCROLL_DAMAGE {
            self.pending_scrolls
                .push(ViewportScroll { top, bottom, lines });
        } else {
            self.mark_full_damage();
            return;
        }
        // Rows leave the dirty array only as exposed blanks, so once the
        // scroll spans the whole screen every row is dirty across its width.
        if top == 0
            && bottom == self.size.rows
            && self.pending_scrolls.last().is_some_and(|scroll| {
                scroll.top == top
                    && scroll.bottom == bottom
                    && scroll.lines.unsigned_abs() as usize == bottom
            })
        {
            self.damage_saturated = true;
        }
    }

    pub(super) fn mark(&mut self, row: usize, start: usize, end: usize) {
        self.visual_dirty = true;
        self.mark_damage(row, start, end);
    }

    #[inline]
    pub(super) fn mark_damage(&mut self, row: usize, start: usize, end: usize) {
        if self.damage_saturated {
            debug_assert!(
                self.full_damage
                    || row >= self.size.rows
                    || self.dirty[row + self.display_offset()] == Some((0, self.size.cols)),
                "saturated damage left row {row} partly clean"
            );
            return;
        }
        if row >= self.size.rows || start >= end {
            return;
        }
        let row = row.saturating_add(self.display_offset());
        if let Some(dirty) = self.dirty.get_mut(row) {
            let end = end.min(self.size.cols);
            *dirty = Some(match *dirty {
                Some((old_start, old_end)) => (old_start.min(start), old_end.max(end)),
                None => (start, end),
            });
        }
    }

    pub(super) fn mark_cursor(&mut self, cursor: Cursor) {
        if cursor.visible {
            // Cursor overlays need text damage but do not edit placeholder cells.
            self.mark_damage(cursor.row, cursor.col, cursor.col.saturating_add(1));
        }
    }

    /// Tracks edits independently of damage consumption, including while full
    /// damage is already pending. Graphics consume it to notice placeholder edits.
    pub(in crate::terminal_engine) fn take_visual_dirty(&mut self) -> bool {
        std::mem::take(&mut self.visual_dirty)
    }

    pub(in crate::terminal_engine) fn take_damage(&mut self) -> Damage {
        // Cell-only consumers cannot replay row rotations. Expand the affected
        // regions so their usual span patching still reconstructs the viewport.
        for scroll in &self.pending_scrolls {
            self.dirty[scroll.top..scroll.bottom].fill(Some((0, self.size.cols)));
        }
        self.pending_scrolls.clear();
        self.take_cell_damage()
    }

    pub(in crate::terminal_engine) fn take_render_damage(
        &mut self,
    ) -> (Damage, Vec<ViewportScroll>) {
        let scrolls = self.pending_scrolls.clone();
        self.pending_scrolls.clear();
        (self.take_cell_damage(), scrolls)
    }

    fn take_cell_damage(&mut self) -> Damage {
        self.damage_saturated = false;
        self.flood_exposed = false;
        if std::mem::take(&mut self.full_damage) {
            self.dirty.fill(None);
            return Damage::Full;
        }
        // Sized for every row, as after any flood, rather than grown.
        let mut spans = Vec::with_capacity(self.dirty.len());
        spans.extend(
            self.dirty
                .iter_mut()
                .enumerate()
                .filter_map(|(row, dirty)| {
                    dirty
                        .take()
                        .map(|(start, end)| DirtySpan { row, start, end })
                }),
        );
        Damage::Partial(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{grid, print};
    use super::*;

    // Once a flood has exposed the whole screen, marking skips the dirty
    // rows, which must already hold everything the marks would add.
    #[test]
    fn edits_after_a_whole_screen_scroll_leave_the_exposed_damage_unchanged() {
        let mut grid = grid(8, 4, 10);
        grid.take_damage();
        print(&mut grid, "a\r\nb\r\nc\r\nd");
        for _ in 0..3 {
            grid.scroll_up(1);
        }
        // Three of four rows exposed: marking still records damage.
        assert!(!grid.damage_saturated);
        grid.scroll_up(1);
        assert!(grid.damage_saturated);
        grid.goto(1, 3);
        print(&mut grid, "xyz\r\n界");
        grid.scroll_up(1);
        // A region scroll is still recorded for renderers that replay it.
        grid.set_scroll_region(1, 3);
        grid.scroll_down(1);
        let (damage, scrolls) = grid.take_render_damage();
        assert_eq!(
            damage,
            Damage::Partial(
                (0..4)
                    .map(|row| DirtySpan {
                        row,
                        start: 0,
                        end: 8
                    })
                    .collect()
            )
        );
        assert_eq!(
            scrolls,
            vec![
                ViewportScroll {
                    top: 0,
                    bottom: 4,
                    lines: 4
                },
                ViewportScroll {
                    top: 1,
                    bottom: 3,
                    lines: -1
                }
            ]
        );
        // Taking the damage ends saturation: the cursor leaves its home
        // (where the scroll region put it) and prints.
        assert!(!grid.damage_saturated);
        grid.goto(2, 1);
        print(&mut grid, "q");
        assert_eq!(
            grid.take_damage(),
            Damage::Partial(vec![
                DirtySpan {
                    row: 0,
                    start: 0,
                    end: 1
                },
                DirtySpan {
                    row: 2,
                    start: 0,
                    end: 3
                }
            ])
        );
    }
}
