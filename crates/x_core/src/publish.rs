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

/// One panel write, driven without the window. Dry-run unless `confirm` is
/// exactly `yes`. A confirmed run calls the publisher once and does not retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiWrite {
    Post { text: String },
    Reply { target_id: String, text: String },
    Quote { target_id: String, text: String },
    Repost { target_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiWriteRequest {
    pub write: GuiWrite,
    /// Only `yes` performs the provider call.
    pub confirm: Option<String>,
    pub account: String,
    pub char_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuiWriteOutcome {
    DryRun {
        provider: ProviderId,
        detail: String,
    },
    Sent {
        provider: ProviderId,
        receipt: String,
    },
    Rejected {
        message: String,
    },
}

impl GuiWriteOutcome {
    pub fn saved_a_row(&self) -> bool {
        matches!(self, Self::Sent { .. })
    }
}

pub fn run_gui_write(
    publisher: &dyn Publisher,
    db: &mut crate::action_state::ActionDb,
    request: &GuiWriteRequest,
    now: u64,
    seq: u64,
) -> GuiWriteOutcome {
    let approved = request.confirm.as_deref() == Some("yes");
    let prepared = match prepare_gui_write(&request.write) {
        Ok(prepared) => prepared,
        Err(message) => return GuiWriteOutcome::Rejected { message },
    };
    if !approved {
        return GuiWriteOutcome::DryRun {
            provider: publisher.id(),
            detail: prepared.detail(publisher.id()),
        };
    }
    if request.account.trim().is_empty() {
        return GuiWriteOutcome::Rejected {
            message: "No connected account is saved. Nothing was sent.".into(),
        };
    }
    match prepared.kind {
        PreparedKind::Text { reply_to } => {
            let text = &prepared.text;
            match confirm_and_publish_with_media(
                text,
                reply_to.as_deref(),
                ConfirmDecision::Yes,
                false,
                publisher,
                request.char_limit,
                &[],
            ) {
                GateOutcome::Sent { receipt, .. } => {
                    let kind = if reply_to.is_some() {
                        crate::action_state::ReceiptKind::Reply
                    } else {
                        crate::action_state::ReceiptKind::Post
                    };
                    db.record_receipt(
                        &request.account,
                        kind,
                        reply_to.as_deref().unwrap_or(""),
                        &receipt,
                        seq,
                        now,
                    );
                    GuiWriteOutcome::Sent {
                        provider: publisher.id(),
                        receipt,
                    }
                }
                GateOutcome::Rejected { plan } => GuiWriteOutcome::Rejected {
                    message: plan.session.unwrap_or_else(|| {
                        "The write was rejected. Nothing was retried.".into()
                    }),
                },
                GateOutcome::DryRun { .. } => GuiWriteOutcome::Rejected {
                    message: "The write stayed a dry run. Nothing was sent.".into(),
                },
            }
        }
        PreparedKind::Quote { url, target_id } => {
            match publisher.engage(EngageAction::Quote, &url, Some(&prepared.text), &[]) {
                Ok(receipt) => match prove_quote(&receipt, &target_id) {
                    Ok(()) => {
                        let receipt = quote_saved_receipt(&target_id);
                        db.record_receipt(
                            &request.account,
                            crate::action_state::ReceiptKind::QTweet,
                            &target_id,
                            &receipt,
                            seq,
                            now,
                        );
                        GuiWriteOutcome::Sent {
                            provider: publisher.id(),
                            receipt,
                        }
                    }
                    Err(error) => GuiWriteOutcome::Rejected { message: error },
                },
                Err(error) => GuiWriteOutcome::Rejected {
                    message: format!("{error} Nothing was retried."),
                },
            }
        }
        PreparedKind::Repost { url, target_id } => {
            db.begin_toggle(
                &request.account,
                crate::action_state::ToggleKind::Repost,
                &target_id,
                true,
                seq,
                now,
            );
            match publisher.engage(EngageAction::Retweet, &url, None, &[]) {
                Ok(receipt) => {
                    if let Err(error) = prove_engage(EngageAction::Retweet, &receipt, None) {
                        let _ = db.finish_toggle(
                            &request.account,
                            crate::action_state::ToggleKind::Repost,
                            &target_id,
                            seq,
                            false,
                            Some(&error),
                            now,
                        );
                        return GuiWriteOutcome::Rejected { message: error };
                    }
                    let _ = db.finish_toggle(
                        &request.account,
                        crate::action_state::ToggleKind::Repost,
                        &target_id,
                        seq,
                        true,
                        None,
                        now,
                    );
                    db.note_opencli_contract(
                        &request.account,
                        crate::action_state::ToggleKind::Repost,
                        &target_id,
                    );
                    GuiWriteOutcome::Sent {
                        provider: publisher.id(),
                        receipt,
                    }
                }
                Err(error) => {
                    let _ = db.finish_toggle(
                        &request.account,
                        crate::action_state::ToggleKind::Repost,
                        &target_id,
                        seq,
                        false,
                        Some(&error),
                        now,
                    );
                    GuiWriteOutcome::Rejected {
                        message: format!("{error} Nothing was retried."),
                    }
                }
            }
        }
    }
}

enum PreparedKind {
    Text { reply_to: Option<String> },
    Quote { url: String, target_id: String },
    Repost { url: String, target_id: String },
}

struct PreparedWrite {
    kind: PreparedKind,
    text: String,
}

impl PreparedWrite {
    fn detail(&self, provider: ProviderId) -> String {
        match &self.kind {
            PreparedKind::Text { reply_to } => format!(
                "provider {} · reply_to {} · text: {}",
                provider.as_str(),
                reply_to.as_deref().unwrap_or("none"),
                self.text
            ),
            PreparedKind::Quote { url, target_id } => format!(
                "provider {} · quote menu on {url} · original {target_id} · commentary: {}",
                provider.as_str(),
                self.text
            ),
            PreparedKind::Repost { url, target_id } => format!(
                "provider {} · retweet {} · url {}",
                provider.as_str(),
                target_id,
                url
            ),
        }
    }
}

fn prepare_gui_write(write: &GuiWrite) -> Result<PreparedWrite, String> {
    match write {
        GuiWrite::Post { text } => {
            let text = standalone_tweet(text).map_err(|error| error.to_string())?;
            if standalone_reply_to().is_some() {
                return Err("New tweet was blocked because it had a reply id.".into());
            }
            Ok(PreparedWrite {
                kind: PreparedKind::Text { reply_to: None },
                text,
            })
        }
        GuiWrite::Reply { target_id, text } => {
            let target_id = require_status_id(target_id)?;
            let text = text.trim();
            if text.is_empty() {
                return Err("Type a reply before sending. Nothing was sent.".into());
            }
            Ok(PreparedWrite {
                kind: PreparedKind::Text {
                    reply_to: Some(target_id),
                },
                text: text.to_string(),
            })
        }
        GuiWrite::Quote { target_id, text } => {
            let target_id = require_status_id(target_id)?;
            let text = text.trim();
            if text.is_empty() {
                return Err("Write the quote first. Nothing was sent.".into());
            }
            let url = crate::model::post_url("", &target_id);
            Ok(PreparedWrite {
                kind: PreparedKind::Quote { url, target_id },
                text: text.to_string(),
            })
        }
        GuiWrite::Repost { target_id } => {
            let target_id = require_status_id(target_id)?;
            let url = crate::model::post_url("", &target_id);
            Ok(PreparedWrite {
                kind: PreparedKind::Repost { url, target_id },
                text: String::new(),
            })
        }
    }
}

fn require_status_id(input: &str) -> Result<String, String> {
    let id = crate::model::post_id_from_input(input);
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(
            "The target must be a status id or a status URL. Nothing was sent.".into(),
        );
    }
    Ok(id)
}

/// A new post is confirmed by OpenCLI's sent-toast message. An id alone is not.
pub fn prove_new_post(stdout: &str) -> Result<String, String> {
    prove_opencli_write(OpenCliWrite::Post, stdout, None)?;
    Ok(posted_receipt(stdout))
}

pub fn prove_reply(stdout: &str) -> Result<String, String> {
    prove_opencli_write(OpenCliWrite::Reply, stdout, None)?;
    Ok("Reply sent".into())
}

fn posted_receipt(stdout: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        return "Posted".into();
    };
    let node = success_node(&value).unwrap_or(&value);
    for key in ["id", "id_str", "rest_id"] {
        if let Some(text) = node.get(key).and_then(serde_json::Value::as_str) {
            if !text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit()) {
                return format!("Posted {text}");
            }
        }
    }
    "Posted".into()
}

/// Which OpenCLI write the receipt must match. Messages are the ones
/// `clis/twitter/*.js` returns after it sees the button or sent-toast change.
/// Exit 0, `{"ok":true}`, and an id without that message are not receipts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenCliWrite {
    Like,
    Unlike,
    Retweet,
    Unretweet,
    Follow,
    Unfollow,
    Post,
    Reply,
    Quote,
}

impl OpenCliWrite {
    fn label(self) -> &'static str {
        match self {
            Self::Like => "Like",
            Self::Unlike => "Unlike",
            Self::Retweet => "Retweet",
            Self::Unretweet => "Unretweet",
            Self::Follow => "Follow",
            Self::Unfollow => "Unfollow",
            Self::Post => "Post",
            Self::Reply => "Reply",
            Self::Quote => "Quote",
        }
    }

    fn from_engage(action: EngageAction) -> Option<Self> {
        match action {
            EngageAction::Like => Some(Self::Like),
            EngageAction::Unlike => Some(Self::Unlike),
            EngageAction::Retweet => Some(Self::Retweet),
            EngageAction::Unretweet => Some(Self::Unretweet),
            EngageAction::Quote => Some(Self::Quote),
        }
    }
}

pub fn prove_engage(
    action: EngageAction,
    stdout: &str,
    expected_account: Option<&str>,
) -> Result<(), String> {
    let Some(kind) = OpenCliWrite::from_engage(action) else {
        return Ok(());
    };
    prove_opencli_write(kind, stdout, expected_account)
}

/// Confirmed only when OpenCLI's own success row is present.
/// `subject` is the follow handle when the message names one.
pub fn prove_opencli_write(
    kind: OpenCliWrite,
    stdout: &str,
    subject: Option<&str>,
) -> Result<(), String> {
    let label = kind.label();
    let uncertain = format!(
        "{label} was not confirmed. Not marked done. Check X. Nothing was retried."
    );
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(uncertain);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return Err(uncertain);
    };
    if value_flag(&value, "unconfirmed") == Some(true) {
        return Err(uncertain);
    }
    if json_says_failure(&value) {
        return Err(format!(
            "{label} was rejected. Not marked done. Nothing was retried."
        ));
    }
    let Some(node) = success_node(&value) else {
        return Err(uncertain);
    };
    if node.get("unconfirmed").and_then(serde_json::Value::as_bool) == Some(true) {
        return Err(uncertain);
    }
    let status = node
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if !status.eq_ignore_ascii_case("success") {
        return Err(uncertain);
    }
    let Some(message) = node.get("message").and_then(serde_json::Value::as_str) else {
        return Err(uncertain);
    };
    match subject_mismatch(kind, message, subject) {
        SubjectCheck::Mismatch { found, expected } => {
            return Err(format!(
                "{label} names @{found}, not @{expected}. Not marked done. Nothing was retried."
            ));
        }
        SubjectCheck::Matches | SubjectCheck::NotApplicable => {}
    }
    if message_matches(kind, message) {
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

fn value_flag(value: &serde_json::Value, key: &str) -> Option<bool> {
    value.get(key).and_then(serde_json::Value::as_bool)
}

fn success_node(value: &serde_json::Value) -> Option<&serde_json::Value> {
    if let Some(rows) = value.as_array() {
        return rows.first();
    }
    if let Some(data) = value.get("data") {
        if let Some(rows) = data.as_array() {
            return rows.first();
        }
        if data.get("message").is_some() {
            return Some(data);
        }
    }
    if value.get("message").is_some() {
        return Some(value);
    }
    None
}

enum SubjectCheck {
    Matches,
    Mismatch { found: String, expected: String },
    NotApplicable,
}

fn subject_mismatch(kind: OpenCliWrite, message: &str, subject: Option<&str>) -> SubjectCheck {
    if !matches!(kind, OpenCliWrite::Follow | OpenCliWrite::Unfollow) {
        return SubjectCheck::NotApplicable;
    }
    let Some(found) = message_handle(message) else {
        return SubjectCheck::NotApplicable;
    };
    let Some(expected) = subject else {
        return SubjectCheck::Matches;
    };
    let expected = crate::model::strip_handle(expected);
    if expected.is_empty() || found.eq_ignore_ascii_case(expected) {
        SubjectCheck::Matches
    } else {
        SubjectCheck::Mismatch {
            found,
            expected: expected.to_string(),
        }
    }
}

fn message_handle(message: &str) -> Option<String> {
    let at = message.find('@')?;
    let handle: String = message[at + 1..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect();
    if handle.is_empty() { None } else { Some(handle) }
}

fn message_matches(kind: OpenCliWrite, message: &str) -> bool {
    let message = message.trim();
    match kind {
        OpenCliWrite::Like => {
            message == "Tweet successfully liked." || message == "Tweet is already liked."
        }
        OpenCliWrite::Unlike => {
            message == "Tweet successfully unliked."
                || message == "Tweet is not liked (already unliked)."
        }
        OpenCliWrite::Retweet => {
            message == "Tweet successfully retweeted." || message == "Tweet is already retweeted."
        }
        OpenCliWrite::Unretweet => {
            message == "Tweet successfully unretweeted."
                || message == "Tweet is not retweeted (already removed)."
        }
        OpenCliWrite::Follow => {
            let Some(handle) = message_handle(message) else {
                return false;
            };
            message == format!("Successfully followed @{handle}.")
                || message == format!("Already following @{handle}.")
        }
        OpenCliWrite::Unfollow => {
            let Some(handle) = message_handle(message) else {
                return false;
            };
            message == format!("Successfully unfollowed @{handle}.")
                || message.starts_with(&format!("Not following @{handle} "))
        }
        OpenCliWrite::Post => message == "Tweet posted successfully.",
        OpenCliWrite::Reply => message == "Reply posted successfully.",
        OpenCliWrite::Quote => message == "Quote tweet posted successfully.",
    }
}

/// A quote is confirmed only when the receipt names the original status.
/// OpenCLI's `twitter quote` message alone is not enough: that command
/// opens `/compose/post?url=…`, and a missing card link is reported even
/// when the composer itself loaded. A normal post receipt is not a quote.
pub fn prove_quote(stdout: &str, target_id: &str) -> Result<(), String> {
    let target_id = crate::model::post_id_from_input(target_id);
    if target_id.is_empty() || !target_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("Quote target is missing. Not marked sent. Nothing was retried.".into());
    }
    let trimmed = stdout.trim();
    if let Some(message) = first_message(trimmed) {
        if message == "Tweet posted successfully." {
            return Err(format!(
                "Quote of {target_id} was not established. A normal post receipt is not a quote. Not marked sent. Nothing was retried."
            ));
        }
    }
    prove_opencli_write(OpenCliWrite::Quote, trimmed, None)?;
    let Some(found) = quoted_target_id(trimmed) else {
        return Err(format!(
            "Quote of {target_id} was not in the receipt. Not marked sent. Nothing was retried."
        ));
    };
    if found != target_id {
        return Err(format!(
            "Quote names {found}, not {target_id}. Not marked sent. Nothing was retried."
        ));
    }
    if let Some(text) = first_text(trimmed) {
        for id in status_ids_in_text(&text) {
            if id != target_id {
                return Err(format!(
                    "Quote text names {id}, not {target_id}. Not marked sent. Nothing was retried."
                ));
            }
        }
    }
    Ok(())
}

pub fn quote_saved_receipt(target_id: &str) -> String {
    format!("Quoted {}", crate::model::post_id_from_input(target_id))
}

/// How this OpenCLI build names a browser session. 2.0 uses
/// `browser open <url> --session <name>`. 1.8 uses
/// `browser <session> open <url>`. The 241742ac inspect used only the
/// 1.8 shape, so a 2.0 CLI exited before any page opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserCli {
    pub session_flag: bool,
    pub window_flag: bool,
}

pub fn browser_cli_from_help(help: &str) -> BrowserCli {
    let lower = help.to_ascii_lowercase();
    let positional = lower.contains("browser <session>")
        || lower.contains("browser [session]")
        || lower.contains("<session> open")
        || lower.contains("<session> <command>");
    let session_flag = lower.contains("--session") && !positional;
    BrowserCli {
        session_flag: if positional { false } else { session_flag || lower.contains("--session") || help.trim().is_empty() },
        window_flag: lower.contains("--window") || help.trim().is_empty(),
    }
}

