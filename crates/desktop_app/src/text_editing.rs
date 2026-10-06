use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextCharClass {
    Word,
    Whitespace,
    Other,
}

#[inline]
pub(crate) fn classify_char(ch: char) -> TextCharClass {
    if ch.is_alphanumeric() || ch == '_' {
        TextCharClass::Word
    } else if ch.is_whitespace() {
        TextCharClass::Whitespace
    } else {
        TextCharClass::Other
    }
}

pub(crate) fn previous_char_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }

    let mut index = offset.min(text.len());
    while index > 0 {
        index -= 1;
        if text.is_char_boundary(index) {
            return index;
        }
    }
    0
}

pub(crate) fn next_char_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }

    let mut index = offset + 1;
    while index < text.len() {
        if text.is_char_boundary(index) {
            return index;
        }
        index += 1;
    }
    text.len()
}

pub(crate) fn previous_word_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }

    let mut boundary = 0;
    let mut seen_word = false;
    for (idx, ch) in text[..offset].char_indices().rev() {
        if classify_char(ch) == TextCharClass::Word {
            seen_word = true;
            boundary = idx;
            continue;
        }
        if seen_word {
            boundary = idx + ch.len_utf8();
            break;
        }
        boundary = idx;
    }
    boundary
}

pub(crate) fn next_word_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }

    let mut seen_word = false;
    for (rel_idx, ch) in text[offset..].char_indices() {
        let is_word = classify_char(ch) == TextCharClass::Word;
        if is_word {
            seen_word = true;
        } else if seen_word {
            return offset + rel_idx;
        }
    }
    text.len()
}

pub(crate) fn token_range_at_utf8(text: &str, offset: usize) -> Range<usize> {
    if text.is_empty() {
        return 0..0;
    }

    let mut anchor = clamp_utf8_index(text, offset.min(text.len()));
    if anchor == text.len() && anchor > 0 {
        anchor = previous_char_boundary(text, anchor);
    }
    if anchor >= text.len() {
        return text.len()..text.len();
    }

    let Some(anchor_char) = text[anchor..].chars().next() else {
        return text.len()..text.len();
    };
    let class = classify_char(anchor_char);

    let mut start = anchor;
    while start > 0 {
        let prev = previous_char_boundary(text, start);
        let Some(prev_char) = text[prev..start].chars().next() else {
            break;
        };
        if classify_char(prev_char) != class {
            break;
        }
        start = prev;
    }

    let mut end = next_char_boundary(text, anchor);
    while end < text.len() {
        let next_end = next_char_boundary(text, end);
        let Some(next_char) = text[end..next_end].chars().next() else {
            break;
        };
        if classify_char(next_char) != class {
            break;
        }
        end = next_end;
    }

    start..end
}

pub(crate) fn clamp_utf8_index(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(crate) fn clamped_utf8_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = clamp_utf8_index(text, range.start.min(text.len()));
    let end = clamp_utf8_index(text, range.end.min(text.len()));
    if end < start { end..start } else { start..end }
}

pub(crate) fn utf16_to_utf8(text: &str, utf16_offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;

    for ch in text.chars() {
        if utf16_count >= utf16_offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }

    clamp_utf8_index(text, utf8_offset)
}

pub(crate) fn utf8_to_utf16(text: &str, utf8_offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    let clamped_utf8 = clamp_utf8_index(text, utf8_offset);

    for ch in text.chars() {
        if utf8_count >= clamped_utf8 {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }

    utf16_offset
}

pub(crate) fn range_from_utf16(text: &str, range_utf16: &Range<usize>) -> Range<usize> {
    let start = utf16_to_utf8(text, range_utf16.start);
    let end = utf16_to_utf8(text, range_utf16.end);
    if end < start { end..start } else { start..end }
}

pub(crate) fn range_to_utf16(text: &str, range_utf8: &Range<usize>) -> Range<usize> {
    let start = utf8_to_utf16(text, range_utf8.start);
    let end = utf8_to_utf16(text, range_utf8.end);
    if end < start { end..start } else { start..end }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_utf8_offsets_handle_multibyte_text() {
        let text = "a😄é";
        assert_eq!(utf16_to_utf8(text, 0), 0);
        assert_eq!(utf16_to_utf8(text, 1), 1);
        assert_eq!(utf16_to_utf8(text, 3), 5);
        assert_eq!(utf8_to_utf16(text, 6), 3);
        assert_eq!(utf8_to_utf16(text, 7), 4);
    }

    #[test]
    fn word_boundaries_skip_separator_runs() {
        let text = "hello  world";
        assert_eq!(previous_word_boundary(text, text.len()), 7);
        assert_eq!(next_word_boundary(text, 0), 5);
    }

    #[test]
    fn token_range_groups_matching_character_classes() {
        let text = "foo==bar";
        assert_eq!(token_range_at_utf8(text, 1), 0..3);
        assert_eq!(token_range_at_utf8(text, 3), 3..5);
        assert_eq!(token_range_at_utf8(text, text.len()), 5..8);
    }
}
