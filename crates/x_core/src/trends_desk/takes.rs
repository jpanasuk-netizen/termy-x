use std::path::{Path, PathBuf};

pub const TAKE_CHAIN: [&str; 4] = ["grok", "hermes", "treg", "free"];
pub const HONEST_PREFIX: &str = "No model available";
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
        Some("Agy: Google credits depleted (402)".to_string())
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

pub const TAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

pub fn take_chain_order() -> [&'static str; 6] {
    ["hermes", "vyce", "grok", "muse", "agy", "custom"]
}

pub fn migrate_saved_provider(saved: &str) -> &'static str {
    match saved.trim().to_ascii_lowercase().as_str() {
        "" | "free" | "treg" => "hermes",
        "vyce" | "vyce-direct" => "vyce",
        "grok" => "grok",
        "muse" => "muse",
        "agy" | "antigravity" => "agy",
        "custom" => "custom",
        "hermes" => "hermes",
        _ => "hermes",
    }
}

/// Turn a transport failure into a specific line. Never the words "network error".
pub fn specific_transport_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("connection refused")
        || lower.contains("os error 111")
        || lower.contains("os error 10061")
        || lower.contains("actively refused")
    {
        return "connection refused".into();
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return "timed out".into();
    }
    if lower.contains("404") || lower.contains("not found") {
        return "http 404 not found".into();
    }
    if lower.contains("invalid gateway api key") {
        return "Invalid gateway API key (API_SERVER_KEY)".into();
    }
    if lower.contains("401") || lower.contains("unauthorized") {
        return "http 401 unauthorized".into();
    }
    if lower.contains("403") || lower.contains("forbidden") {
        return "http 403 forbidden".into();
    }
    if super::retry::is_retryable(error) {
        return super::retry::flatten_error(error);
    }
    if let Some(code) = lower.split("http ").nth(1).and_then(|rest| {
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if digits.len() == 3 {
            Some(digits)
        } else {
            None
        }
    }) {
        return format!("http {code}");
    }
    if lower.trim() == "network error" || lower.contains("network error") {
        return "connection failed".into();
    }
    let full = super::retry::flatten_error(error);
    if full.is_empty() {
        "connection failed".into()
    } else {
        full
    }
}

pub fn verify_status(label: &str, ok: bool, millis: u128, error: &str) -> String {
    if ok {
        format!("{label} verified · {millis}ms")
    } else {
        format!("{label} · {}", specific_transport_error(error))
    }
}

pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub const PROBE_PROMPT: &str = "Reply with exactly ok";

pub fn probe_cli_args(program: &str) -> Vec<String> {
    match program {
        "agy" => vec![
            "-p".into(),
            PROBE_PROMPT.into(),
            "--output-format".into(),
            "json".into(),
            "--print-timeout".into(),
            "30s".into(),
            "--dangerously-skip-permissions".into(),
        ],
        "muse" => muse_print_args(PROBE_PROMPT),
        _ => grok_print_args(PROBE_PROMPT),
    }
}

/// Muse Code 1.4 headless mode. `muse` with no subcommand is the TUI.
pub fn muse_print_args(prompt: &str) -> Vec<String> {
    vec![
        "exec".into(),
        "--disable-approval".into(),
        "--max-model-steps".into(),
        "1".into(),
        "--".into(),
        prompt.to_string(),
    ]
}

pub fn grok_print_args(prompt: &str) -> Vec<String> {
    vec![
        "-p".into(),
        prompt.to_string(),
        "--output-format".into(),
        "plain".into(),
        "--always-approve".into(),
        "--no-alt-screen".into(),
    ]
}

