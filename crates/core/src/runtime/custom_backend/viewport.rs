//! Viewport cell reads. Locked visitors stream borrowed cells; the others
//! collect first so their callbacks may call back into the terminal.
use super::*;
use crate::runtime::{DamageCellVisitor, TerminalDamageCells};

impl CustomBackend {
    pub(in crate::runtime) fn visit_viewport_cells(
        &self,
        mut visitor: impl FnMut(usize, i32, usize, &TerminalRenderCell),
    ) -> TerminalViewportMetadata {
        // Preserve reentrant callbacks at the public snapshot boundary. The
        // desktop renderer uses the explicit locked variant below instead.
        let mut cells = Vec::new();
        let metadata = self.visit_viewport_cells_locked(|_, _, _, cell| cells.push(cell.clone()));
        let cols = usize::from(metadata.cols);
        for (index, cell) in cells.iter().enumerate() {
            visitor(
                metadata.display_offset,
                (index / cols) as i32 - metadata.display_offset as i32,
                index % cols,
                cell,
            );
        }
        metadata
    }
    pub(in crate::runtime) fn visit_viewport_cells_locked(
        &self,
        mut visitor: impl FnMut(usize, i32, usize, &TerminalRenderCell),
    ) -> TerminalViewportMetadata {
        let state = self.shared.state();
        let offset = state.engine.display_offset();
        let mut scratch = Vec::new();
        for row in 0..usize::from(state.size.rows) {
            let wrapped = state.engine.viewport_row_wrapped(row);
            state.engine.with_viewport_row(row, &mut scratch, |line| {
                for col in 0..line.len() {
                    visitor(
                        offset,
                        row as i32 - offset as i32,
                        col,
                        &render_cell(&line, col, wrapped && col + 1 == line.len()),
                    );
                }
            });
        }
        state.metadata()
    }
    pub(in crate::runtime) fn visit_viewport_ranges_at_generation(
        &self,
        generation: u64,
        spans: &[TerminalDirtySpan],
        mut visitor: impl FnMut(usize, usize, i32, usize, &TerminalRenderCell),
    ) -> bool {
        let mut cells = Vec::new();
        if !self.visit_viewport_ranges_locked_at_generation(
            generation,
            spans,
            |row, offset, line, col, cell| cells.push((row, offset, line, col, cell.clone())),
        ) {
            return false;
        }
        for (row, offset, line, col, cell) in cells {
            visitor(row, offset, line, col, &cell);
        }
        true
    }
    pub(in crate::runtime) fn visit_viewport_ranges_locked_at_generation(
        &self,
        generation: u64,
        spans: &[TerminalDirtySpan],
        mut visitor: impl FnMut(usize, usize, i32, usize, &TerminalRenderCell),
    ) -> bool {
        let state = self.shared.state();
        if state.generation != generation {
            return false;
        }
        state.visit_ranges(spans, &mut visitor);
        true
    }
    pub(in crate::runtime) fn with_render_damage<R>(
        &self,
        read: impl FnOnce(TerminalRenderDamageSnapshot, TerminalDamageCells<'_>) -> R,
    ) -> R {
        let mut state = self.shared.state();
        let damage = state.take_damage(false);
        let state = &*state;
        let line_bounds = (
            -(state.engine.history_size() as i32),
            i32::from(state.size.rows) - 1,
        );
        let size = state.engine.size();
        // Same contract as the remote backend: reject rather than clip, so a
        // caller with stale spans falls back to a full read.
        let mut visit = |spans: &[TerminalDirtySpan], visitor: &mut DamageCellVisitor<'_>| {
            let in_viewport = spans.iter().all(|span| {
                span.row < size.rows
                    && span.left_col <= span.right_col
                    && span.right_col < size.cols
            });
            if in_viewport {
                state.visit_ranges(spans, visitor);
            }
            in_viewport
        };
        read(
            damage,
            TerminalDamageCells::new(state.engine.display_offset(), line_bounds, &mut visit),
        )
    }
    pub(in crate::runtime) fn line_bounds(&self) -> (i32, i32) {
        let state = self.shared.state();
        (
            -(state.engine.history_size() as i32),
            i32::from(state.size.rows) - 1,
        )
    }
    pub(in crate::runtime) fn visit_line_cells(
        &self,
        requested_first: i32,
        requested_last: i32,
        mut visitor: impl FnMut((i32, i32, usize), i32, usize, &TerminalRenderCell),
    ) -> (i32, i32, usize) {
        let state = self.shared.state();
        let range = (
            -(state.engine.history_size() as i32),
            i32::from(state.size.rows) - 1,
            usize::from(state.size.cols),
        );
        let mut scratch = Vec::new();
        for line in requested_first.max(range.0)..=requested_last.min(range.1) {
            let wrapped = state.engine.line_wrapped(line);
            state.engine.with_line(line, &mut scratch, |cells| {
                for col in 0..cells.len() {
                    visitor(
                        range,
                        line,
                        col,
                        &render_cell(&cells, col, wrapped && col + 1 == cells.len()),
                    );
                }
            });
        }
        range
    }
}

impl State {
    fn visit_ranges(&self, spans: &[TerminalDirtySpan], visitor: &mut DamageCellVisitor<'_>) {
        let offset = self.engine.display_offset();
        let mut scratch = Vec::new();
        for span in spans {
            let wrapped = self.engine.viewport_row_wrapped(span.row);
            self.engine
                .with_viewport_row(span.row, &mut scratch, |line| {
                    let end = span.right_col.saturating_add(1).min(line.len());
                    for col in span.left_col.min(end)..end {
                        visitor(
                            span.row,
                            offset,
                            span.row as i32 - offset as i32,
                            col,
                            &render_cell(&line, col, wrapped && col + 1 == line.len()),
                        );
                    }
                });
        }
    }
}
