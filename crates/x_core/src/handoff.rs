//! Hand a post or trend to an outside assistant (Grok Bot, grok.com,
//! Grok CLI, Cursor): build a prompt, then the panel copies it and opens
//! the target. Replies come back by pasting; `parse_options` turns the
//! pasted text into draft options. Nothing here posts or reads other
//! apps' tokens or sessions.

use crate::model::Post;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    GrokBot,
    GrokWeb,
    GrokCli,
    Cursor,
}

impl Target {
    pub const ALL: [Target; 4] = [Self::GrokBot, Self::GrokWeb, Self::GrokCli, Self::Cursor];

    pub fn label(self) -> &'static str {
        match self {
            Self::GrokBot => "Grok Bot",
            Self::GrokWeb => "grok.com",
            Self::GrokCli => "Grok CLI",
            Self::Cursor => "Cursor",
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::GrokBot => "grokbot",
            Self::GrokWeb => "grokweb",
            Self::GrokCli => "grokcli",
            Self::Cursor => "cursor",
        }
    }
}

/// What the assistant should reply to.
pub enum Subject<'a> {
    Post(&'a Post),
    Trend(&'a str),
}

pub fn build_prompt(subject: Subject<'_>, niche: &str, char_limit: usize) -> String {
    let niche = niche.trim();
    let niche_line = if niche.is_empty() {
        String::new()
    } else {
        format!("My niche/tone: {niche}\n")
    };
    let body = match subject {
        Subject::Post(post) => {
            let url = if post.url.is_empty() {
                format!("https://x.com/{}/status/{}", post.author_handle, post.id)
            } else {
                post.url.clone()
            };
            format!(
                "Write replies to this X post.\nAuthor: @{}\nLink: {url}\nPost:\n\"\"\"\n{}\n\"\"\"\n",
                post.author_handle,
                post.text.trim()
            )
        }
        Subject::Trend(trend) => format!(
            "Write X posts joining this trending topic: {}\n",
            trend.trim()
        ),
    };
    format!(
        "{body}{niche_line}Give exactly 3 options, numbered 1. 2. 3., each under {char_limit} characters. \
Add something real (a fact, a sharp take, or a question). No hashtags, no flattery, no preamble. \
Output only the 3 numbered options."
    )
}

/// https://grok.com/?q=<prompt> prefill.
pub fn grok_web_url(prompt: &str) -> String {
    format!(
        "https://grok.com/?q={}",
        crate::text::percent_encode(prompt)
    )
}

/// Split pasted assistant output into reply options. Understands
/// `1.` / `1)` / `(1)` / `Option 1:` numbering and `-` / `*` / `•`
/// bullets; multi-line options are kept together. Falls back to blank-
/// line paragraphs, then the whole text.
pub fn parse_options(text: &str) -> Vec<String> {
    let mut options: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut saw_marker = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = strip_marker(line) {
            saw_marker = true;
            if let Some(done) = current.take() {
                options.push(done);
            }
            current = Some(rest.to_string());
        } else if let Some(open) = current.as_mut()
            && !line.is_empty()
        {
            open.push('\n');
            open.push_str(line);
        }
    }
    if let Some(done) = current {
        options.push(done);
    }
    if !saw_marker {
        options = text
            .split("\n\n")
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
    }
    options
        .into_iter()
        .map(|o| clean(&o))
        .filter(|o| !o.is_empty())
        .take(10)
        .collect()
}

fn clean(option: &str) -> String {
    let trimmed = option.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    unquoted
        .trim_start_matches("**")
        .trim_end_matches("**")
        .trim()
        .to_string()
}

fn strip_marker(line: &str) -> Option<&str> {
    for bullet in ["- ", "* ", "\u{2022} "] {
        if let Some(rest) = line.strip_prefix(bullet) {
            return Some(rest.trim());
        }
    }
    let lower = line.to_ascii_lowercase();
    let line = if lower.starts_with("option ") {
        &line[7..]
    } else if lower.starts_with("**option ") {
        &line[9..]
    } else {
        line
    };
    let line = line.strip_prefix('(').unwrap_or(line);
    let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 2 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.trim_start_matches("**");
    for sep in [".", ")", ":", " -", " \u{2014}"] {
        if let Some(after) = rest.strip_prefix(sep) {
            let after = after.trim_start_matches("**").trim();
            return Some(after);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post() -> Post {
        Post {
            id: "42".into(),
            author_handle: "dev".into(),
            text: "Rust or Go for CLIs?".into(),
            ..Default::default()
        }
    }

    #[test]
    fn post_prompt_has_text_author_link_and_limit() {
        let p = build_prompt(Subject::Post(&post()), "indie hacker, dry humor", 280);
        assert!(p.contains("@dev"));
        assert!(p.contains("https://x.com/dev/status/42"));
        assert!(p.contains("Rust or Go for CLIs?"));
        assert!(p.contains("under 280 characters"));
        assert!(p.contains("indie hacker, dry humor"));
        assert!(p.contains("exactly 3 options"));
    }

    #[test]
    fn trend_prompt_and_premium_limit() {
        let p = build_prompt(Subject::Trend("Greenland"), "", 25_000);
        assert!(p.contains("Greenland"));
        assert!(p.contains("under 25000 characters"));
        assert!(!p.contains("niche"));
    }

    #[test]
    fn grok_web_url_encodes_prompt() {
        let url = grok_web_url("a b&c");
        assert!(url.starts_with("https://grok.com/?q="));
        assert!(!url.contains(' '));
        assert!(!url[20..].contains('&'));
    }

    #[test]
    fn parses_numbered_variants() {
        let text = "Here you go:\n1. First one\n2) Second\n   continues\n(3) Third\n";
        assert_eq!(
            parse_options(text),
            vec!["First one", "Second\ncontinues", "Third"]
        );
    }

    #[test]
    fn parses_bold_option_labels_and_quotes() {
        let text = "**Option 1:** \"Hot take\"\n**Option 2:** Cold take\nOption 3 - Warm";
        assert_eq!(parse_options(text), vec!["Hot take", "Cold take", "Warm"]);
    }

    #[test]
    fn parses_bullets_and_paragraph_fallback() {
        assert_eq!(parse_options("- a\n* b\n\u{2022} c"), vec!["a", "b", "c"]);
        assert_eq!(parse_options("alpha\n\nbeta"), vec!["alpha", "beta"]);
        assert!(parse_options("   ").is_empty());
    }

    #[test]
    fn years_are_not_list_markers() {
        assert_eq!(parse_options("2026 was wild"), vec!["2026 was wild"]);
    }
}
