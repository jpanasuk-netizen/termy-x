//! A row's cell metadata: `(column, metadata)` pairs sorted by column, beside
//! its cells. Few cells have any, so plain rows keep an empty table and every
//! operation on one returns at once; edits that move or replace cells move or
//! drop their metadata with them.
use super::super::types::{CellExtra, Line};
use super::{Cell, Row};
use std::{ops::Range, sync::Arc};

impl Row {
    #[inline]
    pub(super) fn has_extras(&self) -> bool {
        !self.extras.is_empty()
    }

    /// The row's cells (decoded, if packed) with their metadata.
    pub(in crate::terminal_engine) fn line(&self) -> Line<'_> {
        Line::new(self.cells(), &self.extras)
    }

    pub(in crate::terminal_engine) fn extra(&self, col: usize) -> Option<&Arc<CellExtra>> {
        let index = self.extra_index(col).ok()?;
        Some(&self.extras[index].1)
    }

    /// Replace the metadata of the cell at `col`.
    #[inline]
    pub(super) fn set_extra(&mut self, col: usize, extra: Option<Arc<CellExtra>>) {
        if self.follows_extras(col) {
            if let Some(extra) = extra {
                self.extras.push((col, extra));
            }
            return;
        }
        self.replace_extra(col, extra);
    }

    #[inline(never)]
    fn replace_extra(&mut self, col: usize, extra: Option<Arc<CellExtra>>) {
        match (self.extra_index(col), extra) {
            (Ok(index), Some(extra)) => self.extras[index].1 = extra,
            (Ok(index), None) => {
                self.remove_extra(index);
            }
            (Err(index), Some(extra)) => self.extras.insert(index, (col, extra)),
            (Err(_), None) => {}
        }
    }

    /// Remove the metadata of the cell at `col`, returning it.
    #[inline]
    pub(super) fn take_extra(&mut self, col: usize) -> Option<Arc<CellExtra>> {
        if self.follows_extras(col) {
            return None;
        }
        let index = self.extra_index(col).ok()?;
        Some(self.remove_extra(index))
    }

    /// Whether `col` is past every cell with metadata. Output moves left to
    /// right, so most edits land there or on the last entry.
    #[inline]
    pub(super) fn follows_extras(&self, col: usize) -> bool {
        self.extras.last().is_none_or(|&(last, _)| last < col)
    }

    /// Where the metadata of the cell at `col` is, or would be inserted.
    #[inline]
    fn extra_index(&self, col: usize) -> Result<usize, usize> {
        match self.extras.last() {
            None => Err(0),
            Some(&(last, _)) if last < col => Err(self.extras.len()),
            Some(&(last, _)) if last == col => Ok(self.extras.len() - 1),
            Some(_) => self.extras.binary_search_by_key(&col, |(col, _)| *col),
        }
    }

    #[inline]
    fn remove_extra(&mut self, index: usize) -> Arc<CellExtra> {
        if index + 1 == self.extras.len() {
            let (_, extra) = self.extras.pop().expect("an entry at index");
            return extra;
        }
        self.extras.remove(index).1
    }

    /// Store `cell` with `extra` at `col`.
    #[inline]
    pub(super) fn put(&mut self, col: usize, cell: Cell, extra: Option<Arc<CellExtra>>) {
        self.cells[col] = cell;
        if extra.is_some() || self.has_extras() {
            self.set_extra(col, extra);
        }
    }

    /// Store `blank`, which has no metadata, at `col`.
    #[inline]
    pub(super) fn blank_cell(&mut self, col: usize, blank: Cell) {
        self.cells[col] = blank;
        if self.has_extras() {
            self.set_extra(col, None);
        }
    }

    /// Drop the metadata of the cells in `range`.
    #[inline]
    pub(super) fn clear_extras(&mut self, range: Range<usize>) {
        if !self.follows_extras(range.start) {
            self.drop_extras(range);
        }
    }

    #[inline(never)]
    fn drop_extras(&mut self, range: Range<usize>) {
        let (from, to) = self.extras_in(range);
        if to == self.extras.len() {
            self.extras.truncate(from);
        } else {
            self.extras.drain(from..to);
        }
    }

    /// Give every cell in `range` `extra`.
    pub(super) fn fill_extras(&mut self, range: Range<usize>, extra: &Arc<CellExtra>) {
        self.clear_extras(range.clone());
        let at = self.extras.partition_point(|&(col, _)| col < range.start);
        let fill = range.map(|col| (col, Arc::clone(extra)));
        if at == self.extras.len() {
            self.extras.extend(fill);
        } else {
            self.extras.splice(at..at, fill);
        }
    }

    /// Move the metadata of the cells in `range` as rotating those cells left
    /// by `mid` moves the cells.
    pub(super) fn rotate_extras_left(&mut self, range: Range<usize>, mid: usize) {
        if !self.has_extras() || range.is_empty() {
            return;
        }
        let (from, to) = self.extras_in(range.clone());
        let len = range.len();
        for (col, _) in &mut self.extras[from..to] {
            *col = range.start + (*col - range.start + len - mid % len) % len;
        }
        self.extras[from..to].sort_unstable_by_key(|(col, _)| *col);
    }

    /// Move metadata as rotating the cells of `range` right by `count` does.
    pub(super) fn rotate_extras_right(&mut self, range: Range<usize>, count: usize) {
        let len = range.len();
        if len != 0 {
            self.rotate_extras_left(range, len - count % len);
        }
    }

    /// Exchange the metadata of the cells in `range` with another row's
    /// table, as the cells themselves are exchanged.
    pub(super) fn swap_extras(
        &mut self,
        other: &mut Vec<(usize, Arc<CellExtra>)>,
        range: Range<usize>,
    ) {
        if !self.has_extras() && other.is_empty() {
            return;
        }
        let (from, to) = extras_in(&self.extras, range.clone());
        let mine: Vec<_> = self.extras.drain(from..to).collect();
        let (other_from, other_to) = extras_in(other, range);
        let theirs: Vec<_> = other.drain(other_from..other_to).collect();
        self.extras.splice(from..from, theirs);
        other.splice(other_from..other_from, mine);
    }

    /// The table indices of the metadata of the cells in `range`.
    fn extras_in(&self, range: Range<usize>) -> (usize, usize) {
        extras_in(&self.extras, range)
    }
}

