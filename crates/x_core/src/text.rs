//! Twitter-text v3 weighted length and thread splitting.
//!
//! Config (v3): scale 100, default weight 200, transformed URL length 23,
//! max 280 (standard) or 25000 (Premium). Code points U+0000..=U+10FF, U+2000..=U+200D, U+2010..=U+201F,
//! and U+2032..=U+2037 weigh 100 (one visible character). Everything else
//! weighs 200. An emoji, including a ZWJ sequence, skin-tone modifier, or
//! regional-indicator flag, weighs 200 once. Text is NFC-normalized first.
//! <https://github.com/twitter/twitter-text/blob/master/config/v3.json>

use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

/// Free / non-Premium post cap (Twitter-text v3 default).
pub const STANDARD_CHAR_LIMIT: usize = 280;
/// X Premium / Premium+ long-form cap.
pub const PREMIUM_CHAR_LIMIT: usize = 25_000;
/// Back-compat alias for the free-tier limit used by unit tests.
pub const WEIGHTED_LIMIT: usize = STANDARD_CHAR_LIMIT;
pub const URL_WEIGHT: usize = 23;
const SCALE: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedCount {
    pub weighted: usize,
    pub limit: usize,
    pub urls: usize,
}

impl WeightedCount {
    pub fn remaining(&self) -> isize {
        self.limit as isize - self.weighted as isize
    }

    pub fn over_limit(&self) -> bool {
        self.weighted > self.limit
    }
}

pub fn weighted_len(text: &str) -> WeightedCount {
    weighted_len_limited(text, STANDARD_CHAR_LIMIT)
}

pub fn weighted_len_limited(text: &str, limit: usize) -> WeightedCount {
    let mut count = weighted_normalized(&normalize(text));
    count.limit = limit.max(1);
    count
}

pub fn split_thread(text: &str) -> Vec<String> {
    split_thread_limited(text, STANDARD_CHAR_LIMIT)
}

pub fn split_thread_limited(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let trimmed = normalize(text.trim());
    if trimmed.is_empty() {
        return Vec::new();
    }
    if weighted_normalized(&trimmed).weighted <= limit {
        return vec![trimmed];
    }

    let mut parts = Vec::new();
    let mut rest = trimmed.as_str();
    while !rest.is_empty() {
        if weighted_normalized(rest).weighted <= limit {
            parts.push(rest.to_string());
            break;
        }
        let mut cut = longest_fit(rest, limit);
        if cut == 0 {
            cut = split_units(rest).first().map(|unit| unit.len()).unwrap_or(rest.len());
        }
        cut = snap_grapheme(rest, cut);
        let mut boundary = cut;
        if let Some(space) = rest[..cut].rfind(char::is_whitespace) {
            if space > 0 {
                boundary = snap_grapheme(rest, space);
            }
        }
        if boundary == 0 {
            boundary = cut;
        }
        let (head, tail) = rest.split_at(boundary);
        let head = head.trim();
        if head.is_empty() {
            let unit = split_units(rest).into_iter().next().unwrap_or(rest);
            let end = snap_grapheme(rest, unit.len());
            parts.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
            continue;
        }
        parts.push(head.to_string());
        rest = tail.trim_start();
    }
    parts
}

/// RFC 3986 percent-encoding. Space is `%20`.
pub fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn intent_url(text: &str, in_reply_to: Option<&str>) -> String {
    let mut url = format!("https://x.com/intent/post?text={}", percent_encode(text));
    if let Some(id) = in_reply_to.map(str::trim).filter(|id| !id.is_empty()) {
        url.push_str("&in_reply_to=");
        url.push_str(&percent_encode(id));
    }
    url
}

fn normalize(text: &str) -> String {
    text.nfc().collect()
}

