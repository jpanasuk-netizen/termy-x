//! Output floods: a line feed scrolls one full-screen row at a time into
//! history, and the row it scrolls in is blanked lazily, so the next line's
//! text is not stored on top of a fresh blank. Small histories at their row
//! limit trade dense rows with the screen; large or byte-bound ones are
//! packed into the storage of the rows they evict.
use super::{Cell, Color, Grid, GridEffect, Row, Style};

/// Dense history beyond this many bytes no longer stays in cache while a
/// flood recycles it, so floods pack the rows they scroll in instead (see
/// `packs_scrolled_rows`). Measured: recycling dense rows wins below about
/// 12 MiB and loses above.
const DENSE_FLOOD_HISTORY_BYTES: usize = 16 * 1024 * 1024;

/// Cells `start..end` of a live row that must still become plain blanks.
/// An `end` of zero leaves nothing to fill, so it also means none is
/// pending. The grid stores this by value and reads it a field at a time:
/// it is written while the line's cell stores drain, and a wider read would
/// wait for all of them.
#[derive(Clone, Copy)]
pub(super) struct PendingClear {
    pub(super) row: usize,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) background: Color,
}

impl PendingClear {
    pub(super) const NONE: Self = Self {
        row: 0,
        start: 0,
        end: 0,
        background: Color::DEFAULT,
    };
}

/// The blank a full-screen scroll clears with while the pen has the default
/// background. A constant, so reading it never waits on recent stores.
const DEFAULT_BLANK: Cell = Cell {
    style: Style {
        foreground: Color::DEFAULT,
        background: Color::DEFAULT,
        attributes: 0,
        underline: super::super::types::UnderlineStyle::None,
    },
    flags: 0,
    character: ' ',
};

impl Grid {
    /// Fill a row a line feed left partly unblanked. Every grid operation
    /// other than the text writers that complete it requires this first, and
    /// the engine flushes before anything outside parsing can read the grid.
    #[inline]
    pub(in crate::terminal_engine) fn flush_pending_clear(&mut self) {
        if self.pending_clear.end != 0 {
            let pending = self.pending_clear;
            self.pending_clear.end = 0;
            self.fill_pending_clear(pending.row, pending.start, pending.end, pending.background);
        }
    }

    #[inline]
    pub(super) fn has_pending_clear(&self) -> bool {
        self.pending_clear.end != 0
    }

    #[inline(never)]
    fn fill_pending_clear(&mut self, row: usize, start: usize, end: usize, background: Color) {
        let cells = &mut self.screen_mut().rows[row].cells;
        let end = end.min(cells.len());
        if start < end {
            let style = Style {
                background,
                ..Style::default()
            };
            Cell::fill_plain(&mut cells[start..end], ' ', style);
        }
    }

    /// Account for text a writer just stored at `start..end` of `row`, the
    /// cursor row. A pending clear of another row is applied; on this row
    /// the stale cells left of the text are blanked, and those right of it
    /// stay pending, since the line's next run usually covers them.
    #[inline]
    pub(super) fn write_over_pending_clear(&mut self, row: usize, start: usize, end: usize) {
        if self.pending_clear.end == 0 {
            return;
        }
        if self.pending_clear.row != row {
            self.flush_pending_clear();
            return;
        }
        let pending = self.pending_clear;
        if pending.start < start {
            let left = start.min(pending.end);
            self.fill_pending_clear(row, pending.start, left, pending.background);
        }
        let rest = pending.start.max(end);
        if rest < pending.end {
            self.pending_clear.start = rest;
        } else {
            self.pending_clear.end = 0;
        }
    }

    /// LF from output: a row it scrolls in is blanked lazily, so the line
    /// written next isn't stored twice. See `flush_pending_clear`.
    pub(in crate::terminal_engine) fn linefeed_deferring_clear(&mut self) {
        self.flush_pending_clear();
        self.defer_scroll_clear = true;
        self.linefeed();
        self.defer_scroll_clear = false;
    }