pub fn browser_command(
    cli: BrowserCli,
    session: &str,
    command: &str,
    arg: Option<&str>,
) -> Vec<String> {
    let mut args = vec!["browser".to_string()];
    if !cli.session_flag {
        args.push(session.to_string());
    }
    args.push(command.to_string());
    if let Some(arg) = arg {
        args.push(arg.to_string());
    }
    if cli.session_flag {
        args.push("--session".into());
        args.push(session.to_string());
    }
    if cli.window_flag && command == "open" {
        args.push("--window".into());
        args.push("background".into());
    }
    args
}

pub fn browser_argv_public(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.starts_with("http://") || arg.starts_with("https://") {
                "<status>"
            } else {
                arg.as_str()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote-session browser command. `foreground` asks OpenCLI to show the
/// owned window. Native click is refused on a hidden page, so the quote
/// open never passes `--window background`. Tab list omits the window
/// flag so listing does not move the window.
pub fn quote_browser_args(
    cli: BrowserCli,
    session: &str,
    words: &[&str],
    tab: Option<&str>,
    foreground: bool,
) -> Vec<String> {
    let mut args = vec!["browser".to_string()];
    if !cli.session_flag {
        args.push(session.to_string());
    }
    args.extend(words.iter().map(|word| (*word).to_string()));
    if let Some(tab) = tab.filter(|value| is_browser_target_id(value)) {
        args.push("--tab".into());
        args.push(tab.to_string());
    }
    if cli.session_flag {
        args.push("--session".into());
        args.push(session.to_string());
    }
    if foreground && cli.window_flag {
        args.push("--window".into());
        args.push("foreground".into());
    }
    args
}

pub fn is_browser_target_id(value: &str) -> bool {
    let len = value.len();
    (16..=64).contains(&len) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Hex page id from an OpenCLI browser result. `browser open` in 2.0.2
/// returns `{ok:true,data:{url,page}}`, where `page` is the target id
/// string. `targetId` is accepted for older envelopes.
pub fn quote_surface_target(stdout: &str) -> Option<ParsedBrowserTarget> {
    let value = first_json_value(stdout)?;
    find_browser_target(&value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBrowserTarget {
    pub id: String,
    pub field: &'static str,
}

fn find_browser_target(value: &serde_json::Value) -> Option<ParsedBrowserTarget> {
    if let Some(text) = value.as_str() {
        let text = text.trim().trim_matches('"');
        return is_browser_target_id(text).then(|| ParsedBrowserTarget {
            id: text.to_string(),
            field: "string",
        });
    }
    if let Some(rows) = value.as_array() {
        return rows.iter().find_map(find_browser_target);
    }
    let obj = value.as_object()?;
    for key in ["targetId", "target_id", "target"] {
        if let Some(text) = obj.get(key).and_then(serde_json::Value::as_str) {
            if is_browser_target_id(text) {
                return Some(ParsedBrowserTarget {
                    id: text.to_string(),
                    field: match key {
                        "targetId" => "targetId",
                        "target_id" => "target_id",
                        _ => "target",
                    },
                });
            }
        }
    }
    if let Some(text) = obj.get("page").and_then(serde_json::Value::as_str) {
        if is_browser_target_id(text) {
            return Some(ParsedBrowserTarget {
                id: text.to_string(),
                field: "page",
            });
        }
    }
    if let Some(found) = obj.get("data").and_then(find_browser_target) {
        let field = if found.field == "string" { "page" } else { found.field };
        return Some(ParsedBrowserTarget {
            id: found.id,
            field,
        });
    }
    None
}

/// Field names and value kinds from an open envelope. No URL, title, or page text.
pub fn quote_open_shape(stdout: &str) -> String {
    let Some(value) = first_json_value(stdout) else {
        return "open json none".into();
    };
    let top = object_key_list(value.as_object());
    let data = value.get("data");
    let data_desc = match data {
        Some(serde_json::Value::Object(map)) => format!("keys {}", object_key_list(Some(map))),
        Some(other) => open_value_kind(other),
        None => "absent".into(),
    };
    let page = data
        .and_then(|item| item.get("page"))
        .map(open_value_kind)
        .unwrap_or_else(|| "absent".into());
    format!("open keys {top} · data {data_desc} · page {page}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteSelectAck {
    pub ok: bool,
    pub selected: Option<String>,
    pub note: String,
}

/// `tab select` prints `{ok:true,data:{selected:<target id>}}`. Keep the id
/// and the ok/error flag. Drop every other field.
pub fn quote_select_ack(stdout: &str) -> QuoteSelectAck {
    if stdout.trim().eq_ignore_ascii_case("ok") {
        return QuoteSelectAck {
            ok: true,
            selected: None,
            note: "ok selected absent".into(),
        };
    }
    let Some(value) = first_json_value(stdout) else {
        return QuoteSelectAck {
            ok: false,
            selected: None,
            note: "unparsed".into(),
        };
    };
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        let code = value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        let message = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return QuoteSelectAck {
            ok: false,
            selected: None,
            note: format!("error {} {}", safe_ack_token(code), safe_ack_token(message)),
        };
    }
    let selected = value
        .get("data")
        .and_then(|data| data.get("selected"))
        .or_else(|| value.get("selected"));
    match selected {
        Some(serde_json::Value::String(text)) if is_browser_target_id(text) => QuoteSelectAck {
            ok: true,
            selected: Some(text.clone()),
            note: format!("ok selected {text}"),
        },
        Some(serde_json::Value::Bool(true)) => QuoteSelectAck {
            ok: true,
            selected: None,
            note: "ok selected bool".into(),
        },
        Some(_) => QuoteSelectAck {
            ok: true,
            selected: None,
            note: "ok selected other".into(),
        },
        None => QuoteSelectAck {
            ok: true,
            selected: None,
            note: "ok selected absent".into(),
        },
    }
}

fn safe_ack_token(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if lower.contains("token") || lower.contains("cookie") || lower.contains("authorization") {
        return "omitted".into();
    }
    text.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-' || *ch == ' ')
        .take(40)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteStatusTab {
    pub window_id: u64,
    pub group_id: Option<u64>,
    pub active: bool,
}

/// Tabs whose URL path is exactly this status. Other tabs are dropped.
pub fn quote_status_tabs(stdout: &str, status_id: &str) -> Vec<QuoteStatusTab> {
    let Some(value) = first_json_value(stdout) else {
        return Vec::new();
    };
    let rows = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array());
    let Some(rows) = rows else {
        return Vec::new();
    };
    rows.iter().filter_map(|row| status_tab(row, status_id)).collect()
}

fn status_tab(row: &serde_json::Value, status_id: &str) -> Option<QuoteStatusTab> {
    let url = row.get("url")?.as_str()?;
    if !url_is_status(url, status_id) {
        return None;
    }
    let window_id = row.get("windowId")?.as_u64()?;
    let group_id = row.get("groupId").and_then(serde_json::Value::as_u64);
    let group_id = group_id.filter(|id| *id > 0);
    Some(QuoteStatusTab {
        window_id,
        group_id,
        active: row.get("active").and_then(serde_json::Value::as_bool).unwrap_or(false),
    })
}

fn url_is_status(url: &str, status_id: &str) -> bool {
    let marker = format!("/status/{status_id}");
    let Some(at) = url.find(&marker) else {
        return false;
    };
    let next = url[at + marker.len()..].chars().next();
    next.is_none() || next == Some('?') || next == Some('#') || next == Some('/')
}

pub fn quote_window_state(stdout: &str) -> Option<(&'static str, bool)> {
    let value = first_json_value(stdout)?;
    let data = value.get("data").unwrap_or(&value);
    let state = data.get("state")?.as_str()?;
    let state = match state {
        "minimized" => "minimized",
        "normal" => "normal",
        "maximized" => "maximized",
        "fullscreen" => "fullscreen",
        "locked-fullscreen" => "locked-fullscreen",
        _ => return None,
    };
    let focused = data.get("focused").and_then(serde_json::Value::as_bool).unwrap_or(false);
    Some((state, focused))
}

pub fn quote_group_name(stdout: &str) -> &'static str {
    let Some(value) = first_json_value(stdout) else {
        return "absent";
    };
    let title = value
        .get("data")
        .and_then(|data| data.get("title"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    match title {
        "OpenCLI Browser" => "OpenCLI Browser",
        "OpenCLI Adapter" => "OpenCLI Adapter",
        "" => "absent",
        _ => "other",
    }
}

fn object_key_list(map: Option<&serde_json::Map<String, serde_json::Value>>) -> String {
    let Some(map) = map else {
        return "none".into();
    };
    let mut keys: Vec<&str> = map
        .keys()
        .filter(|key| key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'))
        .map(String::as_str)
        .take(8)
        .collect();
    if keys.is_empty() {
        return "none".into();
    }
    keys.sort_unstable();
    keys.join(",")
}

fn open_value_kind(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) if is_browser_target_id(text) => "target".into(),
        serde_json::Value::String(text) if text.starts_with("http://") || text.starts_with("https://") => {
            "url".into()
        }
        serde_json::Value::String(_) => "string".into(),
        serde_json::Value::Array(rows) => format!("array {}", rows.len()),
        serde_json::Value::Object(_) => "object".into(),
        serde_json::Value::Number(_) => "number".into(),
        serde_json::Value::Bool(_) => "bool".into(),
        serde_json::Value::Null => "null".into(),
    }
}

pub fn quote_owned_group_ids(stdout: &str) -> Vec<u64> {
    let Some(value) = first_json_value(stdout) else {
        return Vec::new();
    };
    let rows = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array());
    let Some(rows) = rows else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            let title = row.get("title").and_then(serde_json::Value::as_str);
            if title.is_some_and(|title| title != "OpenCLI Browser") {
                return None;
            }
            row.get("id").and_then(serde_json::Value::as_u64)
        })
        .collect()
}

/// `data: []` is zero tabs. Unreadable output is `None` (not a guess of zero).
pub fn quote_tab_count(stdout: &str) -> Option<usize> {
    let value = first_json_value(stdout)?;
    let data = value.get("data").unwrap_or(&value);
    data.as_array().map(|rows| rows.len())
}

/// Page Visibility values only. Anything else, including a URL, is `unknown`.
pub fn quote_visibility(stdout: &str) -> String {
    let Some(value) = first_json_value(stdout) else {
        return known_visibility(stdout);
    };
    if let Some(text) = value.as_str() {
        return known_visibility(text);
    }
    if let Some(text) = value.get("visibility").and_then(serde_json::Value::as_str) {
        return known_visibility(text);
    }
    if let Some(data) = value.get("data") {
        if let Some(text) = data.as_str() {
            return known_visibility(text);
        }
        if let Some(text) = data.get("visibility").and_then(serde_json::Value::as_str) {
            return known_visibility(text);
        }
    }
    "unknown".into()
}

fn known_visibility(text: &str) -> String {
    match text.trim().trim_matches('"') {
        "visible" | "hidden" | "prerender" | "unloaded" => text.trim().trim_matches('"').to_string(),
        _ => "unknown".into(),
    }
}

pub fn quote_visibility_expr(status_id: &str) -> String {
    let id: String = status_id.chars().filter(|ch| ch.is_ascii_digit()).collect();
    format!(
        "JSON.stringify({{v:document.visibilityState,h:document.hidden===true,p:document.prerendering===true,f:document.hasFocus(),t:window.top===window,m:(location.pathname.indexOf(\"/status/{id}\")>=0||(location.search||\"\").indexOf(\"{id}\")>=0),c:(location.pathname===\"/compose/post\"||location.pathname.slice(-13)===\"/compose/post\"),host:location.hostname,n:location.pathname,s:location.protocol}})"
    )
}

/// OpenCLI's quote composer. The `url` query attaches the source tweet.
/// The status URL is not percent-encoded. It has no `&` or `?`, and
/// `URLSearchParams` reads the same value as `encodeURIComponent` would
/// produce. Percent-encoding is expanded by Windows `cmd` and by the
/// OpenCLI `.cmd` shim; doubling those percents moved the evaluated page
/// off x.com. A bare `/compose/post` does not attach a quote.
pub fn quote_composer_url(status_id: &str) -> String {
    let id: String = status_id.chars().filter(|ch| ch.is_ascii_digit()).collect();
    format!("https://x.com/compose/post?url=https://x.com/i/status/{id}")
}

/// Sanitized shape of the URL OpenCLI echoed from `browser open`.
/// That echo is the URL the command was given, not `location.href`.
/// `id-match` requires the decoded `url` parameter to be this status.
/// A leftover digit inside a damaged percent sequence does not match.
pub fn quote_landed_query(stdout: &str, status_id: &str) -> String {
    let Some(value) = first_json_value(stdout) else {
        return "open echo unread".into();
    };
    let url = value
        .pointer("/data/url")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("url").and_then(serde_json::Value::as_str));
    match url {
        Some(url) => quote_query_shape(url, status_id),
        None => "open echo url absent".into(),
    }
}

pub fn quote_query_shape(url: &str, status_id: &str) -> String {
    let id: String = status_id.chars().filter(|ch| ch.is_ascii_digit()).collect();
    let Some(query) = url.split_once('?').map(|(_, rest)| rest.split('#').next().unwrap_or("")) else {
        return "open echo url-key no id-match no".into();
    };
    let mut url_key = false;
    let mut id_match = false;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key != "url" {
            continue;
        }
        url_key = true;
        if status_param_matches(&percent_decode_lossy(value), &id) {
            id_match = true;
        }
    }
    format!(
        "open echo url-key {} id-match {}",
        if url_key { "yes" } else { "no" },
        if id_match { "yes" } else { "no" }
    )
}

fn status_param_matches(value: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let Some(url) = value.split(['?', '#']).next() else {
        return false;
    };
    let path = url.split("://").nth(1).and_then(|rest| rest.find('/').map(|i| &rest[i..])).unwrap_or(url);
    let marker = format!("/status/{id}");
    let Some(at) = path.find(&marker) else {
        return false;
    };
    path[at + marker.len()..].chars().next().is_none_or(|ch| !ch.is_ascii_digit())
}

fn percent_decode_lossy(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[index + 1]), hex_nibble(bytes[index + 2])) {
                out.push((hi << 4) | lo);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Target ids from `browser tab list` for this session only.
pub fn quote_session_pages(stdout: &str) -> Vec<String> {
    let Some(value) = first_json_value(stdout) else {
        return Vec::new();
    };
    let rows = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .or_else(|| value.as_array());
    let Some(rows) = rows else {
        return Vec::new();
    };
    let mut pages = Vec::new();
    for row in rows {
        let text = row
            .get("page")
            .and_then(serde_json::Value::as_str)
            .or_else(|| row.as_str())
            .unwrap_or("");
        if is_browser_target_id(text) && !pages.iter().any(|have: &String| have == text) {
            pages.push(text.to_string());
        }
    }
    pages
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteVisibilityRead {
    pub token: String,
    pub summary: String,
    pub ready: bool,
}

/// Decode one visibility eval. A bare word and a JSON object are both read.
/// `ready` requires the top document of this status to report `visible`.
pub fn quote_visibility_read(stdout: &str) -> QuoteVisibilityRead {
    let outer = first_json_value(stdout);
    let data = outer.as_ref().and_then(|value| value.get("data")).or(outer.as_ref());
    let (kind, fields) = match data {
        Some(serde_json::Value::String(text)) => {
            if let Some(inner) = first_json_value(text) {
                ("string", inner)
            } else {
                return plain_visibility(text);
            }
        }
        Some(serde_json::Value::Object(_)) => ("object", data.cloned().unwrap_or(serde_json::Value::Null)),
        _ => {
            return QuoteVisibilityRead {
                token: "unknown".into(),
                summary: "eval data absent visibility unknown hidden-flag absent prerender absent focus absent top absent path absent host absent".into(),
                ready: false,
            };
        }
    };
    visibility_from_fields(kind, &fields)
}

fn visibility_from_fields(kind: &str, fields: &serde_json::Value) -> QuoteVisibilityRead {
    let token = visibility_token(fields.get("v").and_then(serde_json::Value::as_str).unwrap_or(""));
    let hidden_flag = yes_no(fields.get("h"));
    let prerender = yes_no(fields.get("p"));
    let focus = yes_no(fields.get("f"));
    let top = yes_no(fields.get("t"));
    let path = yes_no(fields.get("m"));
    let compose = yes_no(fields.get("c"));
    let host = host_class(fields.get("host").and_then(serde_json::Value::as_str).unwrap_or(""));
    let route = if path == "yes" {
        "status"
    } else if compose == "yes" {
        "compose"
    } else {
        "no"
    };
    let host_name = safe_location_token(fields.get("hn").or_else(|| fields.get("host")).and_then(serde_json::Value::as_str).unwrap_or(""), true);
    let path_name = safe_location_token(fields.get("n").and_then(serde_json::Value::as_str).unwrap_or(""), false);
    let scheme = safe_scheme(fields.get("s").and_then(serde_json::Value::as_str).unwrap_or(""));
    let ready = token == "visible"
        && hidden_flag != "yes"
        && top == "yes"
        && host == "x.com"
        && prerender != "yes"
        && route != "no";
    QuoteVisibilityRead {
        token: token.to_string(),
        summary: format!(
            "eval data {kind} visibility {token} hidden-flag {hidden_flag} prerender {prerender} focus {focus} top {top} path {path} route {route} host {host} eval-host {host_name} eval-path {path_name} scheme {scheme}"
        ),
        ready,
    }
}

fn plain_visibility(text: &str) -> QuoteVisibilityRead {
    let token = visibility_token(text);
    QuoteVisibilityRead {
        token: token.to_string(),
        summary: format!(
            "eval data string visibility {token} hidden-flag absent prerender absent focus absent top absent path absent host absent"
        ),
        ready: false,
    }
}

fn visibility_token(text: &str) -> &'static str {
    match text.trim().trim_matches('"') {
        "visible" => "visible",
        "hidden" => "hidden",
        "prerender" => "prerender",
        "unloaded" => "unloaded",
        _ => "unknown",
    }
}

fn yes_no(value: Option<&serde_json::Value>) -> &'static str {
    match value.and_then(serde_json::Value::as_bool) {
        Some(true) => "yes",
        Some(false) => "no",
        None => "absent",
    }
}

