//! Twitter-style weighted length and thread splitting.
//!
//! URLs count as 23. Most letters count as 1. Emoji, CJK, and other wide
//! characters count as 2, matching the twitter-text scale of 100 / 200.

pub const WEIGHTED_LIMIT: usize = 280;
pub const URL_WEIGHT: usize = 23;

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
    let spans = url_spans(text);
    let mut urls = 0usize;
    let mut sum = 0u32;
    let mut index = 0usize;
    for &(start, end) in &spans {
        sum += weight_slice(&text[index..start]);
        urls += 1;
        index = end;
    }
    sum += weight_slice(&text[index..]);
    WeightedCount {
        weighted: (sum as usize).div_ceil(100) + urls * URL_WEIGHT,
        limit: WEIGHTED_LIMIT,
        urls,
    }
}

pub fn split_thread(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if weighted_len(trimmed).weighted <= WEIGHTED_LIMIT {
        return vec![trimmed.to_string()];
    }

    let mut parts = Vec::new();
    let mut rest = trimmed;
    while !rest.is_empty() {
        if weighted_len(rest).weighted <= WEIGHTED_LIMIT {
            parts.push(rest.to_string());
            break;
        }
        let mut cut = longest_fit(rest);
        if cut == 0 {
            cut = 1;
        }
        let mut boundary = cut;
        if let Some(space) = rest[..cut].rfind(char::is_whitespace) {
            if space > 0 {
                boundary = space;
            }
        }
        let (head, tail) = rest.split_at(boundary);
        let head = head.trim();
        if head.is_empty() {
            let (forced, tail) = split_chars(rest, cut);
            parts.push(forced);
            rest = tail.trim_start();
            continue;
        }
        parts.push(head.to_string());
        rest = tail.trim_start();
    }
    parts
}

fn split_chars(text: &str, max_bytes: usize) -> (String, &str) {
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        if idx >= max_bytes && end > 0 {
            break;
        }
        end = idx + ch.len_utf8();
        if weighted_len(&text[..end]).weighted > WEIGHTED_LIMIT && end > ch.len_utf8() {
            end -= ch.len_utf8();
            break;
        }
    }
    (text[..end].to_string(), &text[end..])
}

fn longest_fit(text: &str) -> usize {
    let mut last_ok = 0;
    for (idx, ch) in text.char_indices() {
        let end = idx + ch.len_utf8();
        if weighted_len(&text[..end]).weighted > WEIGHTED_LIMIT {
            return last_ok;
        }
        last_ok = end;
    }
    last_ok
}

fn weight_slice(text: &str) -> u32 {
    text.chars().map(char_weight).sum()
}

fn char_weight(ch: char) -> u32 {
    match ch {
        '\u{200D}' | '\u{FE0E}' | '\u{FE0F}' => 0,
        ch if is_wide(ch) => 200,
        _ => 100,
    }
}

fn is_wide(ch: char) -> bool {
    matches!(
        ch,
        '\u{1100}'..='\u{115F}'
            | '\u{2329}'..='\u{232A}'
            | '\u{2E80}'..='\u{A4CF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FE10}'..='\u{FE19}'
            | '\u{FE30}'..='\u{FE6F}'
            | '\u{FF01}'..='\u{FF60}'
            | '\u{FFE0}'..='\u{FFE6}'
            | '\u{1F300}'..='\u{1FAFF}'
            | '\u{1F000}'..='\u{1F2FF}'
            | '\u{2600}'..='\u{27BF}'
    )
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
    let mut url = format!(
        "https://x.com/intent/post?text={}",
        percent_encode(text)
    );
    if let Some(id) = in_reply_to.map(str::trim).filter(|id| !id.is_empty()) {
        url.push_str("&in_reply_to=");
        url.push_str(&percent_encode(id));
    }
    url
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
    fn ascii_and_urls_use_weighted_counts() {
        assert_eq!(weighted_len("hello").weighted, 5);
        assert_eq!(weighted_len("https://example.com/a/b/c").weighted, 23);
        assert_eq!(weighted_len("hi https://example.com").weighted, 26);
        assert_eq!(weighted_len("😀").weighted, 2);
        assert_eq!(weighted_len("你").weighted, 2);
        assert_eq!(weighted_len(&"a".repeat(280)).weighted, 280);
        assert!(weighted_len(&"a".repeat(281)).over_limit());
    }

    #[test]
    fn thread_split_keeps_words_and_respects_the_limit() {
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
    }

    #[test]
    fn a_long_url_still_fits_in_one_part() {
        let text = format!("see https://example.com/{}", "a".repeat(300));
        let count = weighted_len(&text);
        assert_eq!(count.urls, 1);
        assert!(count.weighted < 280);
        assert_eq!(split_thread(&text).len(), 1);
    }
}