/// `/v1/models` then a 1-token chat. The key stays in the header only.
pub fn probe_openai(
    http: &dyn crate::io::Http,
    base: &str,
    model: &str,
    key: &str,
) -> Result<(), String> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("base URL is missing".into());
    }
    let bearer = {
        let key = key.trim();
        if key.is_empty() {
            None
        } else {
            Some(key.to_string())
        }
    };
    let models = crate::io::HttpRequest {
        method: "GET",
        url: format!("{base}/models"),
        body: None,
        bearer: bearer.clone(),
        content_type: None,
        headers: Vec::new(),
    };
    http.request(models)
        .map_err(|error| specific_transport_error(&redact_key(&error, key)))?;
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{"role": "user", "content": "ping"}]
    })
    .to_string();
    let chat = crate::io::HttpRequest {
        method: "POST",
        url: format!("{base}/chat/completions"),
        body: Some(body),
        bearer,
        content_type: Some("application/json".into()),
        headers: Vec::new(),
    };
    let text = http
        .request(chat)
        .map_err(|error| specific_transport_error(&redact_key(&error, key)))?;
    if text.to_lowercase().contains("\"error\"") && !text.contains("choices") {
        return Err(specific_transport_error(&redact_key(&text, key)));
    }
    if text.contains(key) && !key.is_empty() {
        return Err("probe echoed the key".into());
    }
    Ok(())
}

fn redact_key(text: &str, key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "[redacted]")
    }
}

pub fn canonical_provider(saved: &str) -> String {
    let saved = saved.trim();
    if let Some((kind, model)) = saved.split_once(':') {
        let model = model.trim();
        if !model.is_empty() && matches!(kind, "hermes" | "vyce" | "custom") {
            return format!("{kind}:{model}");
        }
    }
    migrate_saved_provider(saved).to_string()
}

pub fn provider_kind(saved: &str) -> super::TakeProvider {
    let id = canonical_provider(saved);
    let kind = id.split(':').next().unwrap_or("hermes");
    match kind {
        "vyce" => super::TakeProvider::Vyce,
        "grok" => super::TakeProvider::Grok,
        "muse" => super::TakeProvider::Muse,
        "agy" => super::TakeProvider::Agy,
        "custom" => super::TakeProvider::Custom,
        _ => super::TakeProvider::Hermes,
    }
}

pub fn provider_model(saved: &str) -> Option<String> {
    canonical_provider(saved)
        .split_once(':')
        .map(|(_, model)| model.to_string())
        .filter(|model| !model.is_empty())
}

pub fn provider_label(saved: &str) -> String {
    let model = provider_model(saved);
    match provider_kind(saved) {
        super::TakeProvider::Vyce => {
            format!(
                "Vyce direct · {}",
                model.unwrap_or_else(|| super::agents::VYCE_MODEL.to_string())
            )
        }
        super::TakeProvider::Grok => "Grok".into(),
        super::TakeProvider::Muse => "Muse".into(),
        super::TakeProvider::Agy => "Agy".into(),
        super::TakeProvider::Custom => {
            format!("Custom · {}", model.unwrap_or_else(|| "model".into()))
        }
        _ => {
            let model = model.unwrap_or_else(|| super::agents::VYCE_MODEL.to_string());
            if model == super::agents::VYCE_MODEL {
                super::agents::HERMES_VYCE_LABEL.to_string()
            } else {
                format!("Hermes · {model}")
            }
        }
    }
}

pub fn base_provider_menu() -> Vec<(String, String)> {
    vec![
        ("hermes".into(), provider_label("hermes")),
        ("vyce".into(), provider_label("vyce")),
        ("grok".into(), "Grok".into()),
        ("muse".into(), "Muse".into()),
        ("agy".into(), "Agy".into()),
        ("custom".into(), "Custom".into()),
    ]
}

pub fn models_from_openai(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(data) = value.get("data").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(|id| id.as_str()) else {
            continue;
        };
        let id = id.trim();
        if !id.is_empty() && !models.iter().any(|have: &String| have == id) {
            models.push(id.to_string());
        }
    }
    models
}

