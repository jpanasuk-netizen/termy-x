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

/// A user-facing write. Quote in the provider API is the Q-tweet action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserWrite {
    Like,
    Unlike,
    Retweet,
    Unretweet,
    QTweet,
    Reply,
    Follow,
    Unfollow,
    Post,
}

/// What happened to that write. Only `Confirmed` may use a success note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    Confirmed,
    Failed,
    Cancelled,
    Draft,
    Unsupported,
}

const USER_WRITES: [UserWrite; 9] = [
    UserWrite::Like,
    UserWrite::Unlike,
    UserWrite::Retweet,
    UserWrite::Unretweet,
    UserWrite::QTweet,
    UserWrite::Reply,
    UserWrite::Follow,
    UserWrite::Unfollow,
    UserWrite::Post,
];

const WITHHELD_OUTCOMES: [WriteOutcome; 4] = [
    WriteOutcome::Failed,
    WriteOutcome::Cancelled,
    WriteOutcome::Draft,
    WriteOutcome::Unsupported,
];

/// Success note after the provider confirms the write. Each action has
/// its own text. A Q-tweet says Q-tweet, not the generic post note.
pub fn success_note(action: UserWrite) -> &'static str {
    match action {
        UserWrite::Like => "\u{2713} Liked",
        UserWrite::Unlike => "\u{2713} Unliked",
        UserWrite::Retweet => "\u{2713} Retweeted",
        UserWrite::Unretweet => "\u{2713} Unretweeted",
        UserWrite::QTweet => "\u{2713} Q-tweet",
        UserWrite::Reply => "\u{2713} Reply sent",
        UserWrite::Follow => "\u{2713} Followed",
        UserWrite::Unfollow => "\u{2713} Unfollowed",
        UserWrite::Post => "\u{2713} Posted",
    }
}

/// Note for every outcome that is not a confirmed write. None of these
/// claim the action succeeded.
pub fn withheld_note(action: UserWrite, outcome: WriteOutcome) -> &'static str {
    match outcome {
        WriteOutcome::Confirmed => success_note(action),
        WriteOutcome::Cancelled => match action {
            UserWrite::Follow | UserWrite::Unfollow => "Follow cancelled. Nothing was changed.",
            _ => "Cancelled. Nothing was posted.",
        },
        WriteOutcome::Draft => "Draft only. Nothing was posted.",
        WriteOutcome::Unsupported => match action {
            UserWrite::Follow | UserWrite::Unfollow => {
                "This posting path cannot follow or unfollow. Nothing was changed."
            }
            UserWrite::Like | UserWrite::Unlike | UserWrite::Retweet | UserWrite::Unretweet => {
                "This posting path cannot like, unlike, retweet, or quote. Nothing changed on X."
            }
            _ => "Preview/mock never posts. Nothing was posted.",
        },
        WriteOutcome::Failed => match action {
            UserWrite::QTweet => "Could not send the Q-tweet. Nothing was posted.",
            UserWrite::Reply => "Could not reply. Nothing was posted.",
            UserWrite::Post => "Could not post. Nothing was posted.",
            UserWrite::Follow | UserWrite::Unfollow => {
                "That did not go through. Nothing changed on X."
            }
            _ => "That did not go through. Nothing changed on X.",
        },
    }
}

pub fn user_writes() -> &'static [UserWrite] {
    &USER_WRITES
}

pub fn withheld_outcomes() -> &'static [WriteOutcome] {
    &WITHHELD_OUTCOMES
}

/// Card burst after like, unlike, retweet, or unretweet. A Q-tweet uses
/// [`success_note`] on the composer instead, so this stays empty for Quote
/// and does not also show the generic post note.
pub fn engage_burst_label(action: EngageAction) -> Option<&'static str> {
    match action {
        EngageAction::Like => Some(success_note(UserWrite::Like)),
        EngageAction::Unlike => Some(success_note(UserWrite::Unlike)),
        EngageAction::Retweet => Some(success_note(UserWrite::Retweet)),
        EngageAction::Unretweet => Some(success_note(UserWrite::Unretweet)),
        EngageAction::Quote => None,
    }
}

