//! Caret, selection, and hit testing for Termy X draft fields.
//!
//! Compose, reply, quote, and the single-line ask/search boxes share this
//! model. Boundaries are extended grapheme clusters so a ZWJ family, a flag,
//! a skin-tone emoji, or a combining mark moves and deletes as one unit.
//! Visual lines are supplied by the caller (soft-wrap ranges); this module
//! does not know about pixels except the hit-test geometry the UI records
//! after shaping.

use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftCaret {
    pub caret: usize,
    pub anchor: Option<usize>,
    /// Grapheme column to restore when moving vertically across a shorter line.
    pub preferred_col: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    /// Start of the current visual line.
    Home,
    /// End of the current visual line.
    End,
    WordLeft,
    WordRight,
    DocStart,
    DocEnd,
}

/// One shaped visual line in window coordinates.
/// `stops` are `(utf-8 byte, absolute x)` grapheme boundaries, including
/// the line start. `top`/`bottom` are the line's painted box.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintedLine {
    pub top: f32,
    pub bottom: f32,
    pub stops: Vec<(usize, f32)>,
}

impl DraftCaret {
    pub fn at_end(len: usize) -> Self {
        Self {
            caret: len,
            anchor: None,
            preferred_col: None,
        }
    }

    pub fn collapsed(at: usize) -> Self {
        Self {
            caret: at,
            anchor: None,
            preferred_col: None,
        }
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        let a = anchor.min(self.caret);
        let b = anchor.max(self.caret);
        if a == b { None } else { Some((a, b)) }
    }