pub fn menu_with_models(
    hermes: &[String],
    vyce: &[String],
    custom: &[String],
) -> Vec<(String, String)> {
    let mut menu = base_provider_menu();
    for (kind, models) in [("hermes", hermes), ("vyce", vyce), ("custom", custom)] {
        for model in models {
            let id = format!("{kind}:{model}");
            let label = provider_label(&id);
            if menu.iter().any(|(_, have)| have == &label) {
                continue;
            }
            menu.push((id, label));
        }
    }
    menu
}

/// Official Antigravity headless print mode. The prompt is an argument, not stdin.
pub fn agy_print_args(prompt: &str) -> Vec<String> {
    vec![
        "-p".into(),
        prompt.to_string(),
        "--output-format".into(),
        "json".into(),
        "--print-timeout".into(),
        "45s".into(),
        "--dangerously-skip-permissions".into(),
    ]
}

pub fn agy_print_text(stdout: &str) -> Result<String, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(stdout.trim()) else {
        let text = stdout.trim();
        return if text.is_empty() {
            Err("Agy returned no text.".into())
        } else {
            Ok(text.to_string())
        };
    };
    let status = value
        .get("status")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if let Some(error) = value.get("error").and_then(|item| item.as_str())
        && !error.is_empty()
        && !status.eq_ignore_ascii_case("success")
    {
        return Err(error.to_string());
    }
    let response = value
        .get("response")
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if response.is_empty() {
        Err("Agy returned no text.".into())
    } else {
        Ok(response)
    }
}

pub fn run_take_chain(
    preferred: super::TakeProvider,
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
) -> super::TakeDraft {
    let choice = match preferred {
        super::TakeProvider::Free | super::TakeProvider::Treg => super::TakeProvider::Hermes,
        other => other,
    };
    let env_text = read_hermes_env();
    let yaml_text = read_hermes_yaml();
    let mut process = Vec::new();
    for name in [
        "VYCE_API_KEY",
        "VYCE_BASE_URL",
        "VYCE_API_BASE",
        "OPENAI_API_KEY",
    ] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            process.push((name, value));
        }
    }
    let mut user = Vec::new();
    if let Some(value) = super::agents::user_env_value("VYCE_API_KEY") {
        user.push(("VYCE_API_KEY".to_string(), value));
    }
    let process_ref: Vec<(&str, &str)> = process.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let user_ref: Vec<(&str, &str)> = user.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    run_take_chain_with(
        choice,
        config,
        post,
        http,
        tone,
        TakeSources {
            env_text: &env_text,
            yaml_text: &yaml_text,
            process: &process_ref,
            user: &user_ref,
        },
    )
}

pub struct TakeSources<'a> {
    pub env_text: &'a str,
    pub yaml_text: &'a str,
    pub process: &'a [(&'a str, &'a str)],
    pub user: &'a [(&'a str, &'a str)],
}

pub fn run_take_chain_with(
    choice: super::TakeProvider,
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
    sources: TakeSources<'_>,
) -> super::TakeDraft {
    let choice = match choice {
        super::TakeProvider::Free | super::TakeProvider::Treg => super::TakeProvider::Hermes,
        other => other,
    };
    match choice {
        super::TakeProvider::Hermes => hermes_vyce_take(config, post, http, tone),
        super::TakeProvider::Vyce => vyce_direct_take(config, post, http, tone, &sources),
        super::TakeProvider::Grok => cli_take("grok", "Grok", post, tone),
        super::TakeProvider::Muse => cli_take("muse", "Muse", post, tone),
        super::TakeProvider::Agy => cli_take("agy", "Agy", post, tone),
        super::TakeProvider::Custom => custom_take(config, post, http, tone, &sources),
        super::TakeProvider::Free | super::TakeProvider::Treg => {
            hermes_vyce_take(config, post, http, tone)
        }
    }
}

