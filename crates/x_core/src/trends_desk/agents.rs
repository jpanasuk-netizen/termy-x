//! Termy XL agent launcher.
//!
//! Grok, Hermes, Agy, and Muse draft replies. Hermes on Windows talks to the
//! local gateway. Nothing in this module posts.

use std::path::{Path, PathBuf};

use crate::agents::Agent;
use crate::io::{Http, HttpRequest};

pub const HERMES_BASE: &str = "http://127.0.0.1:8642/v1";
pub const HERMES_MODEL: &str = "hermes-agent";
pub const HERMES_HEALTH_PATH: &str = "/health";
const VYCE_MODEL: &str = "glm-5.3-flash";

/// Command line used when Termy XL's x.toml does not set one.
/// Hermes is the Windows-native CLI, not `wsl.exe`.
pub fn default_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Grok => "grok",
        Agent::Hermes => "hermes",
        Agent::Agy => "agy",
        Agent::Muse => "muse",
    }
}

pub fn known_hermes_exe(local_app_data: Option<&Path>) -> Option<PathBuf> {
    Some(
        local_app_data?
            .join("hermes")
            .join("bin")
            .join("hermes.exe"),
    )
}

pub fn missing_line(agent: Agent, program: &str) -> String {
    format!("{} is missing: `{program}` was not found.", agent.label())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub agent: Agent,
    pub program: String,
    pub args: Vec<String>,
    pub display: String,
    pub source: String,
}

pub fn resolve_launch(
    agent: Agent,
    configured: Option<&str>,
    exists: &dyn Fn(&str) -> bool,
    hermes_exe: Option<&str>,
) -> Result<Launch, String> {
    if let Some(line) = configured.map(str::trim).filter(|line| !line.is_empty()) {
        let (program, args) = split_command(line);
        if program.is_empty() || !exists(&program) {
            let shown = if program.is_empty() {
                agent.tag()
            } else {
                program.as_str()
            };
            return Err(missing_line(agent, shown));
        }
        return Ok(Launch {
            agent,
            program,
            args,
            display: line.to_string(),
            source: "x.toml".into(),
        });
    }
    if agent == Agent::Hermes
        && let Some(exe) = hermes_exe.filter(|path| exists(path))
    {
        return Ok(Launch {
            agent,
            program: exe.to_string(),
            args: Vec::new(),
            display: exe.to_string(),
            source: "hermes.exe".into(),
        });
    }
    let program = default_command(agent);
    if exists(program) {
        return Ok(Launch {
            agent,
            program: program.to_string(),
            args: Vec::new(),
            display: program.to_string(),
            source: "default".into(),
        });
    }
    Err(missing_line(agent, program))
}

pub fn split_command(line: &str) -> (String, Vec<String>) {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(mark) if ch == mark => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    let mut parts = parts.into_iter();
    (parts.next().unwrap_or_default(), parts.collect())
}

/// Add the default agent commands under `[agents]` when they are absent.
/// Does not write an API key.
pub fn seed_agent_block(text: &str) -> String {
    let defaults = [
        ("grok", "grok"),
        ("hermes", "hermes"),
        ("agy", "agy"),
        ("muse", "muse"),
        ("hermes_base_url", HERMES_BASE),
        ("hermes_model", HERMES_MODEL),
    ];
    let mut text = text.to_string();
    for (key, value) in defaults {
        if agents_key_present(&text, key) {
            continue;
        }
        text = insert_agents_line(&text, key, value);
    }
    text
}

fn agents_key_present(text: &str, key: &str) -> bool {
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim().to_string();
            continue;
        }
        let Some((left, _)) = line.split_once('=') else {
            continue;
        };
        let left = left.trim();
        if section == "agents" && left == key {
            return true;
        }
        if left == format!("agents.{key}") {
            return true;
        }
    }
    false
}