    pub fn selected_text<'a>(&self, text: &'a str) -> Option<&'a str> {
        let (a, b) = self.selection()?;
        let b = b.min(text.len());
        let a = a.min(b);
        text.get(a..b)
    }

    pub fn clamp(&mut self, text: &str) {
        self.caret = floor_grapheme(text, self.caret);
        if let Some(anchor) = self.anchor {
            let anchor = floor_grapheme(text, anchor);
            self.anchor = Some(anchor);
            if anchor == self.caret {
                self.anchor = None;
            }
        }
    }

    pub fn select_all(&mut self, text: &str) {
        self.anchor = if text.is_empty() { None } else { Some(0) };
        self.caret = text.len();
        self.preferred_col = None;
    }

    pub fn move_by(&mut self, text: &str, lines: &[(usize, usize)], motion: Motion, extend: bool) {
        self.clamp(text);
        let lines = visual_lines(text, lines);
        let pos = self.caret;
        match motion {
            Motion::Left if !extend => {
                if let Some((start, _)) = self.selection() {
                    self.anchor = None;
                    self.caret = start;
                    self.preferred_col = None;
                    return;
                }
                self.caret = prev_grapheme(text, pos);
                self.preferred_col = None;
            }
            Motion::Right if !extend => {
                if let Some((_, end)) = self.selection() {
                    self.anchor = None;
                    self.caret = end;
                    self.preferred_col = None;
                    return;
                }
                self.caret = next_grapheme(text, pos);
                self.preferred_col = None;
            }
            Motion::Left => {
                self.place(text, prev_grapheme(text, pos), true);
                self.preferred_col = None;
            }
            Motion::Right => {
                self.place(text, next_grapheme(text, pos), true);
                self.preferred_col = None;
            }
            Motion::WordLeft => {
                self.place(text, word_before(text, pos), extend);
                self.preferred_col = None;
            }
            Motion::WordRight => {
                self.place(text, word_after(text, pos), extend);
                self.preferred_col = None;
            }
            Motion::DocStart => {
                self.place(text, 0, extend);
                self.preferred_col = Some(0);
            }
            Motion::DocEnd => {
                self.place(text, text.len(), extend);
                self.preferred_col = None;
            }
            Motion::Home => {
                let idx = line_index(&lines, pos);
                self.place(text, lines[idx].0, extend);
                self.preferred_col = Some(0);
            }
            Motion::End => {
                let idx = line_index(&lines, pos);
                let end = lines[idx].1;
                self.place(text, end, extend);
                self.preferred_col = Some(grapheme_col(text, lines[idx].0, end, end));
            }
            Motion::Up | Motion::Down => {
                let idx = line_index(&lines, pos);
                let (start, end) = lines[idx];
                if self.preferred_col.is_none() {
                    self.preferred_col = Some(grapheme_col(text, start, end, pos));
                }
                let col = self.preferred_col.unwrap_or(0);
                let next_idx = if motion == Motion::Up {
                    if idx == 0 {
                        self.place(text, pos, extend);
                        return;
                    }
                    idx - 1
                } else if idx + 1 >= lines.len() {
                    self.place(text, pos, extend);
                    return;
                } else {
                    idx + 1
                };
                let (ns, ne) = lines[next_idx];
                let at = offset_at_col(text, ns, ne, col);
                self.place(text, at, extend);
            }
        }
    }

    pub fn place(&mut self, text: &str, index: usize, extend: bool) {
        let index = floor_grapheme(text, index);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
            self.caret = index;
            if self.anchor == Some(self.caret) {
                self.anchor = None;
            }
        } else {
            self.anchor = None;
            self.caret = index;
        }
    }

    /// Plain click-drag origin: a collapsed caret that later drags extend.
    pub fn drag_anchor(&mut self, text: &str, index: usize) {
        let index = floor_grapheme(text, index);
        self.anchor = Some(index);
        self.caret = index;
        self.preferred_col = None;
    }

    pub fn drag_to(&mut self, text: &str, index: usize) {
        let index = floor_grapheme(text, index);
        if self.anchor.is_none() {
            self.anchor = Some(self.caret);
        }
        self.caret = index;
        if self.anchor == Some(self.caret) {
            self.anchor = None;
        }
        self.preferred_col = None;
    }

    pub fn select_word_at(&mut self, text: &str, index: usize) {
        let (start, end) = word_span(text, index);
        if start == end {
            self.anchor = None;
            self.caret = start;
        } else {
            self.anchor = Some(start);
            self.caret = end;
        }
        self.preferred_col = None;
    }

    pub fn select_line_at(&mut self, text: &str, lines: &[(usize, usize)], index: usize) {
        let lines = visual_lines(text, lines);
        let index = floor_grapheme(text, index.min(text.len()));
        let idx = line_index(&lines, index);
        let (start, end) = lines[idx];
        if start == end {
            self.anchor = None;
            self.caret = start;
        } else {
            self.anchor = Some(start);
            self.caret = end;
        }
        self.preferred_col = None;
    }

    pub fn insert(&mut self, text: &mut String, insert: &str) {
        self.clamp(text);
        if let Some((start, end)) = self.selection() {
            text.replace_range(start..end, insert);
            self.caret = start + insert.len();
        } else {
            let at = self.caret.min(text.len());
            text.insert_str(at, insert);
            self.caret = at + insert.len();
        }
        self.anchor = None;
        self.preferred_col = None;
        self.caret = floor_grapheme(text, self.caret);
    }

    pub fn backspace(&mut self, text: &mut String) {
        self.clamp(text);
        if let Some((start, end)) = self.selection() {
            text.replace_range(start..end, "");
            self.caret = start;
            self.anchor = None;
            self.preferred_col = None;
            return;
        }
        if self.caret == 0 {
            return;
        }
        let prev = prev_grapheme(text, self.caret);
        text.replace_range(prev..self.caret, "");
        self.caret = prev;
        self.preferred_col = None;
    }

    pub fn delete_forward(&mut self, text: &mut String) {
        self.clamp(text);
        if let Some((start, end)) = self.selection() {
            text.replace_range(start..end, "");
            self.caret = start;
            self.anchor = None;
            self.preferred_col = None;
            return;
        }
        if self.caret >= text.len() {
            return;
        }
        let next = next_grapheme(text, self.caret);
        text.replace_range(self.caret..next, "");
        self.preferred_col = None;
    }

    /// Ctrl+Backspace. At the end of `"rust build in public  "` this leaves
    /// `"rust build in "`, matching the previous end-caret word delete.
    pub fn delete_word_backward(&mut self, text: &mut String) {
        self.clamp(text);
        if let Some((start, end)) = self.selection() {
            text.replace_range(start..end, "");
            self.caret = start;
            self.anchor = None;
            self.preferred_col = None;
            return;
        }
        let end = self.caret;
        let start = word_before(text, end);
        if start == end {
            return;
        }
        text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = None;
        self.preferred_col = None;
    }
}

/// Arrow / Home / End keys the focused field must swallow so the terminal
/// behind the panel never sees them.
pub fn is_navigation_key(key: &str) -> bool {
    matches!(key, "left" | "right" | "up" | "down" | "home" | "end")
}

pub fn floor_grapheme(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let mut at = 0usize;
    for (start, grapheme) in text.grapheme_indices(true) {
        if start >= offset {
            break;
        }
        let end = start + grapheme.len();
        if offset < end {
            return start;
        }
        at = end;
    }
    at.min(text.len())
}

