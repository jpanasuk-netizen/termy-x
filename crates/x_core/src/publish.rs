use crate::model::{ProviderId, SESSION_OWN};
use crate::text::{WeightedCount, intent_url};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmDecision {
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPlan {
    pub text: String,
    pub parts: Vec<String>,
    pub count: WeightedCount,
    pub provider: ProviderId,
    pub session: Option<String>,
    pub intent_urls: Vec<String>,
    pub dry_run: bool,
    /// Local image paths to attach (jpg/png/gif/webp). Max 4 for OpenCLI.
    pub media: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    Rejected { plan: PublishPlan },
    DryRun { plan: PublishPlan },
    Sent { plan: PublishPlan, receipt: String },
}

/// A like / retweet / quote engagement on one existing tweet, aimed at
/// its status URL. Quote carries the user's own text (and ≤1 image).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngageAction {
    Like,
    Unlike,
    Retweet,
    Unretweet,
    Quote,
}

/// Burst label for a verified engagement, or None for no burst.
/// Posts/replies/quotes and Retweet all read "\u{2713} Posted"
/// (Jeremy's wording: a retweet posts). Like reads "\u{2713} Liked".
/// Removals (unlike/unretweet) get no burst — taking a post away is
/// not a post; the state flip alone confirms them.
pub fn engage_burst_label(action: EngageAction) -> Option<&'static str> {
    match action {
        EngageAction::Like => Some("\u{2713} Liked"),
        EngageAction::Retweet => Some("\u{2713} Posted"),
        EngageAction::Unlike | EngageAction::Unretweet | EngageAction::Quote => None,
    }
}

impl EngageAction {
    /// The `opencli twitter <verb>` subcommand (verified against
    /// OpenCLI 1.8.8 `--help`: like/unlike/retweet/unretweet/quote all
    /// take the tweet URL first; quote takes text second).
    pub fn opencli_verb(self) -> &'static str {
        match self {
            EngageAction::Like => "like",
            EngageAction::Unlike => "unlike",
            EngageAction::Retweet => "retweet",
            EngageAction::Unretweet => "unretweet",
            EngageAction::Quote => "quote",
        }
    }
}

pub trait Publisher: Send + Sync {
    fn id(&self) -> ProviderId;
    fn available(&self) -> bool;
    fn session_note(&self) -> Option<String>;
    /// Engage with an existing tweet. Default: this posting path cannot
    /// (web-intent/mock), and the UI must say so instead of faking it.
    fn engage(
        &self,
        _action: EngageAction,
        _target_url: &str,
        _text: Option<&str>,
        _media: &[PathBuf],
    ) -> Result<String, String> {
        Err("this posting path cannot like / retweet / quote".into())
    }
    /// Follow or unfollow an account by handle. Default: this posting
    /// path cannot (web-intent/mock) — the UI must say so, not fake it.
    fn set_follow(&self, _handle: &str, _follow: bool) -> Result<String, String> {
        Err("this posting path cannot follow / unfollow".into())
    }
    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String>;
    /// True when this publisher can upload the given media paths itself.
    fn supports_media(&self) -> bool {
        false
    }
}

pub struct IntentPublisher {
    pub open_browser: bool,
    pub opener: std::sync::Arc<dyn crate::io::Opener>,
}

impl Publisher for IntentPublisher {
    fn id(&self) -> ProviderId {
        ProviderId::WebIntent
    }

    fn available(&self) -> bool {
        true
    }

    fn session_note(&self) -> Option<String> {
        Some("you click Post in the browser".to_string())
    }

    fn publish(
        &self,
        parts: &[String],
        reply_to: Option<&str>,
        media: &[PathBuf],
    ) -> Result<String, String> {
        let urls = intent_urls(parts, reply_to);
        if self.open_browser
            && let Some(url) = urls.first()
        {
            self.opener.open(url)?;
        }
        if media.is_empty() {
            Ok(format!("opened web intent for {} part(s)", parts.len()))
        } else {
            let list = media
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "opened web intent for {} part(s). Attach these images in the browser compose UI (web intent cannot upload files): {list}",
                parts.len()
            ))
        }
    }
}

