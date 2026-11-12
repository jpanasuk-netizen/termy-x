//! Real takes. A canned template is never shown as a model reply.
//!
//! The card chain is Grok, then Hermes, then Treg, then Free. A provider that
//! errors, returns 402, or returns Hermes's "no reply" fallback is skipped.

use std::path::{Path, PathBuf};

pub const TAKE_CHAIN: [&str; 4] = ["grok", "hermes", "treg", "free"];
pub const HONEST_PREFIX: &str = "No model available";
/// Hermes fails over and sends a large system prompt. 90s covers that.
pub const HERMES_TAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

pub fn is_template_angle(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("agree, and one thing to add")
        || lower.contains("only half true")
        || lower.contains("skips who pays the cost")
}

pub fn is_provider_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    // "credit" by itself is ordinary spicy copy ("no credit for that").
    // Only the payment phrases are a provider failure.
    lower.contains("402")
        || lower.contains("401")
        || lower.contains("payment required")
        || lower.contains("credits depleted")
        || lower.contains("insufficient credit")
        || lower.contains("no reply:")
        || lower.contains("every provider in the fallback chain")
        || text.contains('⚠')
        || is_template_angle(text)
}

pub fn honest_failure(reasons: &[String]) -> String {
    let reason = reasons
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map_or("no provider answered", |line| line.trim());
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    let reason: String = reason.chars().take(180).collect();
    format!("{HONEST_PREFIX}: {reason}")
}

/// Keep distinct non-empty lines. Templates are dropped. Nothing is invented.
pub fn accept_lines(generated: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for line in generated {
        let line = line.trim();
        if line.is_empty()
            || is_provider_failure(line)
            || out.iter().any(|have: &String| have == line)
        {
            continue;
        }
        out.push(line.to_string());
        if out.len() == 3 {
            break;
        }
    }
    out
}

/// A short reply is not padded. The caller asks the provider again and merges.
pub fn merge_attempts(first: &[String], second: &[String]) -> Vec<String> {
    let mut combined = first.to_vec();
    combined.extend(second.iter().cloned());
    accept_lines(&combined)
}

pub fn chain_order() -> Vec<&'static str> {
    TAKE_CHAIN.to_vec()
}

/// Walk provider results in chain order. The first real reply wins.
/// `Err` or a failure string is skipped. An empty chain is an honest line.
pub fn choose_chain(
    attempts: &[(&str, Result<String, String>)],
) -> Result<(String, Vec<String>), String> {
    let mut reasons = Vec::new();
    for (name, body) in attempts {
        match body {
            Err(error) => reasons.push(format!("{name}: {error}")),
            Ok(text)
                if is_provider_failure(text) || accept_lines(&split_rough(text)).is_empty() =>
            {
                reasons.push(format!("{name}: {text}"));
            }
            Ok(text) => {
                let lines = accept_lines(&split_rough(text));
                return Ok(((*name).to_string(), lines));
            }
        }
    }
    Err(honest_failure(&reasons))
}

fn split_rough(text: &str) -> Vec<String> {
    if text.contains("---") {
        text.split("---")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        crate::handoff::parse_options(text)
    }
}

pub fn agy_credit_error(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    if lower.contains("402")
        || lower.contains("credit")
        || (lower.contains("gemini") && (lower.contains("deplet") || lower.contains("quota")))
    {
        Some("Agy needs credits (402)".to_string())
    } else {
        None
    }
}

pub fn agent_work_dir(data_root: &Path) -> PathBuf {
    data_root.join("agent-work")
}

pub fn muse_trust_line(dir: &Path) -> String {
    format!("run muse once in {} to trust it", dir.display())
}

pub fn muse_needs_trust(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("trust") || lower.contains("workspace")
}