pub fn prev_grapheme(text: &str, offset: usize) -> usize {
    let offset = floor_grapheme(text, offset);
    if offset == 0 {
        return 0;
    }
    let mut prev = 0usize;
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if end >= offset {
            return start;
        }
        prev = start;
    }
    prev
}

/// `(start, end)` byte ranges of each extended grapheme in `text`.
pub fn grapheme_ranges(text: &str) -> Vec<(usize, usize)> {
    text.grapheme_indices(true)
        .map(|(start, grapheme)| (start, start + grapheme.len()))
        .collect()
}

pub fn next_grapheme(text: &str, offset: usize) -> usize {
    let offset = floor_grapheme(text, offset);
    if offset >= text.len() {
        return text.len();
    }
    for (start, grapheme) in text.grapheme_indices(true) {
        if start >= offset {
            return start + grapheme.len();
        }
    }
    text.len()
}

/// Which caller-supplied visual line contains `caret`. A caret sitting on a
/// skipped wrap-space belongs to the following line. Matches the panel's
/// historical `caret_visual_line`.
pub fn line_index(lines: &[(usize, usize)], caret: usize) -> usize {
    if lines.is_empty() {
        return 0;
    }
    for (idx, &(_start, end)) in lines.iter().enumerate() {
        if caret <= end {
            return idx;
        }
        if caret > end && lines.get(idx + 1).is_some_and(|&(ns, _)| caret < ns) {
            return idx + 1;
        }
    }
    lines.len() - 1
}

pub fn grapheme_col(text: &str, line_start: usize, line_end: usize, caret: usize) -> usize {
    let end = line_end.min(text.len()).max(line_start.min(text.len()));
    let start = line_start.min(end);
    let caret = caret.clamp(start, end);
    text.get(start..caret)
        .map_or(0, |slice| slice.graphemes(true).count())
}

pub fn offset_at_col(text: &str, line_start: usize, line_end: usize, col: usize) -> usize {
    let end = line_end.min(text.len()).max(line_start.min(text.len()));
    let start = line_start.min(end);
    let mut seen = 0usize;
    for (index, grapheme) in text[start..end].grapheme_indices(true) {
        if seen == col {
            return start + index;
        }
        seen += 1;
        if seen == col {
            return start + index + grapheme.len();
        }
    }
    end
}

fn visual_lines(text: &str, lines: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if lines.is_empty() {
        vec![(0, text.len())]
    } else {
        lines.to_vec()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class_of(grapheme: &str) -> Class {
    let Some(ch) = grapheme.chars().next() else {
        return Class::Other;
    };
    if ch.is_whitespace() {
        Class::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        Class::Word
    } else {
        Class::Other
    }
}

fn graphemes_before(text: &str, caret: usize) -> Vec<(usize, Class)> {
    text[..caret.min(text.len())]
        .grapheme_indices(true)
        .map(|(index, grapheme)| (index, class_of(grapheme)))
        .collect()
}

fn word_before(text: &str, caret: usize) -> usize {
    let caret = floor_grapheme(text, caret);
    let gs = graphemes_before(text, caret);
    if gs.is_empty() {
        return 0;
    }
    let mut i = gs.len();
    while i > 0 && gs[i - 1].1 == Class::Space {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    let class = gs[i - 1].1;
    while i > 0 && gs[i - 1].1 == class {
        i -= 1;
    }
    gs[i].0
}

fn word_after(text: &str, caret: usize) -> usize {
    let caret = floor_grapheme(text, caret);
    let rest: Vec<(usize, Class)> = text[caret..]
        .grapheme_indices(true)
        .map(|(index, grapheme)| (caret + index, class_of(grapheme)))
        .collect();
    if rest.is_empty() {
        return text.len();
    }
    let mut i = 0usize;
    while i < rest.len() && rest[i].1 == Class::Space {
        i += 1;
    }
    if i >= rest.len() {
        return text.len();
    }
    let class = rest[i].1;
    while i < rest.len() && rest[i].1 == class {
        i += 1;
    }
    rest.get(i).map_or(text.len(), |(index, _)| *index)
}

fn word_span(text: &str, index: usize) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }
    let mut index = floor_grapheme(text, index.min(text.len()));
    if index == text.len() && index > 0 {
        index = prev_grapheme(text, index);
    }
    let gs: Vec<(usize, usize, Class)> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| (start, start + grapheme.len(), class_of(grapheme)))
        .collect();
    let Some(pos) = gs.iter().position(|(start, _, _)| *start == index) else {
        return (index, index);
    };
    let class = gs[pos].2;
    let mut lo = pos;
    while lo > 0 && gs[lo - 1].2 == class {
        lo -= 1;
    }
    let mut hi = pos + 1;
    while hi < gs.len() && gs[hi].2 == class {
        hi += 1;
    }
    (gs[lo].0, gs[hi - 1].1)
}