fn hermes_vyce_take(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
) -> super::TakeDraft {
    let mut config = config.clone();
    config.hermes_model = super::agents::HERMES_MODEL.to_string();
    config.hermes_api_key = super::agents::hermes_key_resolved(&config.hermes_api_key);
    if config.hermes_base_url.trim().is_empty() {
        config.hermes_base_url = super::agents::HERMES_BASE.to_string();
    }
    match hermes_raced(&config, post, tone, http) {
        Ok(text) => labeled_take("hermes", super::agents::HERMES_VYCE_LABEL, &text, &[]),
        Err(error) => failed_take("hermes", &super::retry::hermes_failure_note(&error)),
    }
}

fn vyce_direct_take(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
    sources: &TakeSources<'_>,
) -> super::TakeDraft {
    let mut auth = super::agents::vyce_direct_auth(
        sources.env_text,
        sources.yaml_text,
        sources.process,
        sources.user,
    );
    if config.ai_explicit && !config.ai_model.trim().is_empty() {
        auth.model = config.ai_model.trim().to_string();
    }
    let label = format!("Vyce direct · {}", auth.model);
    let prompt = super::agents::reply_prompt(&post.author, &post.text, tone);
    match super::agents::vyce_complete(&auth, http, &prompt) {
        Ok(text) => labeled_take("vyce", &label, &text, &[]),
        Err(error) => failed_take(
            "vyce",
            &super::retry::provider_failure_note("Vyce", &auth.model, &error),
        ),
    }
}

fn custom_take(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    http: &dyn crate::io::Http,
    tone: &str,
    sources: &TakeSources<'_>,
) -> super::TakeDraft {
    let env_text = sources.env_text;
    let process = sources.process;
    let user = sources.user;
    let base = config.ai_base_url.trim().trim_end_matches('/').to_string();
    let model = if config.ai_model.trim().is_empty() {
        super::agents::VYCE_MODEL.to_string()
    } else {
        config.ai_model.trim().to_string()
    };
    let key_name = if config.ai_key_env.trim().is_empty() {
        "OPENAI_API_KEY"
    } else {
        config.ai_key_env.trim()
    };
    let key = if config.ai_api_key.trim().is_empty() {
        super::agents::lookup_named_env(key_name, env_text, process, user).unwrap_or_default()
    } else {
        config.ai_api_key.trim().to_string()
    };
    let auth = super::agents::VyceAuth {
        base,
        model: model.clone(),
        key,
    };
    let prompt = super::agents::reply_prompt(&post.author, &post.text, tone);
    match super::agents::vyce_complete(&auth, http, &prompt) {
        Ok(text) => labeled_take("custom", &format!("Custom · {model}"), &text, &[]),
        Err(error) => failed_take(
            "custom",
            &super::retry::provider_failure_note("Custom", &model, &error),
        ),
    }
}

fn cli_take(program: &str, label: &str, post: &super::SourcePost, tone: &str) -> super::TakeDraft {
    match hidden_agent_draft(program, label, post, tone) {
        Ok(text) => labeled_take(cli_source(program), label, &text, &[]),
        Err(error) => failed_take(cli_source(program), &format!("{label} · {error}")),
    }
}

fn cli_source(program: &str) -> &'static str {
    match program {
        "grok" => "grok",
        "muse" => "muse",
        "agy" => "agy",
        _ => "none",
    }
}

fn failed_take(source: &'static str, note: &str) -> super::TakeDraft {
    super::TakeDraft {
        text: String::new(),
        source,
        note: note.to_string(),
        variants: Vec::new(),
    }
}

