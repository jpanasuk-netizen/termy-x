//! Scrollback storage. History is bounded by a row limit and by a byte budget.
//! Each row is charged a canonical size: its cells as though packed with room
//! for the whole row and one style run, and its metadata. That is what a flood
//! stores, but the charge doesn't depend on whether a row is actually stored
//! dense or packed, nor on its buffers' capacities: those depend on when idle
//! compaction ran and on how output was split into feeds, so charging them let
//! a mux host and its replica evict different rows. Now engines fed the same
//! bytes evict the same rows, whenever each compacts.
//!
//! Rows queued for compaction are usually dense, which takes more than their
//! charge: once the charge with that excess passes the budget, the oldest
//! queued rows are packed, which changes no charge. Rows that resist packing
//! (a style change every few cells) take up to four times their charge.
//!
//! The charge follows the width, but a widen evicts nothing the old width
//! kept (see `Grid::resize`), so a transient widen loses no history; later
//! output evicts down to the budget.
use super::{Cell, Grid, Row, Size};

/// The default byte budget: what one million dense cells take (16 MiB).
pub(in crate::terminal_engine) const DEFAULT_HISTORY_BYTES: usize =
    Size::MAX_CELLS * size_of::<Cell>();

/// What a history row is charged: its ring slot and [`Row::charged_bytes`].
/// Counting the slot keeps an unlimited row count bounded by the byte budget
/// alone.
#[inline]
pub(super) fn history_charge(row: &Row) -> usize {
    size_of::<Row>() + row.charged_bytes()
}

/// What a history row really takes: its ring slot and its heap bytes.
#[inline]
pub(super) fn history_heap(row: &Row) -> usize {
    size_of::<Row>() + row.heap_bytes()
}

/// Queued rows packed per step when history crosses its byte budget.
const BUDGET_COMPACTION_CHUNK: usize = 256;

impl Grid {
    pub(super) fn push_history(&mut self, row: Row) {
        // Charged in place: editing the row before the push would make the
        // push wait for that store (and every cell store queued before it).
        self.history.push_back(row);
        let row = self.history.back_mut().expect("pushed history row");
        row.bound_extras();
        self.history_bytes += history_charge(row);
    }

    pub(super) fn pop_history_front(&mut self) -> Option<Row> {
        let row = self.history.pop_front()?;
        self.history_bytes -= history_charge(&row);
        Some(row)
    }

    pub(super) fn pop_history_back(&mut self) -> Option<Row> {
        let row = self.history.pop_back()?;
        self.history_bytes -= history_charge(&row);
        Some(row)
    }

    pub(super) fn clear_history_rows(&mut self) {
        self.history.clear();
        self.history.shrink_to_fit();
        self.history_bytes = 0;
    }

    /// Recount history's bytes, for paths that replace its rows. Each row
    /// keeps the metadata charge it was measured.
    pub(super) fn recount_history_bytes(&mut self) {
        self.history_bytes = self.history.iter().map(history_charge).sum();
    }

    /// History's charge plus what its queued rows take beyond theirs while
    /// dense: an estimate of the bytes it takes that only packing reduces.
    #[inline]
    fn queued_history_bytes(&self) -> usize {
        let dense_excess =
            (self.size.cols * size_of::<Cell>()).saturating_sub(Row::charged_cells(self.size.cols));
        self.pending_compaction
            .min(self.history.len())
            .saturating_mul(dense_excess)
            .saturating_add(self.history_bytes)
    }

    /// Pack the oldest queued rows while history holds more than its byte
    /// budget, then evict the oldest rows until their charge fits it. Cheap
    /// when both fit.
    #[inline]
    pub(super) fn enforce_history_bytes(&mut self) {
        // At least the charge, so this covers both.
        if self.queued_history_bytes() > self.history_byte_limit {
            self.fit_history_bytes();
        }
    }