fn safe_scheme(protocol: &str) -> &'static str {
    match protocol.trim().trim_end_matches(':') {
        "https" => "https",
        "http" => "http",
        "chrome" => "chrome",
        "about" => "about",
        "" => "absent",
        _ => "other",
    }
}

fn safe_location_token(value: &str, host: bool) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "absent".into();
    }
    let ok = value.len() <= 120
        && value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/'));
    let shaped = if host { !value.contains('/') } else { value.starts_with('/') };
    if ok && shaped {
        value.to_ascii_lowercase()
    } else {
        "unreadable".into()
    }
}

/// Sanitized `location.href` from an eval. Scheme, host, and path only.
pub fn quote_eval_location(stdout: &str, status_id: &str) -> String {
    let Some(value) = first_json_value(stdout) else {
        return location_shape(stdout, status_id);
    };
    let text = value
        .as_str()
        .or_else(|| value.get("data").and_then(serde_json::Value::as_str))
        .or_else(|| value.pointer("/data/url").and_then(serde_json::Value::as_str));
    match text {
        Some(text) => location_shape(text, status_id),
        None => "eval location unread".into(),
    }
}

fn location_shape(text: &str, status_id: &str) -> String {
    let trimmed = text.trim().trim_matches('"');
    let Some(rest) = trimmed.split("://").nth(1) else {
        return "eval location unread".into();
    };
    let scheme = safe_scheme(trimmed.split("://").next().unwrap_or(""));
    let (host, path_and_more) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split('@').next_back().unwrap_or(host).split(':').next().unwrap_or(host);
    let path_tail = path_and_more.split(['?', '#']).next().unwrap_or("");
    let path = if path_tail.is_empty() {
        "/".to_string()
    } else {
        format!("/{path_tail}")
    };
    let query = quote_query_shape(trimmed, status_id).replace("open echo ", "");
    format!(
        "eval location scheme {scheme} host {} path {} {query}",
        safe_location_token(host, true),
        safe_location_token(&path, false)
    )
}

fn host_class(host: &str) -> &'static str {
    match host {
        "x.com" | "twitter.com" | "mobile.twitter.com" => "x.com",
        "" => "absent",
        _ => "other",
    }
}

pub fn quote_window_bounds(stdout: &str) -> &'static str {
    let Some(value) = first_json_value(stdout) else {
        return "absent";
    };
    let data = value.get("data").unwrap_or(&value);
    let Some(left) = data.get("left").and_then(serde_json::Value::as_i64) else {
        return "absent";
    };
    let Some(top) = data.get("top").and_then(serde_json::Value::as_i64) else {
        return "absent";
    };
    let Some(width) = data.get("width").and_then(serde_json::Value::as_i64) else {
        return "absent";
    };
    let Some(height) = data.get("height").and_then(serde_json::Value::as_i64) else {
        return "absent";
    };
    if width < 50 || height < 50 {
        return "tiny";
    }
    // (-32000, -32000) is the position Termy used to park a window off the
    // desktop. A monitor to the left of the primary uses a small negative
    // origin and is still on screen.
    if left <= -30_000 || top <= -30_000 {
        "parked"
    } else {
        "onscreen"
    }
}

pub fn quote_view_line(tabs: &str, target: &str, field: &str, opened: &str, click: &str) -> String {
    format!(
        "session termyx-quote · window foreground · tabs before {tabs} · target {target} · target field {field} · visibility open {opened} click {click}"
    )
}

/// Keep OpenCLI's own code and message. Drop credential-shaped lines.
pub fn explain_opencli_failure(stage: &str, argv: &str, status: i32, stdout: &str, stderr: &str) -> String {
    let cause = opencli_error_cause(stdout)
        .or_else(|| opencli_error_cause(stderr))
        .or_else(|| first_safe_line(stdout))
        .or_else(|| first_safe_line(stderr))
        .unwrap_or_else(|| "opencli printed nothing".into());
    format!("stage {stage} · argv {argv} · exit {status} · {cause} · Post was not clicked")
}

fn opencli_error_cause(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;
    let error = value.get("error")?;
    let code = error.get("code").and_then(serde_json::Value::as_str).unwrap_or("");
    let message = error.get("message").and_then(serde_json::Value::as_str).unwrap_or("");
    let hint = error.get("hint").and_then(serde_json::Value::as_str).unwrap_or("");
    if code.is_empty() && message.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !code.is_empty() {
        parts.push(code.to_string());
    }
    if !message.is_empty() {
        parts.push(message.to_string());
    }
    if !hint.is_empty() && !line_has_secret(hint) {
        parts.push(hint.to_string());
    }
    Some(parts.join(" · "))
}

fn first_safe_line(text: &str) -> Option<String> {
    for line in text.lines() {
        if line_has_secret(line) {
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut out = String::new();
        for ch in trimmed.chars().take(160) {
            if ch != '{' && ch != '}' {
                out.push(ch);
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn line_has_secret(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("cookie")
        || lower.contains("authorization")
        || lower.contains("ct0=")
}

fn first_message(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    success_node(&value)?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

fn first_text(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    success_node(&value)?
        .get("text")?
        .as_str()
        .map(str::to_string)
}

fn quoted_target_id(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let node = success_node(&value)?;
    for key in [
        "quoted_status_id_str",
        "quoted_status_id",
        "quote_id",
        "attachment_url",
    ] {
        let Some(text) = node.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let id = crate::model::post_id_from_input(text);
        if !id.is_empty() && id.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(id);
        }
    }
    None
}

fn status_ids_in_text(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find("/status/") {
        let after = &rest[pos + "/status/".len()..];
        let id: String = after.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !id.is_empty() {
            ids.push(id);
        }
        rest = after;
    }
    ids
}

/// In-page quote attach. Opens the quote composer from the tweet's repost
/// menu. Direct `/compose/post?url=` is not used: that page can show the
/// text box without a quote card, which is the 8:32 CT failure. The script
/// opens the repost menu and records the Quote row. It does not click that
/// row: a capture-phase preventDefault left the page unchanged, and an
/// untrusted click on the bare `/compose/post` anchor navigates without the
/// quote id. The native click is `opencli browser click --role menuitem
/// --name Quote`. This script never clicks `retweetConfirm`,
/// `unretweetConfirm`, or Post.
pub fn quote_page_script(target_id: &str, commentary: Option<&str>, inspect: bool) -> String {
    let id_js = js_ascii_string(target_id);
    let text_js = if inspect {
        "null".to_string()
    } else {
        commentary
            .map(js_ascii_string)
            .unwrap_or_else(|| "null".to_string())
    };
    let inspect_js = if inspect { "true" } else { "false" };
    format!(
        r#"(async () => {{
  const TARGET = {id_js};
  const TEXT = {text_js};
  const INSPECT = {inspect_js};
  window.__qxReport = null;
  const finish = (obj) => {{ window.__qxReport = obj; return JSON.stringify(obj); }};
  const statusId = (href) => {{
    try {{
      const path = new URL(href, location.origin).pathname;
      const match = path.match(/\/status\/(\d+)/);
      return match ? match[1] : null;
    }} catch (e) {{ return null; }}
  }};
  const statusIds = (root) => {{
    const ids = [];
    if (!root) return ids;
    for (const link of root.querySelectorAll('a[href*="/status/"]')) {{
      const id = statusId(link.href);
      if (id && ids.indexOf(id) < 0 && ids.length < 8) ids.push(id);
    }}
    return ids;
  }};
  const quoteAttachment = () => {{
    const node = document.querySelector('[data-testid="quoteTweet"]');
    return !!node && statusIds(node).indexOf(TARGET) >= 0;
  }};
  const composerKind = () => {{
    if (quoteAttachment()) return "quote";
    const text = (document.body && document.body.innerText || "").slice(0, 2500);
    if (/replying to/i.test(text)) return "reply";
    return "unknown";
  }};
  let picked = {{ tag: "", testid: "", label: "", href_kind: "none", scope: "none" }};
  let beforeTopo = "none";
  let menuShape = "none";
  const hrefShape = (el) => {{
    const raw = el.getAttribute("href") || "";
    if (!raw) return "none";
    try {{
      const parsed = new URL(raw, location.origin);
      const keys = Array.from(parsed.searchParams.keys()).slice(0, 4);
      const path = parsed.pathname.slice(0, 80);
      return "path=" + path + " keys=" + (keys.length ? keys.join(",") : "none");
    }} catch (e) {{
      return "unparsed";
    }}
  }};
  const topology = () => {{
    const boxes = Array.from(document.querySelectorAll('[data-testid="tweetTextarea_0"]'));
    let place = "none";
    if (boxes.length) {{
      const box = boxes[boxes.length - 1];
      if (box.closest('[role="dialog"], [aria-modal="true"]')) place = "dialog";
      else if (box.closest("article")) place = "article";
      else place = "page";
    }}
    return place
      + " textareas=" + boxes.length
      + " quotes=" + document.querySelectorAll('[data-testid="quoteTweet"]').length
      + " dialogs=" + document.querySelectorAll('[role="dialog"], [aria-modal="true"]').length;
  }};
  const report = (stage, message) => {{
    const button = document.querySelector('[data-testid="tweetButton"], [data-testid="tweetButtonInline"]');
    const attached = quoteAttachment();
    return {{
      stage: stage,
      path: location.pathname,
      testid_count: document.querySelectorAll("[data-testid]").length,
      composer: !!document.querySelector('[data-testid="tweetTextarea_0"]'),
      composer_kind: composerKind(),
      card: attached,
      status_ids: statusIds(document),
      quote_testids: document.querySelectorAll('[data-testid="quoteTweet"]').length,
      retweet_button: !!document.querySelector('[data-testid="retweet"]'),
      unretweet_button: !!document.querySelector('[data-testid="unretweet"]'),
      post_enabled: !!button && !button.disabled && button.getAttribute("aria-disabled") !== "true",
      text_landed: false,
      clicked_post: false,
      clicked_repost_confirm: false,
      new_status_id: null,
      quoted_status_id: attached ? TARGET : null,
      selected_tag: picked.tag,
      selected_testid: picked.testid,
      selected_label: picked.label,
      selected_href_kind: picked.href_kind,
      selected_scope: picked.scope,
      topo_before: beforeTopo,
      topo_after: topology(),
      menu_shape: menuShape,
      message: message
    }};
  }};
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  for (let i = 0; i < 20; i++) {{
    if (document.querySelector("article, [data-testid='primaryColumn']")) break;
    await sleep(250);
  }}
  const articles = Array.from(document.querySelectorAll("article"));
  const article = articles.find((node) => statusIds(node).indexOf(TARGET) >= 0) || articles[0] || document;
  const opener = article.querySelector('[data-testid="retweet"], [data-testid="unretweet"]');
  beforeTopo = topology();
  if (!opener) {{
    const missing = report("page", "Repost menu button was not on the tweet. Post was not clicked.");
    return finish(missing);
  }}
  opener.click();
  let quoteItem = null;
  let menu = null;
  for (let i = 0; i < 20; i++) {{
    menu = document.querySelector('[role="menu"], [data-testid="Dropdown"]');
    const root = menu || null;
    const items = root ? Array.from(root.querySelectorAll('[role="menuitem"]')) : [];
    quoteItem = items.find((el) => {{
      const tid = el.getAttribute("data-testid") || "";
      if (tid === "retweetConfirm" || tid === "unretweetConfirm" || tid === "reply") return false;
      const label = (el.innerText || "").trim().split("\\n")[0].trim();
      return /^quote$/i.test(label);
    }});
    if (quoteItem) break;
    await sleep(250);
  }}
  if (menu) {{
    menuShape = Array.from(menu.querySelectorAll('[role="menuitem"]')).slice(0, 4).map((el) => {{
      const label = (el.innerText || "").trim().split("\\n")[0].replace(/[^A-Za-z0-9 ]/g, "").trim().slice(0, 16) || "-";
      const tid = el.getAttribute("data-testid") || "-";
      return el.tagName + ":" + (el.getAttribute("role") || "-") + ":" + tid + ":" + label + ":" + hrefShape(el);
    }}).join(" | ");
  }}
  if (!quoteItem) {{
    return finish(report("menu", "Quote menu item did not appear. Post was not clicked."));
  }}
  const href = quoteItem.getAttribute("href") || "";
  const shape = hrefShape(quoteItem);
  picked = {{
    tag: quoteItem.tagName || "",
    testid: quoteItem.getAttribute("data-testid") || "",
    label: (quoteItem.innerText || "").trim().split("\\n")[0].trim().slice(0, 24),
    href_kind: shape,
    scope: menu ? "menu" : "page"
  }};
  if (picked.testid === "retweetConfirm" || picked.testid === "unretweetConfirm" || href.indexOf("in_reply_to") >= 0) {{
    return finish(report("menu", "Refused the repost confirm. Post was not clicked."));
  }}
  return finish(report("menu", "Quote row recorded. Post was not clicked."));
}})()"#
    )
}

pub fn quote_read_script(target_id: &str) -> String {
    let id_js = js_ascii_string(target_id);
    format!(
        r#"(async () => {{
  const TARGET = {id_js};
  const statusId = (href) => {{
    try {{
      const path = new URL(href, location.origin).pathname;
      const match = path.match(/\/status\/(\d+)/);
      return match ? match[1] : null;
    }} catch (e) {{ return null; }}
  }};
  const ids = [];
  for (const link of document.querySelectorAll('a[href*="/status/"]')) {{
    const id = statusId(link.href);
    if (id && ids.indexOf(id) < 0 && ids.length < 8) ids.push(id);
  }}
  const quoteNode = document.querySelector('[data-testid="quoteTweet"]');
  const quoteIds = [];
  if (quoteNode) {{
    for (const link of quoteNode.querySelectorAll('a[href*="/status/"]')) {{
      const id = statusId(link.href);
      if (id && quoteIds.indexOf(id) < 0) quoteIds.push(id);
    }}
  }}
  const attached = quoteIds.indexOf(TARGET) >= 0;
  const replying = /replying to/i.test((document.body && document.body.innerText || "").slice(0, 2500));
  const report = {{
    stage: "read",
    path: location.pathname,
    composer_kind: attached ? "quote" : (replying || ids.indexOf(TARGET) >= 0 ? "reply" : "unknown"),
    card: attached,
    status_ids: ids,
    quote_testids: document.querySelectorAll('[data-testid="quoteTweet"]').length,
    clicked_post: false,
    clicked_repost_confirm: false,
    quoted_status_id: attached ? TARGET : null,
    message: "Read only. Post was not clicked."
  }};
  window.__qxReport = report;
  return JSON.stringify(report);
}})()"#
    )
}

/// ASCII-only JavaScript string. `atob` + `eval` reads bytes as Latin-1,
/// which turned U+2019 into `â€™` on the published reply.
fn js_ascii_string(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ch if (ch as u32) < 128 && ch >= ' ' => out.push(ch),
            ch => out.push_str(&format!("\\u{:04x}", ch as u32)),
        }
    }
    out.push('"');
    out
}