fn weighted_normalized(text: &str) -> WeightedCount {
    let spans = url_spans(text);
    let mut urls = 0usize;
    let mut sum = 0u32;
    let mut index = 0usize;
    for &(start, end) in &spans {
        sum += weight_text(&text[index..start]);
        urls += 1;
        index = end;
    }
    sum += weight_text(&text[index..]);
    WeightedCount {
        weighted: (sum / SCALE) as usize + urls * URL_WEIGHT,
        limit: STANDARD_CHAR_LIMIT,
        urls,
    }
}

fn longest_fit(text: &str, limit: usize) -> usize {
    let mut last_ok = 0usize;
    let mut end = 0usize;
    for unit in split_units(text) {
        end += unit.len();
        end = snap_grapheme(text, end);
        if weighted_normalized(&text[..end]).weighted > limit {
            return last_ok;
        }
        last_ok = end;
    }
    last_ok
}

fn snap_grapheme(text: &str, end: usize) -> usize {
    if end >= text.len() {
        return text.len();
    }
    for (start, grapheme) in text.grapheme_indices(true) {
        let grapheme_end = start + grapheme.len();
        if end > start && end < grapheme_end {
            return grapheme_end;
        }
    }
    end
}

fn weight_text(text: &str) -> u32 {
    split_units(text).into_iter().map(unit_weight).sum()
}

fn unit_weight(unit: &str) -> u32 {
    let mut chars = unit.chars();
    let Some(first) = chars.next() else {
        return 0;
    };
    if is_regional(first) || is_emoji_pict(first) {
        return 200;
    }
    codepoint_weight(first) + chars.map(codepoint_weight).sum::<u32>()
}

/// Emoji sequences stay one unit: a pictograph plus variation selectors,
/// skin tones, and ZWJ tails, or a regional-indicator pair (a flag).
fn split_units(text: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut units = Vec::new();
    let mut index = 0usize;
    let mut start_byte = 0usize;
    while index < chars.len() {
        let ch = chars[index].1;
        index += 1;
        if is_regional(ch) {
            if index < chars.len() && is_regional(chars[index].1) {
                index += 1;
            }
        } else if is_emoji_pict(ch) {
            loop {
                if index < chars.len() && (is_variation(chars[index].1) || is_skin(chars[index].1)) {
                    index += 1;
                    continue;
                }
                if index + 1 < chars.len()
                    && chars[index].1 == '\u{200D}'
                    && (is_emoji_pict(chars[index + 1].1) || is_regional(chars[index + 1].1))
                {
                    index += 2;
                    continue;
                }
                break;
            }
        }
        let end_byte = if index < chars.len() {
            chars[index].0
        } else {
            text.len()
        };
        units.push(&text[start_byte..end_byte]);
        start_byte = end_byte;
    }
    units
}

fn codepoint_weight(ch: char) -> u32 {
    let cp = ch as u32;
    if (0..=4351).contains(&cp)
        || (8192..=8205).contains(&cp)
        || (8208..=8223).contains(&cp)
        || (8242..=8247).contains(&cp)
    {
        100
    } else {
        200
    }
}

fn is_regional(ch: char) -> bool {
    matches!(ch, '\u{1F1E6}'..='\u{1F1FF}')
}

fn is_skin(ch: char) -> bool {
    matches!(ch, '\u{1F3FB}'..='\u{1F3FF}')
}

fn is_variation(ch: char) -> bool {
    matches!(ch, '\u{FE0E}' | '\u{FE0F}')
}

fn is_emoji_pict(ch: char) -> bool {
    matches!(
        ch,
        '\u{2300}'..='\u{23FF}'
            | '\u{2600}'..='\u{27BF}'
            | '\u{2B05}'..='\u{2B55}'
            | '\u{1F000}'..='\u{1F1E5}'
            | '\u{1F200}'..='\u{1F2FF}'
            | '\u{1F300}'..='\u{1FAFF}'
    )
}