fn insert_agents_line(text: &str, key: &str, value: &str) -> String {
    let assignment = format!("{key} = \"{}\"", value.replace('"', "\\\""));
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut section_at = None;
    let mut insert_at = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            if name.trim() == "agents" {
                section_at = Some(index);
                insert_at = None;
            } else if section_at.is_some() {
                insert_at = Some(index);
                break;
            }
        }
    }
    if section_at.is_none() {
        let mut out = text.trim_end().to_string();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("[agents]\n");
        out.push_str(&assignment);
        out.push('\n');
        return out;
    }
    let at = insert_at.unwrap_or(lines.len());
    let mut owned = lines;
    owned.insert(at, assignment);
    let mut out = owned.join("\n");
    out.push('\n');
    out
}

pub fn write_hermes_settings(text: &str, base: &str, model: &str, key: &str) -> String {
    let mut text = seed_agent_block(text);
    text = upsert_agents_value(&text, "hermes_base_url", base);
    text = upsert_agents_value(&text, "hermes_model", model);
    if key.trim().is_empty() {
        remove_agents_key(&text, "hermes_api_key")
    } else {
        upsert_agents_value(&text, "hermes_api_key", key.trim())
    }
}

fn upsert_agents_value(text: &str, key: &str, value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return remove_agents_key(text, key);
    }
    insert_agents_line(&remove_agents_key(text, key), key, value)
}

pub fn remove_agents_key(text: &str, key: &str) -> String {
    let mut section = String::new();
    let mut out = String::new();
    for raw in text.lines() {
        let code = raw.split('#').next().unwrap_or("").trim();
        if let Some(name) = code
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = name.trim().to_string();
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        if section == "agents"
            && let Some((left, _)) = code.split_once('=')
            && left.trim() == key
        {
            continue;
        }
        out.push_str(raw);
        out.push('\n');
    }
    out
}

pub fn reply_prompt(author: &str, tweet: &str, tone: &str) -> String {
    format!(
        "Tone: {tone}\nAuthor: @{author}\nPost:\n{tweet}\n\nWrite 3 different replies. One agrees and adds a point. One asks a question. One is contrarian or witty. Use different length and angle. Separate them with a line containing only ---. Each under 240 characters. No hashtags. Output only the 3 replies."
    )
}