    /// Packing stops at the first chunk that brings history under budget, so
    /// crossing it costs one chunk rather than packing every queued row.
    #[inline(never)]
    fn fit_history_bytes(&mut self) {
        self.pack_history_to_budget();
        let mut removed = 0;
        while self.history_bytes > self.history_byte_limit && self.pop_history_front().is_some() {
            removed += 1;
        }
        if removed != 0 {
            self.pending_compaction = self.pending_compaction.min(self.history.len());
            self.display_offset = self.display_offset.min(self.history.len());
            self.effect(super::GridEffect::ClearHistory { removed });
        }
    }

    /// Pack the oldest queued rows while history takes more than its budget.
    pub(super) fn pack_history_to_budget(&mut self) {
        while self.queued_history_bytes() > self.history_byte_limit && self.needs_compaction() {
            self.compact_pending_history(BUDGET_COMPACTION_CHUNK);
        }
    }

    /// Pack queued history without switching scrolling rows to immediate
    /// packing, for steps forced while output is still arriving.
    pub(in crate::terminal_engine) fn compact_pending_history(&mut self, limit: usize) {
        self.pending_compaction = self.pending_compaction.min(self.history.len());
        let start = self.history.len() - self.pending_compaction;
        let count = limit.min(self.pending_compaction);
        for row in self.history.iter_mut().skip(start).take(count) {
            // Packing changes the row's storage, never its charge.
            row.compact();
        }
        self.pending_compaction -= count;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Color, GridEffect, tests::grid};
    use super::*;
    use std::{collections::HashSet, sync::Arc};

    fn line(grid: &mut Grid, text: &[u8]) {
        grid.write_ascii(text);
        grid.carriage_return();
        grid.linefeed();
    }

    #[test]
    fn crossing_the_byte_budget_packs_one_chunk_of_the_oldest_queued_rows() {
        // Pen metadata makes dense rows enter history charged a bound, which
        // may cost earlier packing but no history.
        for colored in [false, true] {
            let mut grid = grid(120, 4, 20_000);
            // A status line below the scroll region keeps scrolls off the
            // flood paths: their rows enter history dense.
            grid.set_scroll_region(0, 3);
            if colored {
                grid.pen.set_underline_color(Color::indexed(1));
            }
            let mut lines = 0;
            // The oldest queued rows are packed first.
            while !grid.history.front().is_some_and(Row::is_packed) {
                line(&mut grid, b"a short log line");
                lines += 1;
            }
            // The crossing packed one chunk and evicted nothing; newer queued
            // rows stay dense for the idle steps.
            let packed = grid.history.iter().filter(|row| row.is_packed()).count();
            assert!(packed <= BUDGET_COMPACTION_CHUNK, "{packed}");
            assert!(grid.history.iter().take(packed).all(Row::is_packed));
            assert_eq!(grid.pending_compaction, grid.history.len() - packed);
            // The region's first two lines don't scroll.
            assert_eq!(grid.history_size(), lines - 2);
            assert!(grid.history_bytes <= DEFAULT_HISTORY_BYTES);

            // Later crossings keep packing chunks instead of evicting rows.
            for _ in 0..4000 {
                line(&mut grid, b"a short log line");
                lines += 1;
            }
            assert_eq!(grid.history_size(), lines - 2);
            assert!(grid.history_bytes <= DEFAULT_HISTORY_BYTES);
            let bytes: usize = grid.history.iter().map(history_charge).sum();
            assert_eq!(grid.history_bytes, bytes);
        }
    }

    fn text(row: &Row) -> String {
        let text: String = row.cells().iter().map(|cell| cell.character).collect();
        text.trim_end().to_owned()
    }

    fn numbered(index: usize) -> Vec<u8> {
        format!("line {index} {}", "x".repeat(index * 7 % 110)).into_bytes()
    }

    /// History holds lines `..=newest` in order, packed, and its charge.
    fn assert_packed_history_ends_at(grid: &Grid, newest: usize) {
        for (offset, row) in grid.history.iter().rev().enumerate() {
            assert!(row.is_packed(), "row {offset} from the end");
            let expected = String::from_utf8(numbered(newest - offset)).unwrap();
            assert_eq!(text(row), expected.trim_end(), "row {offset} from the end");
        }
        let bytes: usize = grid.history.iter().map(history_charge).sum();
        assert_eq!(grid.history_bytes, bytes);
        assert_eq!(grid.pending_compaction, 0);
    }