fn agy_headless(program: &str, prompt: &str) -> Result<String, String> {
    let mut command = std::process::Command::new(program);
    command.args(agy_print_args(prompt));
    apply_hidden_command(&mut command);
    let Ok(child) = command.spawn() else {
        return Err("Agy CLI was not found. Nothing was drafted.".into());
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    if let Ok(Ok(output)) = rx.recv_timeout(TAKE_TIMEOUT) {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if stdout.trim().is_empty() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if err.is_empty() {
                Err("Agy returned no text. Headless print mode produced an empty response.".into())
            } else {
                Err(agy_credit_error(&err).unwrap_or(err))
            }
        } else {
            agy_print_text(&stdout).map_err(|error| agy_credit_error(&error).unwrap_or(error))
        }
    } else {
        kill_pid(pid);
        Err("Agy did not return text within 45s. Nothing was drafted.".into())
    }
}

pub fn hidden_agent_draft(
    program: &str,
    label: &str,
    post: &super::SourcePost,
    tone: &str,
) -> Result<String, String> {
    if program == "grok" {
        return grok_cli(post, tone);
    }
    let prompt = super::agents::reply_prompt(&post.author, &post.text, tone);
    if label == "Agy" {
        return agy_headless(program, &prompt);
    }
    let mut command = std::process::Command::new(program);
    command.args(muse_print_args(&prompt));
    apply_hidden_command(&mut command);
    let _ = command.stdin(std::process::Stdio::null());
    let Ok(child) = command.spawn() else {
        return Err(format!("{label} CLI was not found. Nothing was drafted."));
    };
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    if let Ok(Ok(output)) = rx.recv_timeout(HERMES_TAKE_TIMEOUT) {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if err.is_empty() {
                Err(format!(
                    "{label} did not return text. Its CLI stays interactive, so this draft was not written."
                ))
            } else if label == "Agy" && agy_credit_error(&err).is_some() {
                Err(agy_credit_error(&err).unwrap_or(err))
            } else if label == "Muse" && muse_needs_trust(&err) {
                Err(muse_trust_line(std::path::Path::new("agent-work")))
            } else {
                Err(err)
            }
        } else {
            Ok(text)
        }
    } else {
        kill_pid(pid);
        Err(format!(
            "{label} did not return text within 90s. Nothing was drafted."
        ))
    }
}

