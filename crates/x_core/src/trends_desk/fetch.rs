//! OpenCLI reads for Trends Desk.
//!
//! Searches go through the existing command runner as an argument vector.
//! There is no shell, no `x.exe`, no paid API, and no mock-fixture success.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::config::XConfig;
use crate::drafts::{self, new_id, timestamp};
use crate::io::CommandRunner;
use crate::model::Draft;
use crate::publish::humanize_error;

use super::domain::{
    DeskSettings, RunOutcome, SourcePost, Topic, draft_text_from_title, outcome_error,
    outcome_from_posts, redact_secrets, search_query,
};
use super::parse::{ParseError, posts_from_stdout};

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(120);
pub const WHOAMI_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Missing,
    Ready,
    NotLoggedIn,
    ProbeFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStatus {
    pub state: SessionState,
    pub label: String,
    pub detail: String,
}

pub struct OpenCliDesk {
    pub bin: String,
    runner: Arc<dyn CommandRunner>,
    hermes_dir: Option<PathBuf>,
}

impl OpenCliDesk {
    pub fn new(config: &XConfig, runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            bin: config.opencli_bin.clone(),
            runner,
            hermes_dir: hermes_dir_from_env(),
        }
    }

    pub fn with_hermes_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.hermes_dir = dir;
        self
    }

    pub fn probe(&self, cancel: &AtomicBool) -> SessionStatus {
        if let Err(error) = refuse_unsafe_cli_name(&self.bin) {
            return SessionStatus {
                state: SessionState::Missing,
                label: "OpenCLI blocked".into(),
                detail: error,
            };
        }
        if self.hermes_dir.is_some() {
            return SessionStatus {
                state: SessionState::Ready,
                label: "OpenCLI via OPENCLI_HERMES_NODE_DIR".into(),
                detail: diagnostic_ready(),
            };
        }
        if !self.runner.exists(&self.bin) {
            return SessionStatus {
                state: SessionState::Missing,
                label: "OpenCLI missing".into(),
                detail: missing_detail(&self.bin),
            };
        }
        if cancel.load(Ordering::Relaxed) {
            return SessionStatus {
                state: SessionState::ProbeFailed,
                label: "OpenCLI probe cancelled".into(),
                detail: "Cancelled before whoami.".into(),
            };
        }
        match self.run(&whoami_args(), WHOAMI_TIMEOUT, cancel) {
            Ok(stdout) => match posts_or_ok(&stdout) {
                Ok(()) => SessionStatus {
                    state: SessionState::Ready,
                    label: whoami_label(&stdout),
                    detail: diagnostic_ready(),
                },
                Err(ParseError::Provider(message)) => SessionStatus {
                    state: SessionState::NotLoggedIn,
                    label: "OpenCLI is not signed in".into(),
                    detail: shorten(&redact_secrets(&message)),
                },
                Err(ParseError::Malformed(_)) => SessionStatus {
                    state: SessionState::Ready,
                    label: "OpenCLI present".into(),
                    detail: "whoami did not return the search envelope. Run still uses OpenCLI and will show a real error if the search fails.".into(),
                },
            },
            Err(error) => SessionStatus {
                state: SessionState::ProbeFailed,
                label: "OpenCLI probe failed".into(),
                detail: shorten(&redact_secrets(&humanize_error(&error))),
            },
        }
    }

    pub fn run_topic(&self, topic: &Topic, focus: &str, cancel: &AtomicBool) -> RunOutcome {
        let query = search_query(&topic.desc, focus);
        if query.is_empty() {
            return outcome_error(topic, focus, "column description is empty");
        }
        if let Err(error) = self.preflight() {
            return outcome_error(topic, focus, error);
        }
        let args = search_args(&query, 100);
        match self.run(&args, SEARCH_TIMEOUT, cancel) {
            Err(error) => outcome_error(
                topic,
                focus,
                shorten(&redact_secrets(&humanize_error(&error))),
            ),
            Ok(stdout) => match posts_from_stdout(&stdout) {
                Ok(posts) => outcome_from_posts(topic, focus, posts),
                Err(ParseError::Provider(message) | ParseError::Malformed(message)) => {
                    outcome_error(topic, focus, shorten(&redact_secrets(&message)))
                }
            },
        }
    }

    pub fn inspect_replies(
        &self,
        query: &str,
        cancel: &AtomicBool,
    ) -> Result<Vec<InspectedPost>, String> {
        self.preflight()?;
        let posts = match self.run(&search_args(query, 8), SEARCH_TIMEOUT, cancel) {
            Ok(stdout) => posts_from_stdout(&stdout).map_err(|error| error.to_string())?,
            Err(error) => return Err(shorten(&redact_secrets(&humanize_error(&error)))),
        };
        let mut ranked = posts;
        ranked.sort_by_key(|post| std::cmp::Reverse(post.likes + post.reposts * 3));
        let mut out = Vec::new();
        for post in ranked.into_iter().take(3) {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let mut replies = Vec::new();
            if let Ok(stdout) = self.run(&thread_args(&post.id), SEARCH_TIMEOUT, cancel)
                && let Ok(thread) = posts_from_stdout(&stdout)
            {
                replies = thread
                    .into_iter()
                    .filter(|row| row.id != post.id && !row.text.trim().is_empty())
                    .take(3)
                    .map(InspectedReply::from_post)
                    .collect();
            }
            out.push(InspectedPost {
                author: display_handle(&post.author),
                url: post.url.clone(),
                text: post.text.chars().take(400).collect(),
                engagement: post.engagement_label(),
                created_at: post.created_at.clone(),
                replies,
            });
        }
        Ok(out)
    }

    fn preflight(&self) -> Result<(), String> {
        refuse_unsafe_cli_name(&self.bin)?;
        if self.hermes_dir.is_some() || self.runner.exists(&self.bin) {
            Ok(())
        } else {
            Err(missing_detail(&self.bin))
        }
    }

    fn run(
        &self,
        args: &[String],
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<String, String> {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if let Some(dir) = &self.hermes_dir {
            return run_hermes(dir, args, timeout, cancel);
        }
        refuse_unsafe_cli_name(&self.bin)?;
        let output = self.runner.run_timeout(&self.bin, args, timeout)?;
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if output.status != 0 {
            let combined = format!("{}\n{}", output.stdout, output.stderr);
            return Err(if combined.trim().is_empty() {
                format!("opencli exited {}", output.status)
            } else {
                combined
            });
        }
        Ok(output.stdout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedReply {
    pub author: String,
    pub url: String,
    pub text: String,
    pub engagement: String,
    pub created_at: Option<String>,
}

impl InspectedReply {
    fn from_post(post: SourcePost) -> Self {
        let engagement = post.engagement_label();
        Self {
            author: display_handle(&post.author),
            url: post.url,
            text: post.text.chars().take(1000).collect(),
            engagement,
            created_at: post.created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedPost {
    pub author: String,
    pub url: String,
    pub text: String,
    pub engagement: String,
    pub created_at: Option<String>,
    pub replies: Vec<InspectedReply>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPrep {
    pub text: String,
    pub reply_to: Option<String>,
    pub saved_id: Option<String>,
    pub save_error: Option<String>,
}

/// Save a draft in the Termy XL data directory. This never publishes and
/// does not write the Termy X draft file.
pub fn prepare_draft_file(
    path: &std::path::Path,
    title: &str,
    reply_to: Option<&str>,
) -> DraftPrep {
    let text = draft_text_from_title(title);
    let reply_to = reply_to
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let draft = Draft {
        id: new_id(),
        text: text.clone(),
        tone: Some("reply".into()),
        source: "termy-xl".into(),
        created_at: timestamp(),
        reply_to: reply_to.clone(),
    };
    match drafts::replace_draft_for_target(path, draft) {
        Ok(kept) => DraftPrep {
            text,
            reply_to,
            saved_id: Some(kept.id),
            save_error: None,
        },
        Err(error) => DraftPrep {
            text,
            reply_to,
            saved_id: None,
            save_error: Some(error),
        },
    }
}

pub fn prepare_draft(config: &XConfig, title: &str, reply_to: Option<&str>) -> DraftPrep {
    prepare_draft_file(&config.drafts_path(), title, reply_to)
}

/// Explicit copy into the Termy X draft file. Nothing else in Termy XL calls this.
pub fn handoff_draft_to_termy_x(text: &str, reply_to: Option<&str>) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("draft text is empty".into());
    }
    let reply_to = reply_to
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let draft = Draft {
        id: new_id(),
        text: text.to_string(),
        tone: Some("reply".into()),
        source: "termy-xl-handoff".into(),
        created_at: timestamp(),
        reply_to,
    };
    let path = crate::config::default_config_dir().join("x-drafts.json");
    drafts::upsert_drafts(&path, vec![draft.clone()])?;
    Ok(draft.id)
}

/// One explicit reply through OpenCLI. Callers must already have an explicit send.
/// Never uses a program named `x.exe` and never the official X API.
pub fn post_reply_opencli(
    bin: &str,
    runner: &dyn CommandRunner,
    target_url: &str,
    text: &str,
) -> Result<String, String> {
    refuse_unsafe_cli_name(bin)?;
    let text = text.trim();
    if text.is_empty() {
        return Err("Write a reply first.".into());
    }
    if !target_url.contains("/status/") {
        return Err("Reply target is not a status URL. Nothing was posted.".into());
    }
    let args = vec![
        "twitter".into(),
        "reply".into(),
        target_url.to_string(),
        text.to_string(),
        "--window".into(),
        "background".into(),
        "-f".into(),
        "json".into(),
    ];
    let output = runner.run_timeout(bin, &args, Duration::from_secs(180))?;
    if output.status != 0 {
        let combined = format!("{}\n{}", output.stdout, output.stderr);
        return Err(shorten(&redact_secrets(&humanize_error(&combined))));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.stdout.trim())
        && let Err(error) = super::parse::unwrap_opencli_data(value)
    {
        return Err(shorten(&redact_secrets(&error.to_string())));
    }
    if let Some(url) = super::pack::status_urls(output.stdout.trim())
        .into_iter()
        .find(|url| url != target_url)
    {
        return Ok(format!("reply receipt · {url}"));
    }
    Ok(format!("reply receipt · {target_url}"))
}

/// Like or repost through the same OpenCLI path. The caller shows the result in the window.
pub fn engage_opencli(
    bin: &str,
    runner: &dyn CommandRunner,
    verb: &str,
    url: &str,
) -> Result<String, String> {
    refuse_unsafe_cli_name(bin)?;
    if !matches!(verb, "like" | "retweet") {
        return Err("That action is not available in this window.".into());
    }
    if !url.contains("/status/") {
        return Err("That post has no status URL.".into());
    }
    let args = vec![
        "twitter".into(),
        verb.into(),
        url.to_string(),
        "--window".into(),
        "background".into(),
        "-f".into(),
        "json".into(),
    ];
    let output = runner.run_timeout(bin, &args, Duration::from_secs(60))?;
    if output.status != 0 {
        let combined = format!("{}\n{}", output.stdout, output.stderr);
        return Err(shorten(&redact_secrets(&humanize_error(&combined))));
    }
    let name = if verb == "like" { "Liked" } else { "Reposted" };
    Ok(format!("{name} ✓"))
}

pub fn search_args(query: &str, limit: usize) -> Vec<String> {
    let query = argv_text(query);
    vec![
        "twitter".into(),
        "search".into(),
        query,
        "--product".into(),
        "top".into(),
        "--limit".into(),
        limit.clamp(1, 100).to_string(),
        "-f".into(),
        "json".into(),
    ]
}

pub fn thread_args(id: &str) -> Vec<String> {
    vec![
        "twitter".into(),
        "thread".into(),
        argv_text(id),
        "--limit".into(),
        "25".into(),
        "--top-by-engagement".into(),
        "8".into(),
        "-f".into(),
        "json".into(),
    ]
}

fn whoami_args() -> Vec<String> {
    vec![
        "twitter".into(),
        "whoami".into(),
        "-f".into(),
        "json".into(),
    ]
}

fn argv_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '\0')
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn refuse_unsafe_cli_name(program: &str) -> Result<(), String> {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let stem = name.split('.').next().unwrap_or(name);
    if stem.eq_ignore_ascii_case("x") {
        return Err(
            "Refusing to launch a program named x.exe. That name collides with Overwolf on Windows. Live X access for this desk uses opencli. The packaged Termy X CLI is termy-xcli.exe."
                .into(),
        );
    }
    Ok(())
}

fn hermes_dir_from_env() -> Option<PathBuf> {
    let raw = std::env::var("OPENCLI_HERMES_NODE_DIR").ok()?;
    let dir = PathBuf::from(raw.trim());
    hermes_ready(&dir).then_some(dir)
}

fn hermes_ready(dir: &Path) -> bool {
    let node = if cfg!(windows) {
        dir.join("node.exe")
    } else {
        dir.join("node")
    };
    let main = dir.join("node_modules/@jackwener/opencli/dist/src/main.js");
    node.is_file() && main.is_file() && refuse_unsafe_cli_name(&node.to_string_lossy()).is_ok()
}

fn run_hermes(
    dir: &Path,
    args: &[String],
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let node = if cfg!(windows) {
        dir.join("node.exe")
    } else {
        dir.join("node")
    };
    refuse_unsafe_cli_name(&node.to_string_lossy())?;
    let mut command = Command::new(&node);
    command
        .arg("./node_modules/@jackwener/opencli/dist/src/main.js")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_handle = thread::spawn(move || read_all(stdout));
    let err_handle = thread::spawn(move || read_all(stderr));
    let started = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".into());
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("OpenCLI timed out after {}s", timeout.as_secs()));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    };
    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    if !status.success() {
        let combined = format!("{stdout}\n{stderr}");
        return Err(if combined.trim().is_empty() {
            format!("opencli exited {}", status.code().unwrap_or(-1))
        } else {
            combined
        });
    }
    Ok(stdout)
}

fn read_all(mut pipe: Option<impl Read>) -> String {
    let Some(pipe) = pipe.as_mut() else {
        return String::new();
    };
    let mut text = String::new();
    let _ = pipe.read_to_string(&mut text);
    text
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let _ = command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

fn posts_or_ok(stdout: &str) -> Result<(), ParseError> {
    let value = super::parse::parse_json_blob(stdout)?;
    super::parse::unwrap_opencli_data(value).map(|_| ())
}

fn whoami_label(stdout: &str) -> String {
    let Ok(value) = super::parse::parse_json_blob(stdout) else {
        return "OpenCLI ready".into();
    };
    let Ok(data) = super::parse::unwrap_opencli_data(value) else {
        return "OpenCLI ready".into();
    };
    let handle = ["screen_name", "username", "handle"]
        .iter()
        .find_map(|key| data.get(*key).and_then(|item| item.as_str()))
        .or_else(|| {
            data.get("data").and_then(|node| {
                ["screen_name", "username", "handle"]
                    .iter()
                    .find_map(|key| node.get(*key).and_then(|item| item.as_str()))
            })
        });
    match handle {
        Some(handle) if !handle.is_empty() => {
            format!("OpenCLI · @{}", handle.trim_start_matches('@'))
        }
        _ => "OpenCLI ready".into(),
    }
}

fn missing_detail(bin: &str) -> String {
    format!(
        "OpenCLI is required for a live Trends Desk search and was not found as `{bin}`. Install @jackwener/opencli so `opencli` is on PATH, or set OPENCLI_HERMES_NODE_DIR to the directory that contains node.exe and node_modules/@jackwener/opencli. The packaged Termy X CLI is termy-xcli.exe. Do not run x.exe. This desk does not fall back to sample posts or the paid X API. {note}",
        note = super::domain::GPU_NOTE
    )
}

fn diagnostic_ready() -> String {
    format!(
        "Reads use your logged-in OpenCLI session. Nothing is posted from Run or Get replies. {}",
        super::domain::GPU_NOTE
    )
}

fn display_handle(author: &str) -> String {
    if author.is_empty() {
        "@unknown".into()
    } else {
        format!("@{author}")
    }
}

fn shorten(text: &str) -> String {
    text.chars().take(400).collect()
}

pub fn run_topics(
    desk: &OpenCliDesk,
    topics: &[Topic],
    settings: &DeskSettings,
    cancel: &AtomicBool,
) -> Vec<RunOutcome> {
    let mut outcomes = Vec::new();
    for (index, topic) in topics.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            outcomes.push(outcome_error(topic, &settings.focus, "cancelled"));
            continue;
        }
        if index > 0 {
            sleep_paced(cancel);
        }
        outcomes.push(desk.run_topic(topic, &settings.focus, cancel));
    }
    outcomes
}

fn sleep_paced(cancel: &AtomicBool) {
    let pause = Duration::from_millis(1500);
    let started = Instant::now();
    while started.elapsed() < pause {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::ScriptedRunner;
    use crate::trends_desk::OutcomeKind;
    use serde_json::{Value, json};
    use std::sync::atomic::AtomicBool;

    fn config(dir: &std::path::Path) -> XConfig {
        XConfig {
            config_dir: dir.to_path_buf(),
            opencli_bin: "opencli".into(),
            ..XConfig::default()
        }
    }

    fn envelope(posts: Value) -> String {
        json!({"ok": true, "data": posts}).to_string()
    }

    fn sample_posts() -> Value {
        json!([
            {
                "id": "1000000000000000001",
                "author": "example_user_a",
                "text": "SYNTHETIC FIXTURE: example discussion of court procedure. Not a real post.",
                "created_at": "Wed Nov 11 16:00:00 +0000 2026",
                "likes": 12,
                "views": "100",
                "url": "https://x.com/example_user_a/status/1000000000000000001"
            }
        ])
    }

    #[test]
    fn search_uses_ok_data_and_does_not_shell_out() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ScriptedRunner::new();
        runner.script("opencli", &["twitter", "search"], envelope(sample_posts()));
        let desk =
            OpenCliDesk::new(&config(dir.path()), Arc::new(runner.clone())).with_hermes_dir(None);
        let cancel = AtomicBool::new(false);
        let outcome = desk.run_topic(&Topic::law(), "AI", &cancel);
        assert_eq!(outcome.kind, OutcomeKind::Fresh);
        assert!(outcome.kind.writes_report());
        assert_eq!(
            outcome.posts[0].created_at.as_deref(),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert!(outcome.posts[0].url.contains("/status/1000000000000000001"));
        let calls = runner.calls.lock().unwrap();
        let (_program, args) = &calls[0];
        assert_eq!(args[0], "twitter");
        assert_eq!(args[1], "search");
        assert!(args[2].contains("legal news"));
        assert!(args[2].ends_with("AI"));
        assert!(
            args.iter()
                .all(|arg| arg != "sh" && arg != "cmd.exe" && arg != "x.exe")
        );
        assert_eq!(
            args.iter().filter(|arg| arg.contains("legal news")).count(),
            1
        );
    }

    #[test]
    fn ok_false_empty_and_malformed_do_not_look_like_fresh_posts() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "search"],
            json!({"ok": false, "error": "SYNTHETIC FIXTURE: provider failed"}).to_string(),
        );
        let desk = OpenCliDesk::new(&config(dir.path()), Arc::new(runner)).with_hermes_dir(None);
        let failed = desk.run_topic(&Topic::finance(), "", &cancel);
        assert_eq!(failed.kind, OutcomeKind::Error);
        assert!(!failed.kind.writes_report());
        assert!(failed.posts.is_empty());

        let runner = ScriptedRunner::new();
        runner.script("opencli", &["twitter", "search"], envelope(json!([])));
        let desk = OpenCliDesk::new(&config(dir.path()), Arc::new(runner)).with_hermes_dir(None);
        let empty = desk.run_topic(&Topic::ai(), "", &cancel);
        assert_eq!(empty.kind, OutcomeKind::Empty);
        assert!(!empty.message.is_empty());

        let runner = ScriptedRunner::new();
        runner.script("opencli", &["twitter", "search"], "not-json");
        let desk = OpenCliDesk::new(&config(dir.path()), Arc::new(runner)).with_hermes_dir(None);
        let malformed = desk.run_topic(&Topic::ai(), "", &cancel);
        assert_eq!(malformed.kind, OutcomeKind::Error);
        assert!(!malformed.markdown.contains("https://x.com/"));
    }

    #[test]
    fn missing_opencli_is_an_error_without_fixture_posts() {
        let dir = tempfile::tempdir().unwrap();
        let desk = OpenCliDesk::new(&config(dir.path()), Arc::new(ScriptedRunner::new()))
            .with_hermes_dir(None);
        let outcome = desk.run_topic(&Topic::law(), "AI", &AtomicBool::new(false));
        assert_eq!(outcome.kind, OutcomeKind::Error);
        assert!(outcome.message.contains("opencli"));
        assert!(outcome.message.contains("termy-xcli.exe"));
        assert!(outcome.message.contains("x.exe"));
        assert!(outcome.posts.is_empty());
        assert!(!outcome.kind.writes_report());
    }

    #[test]
    fn refuses_x_exe_even_if_named_as_the_bridge() {
        let error = refuse_unsafe_cli_name(r"C:\Tools\x.exe").unwrap_err();
        assert!(error.contains("x.exe"));
        assert!(error.contains("termy-xcli.exe"));
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        config.opencli_bin = "x.exe".into();
        let runner = ScriptedRunner::new();
        runner.install("x.exe");
        let desk = OpenCliDesk::new(&config, Arc::new(runner)).with_hermes_dir(None);
        let outcome = desk.run_topic(&Topic::law(), "", &AtomicBool::new(false));
        assert_eq!(outcome.kind, OutcomeKind::Error);
        assert!(outcome.posts.is_empty());
    }

    #[test]
    fn shell_metacharacters_stay_one_argument() {
        let args = search_args("law; rm -rf /", 10);
        let wide = search_args("ai", 500);
        assert!(wide.iter().any(|arg| arg == "100"));
        assert_eq!(args[2], "law; rm -rf /");
        assert_eq!(args.iter().filter(|arg| *arg == "law; rm -rf /").count(), 1);
    }

    #[test]
    fn cancelled_before_start_does_not_become_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ScriptedRunner::new();
        runner.script("opencli", &["twitter", "search"], envelope(sample_posts()));
        let desk = OpenCliDesk::new(&config(dir.path()), Arc::new(runner)).with_hermes_dir(None);
        let outcome = desk.run_topic(&Topic::law(), "AI", &AtomicBool::new(true));
        assert_eq!(outcome.kind, OutcomeKind::Error);
        assert!(outcome.message.to_lowercase().contains("cancelled"));
    }

    #[test]
    fn draft_is_kept_when_the_draft_file_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, "x").unwrap();
        let mut config = config(&blocker);
        config.config_dir = blocker;
        let prep = prepare_draft(&config, "Court filing moves", Some("123"));
        assert!(prep.text.starts_with("Worth watching:"));
        assert!(prep.saved_id.is_none());
        assert!(prep.save_error.is_some());
        assert_eq!(prep.reply_to.as_deref(), Some("123"));
    }

    #[test]
    fn draft_save_uses_the_existing_draft_file_and_does_not_publish() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let prep = prepare_draft(&config, "Markets open", Some("https://x.com/a/status/5"));
        assert!(prep.save_error.is_none());
        let id = prep.saved_id.unwrap();
        let drafts = crate::drafts::load_drafts(&config.drafts_path());
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].id, id);
        assert_eq!(drafts[0].source, "termy-xl");
        assert_ne!(
            config.drafts_path(),
            crate::config::default_config_dir().join("x-drafts.json")
        );
        assert_eq!(drafts[0].text, prep.text);
        let again = prepare_draft(
            &config,
            "Markets open later",
            Some("https://x.com/a/status/5"),
        );
        assert_eq!(again.saved_id.as_deref(), Some(id.as_str()));
        assert_eq!(crate::drafts::load_drafts(&config.drafts_path()).len(), 1);
    }

    #[test]
    fn like_and_repost_use_opencli_and_a_mock_confirms_them() {
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "like"],
            r#"{"ok":true,"data":{"id":"1"}}"#,
        );
        let liked =
            engage_opencli("opencli", &runner, "like", "https://x.com/ada/status/9").unwrap();
        assert_eq!(liked, "Liked ✓");
        runner.script(
            "opencli",
            &["twitter", "retweet"],
            r#"{"ok":true,"data":{"id":"1"}}"#,
        );
        let reposted =
            engage_opencli("opencli", &runner, "retweet", "https://x.com/ada/status/9").unwrap();
        assert_eq!(reposted, "Reposted ✓");
        let missing = engage_opencli("opencli", &runner, "like", "not-a-status").unwrap_err();
        assert!(missing.contains("status"));
        let calls = runner.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.iter().any(|arg| arg == "like"))
        );
        assert!(
            calls
                .iter()
                .any(|(_, args)| args.iter().any(|arg| arg == "retweet"))
        );
    }

    #[test]
    fn a_mocked_reply_returns_the_posted_status_link() {
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "reply"],
            "https://x.com/me/status/44\n",
        );
        let receipt = post_reply_opencli(
            "opencli",
            &runner,
            "https://x.com/ada/status/9",
            "the reply",
        )
        .unwrap();
        assert!(receipt.contains("https://x.com/me/status/44"));
        assert!(!receipt.contains("secret"));
    }
}