/// The status page. The Quote menu on this page is what attaches the source
/// card. `compose/post?url=` put that URL into the editor as text.
pub fn quote_status_url(status_id: &str) -> String {
    let id: String = status_id.chars().filter(|ch| ch.is_ascii_digit()).collect();
    format!("https://x.com/i/status/{id}")
}

/// Trusted click on the target article's Repost control. `testid` is only
/// `retweet` or `unretweet`. The confirm rows are not this selector.
pub fn quote_opener_click_args(
    cli: BrowserCli,
    session: &str,
    status_id: &str,
    testid: &str,
    tab: Option<&str>,
) -> Vec<String> {
    let id: String = status_id.chars().filter(|ch| ch.is_ascii_digit()).collect();
    let testid = if testid == "unretweet" { "unretweet" } else { "retweet" };
    let css = format!(r#"article:has(a[href*="/status/{id}"] time) [data-testid="{testid}"]"#);
    let mut args = vec!["browser".to_string()];
    if !cli.session_flag {
        args.push(session.to_string());
    }
    args.push("click".into());
    args.push(css);
    if let Some(tab) = tab.filter(|value| is_browser_target_id(value)) {
        args.push("--tab".into());
        args.push(tab.to_string());
    }
    if cli.session_flag {
        args.push("--session".into());
        args.push(session.to_string());
    }
    if cli.window_flag {
        args.push("--window".into());
        args.push("foreground".into());
    }
    args
}

pub fn quote_click_note(stdout: &str) -> String {
    let Some(value) = first_json_value(stdout) else {
        return "click unread".into();
    };
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        let code = value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("error");
        let safe: String = code
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
            .take(32)
            .collect();
        return format!("click error {}", if safe.is_empty() { "error" } else { &safe });
    }
    let data = value.get("data").unwrap_or(&value);
    let method = match data.get("click_method").and_then(serde_json::Value::as_str) {
        Some("cdp") => "cdp",
        Some("ax") => "ax",
        Some("js") => "js",
        Some(_) => "other",
        None => "absent",
    };
    format!("click {method}")
}

pub fn quote_click_trusted(stdout: &str) -> bool {
    matches!(quote_click_note(stdout).as_str(), "click cdp" | "click ax")
}

/// Trusted click. `--name Quote` is the accessible name of the
/// menu row. It is not `unretweetConfirm` and it is not Post.
pub fn quote_native_click_args(cli: BrowserCli, session: &str, tab: Option<&str>) -> Vec<String> {
    let mut args = vec!["browser".to_string()];
    if !cli.session_flag {
        args.push(session.to_string());
    }
    args.push("click".into());
    args.push("--role".into());
    args.push("menuitem".into());
    args.push("--name".into());
    args.push("Quote".into());
    if let Some(tab) = tab.filter(|value| is_browser_target_id(value)) {
        args.push("--tab".into());
        args.push(tab.to_string());
    }
    if cli.session_flag {
        args.push("--session".into());
        args.push(session.to_string());
    }
    if cli.window_flag {
        args.push("--window".into());
        args.push("foreground".into());
    }
    args
}

/// Waits for the repost menu and records the Quote row. It does not click.
pub fn quote_menu_read_script(target_id: &str) -> String {
    let id_js = js_ascii_string(target_id);
    format!(
        r#"(async () => {{
  const TARGET = {id_js};
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const hrefShape = (el) => {{
    const raw = el.getAttribute("href") || "";
    if (!raw) return "none";
    try {{
      const parsed = new URL(raw, location.origin);
      const keys = Array.from(parsed.searchParams.keys()).slice(0, 4);
      return "path=" + parsed.pathname.slice(0, 80) + " keys=" + (keys.length ? keys.join(",") : "none");
    }} catch (e) {{
      return "unparsed";
    }}
  }};
  let quoteItem = null;
  let menu = null;
  for (let i = 0; i < 20; i++) {{
    menu = document.querySelector('[role="menu"], [data-testid="Dropdown"]');
    const items = menu ? Array.from(menu.querySelectorAll('[role="menuitem"]')) : [];
    quoteItem = items.find((el) => {{
      const tid = el.getAttribute("data-testid") || "";
      if (tid === "retweetConfirm" || tid === "unretweetConfirm" || tid === "reply") return false;
      const label = (el.innerText || "").trim().split("\n")[0].trim();
      return /^quote$/i.test(label);
    }});
    if (quoteItem) break;
    await sleep(250);
  }}
  const menuShape = menu ? Array.from(menu.querySelectorAll('[role="menuitem"]')).slice(0, 4).map((el) => {{
    const label = (el.innerText || "").trim().split("\n")[0].replace(/[^A-Za-z0-9 ]/g, "").trim().slice(0, 16) || "-";
    const tid = el.getAttribute("data-testid") || "-";
    return el.tagName + ":" + (el.getAttribute("role") || "-") + ":" + tid + ":" + label + ":" + hrefShape(el);
  }}).join(" | ") : "none";
  const href = quoteItem ? (quoteItem.getAttribute("href") || "") : "";
  const tid = quoteItem ? (quoteItem.getAttribute("data-testid") || "") : "";
  const refused = tid === "retweetConfirm" || tid === "unretweetConfirm" || href.indexOf("in_reply_to") >= 0;
  const report = {{
    stage: "menu",
    path: location.pathname,
    composer: false,
    composer_kind: "unknown",
    card: false,
    quote_testids: 0,
    clicked_post: false,
    clicked_repost_confirm: refused,
    selected_label: quoteItem ? (quoteItem.innerText || "").trim().split("\n")[0].trim().slice(0, 24) : "",
    selected_testid: tid,
    selected_href_kind: quoteItem ? hrefShape(quoteItem) : "none",
    selected_scope: menu ? "menu" : "none",
    menu_shape: menuShape,
    message: !quoteItem ? "Quote menu item did not appear. Post was not clicked." : (refused ? "Refused the repost confirm. Post was not clicked." : "Quote row recorded. Post was not clicked.")
  }};
  window.__qxReport = report;
  return JSON.stringify(report);
}})()"#
    )
}

pub fn quote_type_and_post_script(target_id: &str, commentary: &str) -> String {
    quote_type_and_post_script_bound(target_id, commentary, "", "")
}

/// Types the commentary and clicks Post once. Runs only after a snap has
/// already shown the source card for this status. Refuses otherwise.
pub fn quote_type_and_post_script_bound(
    target_id: &str,
    commentary: &str,
    screen: &str,
    source_text: &str,
) -> String {
    let id_js = js_ascii_string(target_id);
    let text_js = js_ascii_string(commentary);
    let screen_js = js_ascii_string(screen);
    let source_js = js_ascii_string(source_text);
    format!(
        r#"(async () => {{
  const TARGET = {id_js};
  const TEXT = {text_js};
  const SCREEN = {screen_js};
  const SOURCE = {source_js};
  window.__qxReport = null;
  const finish = (obj) => {{ window.__qxReport = obj; return JSON.stringify(obj); }};
  const statusId = (href) => {{ try {{ const path = new URL(href, location.origin).pathname; const match = path.match(/\/status\/(\d+)/); return match ? match[1] : null; }} catch (e) {{ return null; }} }};
  const quoteAttachment = () => {{
    const boxes = Array.from(document.querySelectorAll('[data-testid="tweetTextarea_0"]'));
    const box = boxes.length ? boxes[boxes.length - 1] : null;
    const dialog = box ? box.closest('[role="dialog"], [aria-modal="true"]') : null;
    if (!dialog) return false;
    const hits = Array.from(dialog.querySelectorAll('a[href*="/status/"]')).filter((link) => {{
      if (statusId(link.href) !== TARGET) return false;
      if ((link.getAttribute("href") || "").indexOf("in_reply_to") >= 0) return false;
      if (link.closest('[data-testid="tweetTextarea_0"], [contenteditable="true"]')) return false;
      return true;
    }});
    if (!hits.length) {{
      const norm = (value) => String(value || "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim();
      const tokenExact = (value) => {{
        const text = String(value || "");
        const at = text.indexOf(TARGET);
        if (at < 0) return false;
        const prev = at > 0 ? text.charAt(at - 1) : "";
        const next = text.charAt(at + TARGET.length);
        const digit = (ch) => ch >= "0" && ch <= "9";
        return !digit(prev) && !digit(next);
      }};
      const card = Array.from(dialog.querySelectorAll('[data-testid="attachments"]')).find((node) => !node.closest('[data-testid="tweetTextarea_0"], [contenteditable="true"]'));
      if (!card) return false;
      const avatar = card.querySelector('[data-testid="Tweet-User-Avatar"]');
      const textNode = card.querySelector('[data-testid="tweetText"]');
      if (!avatar || !textNode) return false;
      const cardText = norm(textNode.innerText || textNode.textContent);
      const blob = norm(card.innerText || card.textContent).toLowerCase();
      let attrExact = false;
      let attrOther = false;
      card.querySelectorAll("*").forEach((el) => {{
        ["href", "aria-label", "title", "id", "datetime"].forEach((key) => {{
          const value = el.getAttribute ? (el.getAttribute(key) || "") : "";
          if (!value) return;
          const sid = statusId(value);
          if (sid === TARGET || tokenExact(value)) attrExact = true;
          else if (sid) attrOther = true;
        }});
      }});
      if (attrOther) return false;
      const source = norm(SOURCE);
      const handleOk = !!SCREEN && blob.indexOf("@" + String(SCREEN).toLowerCase()) >= 0;
      const textOk = !!source && !!cardText && (cardText === source || (cardText.length >= 12 && source.indexOf(cardText) === 0));
      return attrExact || (handleOk && textOk);
    }}
    if (hits.some((link) => !!link.closest('[data-testid="quoteTweet"]'))) return true;
    let paramId = null;
    try {{ const raw = new URL(location.href).searchParams.get("url") || ""; const match = raw.match(/status\/(\d+)/); paramId = match ? match[1] : null; }} catch (e) {{ paramId = null; }}
    return location.pathname === "/compose/post" && paramId === TARGET;
  }};
  const base = (stage, message, clicked) => ({{
    stage: stage, path: location.pathname, composer: !!document.querySelector('[data-testid="tweetTextarea_0"]'),
    composer_kind: quoteAttachment() ? "quote" : "unknown", card: quoteAttachment(),
    quote_testids: quoteAttachment() ? 1 : 0,
    clicked_post: clicked, clicked_repost_confirm: false, text_landed: false,
    quoted_status_id: quoteAttachment() ? TARGET : null, message: message
  }});
  if (!quoteAttachment()) {{
    return finish(base("composer", "No quote attachment for " + TARGET + ". Post was not clicked.", false));
  }}
  const box = document.querySelector('[data-testid="tweetTextarea_0"]');
  if (!box) {{
    return finish(base("composer", "Quote attachment is visible but the composer box is not. Post was not clicked.", false));
  }}
  box.focus();
  if (!document.execCommand("insertText", false, TEXT)) {{
    const data = new DataTransfer();
    data.setData("text/plain", TEXT);
    box.dispatchEvent(new ClipboardEvent("paste", {{ clipboardData: data, bubbles: true, cancelable: true }}));
  }}
  await new Promise((resolve) => setTimeout(resolve, 600));
  const norm = (value) => String(value || "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim();
  const landed = norm(box.innerText || box.textContent || "").indexOf(norm(TEXT)) >= 0;
  const button = Array.from(document.querySelectorAll('[data-testid="tweetButton"], [data-testid="tweetButtonInline"]'))
    .find((el) => !el.disabled && el.getAttribute("aria-disabled") !== "true");
  if (!landed || !button || !quoteAttachment()) {{
    const stuck = base("editor", "Quote attachment, commentary, or Post button was not proven. Post was not clicked.", false);
    stuck.text_landed = landed;
    return finish(stuck);
  }}
  button.click();
  for (let i = 0; i < 30; i++) {{
    await new Promise((resolve) => setTimeout(resolve, 500));
    const toasts = Array.from(document.querySelectorAll('[role="alert"], [data-testid="toast"]'));
    const sent = toasts.find((el) => /sent|posted/i.test(el.textContent || ""));
    if (sent) {{
      const link = sent.querySelector('a[href*="/status/"]');
      const done = base("posted", "Quote tweet posted successfully.", true);
      done.text_landed = true;
      done.new_status_id = link ? statusId(link.href) : null;
      return finish(done);
    }}
  }}
  const unclear = base("posted", "Post was clicked once but no sent toast appeared. Not marked sent. Nothing was retried.", true);
  return finish(unclear);
}})()"#
    )
}

/// Reads the clicked article's handle and tweet text. No clicks.
pub fn quote_source_script(target_id: &str) -> String {
    let id_js = js_ascii_string(target_id);
    format!(
        r#"( () => {{ const TARGET = {id_js};
  const statusId = (href) => {{ try {{ const path = new URL(href, location.origin).pathname; const match = path.match(/\/status\/(\d+)/); return match ? match[1] : null; }} catch (e) {{ return null; }} }};
  const article = Array.from(document.querySelectorAll("article")).find((node) => Array.from(node.querySelectorAll("a[href]")).some((link) => statusId(link.href) === TARGET));
  let screen = "";
  if (article) {{
    for (const link of article.querySelectorAll("a[href]")) {{
      try {{
        const parsed = new URL(link.href, location.origin);
        const match = parsed.pathname.match(/^\/([A-Za-z0-9_]{{1,15}})\/status\/(\d+)\/?$/);
        if (match && match[2] === TARGET && match[1] !== "i") {{ screen = match[1]; break; }}
      }} catch (e) {{}}
    }}
  }}
  const textNode = article ? (Array.from(article.querySelectorAll('[data-testid="tweetText"]')).find((node) => !node.closest('[data-testid="quoteTweet"]')) || article.querySelector('[data-testid="tweetText"]')) : null;
  const text = textNode ? String(textNode.innerText || textNode.textContent || "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim().slice(0, 500) : "";
  const report = {{ stage: "source", source_screen: screen, source_text: text, card: false, clicked_post: false, composer_kind: "unknown", message: screen && text ? "source ready" : "source missing" }};
  return JSON.stringify(report);
}} )()"#
    )
}

pub fn quote_snap_script(target_id: &str) -> String {
    quote_snap_script_bound(target_id, "", "")
}