/// X allows at most 4 images per post.
pub const MAX_MEDIA: usize = 4;
/// X photo limit (jpg/png/webp); GIFs may be larger.
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_GIF_BYTES: u64 = 15 * 1024 * 1024;

/// Validate one image path: exists, supported type, within X size limits.
/// Friendly, dummy-proof errors (no panics, no raw OS errors leaked verbatim).
pub fn validate_media_path(path: &std::path::Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!(
            "image not found: {} — pick a file that exists",
            path.display()
        ));
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") {
        return Err(format!(
            "unsupported image {}: use jpg, png, gif, or webp",
            path.display()
        ));
    }
    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let limit = if ext == "gif" {
        MAX_GIF_BYTES
    } else {
        MAX_IMAGE_BYTES
    };
    if size > limit {
        return Err(format!(
            "image too large: {} is {:.1} MB — X allows at most {} MB for .{ext}",
            path.display(),
            size as f64 / 1_048_576.0,
            limit / 1_048_576
        ));
    }
    if size == 0 {
        return Err(format!("image is empty: {}", path.display()));
    }
    Ok(())
}

/// Validate a whole attachment list (count + every file).
pub fn validate_media_paths(media: &[PathBuf]) -> Result<(), String> {
    if media.len() > MAX_MEDIA {
        return Err(format!(
            "at most {MAX_MEDIA} images per post (you picked {})",
            media.len()
        ));
    }
    for path in media {
        validate_media_path(path)?;
    }
    Ok(())
}

/// The text of a quote post published through the plain post path:
/// the user's own words plus the source tweet's URL on its own line.
/// X renders a bare status URL in a post as a native quote embed, so
/// this IS a quote tweet — it just avoids OpenCLI's `twitter quote`,
/// whose compose-page card check fails against current x.com (the
/// `compose/post?url=…` page no longer renders the quoted card, so the
/// command aborts with "Quote target did not render" and posts nothing;
/// verified live 2026-10-19 against OpenCLI 1.8.8, whose quote.js is
/// byte-identical to upstream 2.0.0).
pub fn quote_post_text(user_text: &str, target_url: &str) -> String {
    let text = user_text.trim();
    let url = target_url.trim();
    if text.is_empty() || url.is_empty() || text.contains(url) {
        return text.to_string();
    }
    format!("{text}\n{url}")
}

/// Turn a raw provider failure (OpenCLI JSON dump, stderr, exit-code
/// prose) into ONE short human sentence for the panel. Raw text must
/// never reach the UI: it goes to the log file instead (see
/// `providers::log_raw_provider_error`). The result never contains
/// JSON, `ok:false`, `command_exec`, or exit codes.
pub fn humanize_error(raw: &str) -> String {
    let extracted = extract_error_message(raw);
    let msg = extracted.as_deref().unwrap_or(raw).trim();
    let collapsed = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = collapsed.to_lowercase();
    let mapped: &str =
        if lower.contains("did not render") || lower.contains("deleted or restricted") {
            "X didn't attach the quoted tweet."
        } else if lower.contains("could not find the") && lower.contains("button") {
            "X didn't show that button on the tweet."
        } else if lower.contains("429") || (lower.contains("rate") && lower.contains("limit")) {
            "X is rate-limiting right now. Wait a few minutes and try again."
        } else if lower.contains("not logged")
            || lower.contains("are you logged in")
            || lower.contains("login required")
            || lower.contains("not signed in")
        {
            "X isn't signed in right now."
        } else if lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("did not complete")
            || lower.contains("did not confirm")
        {
            "X took too long to confirm that. Check your profile before retrying."
        } else if lower.contains("empty") && lower.contains("text") {
            "Write something first."
        } else if collapsed.is_empty()
            || collapsed.len() > 160
            || collapsed.contains('{')
            || collapsed.contains('}')
            || lower.contains("ok:false")
            || lower.contains("ok: false")
            || lower.contains("command_exec")
            || lower.contains("exitcode")
            || lower.contains("exit code")
            || lower.starts_with("opencli exited")
            || lower.starts_with("twitter exited")
        {
            "X couldn't do that right now."
        } else {
            // A short plain message from the provider is fine as-is.
            return ensure_sentence(&collapsed);
        };
    mapped.to_string()
}

