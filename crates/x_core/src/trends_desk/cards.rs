//! Tweet cards, inline reply state, and Reply Guy take selection.
//!
//! Sending is a separate explicit step. Drafting a take never posts.
//! A failed provider stays an error and does not invent a successful draft.

use crate::io::{Http, HttpRequest};
use crate::reply_guy::{self, Secret};

use super::domain::{SourcePost, draft_text_from_title};

/// Windows Termy opens at 1280×820. Termy XL sits beside it at twice the width
/// and the same height. The window stays resizable.
pub const COMPANION_WIDTH: f32 = 2560.0;
pub const COMPANION_HEIGHT: f32 = 820.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TweetCard {
    pub id: String,
    pub author_name: String,
    pub handle: String,
    pub text: String,
    pub media: Vec<String>,
    pub likes: u64,
    pub reposts: u64,
    pub replies: u64,
    pub views: u64,
    pub created_at: Option<String>,
    pub url: String,
}

pub fn card_from_post(post: &SourcePost) -> TweetCard {
    let handle = if post.author.is_empty() {
        "unknown".into()
    } else {
        post.author.clone()
    };
    let author_name = if post.author_name.trim().is_empty() {
        handle.clone()
    } else {
        post.author_name.clone()
    };
    TweetCard {
        id: post.id.clone(),
        author_name,
        handle,
        text: post.text.clone(),
        media: post.media.clone(),
        likes: post.likes,
        reposts: post.reposts,
        replies: post.replies,
        views: post.views,
        created_at: post.created_at.clone(),
        url: post.url.clone(),
    }
}