/// Read-only snapshot of the compose page. No clicks.
/// A quote card is the composer attachment that matches the article that was
/// quoted: same author handle and same tweet text, or an attribute in that
/// attachment that carries this exact status id. An editor URL is not a card.
/// A different author's card, or the same author with different text, is not.
pub fn quote_snap_script_bound(target_id: &str, screen: &str, source_text: &str) -> String {
    let id_js = js_ascii_string(target_id);
    let screen_js = js_ascii_string(screen);
    let source_js = js_ascii_string(source_text);
    format!(
        r#"( () => {{ const TERMY_QUOTE_SNAP = 1; const TARGET = {id_js}; const SCREEN = {screen_js}; const SOURCE = {source_js};
  const statusId = (href) => {{ try {{ const path = new URL(href, location.origin).pathname; const match = path.match(/\/status\/(\d+)/); return match ? match[1] : null; }} catch (e) {{ return null; }} }};
  const boxes = Array.from(document.querySelectorAll('[data-testid="tweetTextarea_0"]'));
  const box = boxes.length ? boxes[boxes.length - 1] : null;
  let place = "none";
  const paramId = (() => {{ try {{ const raw = new URL(location.href).searchParams.get("url") || ""; const match = raw.match(/status\/(\d+)/); return match ? match[1] : null; }} catch (e) {{ return null; }} }})();
  const safeName = (value) => String(value || "").replace(/[^A-Za-z0-9_-]/g, "").slice(0, 24);
  const chainOf = (node) => {{
    const parts = [];
    let cur = node;
    for (let i = 0; i < 6 && cur && cur !== document.body; i++) {{
      const role = cur.getAttribute ? (cur.getAttribute("role") || "-") : "-";
      const tid = cur.getAttribute ? (safeName(cur.getAttribute("data-testid")) || "-") : "-";
      parts.push((cur.tagName || "-") + ":" + role + ":" + tid);
      cur = cur.parentElement;
    }}
    return parts.join(">") || "none";
  }};
  const inEditor = (node) => !!(node && node.closest && node.closest('[data-testid="tweetTextarea_0"], [contenteditable="true"]'));
  const outsideEditor = (link) => statusId(link.href) === TARGET && (link.getAttribute("href") || "").indexOf("in_reply_to") < 0 && !inEditor(link);
  const relTo = (node, editor) => {{
    if (!editor) return "no-editor";
    if (inEditor(node)) return "inside-editor";
    if (typeof editor.compareDocumentPosition !== "function") return "other";
    const pos = editor.compareDocumentPosition(node);
    if (pos & 4) return "after-editor";
    if (pos & 2) return "before-editor";
    return "other";
  }};
  const pathClass = (pathname, sid) => {{
    const exact = sid === TARGET;
    if (/^\/i\/status\/\d+\/?$/.test(pathname)) return exact ? "/i/status/exact" : "/i/status/other";
    if (/^\/i\/web\/status\/\d+\/?$/.test(pathname)) return exact ? "/i/web/status/exact" : "/i/web/status/other";
    if (/^\/[A-Za-z0-9_]{{1,15}}\/status\/\d+\/?$/.test(pathname)) return exact ? "/screen/status/exact" : "/screen/status/other";
    if (pathname.indexOf("/status/") >= 0) return exact ? "/status/exact" : "/status/other";
    if (/^\/[A-Za-z0-9_./-]{{0,40}}$/.test(pathname)) return pathname.slice(0, 40) || "/";
    return "other";
  }};
  const linkBits = (node) => {{
    const raw = node.getAttribute ? (node.getAttribute("href") || "") : "";
    const role = node.getAttribute ? (node.getAttribute("role") || "-") : "-";
    const tid = node.getAttribute ? (safeName(node.getAttribute("data-testid")) || "-") : "-";
    let path = "none";
    let id = "none";
    let host = "none";
    let substr = "no";
    if (raw) {{
      substr = raw.indexOf("/status/") >= 0 ? "yes" : "no";
      try {{
        const parsed = new URL(raw, location.origin);
        const sid = statusId(parsed.href);
        host = (parsed.hostname === "x.com" || parsed.hostname === "twitter.com" || parsed.hostname === "mobile.twitter.com") ? "x" : "other";
        path = pathClass(parsed.pathname || "/", sid);
        if (sid === TARGET) id = "exact";
        else if (sid) id = "other";
        else if (raw.indexOf(TARGET) >= 0) id = "digits";
      }} catch (e) {{
        path = "unparsed";
        if (raw.indexOf(TARGET) >= 0) id = "digits";
      }}
    }}
    const label = (node.getAttribute && (node.getAttribute("aria-label") || node.getAttribute("title"))) || "";
    let name = "none";
    if (label.indexOf(TARGET) >= 0 || label.indexOf("/status/") >= 0) name = "url";
    else if (label && label.length <= 24 && /^[A-Za-z0-9 .@_-]+$/.test(label)) name = label.replace(/ /g, "-");
    else if (label) name = "prose";
    return (node.tagName || "-") + " role " + role + " testid " + tid + " path " + path + " id " + id + " host " + host + " status-substr " + substr + " name " + name;
  }};
  const textClass = (text) => {{
    const trimmed = String(text || "").replace(/\s+/g, " ").trim();
    if (!trimmed) return "empty";
    const marker = "/status/" + TARGET;
    const at = trimmed.indexOf(marker);
    const next = at < 0 ? "" : trimmed.charAt(at + marker.length);
    const exact = at >= 0 && (next < "0" || next > "9");
    const anyStatus = /\/status\/\d+/.test(trimmed);
    const words = trimmed.split(" ");
    const short = trimmed.length <= 180 && words.length <= 4;
    if (short && exact) return "url-exact";
    if (short && anyStatus) return "url-other";
    if (short && /https?:\/\//i.test(trimmed)) return "url-nonstatus";
    if (exact || trimmed.indexOf(TARGET) >= 0) return "prose-has-id";
    return "prose";
  }};
  const nearEditor = (editor) => {{
    if (!editor || !editor.parentElement) return "none";
    const kids = Array.from(editor.parentElement.children).slice(0, 6);
    if (!kids.length) return "none";
    return kids.map((kid) => {{
      const role = kid.getAttribute ? (kid.getAttribute("role") || "-") : "-";
      const tid = kid.getAttribute ? (safeName(kid.getAttribute("data-testid")) || "-") : "-";
      const text = String(kid.innerText || kid.textContent || "").replace(/\u00a0/g, " ");
      const mark = kid === editor ? "self" : "sib";
      return mark + " " + (kid.tagName || "-") + ":" + role + ":" + tid + " " + textClass(text) + " chars " + text.trim().length;
    }}).join(" ;; ");
  }};
  const dialogs = Array.from(document.querySelectorAll('[role="dialog"], [aria-modal="true"]')).slice(0, 3);
  const pageQuotes = document.querySelectorAll('[data-testid="quoteTweet"]').length;
  const inDialogBoxes = boxes.filter((el) => !!el.closest('[role="dialog"], [aria-modal="true"]')).length;
  let attached = false;
  let proofQuotes = 0;
  const hits = [];
  let urlKey = "no";
  let idMatch = "no";
  try {{
    const raw = new URL(location.href).searchParams.get("url") || "";
    if (raw) urlKey = "yes";
    const match = String(raw).match(/status\/(\d+)/);
    if (match && match[1] === TARGET) idMatch = "yes";
  }} catch (e) {{}}
  const bits = ["query url-key " + urlKey + " id-match " + idMatch + " textareas " + boxes.length + " in-dialog-textareas " + inDialogBoxes + " dialogs " + document.querySelectorAll('[role="dialog"], [aria-modal="true"]').length + " page-quotes " + pageQuotes + " ancestors " + (box ? chainOf(box) : "none")];
  dialogs.forEach((dlg, index) => {{
    const qNodes = Array.from(dlg.querySelectorAll('[data-testid="quoteTweet"]'));
    const qHits = qNodes.filter((node) => Array.from(node.querySelectorAll('a[href*="/status/"]')).some((link) => statusId(link.href) === TARGET));
    const links = Array.from(dlg.querySelectorAll('a[href*="/status/"]')).filter(outsideEditor);
    const names = [];
    for (const el of dlg.querySelectorAll("[data-testid]")) {{
      const name = safeName(el.getAttribute("data-testid"));
      if (!name || !/quote|attach|card|tweet|media|photo|poll/i.test(name)) continue;
      if (names.indexOf(name) < 0 && names.length < 8) names.push(name);
    }}
    const hasBox = !!dlg.querySelector('[data-testid="tweetTextarea_0"]');
    const hasPost = !!dlg.querySelector('[data-testid="tweetButton"], [data-testid="tweetButtonInline"]');
    if (hasBox && qHits.length) {{ attached = true; proofQuotes += qHits.length; }}
    if (hasBox && location.pathname === "/compose/post" && paramId === TARGET && links.length) {{
      attached = true;
      links.forEach((link) => hits.push(link));
    }}
    const norm = (value) => String(value || "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim();
    const tokenExact = (value) => {{
      const text = String(value || "");
      const at = text.indexOf(TARGET);
      if (at < 0) return false;
      const prev = at > 0 ? text.charAt(at - 1) : "";
      const next = text.charAt(at + TARGET.length);
      const digit = (ch) => ch >= "0" && ch <= "9";
      return !digit(prev) && !digit(next);
    }};
    const card = Array.from(dlg.querySelectorAll('[data-testid="attachments"]')).find((node) => !inEditor(node));
    let bind = "none";
    if (card) {{
      const avatar = card.querySelector('[data-testid="Tweet-User-Avatar"]');
      const textNode = card.querySelector('[data-testid="tweetText"]');
      const cardText = norm(textNode && (textNode.innerText || textNode.textContent));
      const blob = norm(card.innerText || card.textContent).toLowerCase();
      let attrExact = false;
      let attrOther = false;
      card.querySelectorAll("*").forEach((el) => {{
        ["href", "aria-label", "title", "id", "datetime"].forEach((key) => {{
          const value = el.getAttribute ? (el.getAttribute(key) || "") : "";
          if (!value) return;
          const sid = statusId(value);
          if (sid === TARGET || tokenExact(value)) attrExact = true;
          else if (sid) attrOther = true;
        }});
      }});
      const source = norm(SOURCE);
      const handleOk = !!SCREEN && blob.indexOf("@" + String(SCREEN).toLowerCase()) >= 0;
      const textOk = !!source && !!cardText && (cardText === source || (cardText.length >= 12 && source.indexOf(cardText) === 0));
      if (hasBox && avatar && textNode && !attrOther && (attrExact || (handleOk && textOk))) {{
        attached = true;
        proofQuotes += 1;
        bind = attrExact ? "id" : "article";
      }} else if (attrOther) bind = "other-id";
      else bind = "miss";
    }}
    const attachNodes = Array.from(dlg.querySelectorAll('[data-testid="attachments"]')).filter((node) => !inEditor(node));
    const dialogTextNode = Array.from(dlg.querySelectorAll('[data-testid="tweetText"]')).find((node) => !inEditor(node)) || null;
    const dialogAvatar = Array.from(dlg.querySelectorAll('[data-testid="Tweet-User-Avatar"]')).find((node) => !inEditor(node)) || null;
    const textIn = !!(card && dialogTextNode && card.contains(dialogTextNode));
    const avatarIn = !!(card && dialogAvatar && card.contains(dialogAvatar));
    const fold = (value) => String(value || "").replace(/\u00a0/g, " ").replace(/\s+/g, " ").trim();
    const sourceNorm = fold(SOURCE);
    const cardNorm = fold(dialogTextNode && (dialogTextNode.innerText || dialogTextNode.textContent));
    const asciiFold = (value) => value.replace(/[^\u0000-\u007F]/g, "");
    const attachBlob = fold(card && (card.innerText || card.textContent)).toLowerCase();
    const dialogBlob = fold(dlg.innerText || dlg.textContent).toLowerCase();
    const needle = SCREEN ? ("@" + String(SCREEN).toLowerCase()) : "";
    const compare = "attach-count " + attachNodes.length
      + " avatar-in " + (avatarIn ? "yes" : "no")
      + " text-in " + (textIn ? "yes" : "no")
      + " dialog-avatar " + (dialogAvatar ? "yes" : "no")
      + " dialog-text " + (dialogTextNode ? "yes" : "no")
      + " source-chars " + sourceNorm.length
      + " card-chars " + cardNorm.length
      + " text-equal " + (sourceNorm && cardNorm && sourceNorm === cardNorm ? "yes" : "no")
      + " text-prefix " + (sourceNorm && cardNorm && cardNorm.length >= 12 && (sourceNorm.indexOf(cardNorm) === 0 || cardNorm.indexOf(sourceNorm) === 0) ? "yes" : "no")
      + " ascii-equal " + (sourceNorm && cardNorm && asciiFold(sourceNorm) === asciiFold(cardNorm) ? "yes" : "no")
      + " handle-attach " + (needle && attachBlob.indexOf(needle) >= 0 ? "yes" : "no")
      + " handle-dialog " + (needle && dialogBlob.indexOf(needle) >= 0 ? "yes" : "no")
      + " text-chain " + (dialogTextNode ? chainOf(dialogTextNode) : "none");
    const editor = dlg.querySelector('[data-testid="tweetTextarea_0"]');
    const plain = editor ? String(editor.innerText || editor.textContent || "").replace(/\u00a0/g, " ") : "";
    const seen = [];
    const add = (node) => {{ if (node && seen.indexOf(node) < 0) seen.push(node); }};
    dlg.querySelectorAll('a, [role="link"]').forEach(add);
    dlg.querySelectorAll("[href]").forEach((node) => {{
      const href = node.getAttribute("href") || "";
      if (href.indexOf("/status/") >= 0 || href.indexOf(TARGET) >= 0) add(node);
    }});
    const ranked = seen.slice().sort((left, right) => {{
      const score = (node) => {{ const rel = relTo(node, editor); return rel === "inside-editor" ? 2 : (rel === "other" || rel === "no-editor" ? 1 : 0); }};
      return score(left) - score(right);
    }});
    const shown = ranked.slice(0, 3).map((node) => relTo(node, editor) + " " + linkBits(node));
    const inside = seen.filter((node) => relTo(node, editor) === "inside-editor").length;
    bits.push("d" + (index + 1) + " box " + (hasBox ? "yes" : "no") + " post " + (hasPost ? "yes" : "no") + " quotes " + qNodes.length + " target-links " + links.length + " anchors " + dlg.querySelectorAll("a").length + " rolelinks " + dlg.querySelectorAll('[role="link"]').length + " inside-editor " + inside + " editor " + textClass(plain) + " chars " + plain.trim().length + " card-bind " + bind + " " + compare + " near " + nearEditor(editor) + " testids " + (names.join(",") || "none") + (shown.length ? " links " + shown.join(" ;; ") : " links none"));
  }});
  const ids = [];
  const composer = dialogs.find((dlg) => dlg.querySelector('[data-testid="tweetTextarea_0"]')) || null;
  if (composer) {{ for (const link of composer.querySelectorAll('a[href*="/status/"]')) {{ if ((link.getAttribute("href") || "").indexOf("in_reply_to") >= 0) continue; if (link.closest('[data-testid="tweetTextarea_0"], [contenteditable="true"]')) continue; const id = statusId(link.href); if (id && ids.indexOf(id) < 0 && ids.length < 8) ids.push(id); }} }}
  const attach = bits.join(" | ");
  const scope = composer;
  if (box) {{ if (scope && scope.contains(box)) place = "dialog"; else if (box.closest("article")) place = "article"; else place = "page"; }}
  const dialogText = scope ? String(scope.innerText || "").slice(0, 500) : "";
  const replying = /replying to/i.test(dialogText);
  const kind = attached ? "quote" : (replying ? "reply" : "unknown");
  const report = {{ stage: attached ? "inspect" : "composer", path: location.pathname, composer: boxes.length > 0, composer_kind: kind, card: attached, status_ids: ids, quote_testids: attached ? Math.max(proofQuotes, hits.length, 1) : 0, testid_count: document.querySelectorAll("[data-testid]").length, clicked_post: false, clicked_repost_confirm: false, quoted_status_id: attached ? TARGET : null, topo_after: place + " textareas=" + boxes.length + " quotes=" + pageQuotes + " dialogs=" + document.querySelectorAll('[role="dialog"], [aria-modal="true"]').length, attach: attach, message: attached ? "Quote attachment is visible. Post was not clicked." : "Quote card was not proven for " + TARGET + ". A visible link is not the card. Post was not clicked." }};
  window.__qxReport = report;
  return JSON.stringify(report);
}} )()"#
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotePageReport {
    pub stage: String,
    pub path: String,
    pub testid_count: u64,
    pub composer: bool,
    /// `quote`, `reply`, or `unknown`. A status link alone is not `quote`.
    pub composer_kind: String,
    pub card: bool,
    pub status_ids: Vec<String>,
    pub quote_testids: u64,
    pub clicked_post: bool,
    pub clicked_repost_confirm: bool,
    pub text_landed: bool,
    pub quoted_status_id: Option<String>,
    pub new_status_id: Option<String>,
    pub selected_label: String,
    pub selected_testid: String,
    pub selected_href_kind: String,
    pub selected_scope: String,
    pub topo_before: String,
    pub topo_after: String,
    pub menu_shape: String,
    /// Sanitized composer-dialog attachment shape. Not proof by itself.
    pub attach: String,
    /// Author handle and tweet text from the clicked status article.
    /// The text is compared to the composer card and is not printed.
    pub source_screen: String,
    pub source_text: String,
    pub message: String,
}

fn first_json_value(stdout: &str) -> Option<serde_json::Value> {
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&stdout[start..=end]).ok()
}

fn unwrap_opencli_data(value: serde_json::Value) -> serde_json::Value {
    let mut current = value;
    for _ in 0..4 {
        if current.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
            return current;
        }
        if let Some(data) = current.get("data").cloned() {
            current = json_if_string(data);
            continue;
        }
        if let Some(nested) = current
            .get("result")
            .cloned()
            .or_else(|| current.get("value").cloned())
        {
            let nested = json_if_string(nested);
            if nested != current {
                current = nested;
                continue;
            }
        }
        break;
    }
    current
}

fn json_if_string(value: serde_json::Value) -> serde_json::Value {
    let Some(text) = value.as_str() else {
        return value;
    };
    if text.trim().is_empty() || text == "pending" {
        return serde_json::Value::String(text.to_string());
    }
    serde_json::from_str(text).unwrap_or(serde_json::Value::String(text.to_string()))
}

