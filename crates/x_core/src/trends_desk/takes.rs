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

/// Search, flip, scroll, and refresh never call a provider. A card action calls once.
pub fn take_spawns(event: DeskEvent) -> usize {
    match event {
        DeskEvent::Load
        | DeskEvent::Search
        | DeskEvent::Flip
        | DeskEvent::Scroll
        | DeskEvent::Refresh => 0,
        DeskEvent::CardClick
        | DeskEvent::DraftTake
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

/// Run Grok and Hermes together. The first real reply wins, so a slow Hermes
/// failover does not hold up a Grok answer. Hermes uses [`HERMES_TAKE_TIMEOUT`].
pub fn race_labeled<G, H>(grok: G, hermes: H) -> Result<(&'static str, String), String>
where
    G: FnOnce() -> Result<String, String> + Send + 'static,
    H: FnOnce() -> Result<String, String> + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let grok_tx = tx.clone();
    std::thread::spawn(move || {
        let _ = grok_tx.send(("grok", grok()));
    });
    std::thread::spawn(move || {
        let _ = tx.send(("hermes", hermes()));
    });
    let mut failures = Vec::new();
    let mut left = 2;
    while left > 0 {
        match rx.recv() {
            Ok(("grok", Ok(text))) if good_reply(&text) => return Ok(("grok", text)),
            Ok(("hermes", Ok(text))) if good_reply(&text) => return Ok(("hermes", text)),
            Ok((name, Ok(text))) => {
                failures.push(format!("{name}: {text}"));
                left -= 1;
            }
            Ok((name, Err(error))) => {
                failures.push(format!("{name}: {error}"));
                left -= 1;
            }
            Err(_) => break,
        }
    }
    Err(failures.join("; "))
}

fn good_reply(text: &str) -> bool {
    !super::agents::keep_distinct(&split_rough(text)).is_empty()
}

pub fn run_take_chain(
    _preferred: super::TakeProvider,
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
) -> super::TakeDraft {
    let mut reasons = Vec::new();
    let config_h = config.clone();
    let post_g = post.clone();
    let post_h = post.clone();
    let tone_g = tone.to_string();
    let tone_h = tone.to_string();
    match race_labeled(
        move || grok_cli(&post_g, &tone_g),
        move || hermes_raced(&config_h, &post_h, &tone_h),
    ) {
        Ok((name, text)) => return draft_from_provider(name, &text),
        Err(error) => reasons.push(error),
    }
    for provider in [super::TakeProvider::Treg, super::TakeProvider::Free] {
        match super::cards::draft_take(provider, config, post, http) {
            Ok(draft)
                if !draft.text.trim().is_empty()
                    && !is_provider_failure(&draft.text)
                    && !is_provider_failure(&draft.note) =>
            {
                return draft;
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
) -> Result<String, String> {
    let http = crate::io::UreqHttp::with_timeout(HERMES_TAKE_TIMEOUT);
    let draft = super::cards::hermes_take_toned(config, &http, post, tone);
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

fn draft_from_provider(name: &str, text: &str) -> super::TakeDraft {
    let lines = super::agents::keep_distinct(&split_rough(text));
    super::TakeDraft {
        text: lines.first().cloned().unwrap_or_default(),
        source: if name == "grok" { "grok" } else { "hermes" },
        note: format!("{name} draft. Not posted."),
        variants: lines,
    }
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
        ] {
            assert_eq!(take_spawns(event), 0, "{event:?}");
        }
        for event in [
            DeskEvent::CardClick,
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
    fn grok_wins_when_it_answers_before_a_slow_hermes() {
        assert!(HERMES_TAKE_TIMEOUT >= std::time::Duration::from_secs(60));
        assert_eq!(HERMES_TAKE_TIMEOUT, std::time::Duration::from_secs(90));
        let started = std::time::Instant::now();
        let (winner, text) = race_labeled(
            || Ok("Grok one\n---\nGrok two\n---\nGrok three".into()),
            || {
                std::thread::sleep(std::time::Duration::from_millis(400));
                Ok("Hermes late reply".into())
            },
        )
        .unwrap();
        assert_eq!(winner, "grok");
        assert!(text.contains("Grok one"));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(300),
            "Hermes blocked the chain for {:?}",
            started.elapsed()
        );
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