/// Shown on the reply composer only after the provider accepts the reply.
pub fn reply_sent_label() -> &'static str {
    success_note(UserWrite::Reply)
}

/// Shown on the author's card only after follow or unfollow is confirmed.
pub fn follow_burst_label(follow: bool) -> &'static str {
    success_note(if follow {
        UserWrite::Follow
    } else {
        UserWrite::Unfollow
    })
}

/// Button and heading for the quote composer. The provider verb stays `quote`.
pub fn quote_action_label() -> &'static str {
    "Q-tweet"
}

/// Actions under a tweet. Reply is the one comment control. Video cards
/// use this same list; they do not drop Reply.
pub fn card_action_labels(show_follow: bool) -> Vec<&'static str> {
    let mut labels = vec!["Like", "Retweet", quote_action_label(), "Reply"];
    if show_follow {
        labels.push("Follow");
    }
    labels
}

/// Comment/reply on a repost uses the original id. On a quote it uses
/// the quote tweet itself, not the quoted post.
pub fn comment_target_id(post: &crate::model::Post) -> &str {
    post.engage_target().id.as_str()
}

/// Retweet uses the same id as other engagement: the original post, not
/// the repost wrapper.
pub fn retweet_target_id(post: &crate::model::Post) -> &str {
    post.engage_target().id.as_str()
}

/// A timeline "New tweet" is a standalone post. It never carries a reply id.
pub fn standalone_tweet(text: &str) -> Result<String, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a tweet before Post.");
    }
    Ok(text.to_string())
}

pub fn standalone_reply_to() -> Option<&'static str> {
    None
}

/// Posted text is confirmed only when the provider JSON includes a status id.
pub fn prove_new_post(stdout: &str) -> Result<String, String> {
    let uncertain = "Post was not confirmed. Not marked Posted. Check X. Nothing was retried.";
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed.to_ascii_lowercase().contains("unconfirmed") {
        return Err(uncertain.into());
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Err(uncertain.into());
    };
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) || value.get("error").is_some()
    {
        return Err("Post was rejected. Not marked Posted. Nothing was retried.".into());
    }
    let node = value
        .get("data")
        .or_else(|| value.as_array().and_then(|rows| rows.first()))
        .unwrap_or(&value);
    let id = ["id", "id_str", "rest_id"]
        .iter()
        .find_map(|key| {
            node.get(*key)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| node.get(*key).and_then(serde_json::Value::as_u64).map(|n| n.to_string()))
        });
    match id {
        Some(id) if !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()) => {
            Ok(format!("Posted {id}"))
        }
        _ => Err(uncertain.into()),
    }
}

/// A retweet is confirmed only by a structured receipt. Exit 0, empty
/// output, and `{"ok":true}` with no retweet id are not enough. Nothing
/// here retries the write.
pub fn prove_engage(
    action: EngageAction,
    stdout: &str,
    expected_account: Option<&str>,
) -> Result<(), String> {
    if !matches!(action, EngageAction::Retweet | EngageAction::Unretweet) {
        return Ok(());
    }
    let verb = if action == EngageAction::Retweet {
        "Retweet"
    } else {
        "Unretweet"
    };
    let uncertain = format!(
        "{verb} was not confirmed. Not marked {verb}ed. Check the post on X. Nothing was retried."
    );
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(uncertain);
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("unconfirmed") || lower.contains("check before") {
        return Err(uncertain);
    }
    let value: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(_) => return Err(uncertain),
    };
    if json_says_failure(&value) {
        return Err(format!(
            "{verb} was rejected by X. Not marked {verb}ed. Nothing was retried."
        ));
    }
    if let (Some(actor), Some(expected)) = (actor_handle(&value), expected_account) {
        let expected = crate::model::strip_handle(expected);
        if !expected.is_empty() && !actor.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "{verb} ran as @{actor}, not @{expected}. Not marked {verb}ed. Nothing was retried."
            ));
        }
    }
    if engage_receipt_present(action, &value) {
        Ok(())
    } else {
        Err(uncertain)
    }
}