/// Nearest grapheme boundary to `x`. Halfway between two stops belongs to
/// the right-hand boundary (the caret sits after that character).
pub fn closest_stop(stops: &[(usize, f32)], x: f32) -> usize {
    if stops.is_empty() {
        return 0;
    }
    for pair in stops.windows(2) {
        let (left_b, left_x) = pair[0];
        let (right_b, right_x) = pair[1];
        let mid = (left_x + right_x) * 0.5;
        if x < mid {
            return left_b;
        }
        if x < right_x {
            return right_b;
        }
    }
    stops.last().map_or(0, |(byte, _)| *byte)
}

pub fn hit_test(lines: &[PaintedLine], x: f32, y: f32) -> usize {
    if lines.is_empty() {
        return 0;
    }
    let mut best = 0usize;
    let mut best_dy = f32::MAX;
    for (idx, line) in lines.iter().enumerate() {
        if y >= line.top && y < line.bottom {
            return closest_stop(&line.stops, x);
        }
        let dy = if y < line.top {
            line.top - y
        } else {
            y - line.bottom
        };
        if dy < best_dy {
            best_dy = dy;
            best = idx;
        }
    }
    closest_stop(&lines[best].stops, x)
}

/// Hit test from soft-wrap ranges and a constant advance per grapheme.
/// Used when a shaped layout is not available yet, and by tests.
pub fn hit_test_uniform(
    text: &str,
    lines: &[(usize, usize)],
    origin_x: f32,
    origin_y: f32,
    x: f32,
    y: f32,
    line_h: f32,
    grapheme_w: f32,
) -> usize {
    let ranges = visual_lines(text, lines);
    let painted: Vec<PaintedLine> = ranges
        .iter()
        .enumerate()
        .map(|(idx, &(start, end))| {
            let top = origin_y + idx as f32 * line_h;
            PaintedLine {
                top,
                bottom: top + line_h,
                stops: uniform_stops(text, start, end, origin_x, grapheme_w),
            }
        })
        .collect();
    hit_test(&painted, x, y)
}