fn ensure_sentence(text: &str) -> String {
    let t = text.trim().trim_end_matches(['.', '!', '?']);
    format!("{t}.")
}

/// Pull `error.message` / `message` out of an OpenCLI JSON error dump;
/// None when the text is not (or does not contain) such JSON.
fn extract_error_message(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&raw[start..=end]).ok()?;
    let error = value.get("error");
    if let Some(message) = error
        .and_then(|e| e.get("message"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(message.to_string());
    }
    if let Some(message) = error.and_then(serde_json::Value::as_str) {
        return Some(message.to_string());
    }
    value
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

pub fn intent_urls(parts: &[String], reply_to: Option<&str>) -> Vec<String> {
    parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let reply = if index == 0 { reply_to } else { None };
            intent_url(part, reply)
        })
        .collect()
}

pub fn build_plan(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
) -> PublishPlan {
    build_plan_limited(
        text,
        reply_to,
        publisher,
        dry_run,
        crate::text::STANDARD_CHAR_LIMIT,
    )
}

pub fn build_plan_limited(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
    char_limit: usize,
) -> PublishPlan {
    build_plan_with_media(text, reply_to, publisher, dry_run, char_limit, &[])
}

pub fn build_plan_with_media(
    text: &str,
    reply_to: Option<&str>,
    publisher: &dyn Publisher,
    dry_run: bool,
    char_limit: usize,
    media: &[PathBuf],
) -> PublishPlan {
    let parts = crate::text::split_thread_limited(text, char_limit);
    let parts = if parts.is_empty() {
        vec![String::new()]
    } else {
        parts
    };
    let media = media.to_vec();
    let session = match (
        publisher.session_note(),
        media.is_empty(),
        publisher.supports_media(),
    ) {
        (Some(note), false, false) => Some(format!(
            "{note}. Media will need a manual attach in the browser if this path cannot upload files."
        )),
        (Some(note), _, _) => Some(note),
        (None, false, false) => {
            Some("media listed below; provider may require manual attach in the browser".into())
        }
        _ => None,
    };
    PublishPlan {
        count: crate::text::weighted_len_limited(text.trim(), char_limit),
        intent_urls: intent_urls(&parts, reply_to),
        text: text.trim().to_string(),
        parts,
        provider: publisher.id(),
        session,
        dry_run,
        media,
    }
}

/// Publish only after an explicit yes. Dry-run never calls the publisher.
pub fn confirm_and_publish(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
) -> GateOutcome {
    confirm_and_publish_limited(
        text,
        reply_to,
        decision,
        dry_run,
        publisher,
        crate::text::STANDARD_CHAR_LIMIT,
    )
}

pub fn confirm_and_publish_limited(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
    char_limit: usize,
) -> GateOutcome {
    confirm_and_publish_with_media(
        text,
        reply_to,
        decision,
        dry_run,
        publisher,
        char_limit,
        &[],
    )
}

pub fn confirm_and_publish_with_media(
    text: &str,
    reply_to: Option<&str>,
    decision: ConfirmDecision,
    dry_run: bool,
    publisher: &dyn Publisher,
    char_limit: usize,
    media: &[PathBuf],
) -> GateOutcome {
    let plan = build_plan_with_media(text, reply_to, publisher, dry_run, char_limit, media);
    if decision != ConfirmDecision::Yes {
        return GateOutcome::Rejected { plan };
    }
    if dry_run {
        return GateOutcome::DryRun { plan };
    }
    match publisher.publish(&plan.parts, reply_to, &plan.media) {
        Ok(receipt) => GateOutcome::Sent { plan, receipt },
        Err(error) => GateOutcome::Rejected {
            plan: PublishPlan {
                session: Some(error),
                ..plan
            },
        },
    }
}