fn json_says_failure(value: &serde_json::Value) -> bool {
    let node = value
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(value);
    if node.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        return true;
    }
    node.get("error").is_some()
        || matches!(
            node.get("status").and_then(serde_json::Value::as_str),
            Some("error" | "failed" | "failure")
        )
}

fn engage_receipt_present(action: EngageAction, value: &serde_json::Value) -> bool {
    let node = value
        .get("data")
        .or_else(|| value.as_array().and_then(|rows| rows.first()))
        .unwrap_or(value);
    let flag = node
        .get("retweeted")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            node.get("data")
                .and_then(|inner| inner.get("retweeted"))
                .and_then(serde_json::Value::as_bool)
        });
    let want = action == EngageAction::Retweet;
    if flag == Some(!want) {
        return false;
    }
    if flag == Some(want) && json_has_id(node) {
        return true;
    }
    if flag.is_some() {
        return false;
    }
    let status = node
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let ok = node.get("ok").and_then(serde_json::Value::as_bool) == Some(true)
        || status.eq_ignore_ascii_case("success");
    ok && json_has_id(node)
}

fn json_has_id(node: &serde_json::Value) -> bool {
    for key in ["id", "rest_id", "tweet_id", "id_str"] {
        if let Some(value) = node.get(key).and_then(serde_json::Value::as_str) {
            if value.chars().all(|ch| ch.is_ascii_digit()) && !value.is_empty() {
                return true;
            }
        }
        if node.get(key).and_then(serde_json::Value::as_u64).is_some() {
            return true;
        }
    }
    false
}