pub fn uniform_stops(
    text: &str,
    line_start: usize,
    line_end: usize,
    origin_x: f32,
    grapheme_w: f32,
) -> Vec<(usize, f32)> {
    let end = line_end.min(text.len()).max(line_start.min(text.len()));
    let start = line_start.min(end);
    let mut stops = vec![(start, origin_x)];
    let Some(slice) = text.get(start..end) else {
        return stops;
    };
    let mut x = origin_x;
    for (index, grapheme) in slice.grapheme_indices(true) {
        x += grapheme_w;
        stops.push((start + index + grapheme.len(), x));
    }
    stops
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAMILY: &str = "👨‍👩‍👧";
    const FLAG: &str = "🇺🇸";
    const TONE: &str = "👍🏽";

    fn lines_of(text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0usize;
        for (index, ch) in text.char_indices() {
            if ch == '\n' {
                out.push((start, index));
                start = index + 1;
            }
        }
        out.push((start, text.len()));
        out
    }

    #[test]
    fn horizontal_motion_treats_emoji_and_combining_marks_as_one_unit() {
        let family_len = FAMILY.len();
        let mut text = format!("a{FAMILY}b{FLAG}{TONE}e\u{0301}");
        let mut caret = DraftCaret::at_end(text.len());
        let lines = lines_of(&text);
        caret.move_by(&text, &lines, Motion::Left, false);
        assert_eq!(caret.caret, text.len() - "e\u{0301}".len());
        caret.move_by(&text, &lines, Motion::Left, false);
        assert_eq!(caret.caret, text.len() - "e\u{0301}".len() - TONE.len());
        caret.move_by(&text, &lines, Motion::Left, false);
        assert_eq!(
            caret.caret,
            text.len() - "e\u{0301}".len() - TONE.len() - FLAG.len()
        );
        caret.caret = 1;
        caret.move_by(&text, &lines, Motion::Right, false);
        assert_eq!(caret.caret, 1 + family_len);
        caret.move_by(&text, &lines, Motion::Left, false);
        assert_eq!(caret.caret, 1);
        // A byte parked inside the cluster floors to the cluster start.
        assert_eq!(floor_grapheme(&text, 2), 1);
        caret.caret = 1 + family_len;
        caret.backspace(&mut text);
        assert!(text.starts_with('a'));
        assert!(!text.contains(FAMILY));
    }

    #[test]
    fn backspace_and_delete_remove_one_cluster_or_the_whole_selection() {
        let mut text = format!("hi{FLAG}!");
        let mut caret = DraftCaret::collapsed(2 + FLAG.len());
        caret.delete_forward(&mut text);
        assert_eq!(text, format!("hi{FLAG}"));
        caret.backspace(&mut text);
        assert_eq!(text, "hi");
        caret.caret = 0;
        caret.backspace(&mut text);
        assert_eq!(text, "hi");
        caret.select_all(&text);
        caret.delete_forward(&mut text);
        assert_eq!(text, "");
        caret.insert(&mut text, "ab");
        caret.anchor = Some(0);
        caret.caret = 2;
        caret.insert(&mut text, "Z");
        assert_eq!(text, "Z");
        assert_eq!(caret.caret, 1);
        assert!(caret.selection().is_none());
    }

    #[test]
    fn typing_and_paste_replace_the_selection_only() {
        let mut text = "hello world".to_string();
        let mut caret = DraftCaret::collapsed(6);
        caret.anchor = Some(0);
        caret.insert(&mut text, "hey ");
        assert_eq!(text, "hey world");
        assert_eq!(caret.caret, 4);
        caret.anchor = Some(4);
        caret.caret = text.len();
        caret.insert(&mut text, "there");
        assert_eq!(text, "hey there");
    }

    #[test]
    fn shift_selection_and_word_delete_keep_utf8_boundaries() {
        let text = "hello  world".to_string();
        let lines = lines_of(&text);
        let mut caret = DraftCaret::collapsed(0);
        caret.move_by(&text, &lines, Motion::Right, true);
        caret.move_by(&text, &lines, Motion::Right, true);
        assert_eq!(caret.selection(), Some((0, 2)));
        assert_eq!(caret.selected_text(&text), Some("he"));
        let mut end = DraftCaret::at_end(text.len());
        let mut owned = text.clone();
        end.delete_word_backward(&mut owned);
        assert_eq!(owned, "hello  ");
        let mut mid = "foo==bar".to_string();
        let mut caret = DraftCaret::collapsed(5);
        caret.delete_word_backward(&mut mid);
        assert_eq!(mid, "foobar");
        let mut phrase = "rust build in public  ".to_string();
        let mut caret = DraftCaret::at_end(phrase.len());
        caret.delete_word_backward(&mut phrase);
        assert_eq!(phrase, "rust build in ");
    }

    #[test]
    fn vertical_motion_uses_visual_lines_and_remembers_the_column() {
        let text = "abcd\nxy\nabcd";
        let lines = lines_of(text);
        assert_eq!(lines, vec![(0, 4), (5, 7), (8, 12)]);
        let mut caret = DraftCaret::collapsed(4);
        caret.move_by(text, &lines, Motion::Down, false);
        assert_eq!(caret.caret, 7, "short line clamps the column");
        assert_eq!(caret.preferred_col, Some(4));
        caret.move_by(text, &lines, Motion::Down, false);
        assert_eq!(caret.caret, 12, "column is restored on the next long line");
        caret.move_by(text, &lines, Motion::Up, false);
        assert_eq!(caret.caret, 7);
        caret.move_by(text, &lines, Motion::Up, false);
        assert_eq!(caret.caret, 4);
        caret.move_by(text, &lines, Motion::Up, false);
        assert_eq!(caret.caret, 4, "up on the first line stays put");
    }

    #[test]
    fn wrapped_lines_home_end_and_up_down_follow_visual_rows() {
        // "hello world" wrapped after "hello", space at byte 5 skipped.
        let text = "hello world";
        let lines = vec![(0, 5), (6, 11)];
        let mut caret = DraftCaret::collapsed(0);
        caret.move_by(text, &lines, Motion::End, false);
        assert_eq!(caret.caret, 5);
        caret.move_by(text, &lines, Motion::Right, false);
        assert_eq!(
            caret.caret, 6,
            "right off a wrap lands on the next visual line"
        );
        caret.move_by(text, &lines, Motion::Home, false);
        assert_eq!(caret.caret, 6);
        caret.move_by(text, &lines, Motion::Up, false);
        assert_eq!(caret.caret, 0);
        caret.caret = 5;
        caret.preferred_col = None;
        caret.move_by(text, &lines, Motion::Down, true);
        assert_eq!(caret.selection(), Some((5, 11)));
        caret.move_by(text, &lines, Motion::DocStart, false);
        assert_eq!(caret.caret, 0);
        caret.move_by(text, &lines, Motion::DocEnd, false);
        assert_eq!(caret.caret, text.len());
    }

    #[test]
    fn newline_insert_then_vertical_motion_crosses_the_break() {
        let mut text = "ab".to_string();
        let mut caret = DraftCaret::collapsed(1);
        caret.insert(&mut text, "\n");
        assert_eq!(text, "a\nb");
        assert_eq!(caret.caret, 2);
        let lines = lines_of(&text);
        caret.move_by(&text, &lines, Motion::Up, false);
        // The caret sits before "b" (column 0 of the second line), so Up
        // lands at column 0 of the line above.
        assert_eq!(caret.caret, 0);
        caret.move_by(&text, &lines, Motion::Down, false);
        assert_eq!(caret.caret, 2);
    }

    #[test]
    fn click_drag_word_and_uniform_hit_testing_are_line_aware() {
        let text = "hello world";
        let lines = vec![(0, 5), (6, 11)];
        // Each grapheme is 10px. Line height 20. Origin at (0, 0).
        // Second line starts at y=20. 'w' occupies x 0..10.
        let at_w = hit_test_uniform(text, &lines, 0.0, 0.0, 4.0, 25.0, 20.0, 10.0);
        assert_eq!(at_w, 6);
        let at_o = hit_test_uniform(text, &lines, 0.0, 0.0, 16.0, 25.0, 20.0, 10.0);
        assert_eq!(at_o, 8, "past the midpoint of 'o' the caret sits after it");
        let before = hit_test_uniform(text, &lines, 0.0, 0.0, 14.0, 5.0, 20.0, 10.0);
        assert_eq!(before, 1, "first line, left of the midpoint of 'e'");
        let below = hit_test_uniform(text, &lines, 0.0, 0.0, 0.0, 80.0, 20.0, 10.0);
        assert_eq!(below, 6, "below the text snaps to the last line");

        let mut caret = DraftCaret::collapsed(0);
        caret.drag_anchor(text, 1);
        caret.drag_to(text, 5);
        assert_eq!(caret.selection(), Some((1, 5)));
        caret.place(text, 0, true);
        assert_eq!(caret.selection(), Some((0, 1)));
        caret.select_word_at(text, 8);
        assert_eq!(caret.selected_text(text), Some("world"));
        caret.select_line_at(text, &lines, 2);
        assert_eq!(caret.selected_text(text), Some("hello"));
        caret.select_all(text);
        assert_eq!(caret.selected_text(text), Some(text));
    }

    #[test]
    fn navigation_keys_are_recognized_and_empty_text_stays_put() {
        assert!(is_navigation_key("left"));
        assert!(is_navigation_key("up"));
        assert!(is_navigation_key("home"));
        assert!(!is_navigation_key("enter"));
        assert!(!is_navigation_key("backspace"));
        let mut text = String::new();
        let mut caret = DraftCaret::collapsed(0);
        caret.move_by(&text, &[], Motion::Left, false);
        caret.move_by(&text, &[], Motion::Down, false);
        caret.backspace(&mut text);
        caret.delete_forward(&mut text);
        caret.insert(&mut text, "a");
        assert_eq!(text, "a");
        assert_eq!(caret.caret, 1);
        caret.clamp("a");
        let mut stale = DraftCaret::collapsed(9);
        stale.anchor = Some(4);
        stale.clamp("ab");
        assert_eq!(stale.caret, 2);
        assert!(stale.anchor.is_none() || stale.anchor == Some(2));
    }

    #[test]
    fn left_from_a_selection_collapses_to_the_start() {
        let text = "abcdef";
        let lines = lines_of(text);
        let mut caret = DraftCaret::collapsed(4);
        caret.anchor = Some(1);
        caret.move_by(text, &lines, Motion::Left, false);
        assert_eq!(caret.caret, 1);
        assert!(caret.selection().is_none());
        caret.anchor = Some(1);
        caret.caret = 4;
        caret.move_by(text, &lines, Motion::Right, false);
        assert_eq!(caret.caret, 4);
        assert!(caret.selection().is_none());
    }
}