pub fn api_server_key_from_env_file(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != "API_SERVER_KEY" {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

pub fn choose_hermes_key(saved: &str, env_text: &str) -> Option<String> {
    let saved = saved.trim();
    if !saved.is_empty() {
        return Some(saved.to_string());
    }
    api_server_key_from_env_file(env_text)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HermesAuth {
    pub base: String,
    pub model: String,
    pub key: String,
}

pub fn hermes_auth(base: &str, model: &str, saved_key: &str, env_text: &str) -> HermesAuth {
    let base = if base.trim().is_empty() {
        HERMES_BASE.to_string()
    } else {
        base.trim().trim_end_matches('/').to_string()
    };
    let model = if model.trim().is_empty() {
        HERMES_MODEL.to_string()
    } else {
        model.trim().to_string()
    };
    HermesAuth {
        base,
        model,
        key: choose_hermes_key(saved_key, env_text).unwrap_or_default(),
    }
}

pub fn hermes_origin(base: &str) -> String {
    let base = if base.trim().is_empty() {
        HERMES_BASE
    } else {
        base.trim()
    };
    let trimmed = base.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed).to_string()
}

pub fn hermes_health_url(base: &str) -> String {
    format!("{}{HERMES_HEALTH_PATH}", hermes_origin(base))
}

pub fn hermes_models_request(base: &str, key: &str) -> HttpRequest {
    let base = if base.trim().is_empty() {
        HERMES_BASE
    } else {
        base.trim().trim_end_matches('/')
    };
    HttpRequest {
        method: "GET",
        url: format!("{base}/models"),
        body: None,
        bearer: {
            let key = key.trim();
            if key.is_empty() {
                None
            } else {
                Some(key.to_string())
            }
        },
        content_type: None,
        headers: Vec::new(),
    }
}

/// One line for the Hermes status pill. The base URL is whatever was configured.
pub fn hermes_models_line(base: &str, result: Result<&str, &str>) -> String {
    let base = if base.trim().is_empty() {
        HERMES_BASE
    } else {
        base.trim().trim_end_matches('/')
    };
    match result {
        Err(error) => {
            let lower = error.to_lowercase();
            if lower.contains("401") || lower.contains("unauthorized") {
                format!("Hermes · needs a key (401)")
            } else if lower.contains("402")
                || lower.contains("payment required")
                || lower.contains("insufficient")
            {
                format!("Hermes · needs credits (402)")
            } else {
                let short = one_line(error);
                let short: String = short.chars().take(80).collect();
                if short.is_empty() {
                    format!("Hermes · unreachable at {base}")
                } else {
                    format!("Hermes · unreachable ({short})")
                }
            }
        }
        Ok(body) => {
            let names = model_ids(body);
            if names.is_empty() {
                format!("Hermes · no models listed at {base}")
            } else {
                format!("Hermes · {} @ {base}", names.join(", "))
            }
        }
    }
}

fn model_ids(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .pointer("/data")
        .and_then(|item| item.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("id").and_then(|id| id.as_str()))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn hermes_health_request(base: &str) -> HttpRequest {
    HttpRequest {
        method: "GET",
        url: hermes_health_url(base),
        body: None,
        bearer: None,
        content_type: None,
        headers: Vec::new(),
    }
}

pub fn hermes_pill(result: Result<(), String>) -> String {
    match result {
        Ok(()) => "Hermes · up".into(),
        Err(error) => {
            let error = error.split_whitespace().collect::<Vec<_>>().join(" ");
            let short: String = error.chars().take(80).collect();
            if short.is_empty() {
                "Hermes · unreachable".into()
            } else {
                format!("Hermes · unreachable ({short})")
            }
        }
    }
}

/// Chat body. A bare `glm-5.3-flash` is sent as the vyce provider form,
/// because the gateway ignores that model name on its own.
pub fn hermes_body(model: &str, prompt: &str) -> String {
    let mut body = serde_json::json!({
        "temperature": 0.9,
        "messages": [
            {"role": "system", "content": "You draft X replies. You never post."},
            {"role": "user", "content": prompt}
        ]
    });
    let model = model.trim();
    let vyce = model == VYCE_MODEL
        || model.eq_ignore_ascii_case("vyce:glm-5.3-flash")
        || model.eq_ignore_ascii_case("vyce/glm-5.3-flash");
    if vyce {
        body["provider"] = serde_json::json!("vyce");
        body["model"] = serde_json::json!(VYCE_MODEL);
    } else if model.is_empty() {
        body["model"] = serde_json::json!(HERMES_MODEL);
    } else {
        body["model"] = serde_json::json!(model);
    }
    body.to_string()
}

pub fn plan_hermes_draft(auth: &HermesAuth, prompt: &str) -> Result<HttpRequest, String> {
    if auth.key.trim().is_empty() {
        return Err(
            "Hermes key is missing. Save it in Termy XL settings or set API_SERVER_KEY in the Hermes .env."
                .into(),
        );
    }
    Ok(HttpRequest {
        method: "POST",
        url: format!("{}/chat/completions", auth.base.trim_end_matches('/')),
        body: Some(hermes_body(&auth.model, prompt)),
        bearer: Some(auth.key.clone()),
        content_type: Some("application/json".into()),
        headers: Vec::new(),
    })
}

pub enum DraftRoute {
    Missing(String),
    Cli {
        program: String,
        args: Vec<String>,
        stdin: String,
    },
    Hermes {
        request: HttpRequest,
        secret: String,
    },
}

pub fn plan_agent_draft(
    agent: Agent,
    launch: Result<&Launch, &str>,
    hermes: Option<&HermesAuth>,
    prompt: &str,
) -> DraftRoute {
    if agent == Agent::Hermes {
        let Some(auth) = hermes else {
            return DraftRoute::Missing("Hermes is not configured.".into());
        };
        return match plan_hermes_draft(auth, prompt) {
            Ok(request) => {
                let secret = auth.key.clone();
                DraftRoute::Hermes { request, secret }
            }
            Err(error) => DraftRoute::Missing(error),
        };
    }
    match launch {
        Ok(launch) => DraftRoute::Cli {
            program: launch.program.clone(),
            args: launch.args.clone(),
            stdin: prompt.to_string(),
        },
        Err(error) => DraftRoute::Missing(error.to_string()),
    }
}

pub trait PromptCli {
    fn run(&self, program: &str, args: &[String], stdin: &str) -> Result<String, String>;
}

/// Ask Hermes for `n` replies. Failure, including 401 and 402, still returns
/// three different fallback lines and a one-line note. Nothing is posted.
pub fn draft_hermes_variants(
    auth: &HermesAuth,
    http: &dyn Http,
    author: &str,
    tweet: &str,
    tone: &str,
    n: usize,
) -> (Vec<String>, String) {
    let n = n.clamp(1, 3);
    let prompt = reply_prompt(author, tweet, tone);
    let result = match plan_hermes_draft(auth, &prompt) {
        Ok(request) => {
            let secret = auth.key.clone();
            match http.request(request) {
                Ok(body) => parse_chat_text(&body).map_err(|error| redact_secret(&error, &secret)),
                Err(error) => Err(redact_secret(&error, &secret)),
            }
        }
        Err(error) => Err(error),
    };
    let (mut lines, note) = variants_from_draft(tweet, result);
    lines.truncate(n);
    while lines.len() < n {
        lines.push(format!("Take {}: {tweet}", lines.len() + 1));
    }
    (lines, note)
}

pub fn execute_draft(
    route: DraftRoute,
    cli: &dyn PromptCli,
    http: &dyn Http,
) -> Result<String, String> {
    match route {
        DraftRoute::Missing(error) => Err(error),
        DraftRoute::Cli {
            program,
            args,
            stdin,
        } => cli.run(&program, &args, &stdin),
        DraftRoute::Hermes { request, secret } => {
            let result = match http.request(request) {
                Ok(body) => parse_chat_text(&body),
                Err(error) => Err(error),
            };
            result.map_err(|error| redact_secret(&error, &secret))
        }
    }
}

pub fn redact_secret(text: &str, secret: &str) -> String {
    let secret = secret.trim();
    if secret.len() < 4 {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

fn parse_chat_text(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("hermes: bad response ({error})"))?;
    if let Some(message) = value
        .pointer("/error/message")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("message").and_then(|item| item.as_str()))
        && value.pointer("/choices/0/message/content").is_none()
    {
        return Err(format!("hermes: {message}"));
    }
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(|item| item.as_str())
        .or_else(|| value.pointer("/output/text").and_then(|item| item.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        Err("hermes returned an empty draft".into())
    } else {
        Ok(text)
    }
}

pub fn split_agent_output(text: &str) -> Vec<String> {
    if text.contains("---") {
        return text
            .split("---")
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect();
    }
    crate::handoff::parse_options(text)
}

/// Model text is deduped. A failure still yields three different fallback replies.
pub fn variants_from_draft(tweet: &str, result: Result<String, String>) -> (Vec<String>, String) {
    match result {
        Ok(text) => (
            super::distinct_variants(tweet, &split_agent_output(&text)),
            String::new(),
        ),
        Err(error) => (super::fallback_angles(tweet), one_line(&error)),
    }
}

fn one_line(error: &str) -> String {
    error.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPane {
    pub agent: String,
    pub command: String,
    pub scrollback: String,
    pub input: String,
    pub notice: String,
}

impl AgentPane {
    pub fn open(agent: Agent, command: &str) -> Self {
        Self {
            agent: agent.label().into(),
            command: command.to_string(),
            scrollback: format!("$ {command}\n"),
            input: String::new(),
            notice: String::new(),
        }
    }

    pub fn missing(agent: Agent, notice: &str) -> Self {
        let mut pane = Self::open(agent, "");
        pane.notice = notice.to_string();
        pane
    }

    pub fn push_str(&mut self, text: &str) {
        self.input.push_str(text);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn submit(&mut self) -> String {
        let line = self.input.trim().to_string();
        self.input.clear();
        if line.is_empty() {
            return line;
        }
        self.scrollback.push_str("> ");
        self.scrollback.push_str(&line);
        self.scrollback.push('\n');
        line
    }

    pub fn append_output(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.scrollback.push_str(text);
        if !self.scrollback.ends_with('\n') {
            self.scrollback.push('\n');
        }
    }

    pub fn use_as_draft(&self) -> String {
        let pending = self.input.trim();
        if !pending.is_empty() {
            return pending.to_string();
        }
        if let Some(line) = self
            .scrollback
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('$') && !line.starts_with('>'))
        {
            return line.to_string();
        }
        self.scrollback
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| line.starts_with('>'))
            .map(|line| line.trim_start_matches('>').trim().to_string())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Http;

    struct ScriptCli {
        output: Result<String, String>,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl PromptCli for ScriptCli {
        fn run(&self, program: &str, args: &[String], stdin: &str) -> Result<String, String> {
            self.seen
                .lock()
                .unwrap()
                .push(format!("{program} {} :: {stdin}", args.join(" ")));
            self.output.clone()
        }
    }

    struct ScriptHttp {
        body: Result<String, String>,
    }

    impl Http for ScriptHttp {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            if request.bearer.as_deref() == Some("leak-me") {
                return Err(format!("gateway rejected leak-me at {}", request.url));
            }
            self.body.clone()
        }
    }

    fn assert_distinct(lines: &[String]) {
        assert_eq!(lines.len(), 3);
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[1], lines[2]);
        assert_ne!(lines[0], lines[2]);
    }

    #[test]
    fn defaults_are_native_commands_and_hermes_is_not_wsl() {
        assert_eq!(default_command(Agent::Grok), "grok");
        assert_eq!(default_command(Agent::Hermes), "hermes");
        assert_eq!(default_command(Agent::Agy), "agy");
        assert_eq!(default_command(Agent::Muse), "muse");
        assert!(!default_command(Agent::Hermes).contains("wsl"));
        let exe = known_hermes_exe(Some(Path::new(r"C:\Users\j\AppData\Local"))).unwrap();
        assert!(exe.ends_with(Path::new("hermes").join("bin").join("hermes.exe")));
    }

    #[test]
    fn x_toml_wins_and_a_missing_binary_is_one_line() {
        let exists = |name: &str| name == "grok";
        let grok = resolve_launch(Agent::Grok, Some("grok --now"), &exists, None).unwrap();
        assert_eq!(grok.source, "x.toml");
        assert_eq!(grok.program, "grok");
        assert_eq!(grok.args, vec!["--now".to_string()]);
        let missing = resolve_launch(Agent::Agy, None, &exists, None).unwrap_err();
        assert_eq!(missing, "Agy is missing: `agy` was not found.");
        assert!(!missing.contains('\n'));
        let hermes = resolve_launch(
            Agent::Hermes,
            None,
            &|name| name.ends_with("hermes.exe"),
            Some(r"C:\Users\j\AppData\Local\hermes\bin\hermes.exe"),
        )
        .unwrap();
        assert_eq!(hermes.source, "hermes.exe");
        assert!(!hermes.display.contains("wsl"));
    }

    #[test]
    fn seed_writes_native_hermes_and_does_not_copy_a_key() {
        let seeded = seed_agent_block("ai.model = \"llama3.2\"\n");
        assert!(seeded.contains("hermes = \"hermes\""));
        assert!(seeded.contains("grok = \"grok\""));
        assert!(seeded.contains("agy = \"agy\""));
        assert!(seeded.contains("muse = \"muse\""));
        assert!(seeded.contains(HERMES_BASE));
        assert!(seeded.contains(HERMES_MODEL));
        assert!(!seeded.contains("wsl"));
        assert!(!seeded.contains("hermes_api_key"));
        assert!(!seeded.contains("API_SERVER_KEY"));
        let saved = write_hermes_settings(&seeded, HERMES_BASE, HERMES_MODEL, "user-saved");
        assert!(saved.contains("hermes_api_key = \"user-saved\""));
        let cleared = write_hermes_settings(&saved, HERMES_BASE, HERMES_MODEL, "");
        assert!(!cleared.contains("hermes_api_key"));
        assert!(!cleared.contains("user-saved"));
        let custom = seed_agent_block("[agents]\nhermes = \"C:\\\\Tools\\\\hermes.exe\"\n");
        assert!(custom.contains("hermes = \"C:\\\\Tools\\\\hermes.exe\""));
        assert_eq!(custom.matches("hermes =").count(), 1);
    }

    #[test]
    fn hermes_key_prefers_the_saved_value_and_stays_out_of_errors() {
        let env = "API_SERVER_KEY=\"from-env\"\nOTHER=1\n";
        assert_eq!(
            choose_hermes_key("saved-key", env).as_deref(),
            Some("saved-key")
        );
        assert_eq!(choose_hermes_key("  ", env).as_deref(), Some("from-env"));
        assert!(choose_hermes_key("", "API_SERVER_KEY=\n").is_none());
        let auth = hermes_auth("", "", "leak-me", env);
        assert_eq!(auth.base, HERMES_BASE);
        assert_eq!(auth.model, HERMES_MODEL);
        assert_eq!(auth.key, "leak-me");
        let prompt = reply_prompt("ada", "Courts are testing models", "Spicy");
        let route = plan_agent_draft(Agent::Hermes, Err("unused"), Some(&auth), &prompt);
        let cli = ScriptCli {
            output: Ok("no".into()),
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let err = execute_draft(
            route,
            &cli,
            &ScriptHttp {
                body: Err("unused".into()),
            },
        )
        .unwrap_err();
        assert!(err.contains("[redacted]"));
        assert!(!err.contains("leak-me"));
        assert!(cli.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn hermes_model_defaults_and_a_bare_vyce_name_is_sent_with_provider() {
        let plain: serde_json::Value =
            serde_json::from_str(&hermes_body(HERMES_MODEL, "hi")).unwrap();
        assert_eq!(plain["model"], "hermes-agent");
        assert!(plain.get("provider").is_none());
        assert_eq!(plain["temperature"], 0.9);
        let vyce: serde_json::Value = serde_json::from_str(&hermes_body(VYCE_MODEL, "hi")).unwrap();
        assert_eq!(vyce["provider"], "vyce");
        assert_eq!(vyce["model"], VYCE_MODEL);
        let health = hermes_health_request("");
        assert_eq!(health.method, "GET");
        assert_eq!(health.url, "http://127.0.0.1:8642/health");
        assert!(health.bearer.is_none());
        let models = hermes_models_request("http://10.0.0.8:8642/v1", "");
        assert_eq!(models.url, "http://10.0.0.8:8642/v1/models");
        assert!(models.bearer.is_none());
        assert_eq!(
            hermes_models_line("http://10.0.0.8:8642/v1", Err("connection refused")),
            "Hermes · unreachable (connection refused)"
        );
        assert_eq!(
            hermes_models_line("", Err("http 401 unauthorized")),
            "Hermes · needs a key (401)"
        );
        assert_eq!(
            hermes_models_line(
                "http://10.0.0.8:8642/v1",
                Ok(r#"{"data":[{"id":"hermes-agent"}]}"#)
            ),
            "Hermes · hermes-agent @ http://10.0.0.8:8642/v1"
        );
        assert_eq!(hermes_pill(Ok(())), "Hermes · up");
        assert!(hermes_pill(Err("connection refused".into())).starts_with("Hermes · unreachable"));
    }

    #[test]
    fn variants_are_pairwise_different_on_the_model_and_fallback_paths() {
        let tweet = "Courts are testing whether models can write opinions";
        let cli = ScriptCli {
            output: Ok("same line\n---\nsame line\n---\nsame line".into()),
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let launch = Launch {
            agent: Agent::Grok,
            program: "fake-grok".into(),
            args: vec!["--draft".into()],
            display: "fake-grok --draft".into(),
            source: "test".into(),
        };
        let route = plan_agent_draft(Agent::Grok, Ok(&launch), None, "prompt");
        let text = execute_draft(
            route,
            &cli,
            &ScriptHttp {
                body: Ok(String::new()),
            },
        )
        .unwrap();
        assert!(cli.seen.lock().unwrap()[0].starts_with("fake-grok --draft ::"));
        let (model_lines, note) = variants_from_draft(tweet, Ok(text));
        assert!(note.is_empty());
        assert_distinct(&model_lines);
        let distinct = execute_draft(
            plan_agent_draft(Agent::Muse, Ok(&launch), None, "prompt"),
            &ScriptCli {
                output: Ok("Agree and add one fact.\n---\nWhat would change this?\n---\nThe cost is the part left out.".into()),
                seen: std::sync::Mutex::new(Vec::new()),
            },
            &ScriptHttp { body: Ok(String::new()) },
        )
        .unwrap();
        let (kept, _) = variants_from_draft(tweet, Ok(distinct));
        assert_distinct(&kept);
        assert!(kept[0].contains("Agree"));
        assert!(kept[1].contains("change"));
        let (fallback, notice) =
            variants_from_draft(tweet, Err("http 402: payment required".into()));
        assert!(notice.contains("402"));
        assert_distinct(&fallback);
        assert!(
            fallback
                .iter()
                .all(|line| line.contains("Courts") || line.contains("models"))
        );
    }

    #[test]
    fn the_pane_can_take_typed_text_without_posting() {
        let mut pane = AgentPane::open(Agent::Hermes, "hermes");
        pane.push_str("a sharp reply");
        assert_eq!(pane.use_as_draft(), "a sharp reply");
        assert_eq!(pane.submit(), "a sharp reply");
        pane.append_output("Agree, and the docket is the real change.");
        assert_eq!(
            pane.use_as_draft(),
            "Agree, and the docket is the real change."
        );
        let missing = AgentPane::missing(Agent::Muse, &missing_line(Agent::Muse, "muse"));
        assert!(missing.notice.contains("missing"));
        assert!(
            !missing.scrollback.contains("https://x.com") || missing.notice.contains("missing")
        );
    }

    #[test]
    fn hermes_chat_uses_the_gateway_and_a_fake_completion() {
        let auth = hermes_auth(HERMES_BASE, HERMES_MODEL, "", "API_SERVER_KEY=env-key");
        assert_eq!(auth.key, "env-key");
        let prompt = reply_prompt("ada", "hello", "Plain");
        let route = plan_agent_draft(Agent::Hermes, Err("cli missing"), Some(&auth), &prompt);
        let cli = ScriptCli {
            output: Err("should not run".into()),
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let body = r#"{"choices":[{"message":{"content":"one\n---\ntwo\n---\nthree"}}]}"#;
        let text = execute_draft(
            route,
            &cli,
            &ScriptHttp {
                body: Ok(body.into()),
            },
        )
        .unwrap();
        assert!(cli.seen.lock().unwrap().is_empty());
        let (lines, note) = variants_from_draft("hello", Ok(text));
        assert!(note.is_empty());
        assert_eq!(
            lines,
            vec!["one".to_string(), "two".to_string(), "three".to_string()]
        );
        let empty = plan_hermes_draft(&hermes_auth("", "", "", ""), "hi").unwrap_err();
        assert!(empty.contains("Hermes key is missing"));
        assert!(!empty.contains("env-key"));
    }
}