    #[test]
    fn byte_bound_floods_pack_into_the_rows_they_evict() {
        let budget = 256 * 1024;
        let mut grid = grid(120, 4, usize::MAX);
        grid.set_history_limits(usize::MAX, budget);
        for index in 0..5000 {
            line(&mut grid, &numbered(index));
        }
        let rows = grid.history_size();
        assert!((300..4997).contains(&rows), "{rows}");
        assert!(grid.history_bytes <= budget);
        // The screen holds lines 4997 to 4999 above the cursor's blank row.
        assert_packed_history_ends_at(&grid, 4996);

        // Each later line evicts one row, reported as a full history's is.
        grid.track_effects = true;
        let mut effects = Vec::new();
        grid.drain_effects(&mut effects);
        line(&mut grid, &numbered(5000));
        grid.drain_effects(&mut effects);
        assert_eq!(grid.history_size(), rows);
        assert_eq!(
            effects,
            [GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: 4,
                lines: 1,
                retains_history: true,
                history_before: rows,
                history_after: rows,
            }]
        );
        assert_packed_history_ends_at(&grid, 4997);
    }

    #[test]
    fn large_histories_pack_floods_and_keep_the_pen_background() {
        // 20,000 dense rows would take 38 MiB: more than the default budget.
        let mut grid = grid(120, 4, 20_000);
        for index in 0..25_000 {
            line(&mut grid, &numbered(index));
        }
        assert_eq!(grid.history_size(), 20_000);
        assert!(grid.history_bytes <= DEFAULT_HISTORY_BYTES);
        assert_packed_history_ends_at(&grid, 24_996);
        // Erased rows take the pen's background (BCE), in history too.
        let background = Color::indexed(4);
        grid.pen.style.background = background;
        for index in 25_000..25_005 {
            line(&mut grid, &numbered(index));
        }
        grid.flush_pending_clear();
        let bottom = grid.screen().rows.back().unwrap();
        assert!(
            bottom
                .cells()
                .iter()
                .all(|cell| cell.style.background == background)
        );
        // Line 25,000's row was erased before the pen changed; 25,001's after.
        let history = grid.history.len();
        for (row, erased) in [(history - 2, Color::DEFAULT), (history - 1, background)] {
            let row = &grid.history[row];
            assert!(row.is_packed());
            assert_eq!(row.cells()[0].style.background, background);
            assert_eq!(row.cells()[119].style.background, erased);
        }
    }

    #[test]
    fn combining_heavy_history_counts_its_metadata_against_the_budget() {
        let mut grid = grid(120, 4, 20_000);
        let mut mark = 0;
        for _ in 0..3000 {
            for _ in 0..119 {
                grid.put_char('e');
                // Distinct two-mark suffixes defeat the composed-cell cache.
                for shift in [0, 7] {
                    grid.put_char(char::from_u32(0x300 + (mark >> shift) % 112).unwrap());
                }
                mark += 1;
            }
            grid.carriage_return();
            grid.linefeed();
        }
        assert!(grid.history_size() < 3000, "{}", grid.history_size());
        assert!(grid.history_bytes <= DEFAULT_HISTORY_BYTES);
        let bytes: usize = grid.history.iter().map(history_charge).sum();
        assert_eq!(grid.history_bytes, bytes);
        // The accounted bytes cover what history really holds: its cells and
        // every distinct extra (allocation and text) they reference.
        let mut seen = HashSet::new();
        let mut real = 0;
        for row in &grid.history {
            real += row.cells.capacity() * size_of::<Cell>();
            for (_, extra) in &row.extras {
                if seen.insert(Arc::as_ptr(extra)) {
                    real += 2 * size_of::<usize>()
                        + size_of_val(extra.as_ref())
                        + extra.combining.capacity();
                }
            }
        }
        assert!(
            real <= grid.history_bytes,
            "{real} > {}",
            grid.history_bytes
        );
    }
}