/// Windows flag that keeps a console subsystem child from opening a window.
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const GROK_TAKE_CAP: usize = 1;
pub const CLICK_TO_DRAFT: &str = "Click to draft take";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskEvent {
    Load,
    Search,
    Flip,
    Scroll,
    Refresh,
    CardClick,
    DraftTake,
    Reply,
    Edit,
    ProviderPill,
    TonePill,
}

/// A take starts only from Draft take, Reply, Edit, or a chip on an open box.
/// Clicking the card body only selects it.
pub fn take_spawns(event: DeskEvent) -> usize {
    match event {
        DeskEvent::Load
        | DeskEvent::Search
        | DeskEvent::Flip
        | DeskEvent::Scroll
        | DeskEvent::Refresh
        | DeskEvent::CardClick => 0,
        DeskEvent::DraftTake
        | DeskEvent::Reply
        | DeskEvent::Edit
        | DeskEvent::ProviderPill
        | DeskEvent::TonePill => 1,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackgroundSpawn {
    pub program: &'static str,
    pub creation_flags: u32,
    pub piped_stdio: bool,
    pub console_wrapper: bool,
}

/// Background takes and drafts. Not the Open Grok button.
pub fn background_grok_spawn() -> BackgroundSpawn {
    BackgroundSpawn {
        program: "grok",
        creation_flags: CREATE_NO_WINDOW,
        piped_stdio: true,
        console_wrapper: false,
    }
}

pub fn admit_grok(running: usize) -> bool {
    running < GROK_TAKE_CAP
}

pub fn apply_hidden_command(command: &mut std::process::Command) {
    let plan = background_grok_spawn();
    if plan.piped_stdio {
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = command.creation_flags(plan.creation_flags);
    }
    #[cfg(not(windows))]
    {
        let _ = plan;
    }
}

/// Visible console command. Windows opens cmd so the CLI has a real session.
pub fn interactive_command(program: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd.exe".into(), "/k".into(), program.to_string()]
    } else {
        vec![program.to_string()]
    }
}

pub fn is_termy_x_image(path: &str) -> bool {
    let lower = path.replace('/', "\\").to_ascii_lowercase();
    let name = lower.rsplit('\\').next().unwrap_or(&lower);
    if name.contains("trends_desk") {
        return false;
    }
    let stem = name.trim_end_matches(".exe");
    stem == "termy"
        || stem.starts_with("termy-")
        || stem.starts_with("termyx")
        || lower.contains("\\termyx-app\\")
        || lower.contains("\\termy x\\")
}

pub fn take_chain_order() -> [&'static str; 4] {
    ["vyce", "hermes", "grok", "treg"]
}

pub fn run_take_chain(
    preferred: super::TakeProvider,
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
) -> super::TakeDraft {
    let _ = preferred;
    run_take_chain_with(
        config,
        post,
        http,
        tone,
        &read_hermes_env(),
        &read_hermes_yaml(),
    )
}

pub fn run_take_chain_with(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
    env_text: &str,
    yaml_text: &str,
) -> super::TakeDraft {
    let mut reasons = Vec::new();
    let auth = super::agents::vyce_auth(
        &config.ai_base_url,
        &config.ai_model,
        &config.ai_api_key,
        env_text,
        yaml_text,
    );
    let prompt = super::agents::reply_prompt(&post.author, &post.text, tone);
    match super::agents::vyce_complete(&auth, http, &prompt) {
        Ok(text) => return labeled_take("vyce", super::agents::VYCE_LABEL, &text, &reasons),
        Err(error) => reasons.push(format!("Vyce · {error}")),
    }
    match hermes_raced(config, post, tone, http) {
        Ok(text) => {
            let model = if config.hermes_model.trim().is_empty() {
                super::agents::HERMES_MODEL
            } else {
                config.hermes_model.trim()
            };
            return labeled_take("hermes", &format!("Hermes · {model}"), &text, &reasons);
        }
        Err(error) => reasons.push(format!("Hermes · {error}")),
    }
    match grok_cli(post, tone) {
        Ok(text) => return labeled_take("grok", "Grok", &text, &reasons),
        Err(error) => reasons.push(format!("Grok · {error}")),
    }
    for provider in [super::TakeProvider::Treg] {
        match super::cards::draft_take(provider, config, post, http) {
            Ok(draft)
                if !draft.text.trim().is_empty()
                    && !is_provider_failure(&draft.text)
                    && !is_provider_failure(&draft.note) =>
            {
                return labeled_take("treg", "Treg", &draft.text, &reasons);
            }
            Ok(draft) => {
                let why = if draft.note.is_empty() {
                    draft.text
                } else {
                    draft.note
                };
                reasons.push(format!("{}: {why}", provider.as_str()));
            }
            Err(error) => reasons.push(format!("{}: {error}", provider.as_str())),
        }
    }
    super::TakeDraft {
        text: String::new(),
        source: "none",
        note: honest_failure(&reasons),
        variants: Vec::new(),
    }
}

fn hermes_raced(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    tone: &str,
    http: &dyn crate::io::Http,
) -> Result<String, String> {
    let draft = super::cards::hermes_take_toned(config, http, post, tone);
    if draft.text.trim().is_empty()
        || is_provider_failure(&draft.text)
        || is_provider_failure(&draft.note)
    {
        Err(if draft.note.is_empty() {
            draft.text
        } else {
            draft.note
        })
    } else {
        Ok(draft.variants.join("\n---\n"))
    }
}

fn labeled_take(
    source: &'static str,
    label: &str,
    text: &str,
    earlier: &[String],
) -> super::TakeDraft {
    let lines = super::agents::keep_distinct(&split_rough(text));
    let note = if earlier.is_empty() {
        label.to_string()
    } else {
        format!("{}. {label}", earlier.join(". "))
    };
    super::TakeDraft {
        text: lines.first().cloned().unwrap_or_default(),
        source,
        note,
        variants: lines,
    }
}

fn read_hermes_env() -> String {
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return String::new();
    };
    std::fs::read_to_string(std::path::PathBuf::from(root).join("hermes").join(".env"))
        .unwrap_or_default()
}