fn payload_summary(value: &serde_json::Value) -> String {
    let keys = value
        .as_object()
        .map(|map| {
            map.keys()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| value_kind(value));
    let data = value.get("data").map(value_kind).unwrap_or_else(|| "missing".into());
    format!("payload keys {keys} · data {data}")
}

fn value_kind(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::String(text) if text == "pending" => "pending".into(),
        serde_json::Value::String(_) => "string".into(),
        serde_json::Value::Array(rows) => format!("array {}", rows.len()),
        serde_json::Value::Object(map) if map.is_empty() => "empty object".into(),
        serde_json::Value::Object(_) => "object".into(),
        serde_json::Value::Bool(flag) => format!("bool {flag}"),
        serde_json::Value::Number(_) => "number".into(),
    }
}

pub fn parse_quote_page_report(stdout: &str) -> Result<QuotePageReport, String> {
    let value = first_json_value(stdout).ok_or_else(|| {
        "stage result · eval returned no JSON · Post was not clicked".to_string()
    })?;
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        return Err(explain_opencli_failure("result", "browser eval", 0, stdout, ""));
    }
    let inner = unwrap_opencli_data(value.clone());
    let report = report_from_value(&inner);
    if report.stage.is_empty() {
        return Err(format!(
            "stage result · {} · no quote report · Post was not clicked",
            payload_summary(&value)
        ));
    }
    Ok(report)
}

/// An empty `{ok:true,data:{}}` envelope is what an un-awaited eval looks like.
/// Poll `window.__qxReport` for that. A real error envelope is final.
pub fn eval_needs_poll(stdout: &str) -> bool {
    if stdout.contains("pending") && !stdout.contains("\"stage\"") {
        return true;
    }
    let Some(value) = first_json_value(stdout) else {
        return false;
    };
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        return false;
    }
    report_from_value(&unwrap_opencli_data(value)).stage.is_empty()
}

