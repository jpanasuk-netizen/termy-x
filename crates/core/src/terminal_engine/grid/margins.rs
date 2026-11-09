//! DECLRMM (`?69`) and DECSLRM left/right margins. While the mode is off the
//! margins span the screen, so the common paths only compare against them.

use super::{Cell, Grid};

impl Grid {
    pub(in crate::terminal_engine) fn lr_margin_mode(&self) -> bool {
        self.lr_margin_mode
    }

    /// Like xterm, resetting DECLRMM also resets the margins.
    pub(in crate::terminal_engine) fn set_lr_margin_mode(&mut self, enabled: bool) {
        self.lr_margin_mode = enabled;
        if !enabled {
            self.reset_lr_margins();
            // A wrap pending at the old right margin no longer applies.
            self.pending_wrap &= self.cursor.col + 1 == self.size.cols;
        }
    }

    pub(in crate::terminal_engine) fn reset_lr_margins(&mut self) {
        self.left_margin = 0;
        self.right_margin = self.size.cols;
    }

    /// The left margin and the exclusive right margin.
    pub(in crate::terminal_engine) fn lr_margins(&self) -> (usize, usize) {
        (self.left_margin, self.right_margin)
    }

    /// DECSLRM, with a 0-based left and an exclusive right margin. Like
    /// DECSTBM it needs two columns and homes the cursor.
    pub(in crate::terminal_engine) fn set_lr_margins(&mut self, left: usize, right: usize) {
        let right = right.min(self.size.cols);
        if left >= right || right - left < 2 {
            return;
        }
        self.left_margin = left;
        self.right_margin = right;
        self.goto(0, 0);
    }

    #[inline(always)]
    pub(super) fn margins_span_screen(&self) -> bool {
        self.left_margin == 0 && self.right_margin == self.size.cols
    }

    /// Where printing wraps. Like xterm, a cursor right of the right margin
    /// wraps at the screen edge instead.
    #[inline(always)]
    pub(super) fn right_limit(&self) -> usize {
        if self.cursor.col < self.right_margin {
            self.right_margin
        } else {
            self.size.cols
        }
    }

    /// Where CR and left motion stop. A cursor left of the left margin can
    /// still reach column 0.
    #[inline(always)]
    pub(super) fn left_limit(&self) -> usize {
        if self.cursor.col >= self.left_margin {
            self.left_margin
        } else {
            0
        }
    }

    #[inline(always)]
    pub(super) fn in_lr_margins(&self) -> bool {
        self.cursor.col >= self.left_margin && self.cursor.col < self.right_margin
    }

    /// Scroll the margin columns of rows `top..bottom` up (or down) by
    /// `count`. Unlike full-width scrolling nothing enters history, and
    /// kitty placements stay where they are.
    pub(super) fn scroll_rect(&mut self, top: usize, bottom: usize, count: usize, up: bool) {
        let count = count.min(bottom - top);
        if count == 0 {
            return;
        }
        let (left, right) = (self.left_margin, self.right_margin);
        let cols = self.size.cols;
        let blank = self.blank();
        let rows = &mut self.screen_mut().rows;
        // A wide pair straddling a margin can't move with only one half.
        for row in rows.range_mut(top..bottom) {
            if left > 0 && row.cells[left].flags & Cell::WIDE_SPACER != 0 {
                Self::clear_wide_at(row, left, &blank);
            }
            if right < cols && row.cells[right - 1].flags & Cell::WIDE != 0 {
                Self::clear_wide_at(row, right - 1, &blank);
            }
        }
        // Each step swaps the destination's margin cells with its source's.
        // The source is visited later as a destination or blanked below.
        let moves = (bottom - top) - count;
        for step in 0..moves {
            let (destination, source) = if up {
                (top + step, top + step + count)
            } else {
                (bottom - 1 - step, bottom - 1 - step - count)
            };
            let mut source_cells = std::mem::take(&mut rows[source].cells);
            let mut source_extras = std::mem::take(&mut rows[source].extras);
            rows[destination].cells[left..right].swap_with_slice(&mut source_cells[left..right]);
            rows[destination].swap_extras(&mut source_extras, left..right);
            rows[source].cells = source_cells;
            rows[source].extras = source_extras;
        }
        let exposed = if up {
            bottom - count..bottom
        } else {
            top..top + count
        };
        for row in exposed {
            rows[row].cells[left..right].fill(blank);
            rows[row].clear_extras(left..right);
        }
        for row in rows.range_mut(top..bottom) {
            row.occupied = row.occupied.max(right);
            if right == cols {
                // A leading spacer only ends a wrapped row.
                row.wrapped = false;
                Self::repair_wide_at(row, cols - 1, &blank);
            }
        }
        for row in top..bottom {
            self.mark(row, left.saturating_sub(1), (right + 1).min(cols));
        }
    }
}