fn extras_in(extras: &[(usize, Arc<CellExtra>)], range: Range<usize>) -> (usize, usize) {
    let from = extras.partition_point(|(col, _)| *col < range.start);
    let to = from + extras[from..].partition_point(|(col, _)| *col < range.end);
    (from, to)
}

#[cfg(test)]
mod tests {
    use super::super::super::types::Color;
    use super::*;

    fn extra(color: u8) -> Arc<CellExtra> {
        Arc::new(CellExtra {
            underline_color: Color::indexed(color),
            ..CellExtra::default()
        })
    }

    fn colors(row: &Row) -> Vec<(usize, u8)> {
        row.extras
            .iter()
            .map(|(col, extra)| (*col, extra.underline_color.as_indexed().unwrap()))
            .collect()
    }

    // Moving metadata must match moving the cells it belongs to, which tag
    // their column in their character here.
    #[test]
    fn metadata_moves_with_its_cells() {
        for (len, count) in [(8, 3), (8, 0), (8, 8), (5, 2), (1, 1)] {
            for left in [false, true] {
                let mut row = Row::new(10, &Cell::default());
                for col in 0..10 {
                    row.cells[col].character = char::from(b'a' + col as u8);
                    if col % 3 != 1 {
                        row.set_extra(col, Some(extra(col as u8)));
                    }
                }
                let range = 1..1 + len;
                if left {
                    row.cells[range.clone()].rotate_left(count % len.max(1));
                    row.rotate_extras_left(range, count);
                } else {
                    row.cells[range.clone()].rotate_right(count % len.max(1));
                    row.rotate_extras_right(range, count);
                }
                assert!(row.extras.is_sorted_by_key(|(col, _)| *col));
                for (col, color) in colors(&row) {
                    let source = (row.cells[col].character as u8 - b'a') as usize;
                    assert_eq!(color as usize, source, "{len} {count} {left}");
                }
                assert_eq!(colors(&row).len(), 7);
            }
        }
    }

    #[test]
    fn edits_keep_the_table_sorted_and_scoped_to_their_columns() {
        let mut row = Row::new(10, &Cell::default());
        row.set_extra(5, Some(extra(5)));
        row.set_extra(2, Some(extra(2)));
        row.fill_extras(6..9, &extra(6));
        assert_eq!(colors(&row), [(2, 2), (5, 5), (6, 6), (7, 6), (8, 6)]);
        row.clear_extras(5..7);
        assert_eq!(colors(&row), [(2, 2), (7, 6), (8, 6)]);
        assert_eq!(
            row.take_extra(7).unwrap().underline_color,
            Color::indexed(6)
        );
        row.blank_cell(2, Cell::default());
        row.put(3, Cell::default(), Some(extra(3)));
        assert_eq!(colors(&row), [(3, 3), (8, 6)]);
        let mut other = Row::new(10, &Cell::default());
        other.set_extra(1, Some(extra(1)));
        other.set_extra(4, Some(extra(4)));
        row.swap_extras(&mut other.extras, 2..5);
        assert_eq!(colors(&row), [(4, 4), (8, 6)]);
        assert_eq!(colors(&other), [(1, 1), (3, 3)]);
    }
}