pub fn first_available(publishers: &[Box<dyn Publisher>]) -> Option<&dyn Publisher> {
    publishers
        .iter()
        .find(|publisher| publisher.available())
        .map(|publisher| publisher.as_ref())
}

pub fn session_label(provider: ProviderId) -> Option<String> {
    match provider {
        ProviderId::OpenCli | ProviderId::TwitterCli => Some(SESSION_OWN.to_string()),
        ProviderId::WebIntent => Some("you click Post in the browser".to_string()),
        ProviderId::Mock | ProviderId::OfficialApi => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::percent_encode;
    use std::sync::{Arc, Mutex};

    struct RecordingPublisher {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        media_calls: Arc<Mutex<Vec<Vec<PathBuf>>>>,
    }

    impl Publisher for RecordingPublisher {
        fn id(&self) -> ProviderId {
            ProviderId::Mock
        }
        fn available(&self) -> bool {
            true
        }
        fn session_note(&self) -> Option<String> {
            None
        }
        fn publish(
            &self,
            parts: &[String],
            _reply_to: Option<&str>,
            media: &[PathBuf],
        ) -> Result<String, String> {
            self.calls.lock().expect("calls").push(parts.to_vec());
            self.media_calls.lock().expect("media").push(media.to_vec());
            Ok("posted".to_string())
        }
    }

    #[test]
    fn no_and_dry_run_never_publish() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            calls: calls.clone(),
            media_calls: Arc::new(Mutex::new(Vec::new())),
        };
        let text = "exact post text";
        let rejected = confirm_and_publish(text, None, ConfirmDecision::No, false, &publisher);
        assert!(matches!(rejected, GateOutcome::Rejected { .. }));
        let dry = confirm_and_publish(text, None, ConfirmDecision::Yes, true, &publisher);
        match dry {
            GateOutcome::DryRun { plan } => {
                assert_eq!(plan.text, text);
                assert_eq!(plan.count.weighted, text.len());
                assert!(plan.dry_run);
            }
            other => panic!("expected dry run, got {other:?}"),
        }
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn yes_without_dry_run_publishes_the_exact_text() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            calls: calls.clone(),
            media_calls: Arc::new(Mutex::new(Vec::new())),
        };
        let outcome = confirm_and_publish(
            "ship it",
            Some("55"),
            ConfirmDecision::Yes,
            false,
            &publisher,
        );
        assert!(matches!(outcome, GateOutcome::Sent { .. }));
        assert_eq!(
            calls.lock().expect("calls").clone(),
            vec![vec!["ship it".to_string()]]
        );
    }

    #[test]
    fn media_reaches_publisher_on_yes_and_never_on_dry_run() {
        let dir = std::env::temp_dir().join(format!("termyx-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let img = dir.join("pic.png");
        std::fs::write(&img, b"\x89PNG fake but exists").expect("write img");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let media_calls = Arc::new(Mutex::new(Vec::new()));
        let publisher = RecordingPublisher {
            calls,
            media_calls: media_calls.clone(),
        };
        // Dry-run: publisher must not be called at all.
        let dry = confirm_and_publish_with_media(
            "hi",
            None,
            ConfirmDecision::Yes,
            true,
            &publisher,
            crate::text::STANDARD_CHAR_LIMIT,
            std::slice::from_ref(&img),
        );
        assert!(matches!(dry, GateOutcome::DryRun { plan } if plan.media == vec![img.clone()]));
        assert!(media_calls.lock().expect("m").is_empty());
        // Real yes: media must arrive at the publisher exactly once.
        let sent = confirm_and_publish_with_media(
            "hi",
            None,
            ConfirmDecision::Yes,
            false,
            &publisher,
            crate::text::STANDARD_CHAR_LIMIT,
            std::slice::from_ref(&img),
        );
        assert!(matches!(sent, GateOutcome::Sent { .. }));
        assert_eq!(media_calls.lock().expect("m").clone(), vec![vec![img]]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_media_rejects_missing_bad_ext_too_many_and_oversize() {
        assert!(validate_media_paths(&[]).is_ok());
        assert!(
            validate_media_path(std::path::Path::new("/no/such/pic.png"))
                .unwrap_err()
                .contains("not found")
        );
        let dir = std::env::temp_dir().join(format!("termyx-val-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp");
        let bad = dir.join("x.bmp");
        std::fs::write(&bad, b"bm").expect("w");
        assert!(
            validate_media_path(&bad)
                .unwrap_err()
                .contains("jpg, png, gif")
        );
        let big = dir.join("big.png");
        std::fs::File::create(&big)
            .expect("f")
            .set_len(MAX_IMAGE_BYTES + 1)
            .expect("len");
        assert!(validate_media_path(&big).unwrap_err().contains("too large"));
        let ok = dir.join("ok.jpg");
        std::fs::write(&ok, b"jpg").expect("w");
        assert!(
            validate_media_paths(&vec![ok.clone(); 5])
                .unwrap_err()
                .contains("at most 4")
        );
        assert!(validate_media_paths(&[ok]).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn burst_labels_cover_posts_and_likes_but_not_removals() {
        assert_eq!(
            engage_burst_label(EngageAction::Like),
            Some("\u{2713} Liked")
        );
        assert_eq!(
            engage_burst_label(EngageAction::Retweet),
            Some("\u{2713} Posted")
        );
        assert_eq!(engage_burst_label(EngageAction::Unlike), None);
        assert_eq!(engage_burst_label(EngageAction::Unretweet), None);
    }

    #[test]
    fn quote_post_text_appends_source_url_once() {
        let url = "https://x.com/jack/status/20";
        assert_eq!(quote_post_text("my words", url), format!("my words\n{url}"));
        // Already containing the URL: never duplicated.
        let with = format!("look {url}");
        assert_eq!(quote_post_text(&with, url), with);
        assert_eq!(quote_post_text("  padded  ", url), format!("padded\n{url}"));
        assert_eq!(quote_post_text("", url), "");
    }

    #[test]
    fn humanize_error_strips_opencli_json_dumps() {
        let raw = r#"{"ok":false,"error":{"code":"command_exec","message":"Quote target did not render in the composer. The source tweet may be deleted or restricted.","help":"Nothing was posted. Open the tweet in the browser and retry."},"exitcode":1}"#;
        let clean = humanize_error(raw);
        assert_eq!(clean, "X didn't attach the quoted tweet.");
        for banned in ["ok:false", "command_exec", "exitcode", "{", "}"] {
            assert!(!clean.contains(banned), "leaked {banned}: {clean}");
        }
        assert_eq!(
            humanize_error("opencli exited 1"),
            "X couldn't do that right now."
        );
        assert_eq!(
            humanize_error(
                "Could not find the Like button on this tweet after waiting 10 seconds. Nothing changed."
            ),
            "X didn't show that button on the tweet."
        );
        assert_eq!(
            humanize_error("HTTP 429 too many requests"),
            "X is rate-limiting right now. Wait a few minutes and try again."
        );
        // A short plain provider sentence survives (gains its period).
        assert_eq!(
            humanize_error("write your quote first"),
            "write your quote first."
        );
    }

    #[test]
    fn intent_urls_encode_spaces_newlines_and_reply_ids() {
        let url = crate::text::intent_url("hello world\nnext & more", Some("99 1"));
        assert_eq!(
            url,
            "https://x.com/intent/post?text=hello%20world%0Anext%20%26%20more&in_reply_to=99%201"
        );
        assert_eq!(percent_encode("a b"), "a%20b");
        assert!(!url.contains('+'));
        let urls = intent_urls(&["one".into(), "two".into()], Some("7"));
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("in_reply_to=7"));
        assert!(!urls[1].contains("in_reply_to"));
    }
}