fn report_from_value(value: &serde_json::Value) -> QuotePageReport {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let flag = |key: &str| value.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
    let ids = value
        .get("status_ids")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    QuotePageReport {
        stage: text("stage"),
        path: text("path"),
        testid_count: value.get("testid_count").and_then(serde_json::Value::as_u64).unwrap_or(0),
        composer: flag("composer"),
        composer_kind: text("composer_kind"),
        card: flag("card"),
        status_ids: ids,
        quote_testids: value
            .get("quote_testids")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        clicked_post: flag("clicked_post"),
        clicked_repost_confirm: flag("clicked_repost_confirm"),
        text_landed: flag("text_landed"),
        quoted_status_id: value
            .get("quoted_status_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        new_status_id: value
            .get("new_status_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        selected_label: text("selected_label"),
        selected_testid: text("selected_testid"),
        selected_href_kind: text("selected_href_kind"),
        selected_scope: text("selected_scope"),
        topo_before: text("topo_before"),
        topo_after: text("topo_after"),
        menu_shape: text("menu_shape"),
        attach: text("attach"),
        source_screen: text("source_screen"),
        source_text: text("source_text"),
        message: text("message"),
    }
}

pub fn quote_report_line(report: &QuotePageReport) -> String {
    let ids = if report.status_ids.is_empty() {
        "none".to_string()
    } else {
        report.status_ids.join(",")
    };
    format!(
        "stage {} · path {} · kind {} · card {} · composer {} · status ids {} · quote nodes {} · testids {} · post clicked {} · selected {} {} {} {} · before {} · after {} · menu {} · attach {} · {}",
        report.stage,
        if report.path.is_empty() { "-" } else { &report.path },
        if report.composer_kind.is_empty() { "-" } else { &report.composer_kind },
        if report.card { "yes" } else { "no" },
        if report.composer { "yes" } else { "no" },
        ids,
        report.quote_testids,
        report.testid_count,
        if report.clicked_post { "yes" } else { "no" },
        if report.selected_scope.is_empty() { "-" } else { &report.selected_scope },
        if report.selected_label.is_empty() { "-" } else { &report.selected_label },
        if report.selected_testid.is_empty() { "-" } else { &report.selected_testid },
        if report.selected_href_kind.is_empty() { "-" } else { &report.selected_href_kind },
        if report.topo_before.is_empty() { "-" } else { &report.topo_before },
        if report.topo_after.is_empty() { "-" } else { &report.topo_after },
        if report.menu_shape.is_empty() { "-" } else { &report.menu_shape },
        if report.attach.is_empty() { "-" } else { &report.attach },
        report.message
    )
}

/// Post is allowed only for a quote attachment of this exact status.
/// A reply composer that merely links the original is not enough.
pub fn quote_inspect_ready(report: &QuotePageReport) -> bool {
    report.composer_kind == "quote"
        && report.card
        && report.quote_testids > 0
        && report.quoted_status_id.is_some()
        && !report.clicked_post
        && !report.clicked_repost_confirm
}

/// Post is allowed only for a quote attachment of this exact status.
/// A reply composer that merely links the original is not enough.
pub fn quote_post_allowed(report: &QuotePageReport, target_id: &str) -> bool {
    let target_id = crate::model::post_id_from_input(target_id);
    report.quote_testids > 0
        && report.card
        && report.composer
        && report.composer_kind == "quote"
        && report.quoted_status_id.as_deref() == Some(target_id.as_str())
        && report.text_landed
        && !report.clicked_repost_confirm
}

pub fn quote_read_confirms(report: &QuotePageReport, target_id: &str) -> Result<(), String> {
    quote_outcome(None, report, target_id)
}

/// When Post already happened, name the published status and what it was.
pub fn quote_outcome(
    new_id: Option<&str>,
    report: &QuotePageReport,
    target_id: &str,
) -> Result<(), String> {
    let target_id = crate::model::post_id_from_input(target_id);
    if report.clicked_repost_confirm {
        return Err("The quote check tried to repost. Not marked sent. Nothing was retried.".into());
    }
    let proven = report.quote_testids > 0
        && report.composer_kind != "reply"
        && report.quoted_status_id.as_deref() == Some(target_id.as_str());
    if proven {
        return Ok(());
    }
    let link = new_id
        .filter(|id| !id.is_empty())
        .map(|id| format!("https://x.com/i/status/{id}"))
        .unwrap_or_else(|| "the new status".into());
    let kind = if report.composer_kind == "reply"
        || (report.quote_testids == 0 && report.status_ids.iter().any(|id| id == &target_id))
    {
        "a reply"
    } else {
        "not a confirmed quote"
    };
    let relation = if kind == "a reply" { "to" } else { "of" };
    Err(format!(
        "Clicked once. {link} is {kind} {relation} {target_id}, not a quote. Draft kept. No retry."
    ))
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
        let opened = if self.open_browser {
            "Opened the browser composer. "
        } else {
            ""
        };
        let attach = if media.is_empty() {
            String::new()
        } else {
            let list = media
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(" Attach these images in the browser: {list}.")
        };
        Err(format!(
            "{opened}Termy cannot confirm a web-intent post.{attach} Not marked Posted."
        ))
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

/// Joins commentary and a status URL. This is not a quote. OpenCLI's
/// `twitter post` confirms only a sent toast, and X can drop the URL
/// from the body without attaching a quoted tweet. Quotes go through
/// `twitter quote` and `prove_quote`.
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

        let confirmed = r#"[{"status":"success","message":"Tweet successfully retweeted."}]"#;
        assert!(prove_engage(EngageAction::Retweet, confirmed, None).is_ok());
        assert!(prove_engage(
            EngageAction::Like,
            r#"[{"status":"success","message":"Tweet successfully liked."}]"#,
            None
        )
        .is_ok());
        assert!(prove_opencli_write(
            OpenCliWrite::Reply,
            r#"[{"status":"success","message":"Reply posted successfully."}]"#,
            None
        )
        .is_ok());
        assert!(prove_opencli_write(
            OpenCliWrite::Follow,
            r#"[{"status":"success","message":"Successfully followed @jack."}]"#,
            Some("jack")
        )
        .is_ok());
        assert!(
            prove_opencli_write(
                OpenCliWrite::Follow,
                r#"[{"status":"success","message":"Successfully followed @other."}]"#,
                Some("jack")
            )
            .unwrap_err()
            .contains("not @jack")
        );

        for bad in [
            "",
            "   ",
            "unconfirmed — check before retrying",
            "not json",
            r#"{"ok":true}"#,
            r#"{"ok":true,"id":"555"}"#,
            r#"{"ok":false,"error":{"message":"nope"}}"#,
            r#"[{"status":"success","id":"555"}]"#,
            r#"[{"status":"success","message":"Tweet successfully retweeted.","unconfirmed":true}]"#,
            "[]",
        ] {
            let err = prove_engage(EngageAction::Retweet, bad, None).unwrap_err();
            assert!(err.contains("Not marked"), "{bad} -> {err}");
            assert!(err.contains("Nothing was retried"), "{err}");
        }

        assert!(prove_new_post(
            r#"[{"status":"success","message":"Tweet posted successfully.","id":"42"}]"#
        )
        .unwrap()
        .contains("42"));
        assert!(prove_new_post(r#"{"ok":true,"id":"42"}"#).is_err());
        assert!(prove_new_post("").is_err());
        assert!(prove_reply(r#"[{"status":"success","message":"Reply posted successfully."}]"#).is_ok());
    }

    #[test]
    fn gui_write_defaults_to_dry_run_and_persists_one_confirmed_call() {
        use crate::action_state::{ActionDb, ReceiptKind, ToggleKind};
        struct LogPublisher {
            publish: Mutex<Vec<(Vec<String>, Option<String>)>>,
            engage: Mutex<Vec<(EngageAction, String, Option<String>)>>,
            fail: Mutex<bool>,
        }
        impl Publisher for LogPublisher {
            fn id(&self) -> ProviderId {
                ProviderId::OpenCli
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
                reply_to: Option<&str>,
                _media: &[PathBuf],
            ) -> Result<String, String> {
                self.publish
                    .lock()
                    .expect("publish")
                    .push((parts.to_vec(), reply_to.map(str::to_string)));
                if *self.fail.lock().expect("fail") {
                    return Err("provider rejected".into());
                }
                Ok("Posted 42".into())
            }
            fn engage(
                &self,
                action: EngageAction,
                target_url: &str,
                text: Option<&str>,
                _media: &[PathBuf],
            ) -> Result<String, String> {
                self.engage.lock().expect("engage").push((
                    action,
                    target_url.to_string(),
                    text.map(str::to_string),
                ));
                if *self.fail.lock().expect("fail") {
                    return Err("provider rejected".into());
                }
                if action == EngageAction::Retweet {
                    Ok(r#"[{"status":"success","message":"Tweet successfully retweeted."}]"#.into())
                } else {
                    Ok(r#"[{"status":"success","message":"Quote tweet posted successfully.","quoted_status_id_str":"2118971448614334959","text":"my words"}]"#.into())
                }
            }
        }
        let publisher = LogPublisher {
            publish: Mutex::new(Vec::new()),
            engage: Mutex::new(Vec::new()),
            fail: Mutex::new(false),
        };
        let mut db = ActionDb::default();
        let dry = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Post {
                    text: "  hello  ".into(),
                },
                confirm: None,
                account: "Jasper_Black".into(),
                char_limit: 280,
            },
            10,
            1,
        );
        assert!(matches!(dry, GuiWriteOutcome::DryRun { .. }));
        assert!(publisher.publish.lock().expect("publish").is_empty());
        assert!(db.receipt_ids("Jasper_Black", ReceiptKind::Post).is_empty());

        let sent = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Post {
                    text: "hello".into(),
                },
                confirm: Some("yes".into()),
                account: "Jasper_Black".into(),
                char_limit: 280,
            },
            11,
            2,
        );
        assert!(matches!(sent, GuiWriteOutcome::Sent { .. }));
        assert_eq!(
            publisher.publish.lock().expect("publish").clone(),
            vec![(vec!["hello".to_string()], None)]
        );

        let reply = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Reply {
                    target_id: "https://x.com/cryptoXRPwhale/status/2118971448614334959".into(),
                    text: "reply text".into(),
                },
                confirm: Some("yes".into()),
                account: "jasper_black".into(),
                char_limit: 280,
            },
            12,
            3,
        );
        assert!(matches!(reply, GuiWriteOutcome::Sent { .. }));
        assert_eq!(
            publisher.publish.lock().expect("publish")[1].1.as_deref(),
            Some("2118971448614334959")
        );
        assert_eq!(
            db.receipt_ids("jasper_black", ReceiptKind::Reply),
            vec!["2118971448614334959".to_string()]
        );

        let quote = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Quote {
                    target_id: "2118971448614334959".into(),
                    text: "my words".into(),
                },
                confirm: Some("yes".into()),
                account: "jasper_black".into(),
                char_limit: 280,
            },
            13,
            4,
        );
        assert!(matches!(quote, GuiWriteOutcome::Sent { .. }));
        let engage = publisher.engage.lock().expect("engage").clone();
        assert_eq!(engage.len(), 1);
        assert_eq!(engage[0].0, EngageAction::Quote);
        assert_eq!(
            engage[0].1,
            "https://x.com/i/status/2118971448614334959"
        );
        assert_eq!(db.receipt_ids("jasper_black", ReceiptKind::QTweet), vec![
            "2118971448614334959".to_string()
        ]);

        let repost = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Repost {
                    target_id: "2118971448614334959".into(),
                },
                confirm: Some("yes".into()),
                account: "jasper_black".into(),
                char_limit: 280,
            },
            14,
            5,
        );
        assert!(matches!(repost, GuiWriteOutcome::Sent { .. }));
        assert_eq!(
            db.confirmed("jasper_black", ToggleKind::Repost, "2118971448614334959"),
            Some(true)
        );
        assert_eq!(publisher.engage.lock().expect("engage").len(), 2);

        *publisher.fail.lock().expect("fail") = true;
        let before = publisher.engage.lock().expect("engage").len();
        let failed = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Repost {
                    target_id: "20".into(),
                },
                confirm: Some("yes".into()),
                account: "jasper_black".into(),
                char_limit: 280,
            },
            15,
            6,
        );
        assert!(matches!(failed, GuiWriteOutcome::Rejected { .. }));
        assert_eq!(publisher.engage.lock().expect("engage").len(), before + 1);
        assert_eq!(
            db.confirmed("jasper_black", ToggleKind::Repost, "20"),
            None
        );
        let not_yes = run_gui_write(
            &publisher,
            &mut db,
            &GuiWriteRequest {
                write: GuiWrite::Post {
                    text: "nope".into(),
                },
                confirm: Some("Yes".into()),
                account: "jasper_black".into(),
                char_limit: 280,
            },
            16,
            7,
        );
        assert!(matches!(not_yes, GuiWriteOutcome::DryRun { .. }));
        assert_eq!(publisher.publish.lock().expect("publish").len(), 2);
    }

    #[test]
    fn quote_receipt_requires_the_quote_relation() {
        let original = "2118971448614334959";
        let commentary = "A token count alone doesn’t tell you whether a position fits your goals. The more useful questions are time horizon, concentration and how much downside you can tolerate.";
        let lost_url = format!(
            "[{{\"status\":\"success\",\"message\":\"Tweet posted successfully.\",\"id\":\"2119067545567121679\",\"text\":\"{commentary}\"}}]"
        );
        let err = prove_quote(&lost_url, original).unwrap_err();
        assert!(err.contains("not a quote"), "{err}");
        assert!(err.contains(original), "{err}");
        assert!(err.contains("Nothing was retried"), "{err}");

        let url_in_text = format!(
            "[{{\"status\":\"success\",\"message\":\"Tweet posted successfully.\",\"id\":\"2119067545567121679\",\"text\":\"{commentary}\\nhttps://x.com/i/status/{original}\"}}]"
        );
        assert!(prove_quote(&url_in_text, original).is_err());

        let other = r#"[{"status":"success","message":"Quote tweet posted successfully.","quoted_status_id_str":"20","text":"words"}]"#;
        let err = prove_quote(other, original).unwrap_err();
        assert!(err.contains("not 2118971448614334959"), "{err}");

        let exact = format!(
            "[{{\"status\":\"success\",\"message\":\"Quote tweet posted successfully.\",\"quoted_status_id_str\":\"{original}\",\"text\":\"{commentary}\"}}]"
        );
        assert!(prove_quote(&exact, original).is_ok());
        assert_eq!(quote_saved_receipt(original), format!("Quoted {original}"));

        let command_receipt = format!(
            "[{{\"status\":\"success\",\"message\":\"Quote tweet posted successfully.\",\"text\":\"{commentary}\"}}]"
        );
        let err = prove_quote(&command_receipt, original).unwrap_err();
        assert!(err.contains("not in the receipt"), "{err}");
        assert!(prove_quote(r#"{"ok":true,"id":"2119067545567121679"}"#, original).is_err());
    }

    #[test]
    fn browser_help_picks_the_installed_session_syntax() {
        let current = "Usage: opencli browser open <url> --session <name>\n--session <name>\n--window <mode>\n";
        let cli = browser_cli_from_help(current);
        assert!(cli.session_flag);
        assert!(cli.window_flag);
        let args = browser_command(cli, "termyx-quote", "open", Some("https://x.com/i/status/20"));
        assert_eq!(
            args,
            vec![
                "browser",
                "open",
                "https://x.com/i/status/20",
                "--session",
                "termyx-quote",
                "--window",
                "background"
            ]
        );
        let older = "Usage: opencli browser <session> <command>\nbrowser <session> open <url>\n--window background\n";
        let cli = browser_cli_from_help(older);
        assert!(!cli.session_flag);
        let args = browser_command(cli, "termyx-quote", "open", Some("https://x.com/i/status/20"));
        assert_eq!(args[0], "browser");
        assert_eq!(args[1], "termyx-quote");
        assert_eq!(args[2], "open");
        let explained = explain_opencli_failure(
            "navigation",
            "browser open <status> --session termyx-quote",
            1,
            r#"{"ok":false,"error":{"code":"NAVIGATION","message":"Navigation rejected.","hint":"ct0=secret"}}"#,
            "",
        );
        assert!(explained.contains("stage navigation"), "{explained}");
        assert!(explained.contains("NAVIGATION"), "{explained}");
        assert!(explained.contains("Navigation rejected"), "{explained}");
        assert!(explained.contains("exit 1"), "{explained}");
        assert!(!explained.contains("ct0"));
        assert!(!explained.contains("secret"));
        let empty = explain_opencli_failure("bridge", "browser --help", 1, "", "");
        assert!(empty.contains("printed nothing"), "{empty}");
        assert!(empty.contains("stage bridge"), "{empty}");
        let wrapped = r#"{"ok":true,"data":"{\"stage\":\"composer\",\"path\":\"/i/status/20\",\"message\":\"Quote card for 20 was not in the composer. Post was not clicked.\",\"card\":false,\"composer\":true,\"status_ids\":[],\"quote_testids\":0,\"testid_count\":12,\"clicked_post\":false}"}"#;
        let report = parse_quote_page_report(wrapped).unwrap();
        assert_eq!(report.stage, "composer");
        assert!(!report.card);
        assert!(report.message.contains("Post was not clicked"));
        let nested = r#"{"ok":true,"data":{"stage":"menu","path":"/i/status/20","message":"Quote menu item did not appear. Post was not clicked.","card":false,"testid_count":4}}"#;
        assert_eq!(parse_quote_page_report(nested).unwrap().stage, "menu");
        let blank = r#"{"ok":true,"data":{}}"#;
        let err = parse_quote_page_report(blank).unwrap_err();
        assert!(err.contains("stage result"), "{err}");
        assert!(err.contains("empty object"), "{err}");
        assert!(err.contains("Post was not clicked"), "{err}");
        assert!(eval_needs_poll(blank));
        assert!(!eval_needs_poll(r#"{"ok":false,"error":{"code":"NAVIGATION","message":"Navigation rejected.","hint":"ct0=secret"}}"#));
        let failed = parse_quote_page_report(
            r#"{"ok":false,"error":{"code":"NAVIGATION","message":"Navigation rejected.","hint":"ct0=secret"}}"#,
        )
        .unwrap_err();
        assert!(failed.contains("Navigation rejected"), "{failed}");
        assert!(!failed.contains("ct0"));
        assert!(parse_quote_page_report("").is_err());
    }

    #[test]
    fn quote_inspect_script_does_not_click_post_or_repost_confirm() {
        let original = "2118971448614334959";
        let inspect = quote_page_script(original, Some("words"), true);
        let write = quote_page_script(original, Some("words"), false);
        assert!(inspect.contains("const INSPECT = true"));
        assert!(write.contains("const INSPECT = false"));
        assert!(!inspect.contains("preventDefault"));
        assert!(!write.contains("preventDefault"));
        assert!(!inspect.contains("quoteItem.click"));
        assert!(!write.contains("button.click"));
        assert!(inspect.contains("opener.click"));
        assert!(!inspect.contains("retweetConfirm'].click"));
        assert!(!write.contains("retweetConfirm'].click"));
        let post = quote_type_and_post_script(original, "doesn’t");
        let guard = post.find("!quoteAttachment()").expect("attachment guard");
        let post_click = post.find("button.click()").expect("post click");
        assert!(guard < post_click);
        assert!(!post.contains("retweetConfirm"));
        assert!(post.is_ascii());
        assert!(post.contains("\\u2019"));
        let snap = quote_snap_script(original);
        assert!(!snap.contains("click("));
        assert!(snap.contains("quoteTweet"));
        assert!(snap.contains("searchParams"));
        assert!(snap.contains("in_reply_to"));
        assert!(snap.contains("tweetTextarea_0"));
        assert!(snap.contains("in-dialog-textareas"));
        assert!(snap.contains("page-quotes"));
        assert!(snap.contains("inside-editor"));
        assert!(snap.contains("rolelinks"));
        assert!(snap.contains("url-exact"));
        assert!(snap.contains("text-equal"));
        assert!(snap.contains("avatar-in"));
        assert!(snap.contains("handle-attach"));
        let menu = crate::publish::quote_menu_read_script(original);
        assert!(!menu.contains(".click("));
        assert!(menu.contains("Quote row recorded"));
        assert!(menu.contains("retweetConfirm"));
        let opener = crate::publish::quote_opener_click_args(
            browser_cli_from_help(""),
            "termyx-quote",
            original,
            "retweet",
            None,
        );
        assert!(opener.iter().any(|arg| arg.contains("[data-testid=\"retweet\"]")));
        assert!(opener.iter().any(|arg| arg.contains("/status/2118971448614334959")));
        assert!(!opener.iter().any(|arg| arg.contains("Confirm")));
        assert_eq!(
            crate::publish::quote_click_note(r#"{"ok":true,"data":{"clicked":true,"click_method":"cdp"}}"#),
            "click cdp"
        );
        assert!(!crate::publish::quote_click_trusted(r#"{"ok":true,"data":{"click_method":"js"}}"#));
        assert!(crate::publish::quote_click_trusted(r#"{"ok":true,"data":{"click_method":"ax"}}"#));
        assert!(snap.contains("A visible link is not the card"));
        assert!(snap.contains("ancestors"));
        assert!(snap.contains("hasBox && qHits.length"));
        let composer = quote_composer_url(original);
        assert!(composer.starts_with("https://x.com/compose/post?url=https://x.com/i/status/"));
        assert!(composer.contains("2118971448614334959"));
        assert!(!composer.contains('%'));
        let pages = quote_session_pages(
            r#"{"ok":true,"data":[{"page":"E54BF7E091EA017B99E02EF7373E08BC","url":"https://x.com/compose/post"},{"page":"not-an-id"}]}"#,
        );
        assert_eq!(pages, vec!["E54BF7E091EA017B99E02EF7373E08BC".to_string()]);
        let click = quote_native_click_args(browser_cli_from_help(""), "termyx-quote", None);
        assert!(click.windows(2).any(|pair| pair[0] == "--role" && pair[1] == "menuitem"));
        assert!(click.windows(2).any(|pair| pair[0] == "--name" && pair[1] == "Quote"));
        assert!(click.windows(2).any(|pair| pair[0] == "--window" && pair[1] == "foreground"));
        assert!(!click.iter().any(|arg| arg == "background"));
        assert!(!click.iter().any(|arg| arg.to_ascii_lowercase().contains("confirm")));
        let positional = quote_native_click_args(
            browser_cli_from_help("Usage: opencli browser <session> <command>\n"),
            "termyx-quote",
            None,
        );
        assert_eq!(positional[1], "termyx-quote");
        assert!(positional.windows(2).any(|pair| pair[0] == "--name" && pair[1] == "Quote"));
        let tab = "A03BB6CCAE9402E75FCA7CB1AAED51FA";
        let targeted = quote_native_click_args(browser_cli_from_help(""), "termyx-quote", Some(tab));
        assert!(targeted.windows(2).any(|pair| pair[0] == "--tab" && pair[1] == tab));
        assert_eq!(quote_tab_count(r#"{"ok":true,"data":[]}"#), Some(0));
        assert_eq!(
            quote_surface_target(
                r#"{"ok":true,"data":{"targetId":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#
            )
            .map(|target| target.id),
            Some(tab.to_string())
        );
        let opened = r#"{"ok":true,"data":{"url":"https://x.com/i/status/2118971448614334959","page":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#;
        let parsed = quote_surface_target(opened).expect("page id");
        assert_eq!(parsed.id, tab);
        assert_eq!(parsed.field, "page");
        assert!(quote_open_shape(opened).contains("page target"), "{}", quote_open_shape(opened));
        assert!(quote_open_shape(r#"{"ok":true,"data":{"url":"https://x.com/i/status/20"}}"#).contains("page absent"));
        assert_eq!(quote_visibility(r#"{"ok":true,"data":"hidden"}"#), "hidden");
        assert_eq!(quote_visibility("https://x.com/i/status/20"), "unknown");
        assert!(quote_view_line("0", tab, "page", "visible", "hidden").contains("target field page"));
        let selected = quote_select_ack(
            r#"{"ok":true,"data":{"selected":"A03BB6CCAE9402E75FCA7CB1AAED51FA"}}"#,
        );
        assert!(selected.ok);
        assert_eq!(selected.selected.as_deref(), Some(tab));
        assert_eq!(quote_select_ack("ok").note, "ok selected absent");
        let tabs = quote_status_tabs(
            r#"{"ok":true,"data":[{"id":3,"windowId":5,"groupId":9,"active":true,"url":"https://x.com/cryptoXRPwhale/status/2118971448614334959"},{"id":4,"windowId":8,"groupId":1,"active":true,"url":"https://example.com/other"}]}"#,
            original,
        );
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].window_id, 5);
        assert_eq!(tabs[0].group_id, Some(9));
        assert!(tabs[0].active);
        assert_eq!(
            quote_window_state(r#"{"ok":true,"data":{"id":5,"state":"minimized","focused":false}}"#),
            Some(("minimized", false))
        );
        assert_eq!(
            quote_group_name(r#"{"ok":true,"data":{"title":"OpenCLI Browser"}}"#),
            "OpenCLI Browser"
        );
        assert_eq!(quote_group_name(r#"{"ok":true,"data":{"title":"Mail"}}"#), "other");
        let hidden_eval = r#"{"ok":true,"data":"{\"v\":\"hidden\",\"h\":true,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#;
        let hidden_read = quote_visibility_read(hidden_eval);
        assert_eq!(hidden_read.token, "hidden");
        assert!(!hidden_read.ready);
        assert!(hidden_read.summary.contains("focus yes"));
        assert!(hidden_read.summary.contains("path yes"));
        assert!(hidden_read.summary.contains("top yes"));
        assert!(quote_visibility_read(
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":true,\"t\":true,\"m\":true,\"host\":\"x.com\"}"}"#
        )
        .ready);
        let compose = quote_visibility_read(
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":false,\"t\":true,\"m\":false,\"c\":true,\"host\":\"x.com\"}"}"#,
        );
        assert!(compose.ready, "{}", compose.summary);
        assert!(compose.summary.contains("route compose"));
        assert!(!quote_visibility_read(
            r#"{"ok":true,"data":"{\"v\":\"visible\",\"h\":false,\"p\":false,\"f\":false,\"t\":true,\"m\":false,\"c\":false,\"host\":\"x.com\"}"}"#
        )
        .ready);
        assert!(!quote_visibility_read(
            r#"{"ok":true,"data":"{\"v\":\"hidden\",\"h\":true,\"p\":false,\"f\":false,\"t\":true,\"m\":false,\"c\":true,\"host\":\"x.com\"}"}"#
        )
        .ready);
        assert!(!quote_visibility_read(r#"{"ok":true,"data":"hidden"}"#).ready);
        assert_eq!(
            quote_window_bounds(
                r#"{"ok":true,"data":{"state":"normal","focused":true,"left":120,"top":80,"width":1280,"height":900}}"#
            ),
            "onscreen"
        );
        assert_eq!(
            quote_window_bounds(
                r#"{"ok":true,"data":{"state":"normal","focused":true,"left":-1920,"top":0,"width":1920,"height":1080}}"#
            ),
            "onscreen"
        );
        assert_eq!(
            quote_window_bounds(
                r#"{"ok":true,"data":{"state":"normal","focused":true,"left":-32000,"top":-32000,"width":1280,"height":900}}"#
            ),
            "parked"
        );
        assert!(inspect.contains(r"\/status\/"));
        assert!(inspect.contains(r"(\d+)"));
        assert!(!inspect.contains("/i/api/"));
        assert!(!write.contains("twitter post"));
        let missing = r#"{"stage":"composer","path":"/i/status/2118971448614334959","testid_count":12,"composer":true,"card":false,"status_ids":[],"quote_testids":0,"clicked_post":false,"clicked_repost_confirm":false,"message":"Quote card for 2118971448614334959 was not in the composer. Post was not clicked."}"#;
        let report = parse_quote_page_report(missing).unwrap();
        assert!(!report.clicked_post);
        assert!(!report.card);
        assert!(quote_report_line(&report).contains("Post was not clicked"));
        let read = r#"{"stage":"read","path":"/user/status/99","card":false,"status_ids":["99"],"clicked_post":false,"clicked_repost_confirm":false,"message":"Read only. Post was not clicked."}"#;
        assert!(quote_read_confirms(&parse_quote_page_report(read).unwrap(), original).is_err());
        let reply_page = r#"{"stage":"read","path":"/Jasper_Black/status/2119115255108526411","card":true,"composer_kind":"reply","quote_testids":0,"status_ids":["2118971448614334959"],"clicked_post":false,"clicked_repost_confirm":false,"message":"Read only. Post was not clicked."}"#;
        let reply = parse_quote_page_report(reply_page).unwrap();
        assert!(!quote_post_allowed(&reply, original));
        let note = quote_outcome(Some("2119115255108526411"), &reply, original).unwrap_err();
        assert!(note.contains("https://x.com/i/status/2119115255108526411"), "{note}");
        assert!(note.contains("a reply"), "{note}");
        assert!(note.contains("not a quote"), "{note}");
        let false_card = r#"{"stage":"inspect","path":"/compose/post","card":true,"composer":true,"composer_kind":"unknown","quote_testids":0,"status_ids":["2118971448614334959"],"testid_count":69,"clicked_post":false,"text_landed":true,"quoted_status_id":"2118971448614334959","message":"Inspect only. Post was not clicked."}"#;
        assert!(!quote_inspect_ready(&parse_quote_page_report(false_card).unwrap()));
        let live = r#"{"stage":"composer","path":"/compose/post","composer_kind":"unknown","composer":true,"card":false,"status_ids":["2118971448614334959"],"quote_testids":0,"testid_count":70,"clicked_post":false,"selected_scope":"menu","selected_label":"Quote","selected_href_kind":"quote-url","message":"No quote attachment. Post was not clicked."}"#;
        let live_report = parse_quote_page_report(live).unwrap();
        assert!(!quote_inspect_ready(&live_report));
        assert!(!quote_post_allowed(&live_report, original));
        let live_line = quote_report_line(&live_report);
        assert!(live_line.contains("quote-url"), "{live_line}");
        assert!(live_line.contains("menu"), "{live_line}");
        let script = quote_page_script(original, Some("doesn’t"), false);
        let real = r#"{"stage":"inspect","path":"/compose/post","card":true,"composer":true,"composer_kind":"quote","quote_testids":1,"quoted_status_id":"2118971448614334959","text_landed":true,"clicked_repost_confirm":false,"message":"Inspect only. Post was not clicked."}"#;
        assert!(quote_post_allowed(&parse_quote_page_report(real).unwrap(), original));
        assert!(quote_inspect_ready(&parse_quote_page_report(real).unwrap()));
        assert!(script.is_ascii(), "quote script must stay ASCII so atob cannot mojibake");
        assert!(script.contains("\\u2019"), "{script}");
        assert!(!script.contains("compose/post"));
        assert!(!script.contains("seen.card = true"));
        assert!(script.contains("quoteTweet"));
        assert!(!script.contains("preventDefault"));
        assert!(script.contains("[role=\"menu\"]"));
        assert!(script.contains("textareas="));
        assert!(script.contains("keys="));
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

    #[test]
    fn quote_owned_group_ids_keep_only_the_opencli_browser_group() {
        let ids = quote_owned_group_ids(
            r#"{"ok":true,"data":[{"id":9,"title":"OpenCLI Browser"},{"id":3,"title":"Personal"},{"id":4}]}"#,
        );
        assert_eq!(ids, vec![9, 4]);
        assert!(quote_owned_group_ids("not json").is_empty());
    }

    #[test]
    fn quote_landed_query_reports_the_url_parameter_without_the_address() {
        let id = "2118971448614334959";
        let compose = quote_composer_url(id);
        let intact = format!(r#"{{"ok":true,"data":{{"url":"{compose}","page":"B94452EA22F530887AF90E615F8D7031"}}}}"#);
        let line = quote_landed_query(&intact, id);
        assert_eq!(line, "open echo url-key yes id-match yes");
        assert!(!line.contains("https://"), "{line}");
        assert_eq!(
            quote_query_shape("https://x.com/compose/post", id),
            "open echo url-key no id-match no"
        );
        assert_eq!(
            quote_query_shape("https://x.com/compose/post?url=https://x.com/i/status/111", id),
            "open echo url-key yes id-match no"
        );
        let damaged = format!(
            "https://x.com/compose/post?url=https2Fx.com2Fstatus%2F{id}"
        );
        assert!(quote_query_shape(&damaged, id).ends_with("id-match no"), "{damaged}");
        assert_eq!(quote_landed_query("not json", id), "open echo unread");
        let href = quote_eval_location(
            r#"{"ok":true,"data":"https://www.google.com/_/chrome/newtab"}"#,
            id,
        );
        assert!(href.contains("scheme https"), "{href}");
        assert!(href.contains("host www.google.com"), "{href}");
        assert!(href.contains("path /_/chrome/newtab"), "{href}");
        assert!(!href.contains("google.com/_"), "{href}");
        let compose = quote_eval_location(&format!(r#"{{"ok":true,"data":"{compose}"}}"#), id);
        assert!(compose.contains("host x.com"), "{compose}");
        assert!(compose.contains("path /compose/post"), "{compose}");
        assert!(compose.contains("id-match yes"), "{compose}");
    }

    #[test]
    fn quote_snap_dom_separates_composer_dialog_from_page_links() {
        let target = "2118971448614334959";
        let snap = quote_snap_script(target);
        let dir = std::env::temp_dir();
        let snap_path = dir.join("termy-quote-snap.js");
        let harness_path = dir.join("termy-quote-snap-harness.js");
        std::fs::write(&snap_path, &snap).unwrap();
        std::fs::write(&harness_path, QUOTE_SNAP_DOM_HARNESS).unwrap();
        let output = std::process::Command::new("node")
            .arg(&harness_path)
            .arg(&snap_path)
            .output()
            .expect("node");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}{stdout}");
        let reports: serde_json::Value = serde_json::from_str(stdout.trim()).expect(&stdout);
        let one = |name: &str| {
            reports
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == name)
                .unwrap_or_else(|| panic!("missing {name}"))
        };
        let outside = one("outside");
        assert_eq!(outside["card"], false);
        assert_eq!(outside["composer_kind"], "unknown");
        assert_eq!(outside["quote_testids"], 0);
        assert_eq!(outside["status_ids"].as_array().unwrap().len(), 0);
        assert_eq!(outside["clicked_post"], false);
        let attach = outside["attach"].as_str().unwrap();
        assert!(attach.contains("query url-key yes"), "{attach}");
        assert!(attach.contains("id-match yes"), "{attach}");
        assert!(attach.contains("textareas 2"), "{attach}");
        assert!(attach.contains("in-dialog-textareas 0"), "{attach}");
        assert!(attach.contains("dialogs 2"), "{attach}");
        assert!(attach.contains("page-quotes 1"), "{attach}");
        assert!(attach.contains("d1 box no"), "{attach}");
        assert!(attach.contains("quotes 1"), "{attach}");
        assert!(attach.contains("target-links 1"), "{attach}");
        assert!(attach.contains("ancestors"), "{attach}");
        assert!(!attach.contains(target), "{attach}");

        let inside = one("inside");
        assert_eq!(inside["card"], true);
        assert_eq!(inside["composer_kind"], "quote");
        assert_eq!(inside["quoted_status_id"], target);
        assert!(inside["quote_testids"].as_u64().unwrap() >= 1);
        assert_eq!(inside["status_ids"][0], target);
        assert!(!inside["status_ids"].to_string().contains("111"));
        assert!(inside["attach"].as_str().unwrap().contains("in-dialog-textareas 1"));

        let pasted = one("pasted");
        assert_eq!(pasted["card"], false);
        assert_eq!(pasted["status_ids"].as_array().unwrap().len(), 0);
        let pasted_attach = pasted["attach"].as_str().unwrap();
        assert!(pasted_attach.contains("target-links 0"), "{pasted_attach}");
        assert!(pasted_attach.contains("inside-editor"), "{pasted_attach}");
        assert!(pasted_attach.contains("id exact"), "{pasted_attach}");
        assert!(pasted_attach.contains("/i/status/exact"), "{pasted_attach}");

        let seen = one("seen");
        assert_eq!(seen["card"], false);
        assert_eq!(seen["quote_testids"], 0);
        let seen_attach = seen["attach"].as_str().unwrap();
        assert!(seen_attach.contains("rolelinks 1"), "{seen_attach}");
        assert!(seen_attach.contains("after-editor"), "{seen_attach}");
        assert!(seen_attach.contains("path /i/web/status/exact"), "{seen_attach}");
        assert!(seen_attach.contains("id exact"), "{seen_attach}");
        assert!(!seen_attach.contains(target), "{seen_attach}");

        let plain = one("plain");
        assert_eq!(plain["card"], false);
        assert_eq!(plain["quote_testids"], 0);
        let plain_attach = plain["attach"].as_str().unwrap();
        assert!(plain_attach.contains("anchors 0"), "{plain_attach}");
        assert!(plain_attach.contains("rolelinks 0"), "{plain_attach}");
        assert!(plain_attach.contains("editor url-exact"), "{plain_attach}");
        assert!(plain_attach.contains("self"), "{plain_attach}");
        assert!(plain_attach.contains("url-exact"), "{plain_attach}");
        assert!(!plain_attach.contains(target), "{plain_attach}");

        let urlcard = one("urlcard");
        assert_eq!(urlcard["card"], true);
        assert_eq!(urlcard["quoted_status_id"], target);
        assert_eq!(urlcard["status_ids"][0], target);

        let wrong = one("wrongparam");
        assert_eq!(wrong["card"], false);
        assert!(wrong["status_ids"].to_string().contains(target));
        assert_eq!(wrong["quoted_status_id"], serde_json::Value::Null);

        let reply = one("replylink");
        assert_eq!(reply["card"], false);
        assert_eq!(reply["status_ids"].as_array().unwrap().len(), 0);
        assert_eq!(one("card")["card"], false);

        let bound = quote_snap_script_bound(target, "cryptoXRPwhale", "IS OWNING 1,000 $XRP ENOUGH?");
        let bound_path = dir.join("termy-quote-snap-bound.js");
        std::fs::write(&bound_path, &bound).unwrap();
        let output = std::process::Command::new("node")
            .arg(&harness_path)
            .arg(&bound_path)
            .output()
            .expect("node");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let reports: serde_json::Value = serde_json::from_str(stdout.trim()).expect(&stdout);
        let row = |name: &str| {
            reports.as_array().unwrap().iter().find(|item| item["name"] == name).unwrap().clone()
        };
        let card = row("card");
        assert_eq!(card["card"], true, "{}", card["attach"]);
        assert_eq!(card["composer_kind"], "quote");
        assert_eq!(card["quoted_status_id"], target);
        assert!(card["attach"].as_str().unwrap().contains("card-bind article"));
        assert!(!card["attach"].as_str().unwrap().contains("IS OWNING"));
        assert_eq!(row("wrongtext")["card"], false);
        assert_eq!(row("wronguser")["card"], false);
        assert_eq!(row("pagecard")["card"], false);
        assert_eq!(row("plain")["card"], false);
        let split = row("split");
        assert_eq!(split["card"], false, "{}", split["attach"]);
        let split_attach = split["attach"].as_str().unwrap();
        assert!(split_attach.contains("text-in no"), "{split_attach}");
        assert!(split_attach.contains("avatar-in no"), "{split_attach}");
        assert!(split_attach.contains("dialog-text yes"), "{split_attach}");
        assert!(split_attach.contains("text-equal yes"), "{split_attach}");
        assert!(split_attach.contains("handle-attach no"), "{split_attach}");
        assert!(split_attach.contains("handle-dialog yes"), "{split_attach}");
        assert!(split_attach.contains("card-bind miss"), "{split_attach}");
        let card_attach = card["attach"].as_str().unwrap();
        assert!(card_attach.contains("text-in yes"), "{card_attach}");
        assert!(card_attach.contains("text-equal yes"), "{card_attach}");
        assert!(card_attach.contains("handle-attach yes"), "{card_attach}");
    }
}

const QUOTE_SNAP_DOM_HARNESS: &str = r#"
const fs = require("fs");
const snap = fs.readFileSync(process.argv[2], "utf8");
function el(tag, attrs, kids) {
  const node = {
    tagName: String(tag).toUpperCase(),
    attrs: attrs || {},
    children: [],
    parentElement: null,
    getAttribute(name) { return Object.prototype.hasOwnProperty.call(this.attrs, name) ? this.attrs[name] : null; },
    get href() { const raw = this.getAttribute("href"); if (!raw) return ""; try { return new URL(raw, globalThis.location.origin).href; } catch (e) { return raw; } },
    closest(sel) { return matchAny(this, sel) ? this : (this.parentElement ? this.parentElement.closest(sel) : null); },
    querySelectorAll(sel) { const out = []; walk(this, (n) => { if (n !== this && matchAny(n, sel)) out.push(n); }); return out; },
    querySelector(sel) { return this.querySelectorAll(sel)[0] || null; },
    contains(other) { let cur = other; while (cur) { if (cur === this) return true; cur = cur.parentElement; } return false; },
    compareDocumentPosition(other) {
      if (!other || other === this) return 0;
      if (this.contains(other)) return 24;
      if (other.contains && other.contains(this)) return 18;
      const order = [];
      let root = this;
      while (root.parentElement) root = root.parentElement;
      walk(root, (n) => order.push(n));
      const a = order.indexOf(this);
      const b = order.indexOf(other);
      if (a < 0 || b < 0) return 1;
      return a < b ? 4 : 2;
    },
    get innerText() {
      const own = this.attrs.innerText || "";
      const kids = (this.children || []).map((kid) => (kid.innerText || "")).join(" ");
      return [own, kids].filter(Boolean).join(" ");
    }
  };
  for (const kid of kids || []) { kid.parentElement = node; node.children.push(kid); }
  return node;
}
function matchOne(node, sel) {
  sel = sel.trim();
  if (!sel) return false;
  if (sel === "article") return node.tagName === "ARTICLE";
  let rest = sel;
  const tag = rest.match(/^([a-zA-Z]+)/);
  if (tag) {
    if (node.tagName !== tag[1].toUpperCase()) return false;
    rest = rest.slice(tag[1].length);
  }
  const attrs = [...rest.matchAll(/\[([^\]=*~|^]+)(\*=|=|\^=)?(?:"([^"]*)")?\]/g)];
  if (!attrs.length) return !!tag;
  for (const found of attrs) {
    const got = node.getAttribute(found[1]);
    if (got == null) return false;
    if (found[2] === "=" && got !== found[3]) return false;
    if (found[2] === "*=" && String(got).indexOf(found[3]) < 0) return false;
  }
  return true;
}
function matchAny(node, sel) { return sel.split(",").some((part) => matchOne(node, part)); }
function walk(node, fn) { fn(node); for (const kid of node.children) walk(kid, fn); }
function run(name, href, kids) {
  const body = el("body", {}, kids);
  globalThis.document = { body, querySelectorAll(sel) { return body.querySelectorAll(sel); }, querySelector(sel) { return body.querySelector(sel); } };
  globalThis.location = new URL(href);
  globalThis.window = {};
  const raw = (0, eval)(snap);
  return { name, report: JSON.parse(raw) };
}
const target = "2118971448614334959";
const quoteHref = "https://x.com/i/status/" + target;
const otherHref = "https://x.com/i/status/111";
const replyHref = quoteHref + "?in_reply_to=1";
const compose = "https://x.com/compose/post?url=" + encodeURIComponent(quoteHref);
const box = () => el("div", { "data-testid": "tweetTextarea_0" }, []);
const link = (href, kids) => el("a", { href }, kids || []);
const rows = [
  run("outside", compose, [
    el("div", {}, [box(), box()]),
    el("div", { role: "dialog" }, [el("div", { "data-testid": "quoteTweet" }, [link(quoteHref)])]),
    el("div", { role: "dialog", "aria-modal": "true" }, [el("span", {}, [])]),
    el("article", {}, [link(otherHref)])
  ]),
  run("inside", compose, [
    el("div", { role: "dialog" }, [box(), el("div", { "data-testid": "quoteTweet" }, [link(quoteHref)]), el("div", { "data-testid": "tweetButton" }, [])]),
    el("article", {}, [link(otherHref)])
  ]),
  run("pasted", compose, [
    el("div", { role: "dialog" }, [el("div", { "data-testid": "tweetTextarea_0", contenteditable: "true" }, [link(quoteHref)])]),
    el("article", {}, [link(quoteHref)])
  ]),
  run("seen", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      box(),
      el("div", { role: "link" }, []),
      link("https://x.com/i/web/status/" + target)
    ])
  ]),
  run("plain", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      el("div", { "data-testid": "tweetTextarea_0", contenteditable: "true", innerText: "https://x.com/i/status/" + target }, []),
      el("div", { "data-testid": "tweetButton", innerText: "Post" }, [])
    ])
  ]),
  run("urlcard", compose, [
    el("div", { role: "dialog" }, [box(), link(quoteHref)])
  ]),
  run("wrongparam", "https://x.com/compose/post?url=" + encodeURIComponent("https://x.com/i/status/111"), [
    el("div", { role: "dialog" }, [box(), link(quoteHref)])
  ]),
  run("replylink", compose, [
    el("div", { role: "dialog" }, [box(), link(replyHref)])
  ]),
  run("card", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      el("div", { "data-testid": "tweetTextarea_0", innerText: "" }, []),
      el("div", { "data-testid": "attachments", innerText: "@cryptoXRPwhale IS OWNING 1,000 $XRP ENOUGH?" }, [
        el("div", { "data-testid": "Tweet-User-Avatar" }, []),
        el("div", { "data-testid": "tweetText", innerText: "IS OWNING 1,000 $XRP ENOUGH?" }, [])
      ]),
      el("div", { "data-testid": "tweetButton" }, [])
    ])
  ]),
  run("wrongtext", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      box(),
      el("div", { "data-testid": "attachments", innerText: "@cryptoXRPwhale a different post" }, [
        el("div", { "data-testid": "Tweet-User-Avatar" }, []),
        el("div", { "data-testid": "tweetText", innerText: "a different post" }, [])
      ])
    ])
  ]),
  run("wronguser", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      box(),
      el("div", { "data-testid": "attachments", innerText: "@otheruser IS OWNING 1,000 $XRP ENOUGH?" }, [
        el("div", { "data-testid": "Tweet-User-Avatar" }, []),
        el("div", { "data-testid": "tweetText", innerText: "IS OWNING 1,000 $XRP ENOUGH?" }, [])
      ])
    ])
  ]),
  run("pagecard", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [el("div", { "data-testid": "tweetTextarea_0", innerText: "" }, [])]),
    el("div", { "data-testid": "attachments", innerText: "@cryptoXRPwhale IS OWNING 1,000 $XRP ENOUGH?" }, [
      el("div", { "data-testid": "Tweet-User-Avatar" }, []),
      el("div", { "data-testid": "tweetText", innerText: "IS OWNING 1,000 $XRP ENOUGH?" }, [])
    ])
  ]),
  run("split", "https://x.com/compose/post", [
    el("div", { role: "dialog" }, [
      el("div", { "data-testid": "tweetTextarea_0", innerText: "" }, []),
      el("div", { "data-testid": "attachments", innerText: "" }, []),
      el("div", { "data-testid": "Tweet-User-Avatar" }, []),
      el("div", { "data-testid": "tweetText", innerText: "IS OWNING 1,000 $XRP ENOUGH?" }, []),
      el("span", { innerText: "@cryptoXRPwhale" }, [])
    ])
  ])
];
process.stdout.write(JSON.stringify(rows.map((row) => Object.assign({ name: row.name }, row.report))));
"#;
