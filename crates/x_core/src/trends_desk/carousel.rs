//! Sideways take cards. A right press on the last card asks for another.
//! Nothing here starts a provider by itself.

use super::TakeDraft;
use super::TakeProvider;
use super::takes::run_take_chain;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slide {
    Stay,
    Move(usize),
    More,
}

pub fn slide(len: usize, index: usize, generating: bool, delta: i32) -> Slide {
    if generating || delta == 0 {
        return Slide::Stay;
    }
    if len == 0 {
        return if delta > 0 { Slide::More } else { Slide::Stay };
    }
    let last = len - 1;
    let index = index.min(last);
    if delta > 0 && index == last {
        Slide::More
    } else if delta < 0 && index == 0 {
        Slide::Stay
    } else {
        let next = (index as i32 + delta).clamp(0, last as i32) as usize;
        Slide::Move(next)
    }
}

pub fn indicator(len: usize, index: usize, generating: bool) -> String {
    if generating {
        let total = len + 1;
        return format!("{total} / {total}");
    }
    if len == 0 {
        return "0 / 0".into();
    }
    format!("{} / {len}", index.min(len - 1) + 1)
}

pub fn failure_banner(source: &str, note: &str) -> String {
    let who = match source {
        "grok" => "Grok",
        "muse" => "Muse",
        "hermes" => "Hermes",
        "agy" => "Agy",
        "vyce" => "Vyce",
        _ => "Take",
    };
    let note = note.trim();
    if note.to_lowercase().contains("timed out") {
        if note.to_lowercase().contains("5 minute") {
            format!("{who} timed out after 5 minutes, no reply")
        } else {
            format!("{who} timed out, no reply")
        }
    } else if note.is_empty() {
        format!("{who} returned no reply")
    } else {
        format!("{who}: {note}")
    }
}

pub fn handle_from_status(url: &str) -> String {
    let path = url.split("://").nth(1).unwrap_or(url);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let _host = parts.next();
    let handle = parts.next().unwrap_or("").trim_start_matches('@');
    if handle.is_empty() || handle == "i" || handle.eq_ignore_ascii_case("status") {
        "post".into()
    } else {
        handle.to_string()
    }
}

pub fn draft_post_label(url: &str) -> String {
    draft_reply_label(url, "")
}

pub fn draft_reply_label(url: &str, author: &str) -> String {
    let author = author.trim().trim_start_matches('@');
    let named = !author.is_empty()
        && author != "i"
        && !author.eq_ignore_ascii_case("post")
        && !author.eq_ignore_ascii_case("status");
    let handle = if named {
        author.to_string()
    } else {
        handle_from_status(url)
    };
    if handle.is_empty() || handle == "post" || handle == "i" {
        "Post reply".into()
    } else {
        format!("Post reply → @{handle}")
    }
}

/// Tweet plus a short instruction. CLI takes stay near 400 characters.
pub fn cli_take_prompt(author: &str, tweet: &str, tone: &str) -> String {
    let author = author.trim().trim_start_matches('@');
    let head = format!(
        "Tone {tone}. One X reply under 280 characters. Reply text only. No tools. @{author}: "
    );
    let room = 400usize.saturating_sub(head.chars().count());
    let tweet: String = tweet
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(room)
        .collect();
    format!("{head}{tweet}")
}

pub fn next_providers(chosen: TakeProvider) -> Vec<TakeProvider> {
    [TakeProvider::Hermes, TakeProvider::Muse]
        .into_iter()
        .filter(|provider| *provider != chosen)
        .collect()
}

pub fn run_take_fallback(
    provider: TakeProvider,
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
    enabled: bool,
) -> TakeDraft {
    let first = run_take_chain(provider, config, post, http, tone);
    if !enabled || !first.text.trim().is_empty() {
        return first;
    }
    for next in next_providers(provider) {
        let draft = run_take_chain(next, config, post, http, tone);
        if !draft.text.trim().is_empty() {
            let label = if draft.note.trim().is_empty() {
                next.as_str().to_string()
            } else {
                draft.note.clone()
            };
            return TakeDraft {
                note: format!("Fallback · {label}"),
                ..draft
            };
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_on_the_last_card_asks_for_another_and_left_stops() {
        assert_eq!(slide(0, 0, false, 1), Slide::More);
        assert_eq!(slide(0, 0, false, -1), Slide::Stay);
        assert_eq!(slide(3, 1, false, 1), Slide::Move(2));
        assert_eq!(slide(3, 2, false, 1), Slide::More);
        assert_eq!(slide(3, 0, false, -1), Slide::Stay);
        assert_eq!(slide(3, 1, true, 1), Slide::Stay);
        assert_eq!(indicator(3, 1, false), "2 / 3");
        assert_eq!(indicator(2, 1, true), "3 / 3");
        assert!(
            cli_take_prompt("ada", &"word ".repeat(200), "Funny")
                .chars()
                .count()
                <= 400
        );
        assert!(cli_take_prompt("ada", "courts posted a draft", "Spicy").contains("courts"));
        assert_eq!(
            failure_banner("grok", "grok timed out after 5 minutes"),
            "Grok timed out after 5 minutes, no reply"
        );
        assert_eq!(
            draft_post_label("https://x.com/Ada/status/9"),
            "Post reply → @Ada"
        );
        assert_eq!(
            draft_reply_label("https://x.com/i/status/9", "jeremy"),
            "Post reply → @jeremy"
        );
        assert_eq!(
            next_providers(TakeProvider::Grok),
            vec![TakeProvider::Hermes, TakeProvider::Muse]
        );
        assert_eq!(
            next_providers(TakeProvider::Hermes),
            vec![TakeProvider::Muse]
        );
        assert!(!super::super::domain::DeskSettings::default().auto_fallback);
    }
}