fn read_hermes_yaml() -> String {
    let mut paths = Vec::new();
    if cfg!(windows) {
        paths.push(std::path::PathBuf::from(r"D:\hermes\config.yaml"));
    }
    if let Some(root) = std::env::var_os("LOCALAPPDATA") {
        paths.push(
            std::path::PathBuf::from(root)
                .join("hermes")
                .join("config.yaml"),
        );
    }
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
    }
    String::new()
}

thread_local! {
    static GROK_STUB: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub fn with_grok_stub<T>(output: &str, body: impl FnOnce() -> T) -> T {
    GROK_STUB.with(|cell| *cell.borrow_mut() = Some(output.to_string()));
    let value = body();
    GROK_STUB.with(|cell| *cell.borrow_mut() = None);
    value
}

pub fn grok_stub_output() -> Option<String> {
    GROK_STUB.with(|cell| cell.borrow().clone())
}

pub fn grok_draft_from_output(post: &super::SourcePost, output: &str) -> super::TakeDraft {
    let (lines, note) = super::agents::variants_from_draft(&post.text, Ok(output.to_string()));
    if lines.is_empty() {
        super::TakeDraft {
            text: String::new(),
            source: "grok",
            note,
            variants: lines,
        }
    } else {
        super::TakeDraft {
            text: lines[0].clone(),
            source: "grok",
            note: "Grok draft. Not posted.".into(),
            variants: lines,
        }
    }
}

pub(crate) fn grok_cli(post: &super::SourcePost, tone: &str) -> Result<String, String> {
    if takes_halted() {
        return Err("takes stopped".into());
    }
    let _permit = GrokPermit::acquire()?;
    let prompt = super::agents::reply_prompt(&post.author, &post.text, tone);
    let plan = background_grok_spawn();
    let mut command = std::process::Command::new(plan.program);
    apply_hidden_command(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let pid = child.id();
    register_job(&post.id, pid);
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(prompt.as_bytes());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let result = if let Ok(Ok(output)) = rx.recv_timeout(std::time::Duration::from_secs(20)) {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if takes_halted() {
            Err("takes stopped".into())
        } else if text.is_empty() {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        } else if is_provider_failure(&text) {
            Err(text)
        } else {
            Ok(text)
        }
    } else {
        kill_pid(pid);
        Err("grok timed out".into())
    };
    unregister_job(&post.id);
    result
}

struct GrokPermit;

impl GrokPermit {
    fn acquire() -> Result<Self, String> {
        let started = std::time::Instant::now();
        loop {
            if takes_halted() {
                return Err("takes stopped".into());
            }
            let mut running = grok_running().lock().unwrap_or_else(|err| err.into_inner());
            if admit_grok(*running) {
                *running += 1;
                return Ok(Self);
            }
            drop(running);
            if started.elapsed() > std::time::Duration::from_secs(45) {
                return Err("grok is busy".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    }
}

impl Drop for GrokPermit {
    fn drop(&mut self) {
        let mut running = grok_running().lock().unwrap_or_else(|err| err.into_inner());
        *running = running.saturating_sub(1);
    }
}

fn grok_running() -> &'static std::sync::Mutex<usize> {
    static RUNNING: std::sync::OnceLock<std::sync::Mutex<usize>> = std::sync::OnceLock::new();
    RUNNING.get_or_init(|| std::sync::Mutex::new(0))
}

fn grok_jobs() -> &'static std::sync::Mutex<Vec<(String, u32)>> {
    static JOBS: std::sync::OnceLock<std::sync::Mutex<Vec<(String, u32)>>> =
        std::sync::OnceLock::new();
    JOBS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

fn takes_flag() -> &'static std::sync::atomic::AtomicBool {
    static HALT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    &HALT
}

pub fn takes_halted() -> bool {
    takes_flag().load(std::sync::atomic::Ordering::SeqCst)
}

pub fn halt_takes() {
    takes_flag().store(true, std::sync::atomic::Ordering::SeqCst);
    kill_grok_jobs(&[]);
}

pub fn resume_takes() {
    takes_flag().store(false, std::sync::atomic::Ordering::SeqCst);
}

pub fn retain_grok_jobs(keep: &[String]) {
    kill_grok_jobs(keep);
}

fn register_job(id: &str, pid: u32) {
    let mut jobs = grok_jobs().lock().unwrap_or_else(|err| err.into_inner());
    jobs.retain(|(have, _)| have != id);
    jobs.push((id.to_string(), pid));
}

fn unregister_job(id: &str) {
    let mut jobs = grok_jobs().lock().unwrap_or_else(|err| err.into_inner());
    jobs.retain(|(have, _)| have != id);
}

fn kill_grok_jobs(keep: &[String]) {
    let mut jobs = grok_jobs().lock().unwrap_or_else(|err| err.into_inner());
    let mut left = Vec::new();
    for (id, pid) in jobs.drain(..) {
        if keep.iter().any(|item| item == &id) {
            left.push((id, pid));
        } else {
            kill_pid(pid);
        }
    }
    *jobs = left;
}

fn kill_pid(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T", "/F"]);
        let _ = command.creation_flags(CREATE_NO_WINDOW);
        let _ = command.output();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_skips_failures_and_never_invents_a_template() {
        let warning = "⚠️ No reply: every provider in the fallback chain failed";
        let chosen = choose_chain(&[
            ("grok", Err("grok: not today".into())),
            ("hermes", Ok(warning.into())),
            ("treg", Err("treg: no key".into())),
            ("free", Err("http 402 payment required".into())),
        ]);
        let err = chosen.unwrap_err();
        assert!(err.starts_with(HONEST_PREFIX));
        assert!(err.contains("402"));
        assert!(!is_template_angle(&err));
        let ok = choose_chain(&[
            ("grok", Ok("same\n---\nsame".into())),
            ("hermes", Ok("should not be used".into())),
        ])
        .unwrap();
        assert_eq!(ok.0, "grok");
        assert_eq!(ok.1, vec!["same".to_string()]);
        let more = merge_attempts(
            &ok.1,
            &["A real second reply".into(), "A real third reply".into()],
        );
        assert_eq!(more.len(), 3);
        assert!(more.iter().all(|line| !is_template_angle(line)));
    }

    #[test]
    fn grok_then_hermes_order_and_agy_credit_and_muse_trust() {
        assert_eq!(chain_order(), vec!["grok", "hermes", "treg", "free"]);
        assert_eq!(
            agy_credit_error("Gemini 402 credits depleted"),
            Some("Agy needs credits (402)".into())
        );
        assert!(agy_credit_error("a normal reply about courts").is_none());
        let dir = agent_work_dir(Path::new(r"C:\Users\j\AppData\Roaming\Termy XL"));
        assert!(dir.ends_with("agent-work"));
        let line = muse_trust_line(&dir);
        assert!(line.contains("run muse once"));
        assert!(line.contains("agent-work"));
        assert!(muse_needs_trust("Workspace trust required"));
        let cmd = interactive_command("grok");
        if cfg!(windows) {
            assert_eq!(cmd[0], "cmd.exe");
            assert_eq!(cmd[1], "/k");
        }
        assert!(cmd.iter().any(|part| part == "grok"));
    }

    #[test]
    fn background_grok_spawn_uses_create_no_window() {
        let plan = background_grok_spawn();
        assert_eq!(plan.creation_flags, 0x0800_0000);
        assert_eq!(plan.program, "grok");
        assert!(plan.piped_stdio);
        assert!(!plan.console_wrapper);
        assert!(!admit_grok(GROK_TAKE_CAP));
        assert!(admit_grok(0));
        assert_eq!(GROK_TAKE_CAP, 1);
        for event in [
            DeskEvent::Load,
            DeskEvent::Search,
            DeskEvent::Flip,
            DeskEvent::Scroll,
            DeskEvent::Refresh,
            DeskEvent::CardClick,
        ] {
            assert_eq!(take_spawns(event), 0, "{event:?}");
        }
        for event in [
            DeskEvent::DraftTake,
            DeskEvent::Reply,
            DeskEvent::Edit,
            DeskEvent::ProviderPill,
            DeskEvent::TonePill,
        ] {
            assert_eq!(take_spawns(event), 1, "{event:?}");
        }
        let visible = interactive_command("grok");
        if cfg!(windows) {
            assert_eq!(visible[0], "cmd.exe");
            assert_ne!(visible[0], plan.program);
        }
    }

    #[test]
    fn termy_x_is_the_process_not_only_the_exact_title() {
        assert!(super::super::is_termy_x_title(
            r"C:\Windows\System32\cmd.exe - Termy"
        ));
        assert!(super::super::is_termy_x_title("Termy"));
        assert!(!super::super::is_termy_x_title("Termy XL"));
        assert!(is_termy_x_image(
            r"C:\Users\j\AppData\Local\termyx-app\termy-7154b8a5.exe"
        ));
        assert!(is_termy_x_image(r"D:\apps\termy.exe"));
        assert!(!is_termy_x_image(
            r"C:\Users\j\AppData\Local\Termy XL\trends_desk.exe"
        ));
    }

    #[test]
    fn the_take_chain_tries_vyce_before_hermes_grok_and_treg() {
        assert_eq!(take_chain_order(), ["vyce", "hermes", "grok", "treg"]);
        assert!(HERMES_TAKE_TIMEOUT >= std::time::Duration::from_secs(60));
    }

    #[test]
    fn spicy_credit_lines_stay_and_a_failed_tone_keeps_the_previous_list() {
        let spicy = "The court gets no credit for outsourcing the opinion.\n---\nThe panel still signed a model draft and called it judgment.\n---\nOutsourcing the signature does not make the opinion cheaper.";
        assert!(
            !is_provider_failure(spicy),
            "a spicy reply that says credit is not a billing error"
        );
        let lines = super::super::agents::keep_distinct(&split_rough(spicy));
        assert_eq!(lines.len(), 3);
        let previous = vec!["Funny line that was already on the card.".to_string()];
        let (kept, note) =
            settle_variants(&previous, &[Vec::new(), Vec::new(), Vec::new()], "Spicy");
        assert_eq!(kept, previous);
        assert!(note.contains("Spicy"));
        assert!(note.contains("3"));
        let (fresh, quiet) = settle_variants(&previous, &[Vec::new(), lines.clone()], "Spicy");
        assert_eq!(fresh, lines);
        assert!(quiet.is_empty());
        let (fallback, error) = settle_variants(&[], &[Vec::new(), Vec::new()], "Spicy");
        assert_eq!(fallback.len(), 1);
        assert!(!fallback[0].is_empty());
        assert!(error.contains("Spicy"));
    }

    #[test]
    fn vyce_503_falls_through_to_hermes_and_the_key_stays_out() {
        assert_eq!(take_chain_order(), ["vyce", "hermes", "grok", "treg"]);
        let post = super::super::domain::SourcePost::from_value(&serde_json::json!({
            "id": "9",
            "author": "ada",
            "text": "the court posted a model draft",
            "url": "https://x.com/ada/status/9"
        }))
        .unwrap();
        let config = crate::config::XConfig {
            ai_base_url: "https://vyce.test/v1".into(),
            ai_model: "glm-5.3-flash".into(),
            ai_api_key: "vyce-secret-key".into(),
            ai_explicit: true,
            hermes_base_url: "http://hermes.test/v1".into(),
            hermes_model: "hermes-agent".into(),
            hermes_api_key: "hermes-secret".into(),
            ..crate::config::XConfig::default()
        };
        struct RouteHttp;
        impl crate::io::Http for RouteHttp {
            fn request(&self, request: crate::io::HttpRequest) -> Result<String, String> {
                if request.url.contains("vyce.test") {
                    assert_eq!(request.bearer.as_deref(), Some("vyce-secret-key"));
                    let body = request.body.unwrap_or_default();
                    assert!(body.contains("glm-5.3-flash"));
                    assert!(!body.contains("vyce-secret-key"));
                    return Err("503 Service Unavailable".into());
                }
                if request.url.contains("hermes.test") {
                    return Ok(
                        r#"{"choices":[{"message":{"content":"Hermes one\n---\nHermes two\n---\nHermes three"}}]}"#
                            .into(),
                    );
                }
                Err(format!("unexpected {}", request.url))
            }
        }
        let http = RouteHttp;
        let draft = run_take_chain_with(&config, &post, &http, "Funny", "", "");
        assert_eq!(draft.source, "hermes");
        assert!(draft.text.contains("Hermes"));
        assert!(draft.note.contains("503"));
        assert!(draft.note.contains("Hermes · hermes-agent"));
        assert!(!draft.note.contains("vyce-secret-key"));
        assert!(!draft.note.contains("hermes-secret"));
    }
}

/// First try plus at most two retries. An empty result keeps the previous
/// variants. If there were none, the list is the one-line error.
pub fn settle_variants(
    previous: &[String],
    attempts: &[Vec<String>],
    tone: &str,
) -> (Vec<String>, String) {
    let mut tries = 0;
    for batch in attempts.iter().take(3) {
        tries += 1;
        let kept = super::agents::keep_distinct(batch);
        if !kept.is_empty() {
            return (kept, String::new());
        }
    }
    let error = format!("{tone} draft failed after {tries} tries");
    if previous.is_empty() {
        (vec![error.clone()], error)
    } else {
        (previous.to_vec(), error)
    }
}