fn url_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &text[index..];
        let lower = rest.to_ascii_lowercase();
        let start_rel = ["https://", "http://", "www."]
            .iter()
            .filter_map(|prefix| lower.find(prefix))
            .min();
        let Some(start_rel) = start_rel else {
            break;
        };
        let start = index + start_rel;
        let mut end = start;
        for (offset, ch) in text[start..].char_indices() {
            if ch.is_whitespace() || matches!(ch, '<' | '>' | '"' | '\'') {
                break;
            }
            end = start + offset + ch.len_utf8();
        }
        while end > start {
            let last = text[..end].chars().next_back().unwrap_or(' ');
            if matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']') {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if end > start {
            spans.push((start, end));
            index = end;
        } else {
            index = start + 1;
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_latin_cjk_and_urls_use_v3_weights() {
        assert_eq!(weighted_len("hello").weighted, 5);
        assert_eq!(weighted_len("é").weighted, 1);
        assert_eq!(weighted_len("https://example.com/a/b/c").weighted, 23);
        assert_eq!(weighted_len("hi https://example.com").weighted, 26);
        assert_eq!(weighted_len("😀").weighted, 2);
        assert_eq!(weighted_len("你").weighted, 2);
        assert_eq!(weighted_len("你好").weighted, 4);
        assert_eq!(weighted_len(&"a".repeat(280)).weighted, 280);
        assert_eq!(weighted_len(&"a".repeat(281)).weighted, 281);
        assert!(weighted_len(&"a".repeat(281)).over_limit());
        assert!(!weighted_len(&"a".repeat(280)).over_limit());
        assert_eq!(weighted_len(&"你".repeat(140)).weighted, 280);
        assert!(weighted_len(&"你".repeat(141)).over_limit());
    }

    #[test]
    fn emoji_sequences_count_once() {
        assert_eq!(weighted_len("\u{1F44D}\u{1F3FD}").weighted, 2, "skin tone");
        assert_eq!(
            weighted_len("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}").weighted,
            2,
            "family zwj"
        );
        assert_eq!(weighted_len("\u{1F1FA}\u{1F1F8}").weighted, 2, "flag");
        assert_eq!(weighted_len("❤️").weighted, 2, "heart plus variation selector");
        assert_eq!(weighted_len("a\u{1F44D}\u{1F3FD}b").weighted, 4);
    }

    #[test]
    fn nfc_collapses_combining_marks_before_counting() {
        let composed = "é";
        let decomposed = "e\u{0301}";
        assert_ne!(composed, decomposed);
        assert_eq!(weighted_len(composed).weighted, 1);
        assert_eq!(weighted_len(decomposed).weighted, 1);
        assert_eq!(weighted_len(decomposed), weighted_len(composed));
    }

    #[test]
    fn a_url_of_any_length_weighs_23() {
        let long = format!("https://example.com/{}", "a".repeat(400));
        assert_eq!(weighted_len(&long).weighted, 23);
        assert_eq!(weighted_len(&long).urls, 1);
        let punctuated = "see https://example.com/a?b=1&c=2#hash";
        let count = weighted_len(punctuated);
        assert_eq!(count.urls, 1);
        assert_eq!(count.weighted, 4 + 23);
        assert_eq!(weighted_len("https://example.com/a").weighted, 23);
        assert_eq!(weighted_len("https://example.com/a.").urls, 1);
        assert_eq!(
            weighted_len("https://example.com/a.").weighted,
            24,
            "a trailing period stays outside the URL"
        );
    }

    #[test]
    fn thread_split_keeps_words_emoji_and_the_limit() {
        let one = split_thread("short post");
        assert_eq!(one, vec!["short post".to_string()]);

        let long = format!("{} extra", "word ".repeat(100));
        let parts = split_thread(&long);
        assert!(parts.len() > 1);
        for part in &parts {
            assert!(weighted_len(part).weighted <= WEIGHTED_LIMIT, "{part}");
            assert!(!part.ends_with(' '));
        }
        assert!(parts.iter().any(|part| part.contains("extra")));

        let boundary = format!("{}{}", "a".repeat(279), "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
        assert_eq!(weighted_len(&boundary).weighted, 281);
        let split = split_thread(&boundary);
        assert!(split.len() >= 2);
        for part in &split {
            assert!(weighted_len(part).weighted <= WEIGHTED_LIMIT, "{part}");
        }
        assert!(split.iter().any(|part| part.contains('\u{1F468}')));
        assert!(split
            .iter()
            .any(|part| part.contains('\u{200D}') && weighted_len(part).weighted <= WEIGHTED_LIMIT));
    }

    #[test]
    fn a_long_url_still_fits_in_one_part() {
        let text = format!("see https://example.com/{}", "a".repeat(300));
        let count = weighted_len(&text);
        assert_eq!(count.urls, 1);
        assert!(count.weighted < 280);
        assert_eq!(split_thread(&text).len(), 1);
    }

    #[test]
    fn grapheme_snap_does_not_cut_inside_a_cluster() {
        let text = "a\u{0301}b";
        let normalized = normalize(text);
        assert_eq!(snap_grapheme(&normalized, 0), 0);
        assert_eq!(snap_grapheme("hi", 2), 2);
        let flag = "\u{1F1FA}\u{1F1F8}";
        assert_eq!(snap_grapheme(flag, 1), flag.len());
    }

    #[test]
    fn premium_limit_keeps_long_posts_as_one_part() {
        let text = "a".repeat(500);
        let count = weighted_len_limited(&text, PREMIUM_CHAR_LIMIT);
        assert_eq!(count.weighted, 500);
        assert_eq!(count.limit, PREMIUM_CHAR_LIMIT);
        assert!(!count.over_limit());
        assert_eq!(split_thread_limited(&text, PREMIUM_CHAR_LIMIT).len(), 1);

        // ~2k weighted: over free tier, under Premium, stays one part.
        let mid = "word ".repeat(400);
        let mid_count = weighted_len_limited(&mid, PREMIUM_CHAR_LIMIT);
        assert!(mid_count.weighted > STANDARD_CHAR_LIMIT);
        assert!(mid_count.weighted < PREMIUM_CHAR_LIMIT);
        assert_eq!(split_thread_limited(&mid, PREMIUM_CHAR_LIMIT).len(), 1);
        assert!(split_thread_limited(&mid, STANDARD_CHAR_LIMIT).len() > 1);

        assert!(weighted_len_limited(&"a".repeat(PREMIUM_CHAR_LIMIT + 1), PREMIUM_CHAR_LIMIT).over_limit());
        // Split path at a modest Premium-style cap (avoid O(n^2) on 25k in unit tests).
        let over = "a".repeat(600);
        let parts = split_thread_limited(&over, 500);
        assert!(parts.len() >= 2);
        for part in &parts {
            assert!(weighted_len_limited(part, 500).weighted <= 500);
        }
    }

    fn standard_mode_still_splits_at_280() {
        let long = format!("{} extra", "word ".repeat(100));
        let parts = split_thread_limited(&long, STANDARD_CHAR_LIMIT);
        assert!(parts.len() > 1);
        for part in &parts {
            assert!(weighted_len_limited(part, STANDARD_CHAR_LIMIT).weighted <= STANDARD_CHAR_LIMIT);
        }
    }

    #[test]
    fn intent_url_encodes_reserved_characters_unicode_and_reply_ids() {
        let text = "a b\nc & d # e ? f + g 你 😀";
        let url = intent_url(text, Some("99+1"));
        assert!(url.starts_with("https://x.com/intent/post?text="));
        assert!(url.contains("a%20b%0Ac%20%26%20d%20%23%20e%20%3F%20f%20%2B%20g%20"));
        assert!(url.contains("%E4%BD%A0"), "cjk: {url}");
        assert!(url.contains("%F0%9F%98%80"), "emoji: {url}");
        assert!(url.contains("&in_reply_to=99%2B1"), "{url}");
        assert_eq!(url.matches('?').count(), 1, "{url}");
        assert!(!url.contains('#'), "{url}");
        assert!(!url.contains('+'), "{url}");
        let raw_ampersands = url.matches('&').count();
        assert_eq!(raw_ampersands, 1, "{url}");
    }
}