    /// `scroll_region_up` of one full-screen line, retaining it in history
    /// at its row limit: the oldest history row is recycled as the new line.
    /// The viewport shows the live screen.
    #[inline]
    pub(super) fn scroll_line_into_full_history(&mut self) {
        self.flush_pending_clear();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let history_before = self.history.len();
        // The top row joins history and the oldest history row becomes the
        // new bottom row: the two trade places, then each ring's first slot
        // becomes its last, which moves no row while the ring is exactly
        // full. Rows moved through the stack, or edited before a move, are
        // read back across narrower stores, which waits for every cell store
        // the line queued; the trade reads two rows no recent line wrote.
        if self.history.capacity() != self.history.len() {
            self.history.shrink_to_fit();
        }
        if self.primary.rows.capacity() != self.primary.rows.len() {
            self.primary.rows.shrink_to_fit();
        }
        let newest = self.history.front_mut().expect("history at its limit");
        let recycled = self.primary.rows.front_mut().expect("screen row");
        // Copied whole, as `swap`'s word-sized steps take twice the stores.
        // SAFETY: `newest` is read out and written back with `recycled`'s
        // row before anything else reads it; nothing in between can panic.
        unsafe {
            let row = std::ptr::read(newest);
            std::ptr::write(newest, std::mem::replace(recycled, row));
        }
        // Charged where they now stay. Both rows are the screen's width, so
        // only their metadata charges differ.
        self.history_bytes -= recycled.metadata_charge();
        newest.bound_extras();
        self.history_bytes += newest.metadata_charge();
        if self.defer_scroll_clear {
            let end = recycled.clear_deferring_fill(cols, &DEFAULT_BLANK);
            self.pending_clear = PendingClear {
                row: rows - 1,
                start: 0,
                end,
                background: Color::DEFAULT,
            };
        } else {
            recycled.clear(cols, &DEFAULT_BLANK);
        }
        // Rotating moves neither row from the slot it was edited in above.
        self.history.rotate_front_to_back();
        self.primary.rows.rotate_front_to_back();
        self.history_activity = true;
        self.scrolled_into_history = self.scrolled_into_history.wrapping_add(1);
        self.pending_compaction = (self.pending_compaction + 1).min(history_before);
        // The caller checked that the viewport shows the live screen.
        self.mark_live_scroll_damage();
        if self.track_effects {
            self.effect(GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: rows,
                lines: 1,
                retains_history: true,
                history_before,
                history_after: self.history.len(),
            });
        }
        self.enforce_history_bytes();
    }

    /// Whether a flood packs the rows it scrolls into history instead of
    /// keeping them dense: while the screen is being packed on scroll
    /// anyway, or when history at its row limit would hold more dense rows
    /// than its byte budget or [`DENSE_FLOOD_HISTORY_BYTES`] allow. Then
    /// dense rows would be packed or evicted under byte pressure regardless,
    /// or recycling them would write a dense row of cold memory per line.
    #[inline]
    fn packs_scrolled_rows(&self) -> bool {
        let dense_history = self.history_limit.saturating_mul(self.dense_row_charge());
        self.compact_on_scroll
            || dense_history > self.history_byte_limit.min(DENSE_FLOOD_HISTORY_BYTES)
    }

    /// What a plain dense row of the current width takes in history.
    #[inline]
    fn dense_row_charge(&self) -> usize {
        size_of::<Row>() + self.size.cols * size_of::<Cell>()
    }

    /// `scroll_region_up` of one full-screen line into history, packed: the
    /// line's text is packed into the storage of the history row it evicts,
    /// at the row limit or when it would not fit the byte budget, and the
    /// screen keeps the line's dense cells as its new bottom row. A flood
    /// then allocates nothing, and each line writes its packed scalars
    /// rather than a dense row of history. The viewport shows the live
    /// screen.
    #[inline(never)]
    fn scroll_line_packing_into_history(&mut self) {
        self.flush_pending_clear();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let history_before = self.history.len();
        // A plain row's charge: a row with metadata may evict more below.
        let charge = size_of::<Row>() + Row::charged_cells(cols);
        let evicts = history_before == self.history_limit
            || self.history_bytes.saturating_add(charge) > self.history_byte_limit;
        let recycled = if evicts {
            let row = self.pop_history_front();
            self.pending_compaction = self.pending_compaction.min(self.history.len());
            row
        } else {
            None
        };
        if evicts {
            // The next lines recycle the rows now oldest: start loading the
            // header of the one after next and the buffers of the next,
            // whose header the previous line loaded.
            if let Some(row) = self.history.get(1) {
                row.prefetch_packed_header();
            }
            if let Some(row) = self.history.get(0) {
                row.prefetch_packed_buffers();
            }
        }
        let blank = if self.pen.style.background == Color::DEFAULT {
            DEFAULT_BLANK
        } else {
            self.blank()
        };
        let top = self.primary.rows.pop_front().expect("screen row");
        // Quiet output packs tightly; a flood packs rows with room for any
        // line, so recycling them (and their byte charge) never changes.
        let (retained, mut live) = top.retain(recycled, cols, &blank, !self.compact_on_scroll);
        // Compaction walks the newest `pending_compaction` rows, so a packed
        // row joins that window only to keep older dense rows inside it.
        if !retained.is_packed() || self.pending_compaction != 0 {
            self.pending_compaction += 1;
        }
        self.push_history(retained);
        let end = if self.defer_scroll_clear {
            live.clear_deferring_fill(cols, &blank)
        } else {
            live.clear(cols, &blank);
            0
        };
        self.primary.rows.push_back(live);
        self.pending_clear = PendingClear {
            row: rows - 1,
            start: 0,
            end,
            background: blank.style.background,
        };
        self.history_activity = true;
        self.scrolled_into_history = self.scrolled_into_history.wrapping_add(1);
        // The caller checked that the viewport shows the live screen.
        self.mark_live_scroll_damage();
        if self.track_effects {
            // Measured before the eviction, so the evicted row is reported
            // as it is for a full history.
            self.effect(GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: rows,
                lines: 1,
                retains_history: true,
                history_before,
                history_after: self.history.len(),
            });
        }
        self.enforce_history_bytes();
    }

    pub(super) fn scroll_region_up(
        &mut self,
        top: usize,
        bottom: usize,
        count: usize,
        retain_history: bool,
    ) {
        // Output floods scroll one full-screen line at a time into history.
        if count == 1
            && top == 0
            && bottom == self.size.rows
            && retain_history
            && self.history_limit != 0
            && self.display_offset == 0
            && self.screen().rows.len() == self.size.rows
        {
            if self.packs_scrolled_rows() {
                self.scroll_line_packing_into_history();
                return;
            }
            if self.history.len() == self.history_limit
                && self.pen.style.background == Color::DEFAULT
            {
                self.scroll_line_into_full_history();
                return;
            }
        }
        self.scroll_region_up_general(top, bottom, count, retain_history);
    }

    #[inline(never)]
    fn scroll_region_up_general(
        &mut self,
        top: usize,
        bottom: usize,
        count: usize,
        retain_history: bool,
    ) {
        let blank = self.blank();
        let cols = self.size.cols;
        let count = count.min(bottom - top);
        let history_before = self.history_size();
        self.flush_pending_clear();
        // A full-screen region rotates the ring instead of shifting rows.
        // Another region moves its first `count` rows to its end at once,
        // where each is then replaced in place.
        let whole_screen = top == 0 && bottom == self.screen().rows.len();
        let defer_clear = self.defer_scroll_clear && count == 1 && whole_screen;
        if !whole_screen {
            self.screen_mut().rows.rotate_range_left(top..bottom, count);
        }
        for index in bottom - count..bottom {
            let rows = &mut self.screen_mut().rows;
            let removed = if whole_screen {
                rows.pop_front().expect("scroll region row")
            } else {
                std::mem::replace(&mut rows[index], Row::new(0, &blank))
            };
            let mut recycled = if retain_history && self.history_limit != 0 {
                let recycled = if self.history.len() == self.history_limit {
                    self.pop_history_front()
                } else {
                    None
                };
                let (retained, recycled) = if self.compact_on_scroll {
                    removed.retain(recycled, cols, &blank, false)
                } else {
                    (removed, recycled.unwrap_or_else(|| Row::new(cols, &blank)))
                };
                self.push_history(retained);
                self.history_activity = true;
                self.scrolled_into_history = self.scrolled_into_history.wrapping_add(1);
                self.pending_compaction = (self.pending_compaction + 1).min(self.history.len());
                if self.display_offset != 0 {
                    self.display_offset = (self.display_offset + 1).min(self.history.len());
                }
                recycled
            } else {
                removed
            };
            let deferred = if defer_clear {
                Some(recycled.clear_deferring_fill(cols, &blank))
            } else {
                recycled.clear(cols, &blank);
                None
            };
            let rows = &mut self.screen_mut().rows;
            if whole_screen {
                rows.push_back(recycled);
            } else {
                rows[index] = recycled;
            }
            if let Some(end) = deferred {
                self.pending_clear = PendingClear {
                    row: bottom - 1,
                    start: 0,
                    end,
                    background: blank.style.background,
                };
            }
        }
        if count != 0 {
            self.mark_scroll_damage(top, bottom, count as i32);
            self.effect(GridEffect::Scroll {
                alternate: self.alternate_active,
                top,
                bottom,
                lines: count as i64,
                retains_history: retain_history && self.history_limit != 0,
                history_before,
                history_after: self.history_size(),
            });
            self.enforce_history_bytes();
        }
    }
}