fn hermes_raced(
    config: &crate::config::XConfig,
    post: &super::SourcePost,
    tone: &str,
    http: &dyn crate::io::Http,
) -> Result<String, String> {
    let draft = super::hermes_draft::hermes_take_toned(config, http, post, tone);
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
    super::grok_take::run(post, tone)
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

pub(crate) fn register_job(id: &str, pid: u32) {
    let mut jobs = grok_jobs().lock().unwrap_or_else(|err| err.into_inner());
    jobs.retain(|(have, _)| have != id);
    jobs.push((id.to_string(), pid));
}

pub(crate) fn unregister_job(id: &str) {
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

pub(crate) fn kill_pid(pid: u32) {
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
            Some("Agy: Google credits depleted (402)".into())
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
        assert_eq!(
            take_chain_order(),
            ["hermes", "vyce", "grok", "muse", "agy", "custom"]
        );
        assert_eq!(migrate_saved_provider("free"), "hermes");
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
        assert!(fallback.is_empty());
        assert!(error.contains("Spicy"));
    }

    #[test]
    fn vyce_503_falls_through_to_hermes_and_the_key_stays_out() {
        assert_eq!(
            take_chain_order(),
            ["hermes", "vyce", "grok", "muse", "agy", "custom"]
        );
        assert_eq!(migrate_saved_provider("free"), "hermes");
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
            hermes_model: String::new(),
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
        let empty = TakeSources {
            env_text: "",
            yaml_text: "",
            process: &[],
            user: &[],
        };
        let draft = run_take_chain_with(
            super::super::TakeProvider::Hermes,
            &config,
            &post,
            &http,
            "Funny",
            empty,
        );
        assert_eq!(draft.source, "hermes");
        assert!(draft.text.contains("Hermes"));
        assert_eq!(draft.note, super::super::agents::HERMES_VYCE_LABEL);
        assert!(!draft.note.contains("treg"));
        assert!(!draft.note.contains("vyce-secret-key"));
        assert!(!draft.note.contains("hermes-secret"));
        struct Down;
        impl crate::io::Http for Down {
            fn request(&self, request: crate::io::HttpRequest) -> Result<String, String> {
                assert!(request.url.contains("hermes.test"));
                assert!(!request.url.contains("treg"));
                let body = request.body.unwrap_or_default();
                assert!(body.contains("hermes-agent"));
                assert!(!body.contains("glm-5.3-flash"));
                assert!(!body.contains("hermes-secret"));
                Err(
                    r#"http 503: {"error":{"message":"This model is currently unavailable. Please try again later.","code":"service_unavailable"}}"#
                        .into(),
                )
            }
        }
        let down = run_take_chain_with(
            super::super::TakeProvider::Free,
            &config,
            &post,
            &Down,
            "Funny",
            TakeSources {
                env_text: "",
                yaml_text: "",
                process: &[],
                user: &[],
            },
        );
        assert!(down.text.is_empty());
        assert!(
            down.note
                .contains("Vyce glm-5.3-flash busy (503), try again or pick another provider")
        );
        assert!(
            down.note
                .contains("This model is currently unavailable. Please try again later.")
        );
        assert!(!down.note.contains("Vyce direct"));
        assert!(!down.note.contains("hermes-secret"));
        let yaml =
            "providers:\n  vyce:\n    api: https://vyceai.com/v1\n    key_env: VYCE_API_KEY\n";
        struct Direct;
        impl crate::io::Http for Direct {
            fn request(&self, request: crate::io::HttpRequest) -> Result<String, String> {
                assert!(
                    request
                        .url
                        .starts_with("https://vyceai.com/v1/chat/completions")
                );
                assert_eq!(request.bearer.as_deref(), Some("from-user-env"));
                let body = request.body.unwrap_or_default();
                assert!(body.contains("glm-5.3-flash"));
                assert!(!body.contains("from-user-env"));
                Err("http 503".into())
            }
        }
        let direct = run_take_chain_with(
            super::super::TakeProvider::Vyce,
            &config,
            &post,
            &Direct,
            "Funny",
            TakeSources {
                env_text: "",
                yaml_text: yaml,
                process: &[("VYCE_API_KEY", "from-user-env")],
                user: &[],
            },
        );
        assert!(direct.note.contains("Vyce"));
        assert!(!direct.note.contains("from-user-env"));
        let log = super::super::agents::take_log_line("hermes", "http 503");
        assert_eq!(log, "INFO take: hermes http 503");
        assert!(!log.contains("from-user-env"));
        let missing =
            hidden_agent_draft("agy-not-a-real-binary", "Agy", &post, "Funny").unwrap_err();
        assert!(missing.contains("not found"));
        assert!(missing.contains("Nothing was drafted"));
        let args = agy_print_args("say hi");
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "say hi");
        assert!(args.windows(2).any(|pair| {
            pair[0] == "--output-format" && pair[1] == "json"
                || pair[0] == "--print-timeout" && pair[1] == "45s"
        }));
        assert!(
            args.iter()
                .any(|arg| arg == "--dangerously-skip-permissions")
        );
        let parsed =
            agy_print_text(r#"{"status":"SUCCESS","response":"a real headless reply"}"#).unwrap();
        assert_eq!(parsed, "a real headless reply");
        assert!(agy_print_text(r#"{"status":"ERROR","error":"authentication required"}"#).is_err());
        let menu = menu_with_models(
            &["glm-5.3-flash".into(), "hermes-agent".into()],
            &[],
            &["my-model".into()],
        );
        assert_eq!(menu[0].1, "Hermes · Vyce glm-5.3-flash");
        assert!(
            menu.iter()
                .any(|(id, label)| id == "hermes:hermes-agent" && label == "Hermes · hermes-agent")
        );
        assert!(menu.iter().any(|(id, _)| id == "custom:my-model"));
        assert_eq!(
            canonical_provider("hermes:hermes-agent"),
            "hermes:hermes-agent"
        );
        assert_eq!(provider_kind("free"), super::super::TakeProvider::Hermes);
        assert_eq!(canonical_provider("grok"), "grok");
        assert_eq!(provider_kind("Grok"), super::super::TakeProvider::Grok);
        let picked = canonical_provider("grok");
        assert_eq!(picked, "grok");
        assert_eq!(provider_kind(&picked), super::super::TakeProvider::Grok);
        let refused = specific_transport_error("Network Error");
        assert_eq!(refused, "connection failed");
        assert!(!refused.to_lowercase().contains("network error"));
        assert_eq!(
            specific_transport_error("tcp connect error: Connection refused (os error 111)"),
            "connection refused"
        );
        let ok_line = verify_status("Grok", true, 40, "");
        assert_eq!(ok_line, "Grok verified · 40ms");
        let bad_line = verify_status("Grok", false, 0, "grok not found");
        assert!(bad_line.contains("not found"));
        let grok_probe = probe_cli_args("grok");
        assert_eq!(grok_probe[0], "-p");
        assert!(grok_probe.iter().any(|arg| arg == PROBE_PROMPT));
        assert!(
            grok_probe
                .windows(2)
                .any(|p| p == ["--output-format", "plain"])
        );
        assert!(grok_probe.iter().any(|arg| arg == "--always-approve"));
        assert!(grok_probe.iter().any(|arg| arg == "--no-alt-screen"));
        assert!(probe_cli_args("agy").iter().any(|arg| arg == "-p"));
        assert_eq!(probe_cli_args("muse"), muse_print_args(PROBE_PROMPT));
        assert_eq!(PROBE_TIMEOUT, std::time::Duration::from_secs(30));
        assert_eq!(
            specific_transport_error("401 Invalid gateway API key (API_SERVER_KEY)"),
            "Invalid gateway API key (API_SERVER_KEY)"
        );
        assert_eq!(
            agy_credit_error("http 402"),
            Some("Agy: Google credits depleted (402)".into())
        );
        struct ProbeHttp {
            fail_models: bool,
        }
        impl crate::io::Http for ProbeHttp {
            fn request(&self, request: crate::io::HttpRequest) -> Result<String, String> {
                assert!(
                    request.bearer.as_deref().unwrap_or("").is_empty()
                        || request.url.contains("models")
                        || request.url.contains("chat")
                );
                let body = request.body.clone().unwrap_or_default();
                assert!(!body.contains("secret-key"));
                if self.fail_models {
                    return Err("connection refused".into());
                }
                if request.url.ends_with("/models") {
                    return Ok(r#"{"data":[{"id":"glm-5.3-flash"}]}"#.into());
                }
                assert!(body.contains("\"max_tokens\":1"));
                Ok(r#"{"choices":[{"message":{"content":"ok"}}]}"#.into())
            }
        }
        probe_openai(
            &ProbeHttp { fail_models: false },
            "http://127.0.0.1:8642/v1",
            "glm-5.3-flash",
            "secret-key",
        )
        .unwrap();
        let failed = probe_openai(
            &ProbeHttp { fail_models: true },
            "http://127.0.0.1:8642/v1",
            "glm-5.3-flash",
            "secret-key",
        )
        .unwrap_err();
        assert_eq!(failed, "connection refused");
        assert!(!failed.contains("secret-key"));
        assert_eq!(
            models_from_openai(r#"{"data":[{"id":"glm-5.3-flash"},{"id":"other"}]}"#),
            vec!["glm-5.3-flash".to_string(), "other".to_string()]
        );
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
    let error = format!("{tone} timed out or failed after {tries} tries");
    if previous.is_empty() {
        (Vec::new(), error)
    } else {
        (previous.to_vec(), error)
    }
}