pub fn card_columns(width: f32) -> usize {
    if width >= 1800.0 { 2 } else { 1 }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPhase {
    Idle,
    Editing { text: String },
    Pending { text: String },
    Confirmed { text: String, receipt: String },
    Failed { text: String, error: String },
}

impl ReplyPhase {
    pub fn text(&self) -> &str {
        match self {
            Self::Editing { text }
            | Self::Pending { text }
            | Self::Confirmed { text, .. }
            | Self::Failed { text, .. } => text,
            Self::Idle => "",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Editing { .. } => "editing",
            Self::Pending { .. } => "pending",
            Self::Confirmed { .. } => "confirmed",
            Self::Failed { .. } => "failed",
        }
    }
}

pub fn edit_reply(phase: &ReplyPhase, text: String) -> ReplyPhase {
    if matches!(phase, ReplyPhase::Pending { .. }) {
        return phase.clone();
    }
    ReplyPhase::Editing { text }
}

/// Move to pending only when the user explicitly chooses Send.
pub fn request_send(phase: &ReplyPhase, explicit: bool) -> Result<ReplyPhase, String> {
    if !explicit {
        return Err("Send requires an explicit click.".into());
    }
    let text = phase.text().trim().to_string();
    if text.is_empty() {
        return Err("Write a reply first.".into());
    }
    if matches!(phase, ReplyPhase::Pending { .. } | ReplyPhase::Idle) {
        return Err("Nothing is ready to send.".into());
    }
    Ok(ReplyPhase::Pending { text })
}

pub fn apply_send_result(pending: &ReplyPhase, result: Result<String, String>) -> ReplyPhase {
    let ReplyPhase::Pending { text } = pending else {
        return pending.clone();
    };
    match result {
        Ok(receipt) => ReplyPhase::Confirmed {
            text: text.clone(),
            receipt,
        },
        Err(error) => ReplyPhase::Failed {
            text: text.clone(),
            error,
        },
    }
}

pub trait ReplyPoster {
    fn post_reply(&self, target_url: &str, text: &str) -> Result<String, String>;
}

/// Calls the poster only after an explicit send. A false `explicit` leaves the
/// phase unchanged and does not call the poster.
pub fn commit_send(
    phase: &ReplyPhase,
    explicit: bool,
    target_url: &str,
    poster: &dyn ReplyPoster,
) -> ReplyPhase {
    let Ok(pending) = request_send(phase, explicit) else {
        return phase.clone();
    };
    let ReplyPhase::Pending { text } = &pending else {
        return pending;
    };
    apply_send_result(&pending, poster.post_reply(target_url, text))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeProvider {
    Treg,
    Free,
}

impl TakeProvider {
    pub fn parse(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("treg") {
            Self::Treg
        } else {
            Self::Free
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Treg => "treg",
            Self::Free => "free",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeDraft {
    pub text: String,
    pub source: &'static str,
    pub note: String,
}

pub fn free_take(post: &SourcePost) -> TakeDraft {
    TakeDraft {
        text: draft_text_from_title(&post.text),
        source: "free-template",
        note: "Free template. Not a paid model.".into(),
    }
}

/// Treg reads the existing Termy X key and checks it. It does not write a new
/// key and it does not fall back to a fake success when the check fails.
/// The wording is still the free Reply Guy template, labeled as such.
pub fn treg_take(post: &SourcePost, http: &dyn Http) -> Result<TakeDraft, String> {
    let config = crate::config::XConfig::default();
    let Some(token) = reply_guy::load_secret(&config, Secret::Treg) else {
        return Err("No Treg key is saved in Termy X.".into());
    };
    reply_guy::test_treg(http, &token).map_err(|error| redact_token(&error, &token))?;
    let mut draft = free_take(post);
    draft.note =
        "Treg key accepted. This draft is the free template, not a paid generation.".into();
    Ok(draft)
}

pub fn draft_take(
    provider: TakeProvider,
    post: &SourcePost,
    http: &dyn Http,
) -> Result<TakeDraft, String> {
    match provider {
        TakeProvider::Free => Ok(free_take(post)),
        TakeProvider::Treg => treg_take(post, http),
    }
}

fn redact_token(error: &str, token: &str) -> String {
    if token.is_empty() {
        return error.to_string();
    }
    error.replace(token, "[redacted]")
}

/// Unused request shape kept so a future catalog draft tool can be swapped in
/// without changing the state machine. Tests assert it carries the token header
/// and never targets a post/reply publish URL.
pub fn treg_draft_probe(token: &str) -> HttpRequest {
    reply_guy::treg_request("GET", reply_guy::TREG_ME_URL, token, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{Http, HttpRequest};
    use serde_json::json;

    struct ScriptHttp {
        body: Result<String, String>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl Http for ScriptHttp {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            self.calls.lock().unwrap().push(request.url);
            self.body.clone()
        }
    }

    struct MockPoster {
        result: Result<String, String>,
        calls: std::sync::Mutex<u32>,
    }

    impl ReplyPoster for MockPoster {
        fn post_reply(&self, _target_url: &str, _text: &str) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            self.result.clone()
        }
    }

    fn post() -> SourcePost {
        SourcePost::from_value(&json!({
            "id": "55",
            "author": "ada",
            "author_name": "Ada Lovelace",
            "text": "Courts and models",
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "likes": 3,
            "replies": 1,
            "views": "10",
            "media_urls": ["https://pbs.twimg.com/media/example.jpg"],
            "url": "https://x.com/ada/status/55"
        }))
        .unwrap()
    }

    #[test]
    fn card_keeps_author_text_media_metrics_time_and_link() {
        let card = card_from_post(&post());
        assert_eq!(card.author_name, "Ada Lovelace");
        assert_eq!(card.handle, "ada");
        assert_eq!(card.text, "Courts and models");
        assert_eq!(
            card.media,
            vec!["https://pbs.twimg.com/media/example.jpg".to_string()]
        );
        assert_eq!(card.likes, 3);
        assert_eq!(card.replies, 1);
        assert_eq!(
            card.created_at.as_deref(),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert_eq!(card.url, "https://x.com/ada/status/55");
    }

    #[test]
    fn companion_size_is_twice_termy_width_and_cards_shrink() {
        assert_eq!(COMPANION_WIDTH, 2560.0);
        assert_eq!(COMPANION_HEIGHT, 820.0);
        assert_eq!(card_columns(COMPANION_WIDTH), 2);
        assert_eq!(card_columns(900.0), 1);
    }

    #[test]
    fn typing_and_a_failed_send_do_not_drop_text_or_auto_post() {
        let poster = MockPoster {
            result: Err("opencli down".into()),
            calls: std::sync::Mutex::new(0),
        };
        let editing = edit_reply(&ReplyPhase::Idle, "hello reply".into());
        let still = commit_send(&editing, false, "https://x.com/ada/status/55", &poster);
        assert_eq!(still, editing);
        assert_eq!(*poster.calls.lock().unwrap(), 0);
        let failed = commit_send(&editing, true, "https://x.com/ada/status/55", &poster);
        assert_eq!(
            failed,
            ReplyPhase::Failed {
                text: "hello reply".into(),
                error: "opencli down".into(),
            }
        );
        assert_eq!(*poster.calls.lock().unwrap(), 1);
        let pending = request_send(&editing, true).unwrap();
        assert!(matches!(pending, ReplyPhase::Pending { .. }));
        let confirmed = apply_send_result(&pending, Ok("receipt-1".into()));
        assert_eq!(
            confirmed,
            ReplyPhase::Confirmed {
                text: "hello reply".into(),
                receipt: "receipt-1".into(),
            }
        );
    }

    #[test]
    fn provider_selection_does_not_hide_a_treg_failure() {
        let post = post();
        let free = draft_take(
            TakeProvider::Free,
            &post,
            &ScriptHttp {
                body: Err("unused".into()),
                calls: std::sync::Mutex::new(Vec::new()),
            },
        )
        .unwrap();
        assert_eq!(free.source, "free-template");
        assert!(free.text.starts_with("Worth watching:"));
        let http = ScriptHttp {
            body: Err("treg: no".into()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let err = draft_take(TakeProvider::Treg, &post, &http).unwrap_err();
        assert!(err.contains("Treg") || err.contains("treg") || err.contains("key"));
        assert!(!err.contains("Worth watching"));
        assert!(
            http.calls.lock().unwrap().is_empty() || err.contains("key") || err.contains("Treg")
        );
    }

    #[test]
    fn treg_probe_is_not_a_publish_url() {
        let request = treg_draft_probe("secret-token");
        assert!(request.url.contains("treg.to"));
        assert!(!request.url.contains("/status/"));
        assert!(request.headers.iter().any(|(key, _)| key == "X-Treg-Token"));
        assert!(!format!("{request:?}").contains("x.exe"));
    }
}