fn actor_handle(value: &serde_json::Value) -> Option<String> {
    let node = value
        .get("data")
        .or_else(|| value.as_array().and_then(|rows| rows.first()))
        .unwrap_or(value);
    for key in ["screen_name", "username", "handle"] {
        if let Some(text) = node.get(key).and_then(serde_json::Value::as_str) {
            let handle = crate::model::strip_handle(text);
            if !handle.is_empty() {
                return Some(handle.to_string());
            }
        }
    }
    let user = node.get("user").or_else(|| node.get("author"))?;
    for key in ["screen_name", "username", "handle"] {
        if let Some(text) = user.get(key).and_then(serde_json::Value::as_str) {
            let handle = crate::model::strip_handle(text);
            if !handle.is_empty() {
                return Some(handle.to_string());
            }
        }
    }
    None
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
    /// OpenCLI's `twitter follow|unfollow` is the only path that writes
    /// a follow. Other publishers must stay false so the UI does not
    /// pick them and then report a fake or impossible success.
    fn supports_follow(&self) -> bool {
        false
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

/// The first publisher that can actually follow. Posting order is not
/// enough: a web-intent or official publisher may be available and still
/// refuse follows.
pub fn first_that_follows(publishers: &[Box<dyn Publisher>]) -> Option<&dyn Publisher> {
    publishers
        .iter()
        .find(|publisher| publisher.available() && publisher.supports_follow())
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
            Some("\u{2713} Retweeted")
        );
        assert_eq!(
            engage_burst_label(EngageAction::Unretweet),
            Some("\u{2713} Unretweeted")
        );
        assert_eq!(
            engage_burst_label(EngageAction::Unlike),
            Some("\u{2713} Unliked")
        );
        assert_eq!(engage_burst_label(EngageAction::Quote), None);
        assert_ne!(
            engage_burst_label(EngageAction::Unretweet),
            engage_burst_label(EngageAction::Retweet)
        );
        assert_eq!(reply_sent_label(), "\u{2713} Reply sent");
        assert_eq!(follow_burst_label(true), "\u{2713} Followed");
        assert_eq!(follow_burst_label(false), "\u{2713} Unfollowed");
        assert_ne!(follow_burst_label(false), follow_burst_label(true));
        assert_eq!(quote_action_label(), "Q-tweet");
        assert!(!quote_action_label().to_ascii_lowercase().contains("quote retweet"));
        let with_follow = card_action_labels(true);
        let text_only = card_action_labels(false);
        assert_eq!(
            with_follow,
            ["Like", "Retweet", "Q-tweet", "Reply", "Follow"]
        );
        assert_eq!(text_only, ["Like", "Retweet", "Q-tweet", "Reply"]);
        assert_eq!(
            with_follow.iter().filter(|label| **label == "Reply" || **label == "Comment").count(),
            1
        );
        let mut repost = crate::model::Post::default();
        repost.id = "wrap".into();
        let mut original = crate::model::Post::default();
        original.id = "orig".into();
        original.media.push(crate::model::MediaItem {
            kind: crate::model::MediaKind::Video,
            url: "https://video.twimg.com/a.mp4".into(),
            poster: None,
            width: None,
            height: None,
        });
        repost.retweeted = Some(Box::new(original));
        assert_eq!(comment_target_id(&repost), "orig");
        let mut quote = crate::model::Post::default();
        quote.id = "quote-id".into();
        quote.quoted = Some(Box::new(crate::model::Post {
            id: "inner".into(),
            ..crate::model::Post::default()
        }));
        assert_eq!(comment_target_id(&quote), "quote-id");
        let mut seen = std::collections::BTreeSet::new();
        for action in user_writes() {
            let success = success_note(*action);
            assert!(success.starts_with('\u{2713}'), "{action:?} {success}");
            assert!(seen.insert(success), "duplicate success note {success}");
            if *action == UserWrite::QTweet {
                assert!(success.contains(quote_action_label()));
                assert_ne!(success, success_note(UserWrite::Post));
            }
            for outcome in withheld_outcomes() {
                let withheld = withheld_note(*action, *outcome);
                assert!(!withheld.contains('\u{2713}'), "{action:?} {outcome:?} {withheld}");
                assert_ne!(withheld, success);
                assert!(!withheld.contains(success), "{action:?} {outcome:?}");
            }
        }
        assert_eq!(
            withheld_note(UserWrite::Follow, WriteOutcome::Cancelled),
            "Follow cancelled. Nothing was changed."
        );
        assert_eq!(
            withheld_note(UserWrite::QTweet, WriteOutcome::Failed),
            "Could not send the Q-tweet. Nothing was posted."
        );
    }

    #[test]
    fn retweet_needs_a_receipt_and_uses_the_original_id() {
        use crate::model::Post;
        let mut wrapper = Post::default();
        wrapper.id = "wrap".into();
        let mut original = Post::default();
        original.id = "555".into();
        wrapper.retweeted = Some(Box::new(original));
        assert_eq!(retweet_target_id(&wrapper), "555");
        assert_ne!(retweet_target_id(&wrapper), "wrap");
        assert_eq!(standalone_tweet("  hello  ").unwrap(), "hello");
        assert!(standalone_tweet("  ").is_err());
        assert!(standalone_reply_to().is_none());
        assert_eq!(
            prove_new_post(r#"{"ok":true,"id":"42"}"#).unwrap(),
            "Posted 42"
        );
        assert!(prove_new_post("").is_err());
        assert!(prove_new_post(r#"{"ok":true}"#).is_err());
        assert!(prove_new_post(r#"{"ok":false,"error":{"message":"no"}}"#).is_err());

        let confirmed = r#"{"ok":true,"retweeted":true,"id":"555","user":{"screen_name":"Jasper_Black"}}"#;
        assert!(prove_engage(EngageAction::Retweet, confirmed, Some("jasper_black")).is_ok());
        assert!(
            prove_engage(EngageAction::Retweet, confirmed, Some("@other"))
                .unwrap_err()
                .contains("not @other")
        );

        for bad in [
            "",
            "   ",
            "unconfirmed — check before retrying",
            "not json",
            r#"{"ok":true}"#,
            r#"{"ok":false,"error":{"message":"nope"}}"#,
            "[]",
        ] {
            let err = prove_engage(EngageAction::Retweet, bad, Some("Jasper_Black")).unwrap_err();
            assert!(err.contains("Not marked"), "{bad} -> {err}");
            assert!(err.contains("Nothing was retried"), "{err}");
            assert!(!err.to_ascii_lowercase().contains("token"));
        }

        let removed = r#"{"ok":true,"retweeted":false,"id":"555"}"#;
        assert!(prove_engage(EngageAction::Unretweet, removed, None).is_ok());
        assert!(prove_engage(EngageAction::Retweet, removed, None).is_err());
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
