use crate::text::{PREMIUM_CHAR_LIMIT, STANDARD_CHAR_LIMIT};

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ProviderId;
use crate::sort::ResultSort;

/// Bird opacity behind content: dimmed into the 15-25% band by default.
pub const DEFAULT_BACKGROUND_OPACITY: f32 = 0.18;
/// Free default for AI drafts: a local Ollama server (no key, no cost).
pub const DEFAULT_AI_BASE_URL: &str = "http://127.0.0.1:11434/v1";
pub const DEFAULT_AI_MODEL: &str = "llama3.2";

#[derive(Debug, Clone, PartialEq)]
pub struct XConfig {
    pub background_art: bool,
    /// Opacity of the bird behind lists (0.0-1.0). The empty/splash state
    /// always shows it at full strength.
    pub background_opacity: f32,
    pub default_limit: usize,
    /// Explicit post character limit. `None` means auto-detect Premium via
    /// OpenCLI (or fall back to [`Self::fallback_char_limit`]).
    pub char_limit: Option<usize>,
    pub provider: Option<ProviderId>,
    pub official_enabled: bool,
    pub client_id_env: String,
    pub bearer_token_env: String,
    pub redirect_uri: String,
    pub token_store: String,
    pub ai_base_url_env: String,
    pub ai_model_env: String,
    pub ai_key_env: String,
    pub ai_base_url: String,
    pub ai_model: String,
    /// Optional key from x.toml. Env vars still win. Never logged.
    pub ai_api_key: String,
    /// Set when a config file or env var chose the endpoint. The free local
    /// default stays implicit so a dead Ollama falls back to templates.
    pub ai_explicit: bool,
    pub min_interval_ms: u64,
    /// Per-run cap for thread "like all" / "follow all" (clamped 25..=50).
    pub bulk_cap: usize,
    /// Last Popular/Newest choice, restored on launch.
    pub list_sort: ResultSort,
    pub post_read_usd: f64,
    pub user_read_usd: f64,
    pub post_write_usd: f64,
    pub opencli_bin: String,
    pub twitter_bin: String,
    pub agent_reach_bin: String,
    pub youtube_bin: String,
    /// Ask here: per-agent launch commands (x.toml `[agents]`). `None` = auto-detect.
    pub agent_agy: Option<String>,
    pub agent_hermes: Option<String>,
    pub agent_grok: Option<String>,
    pub agent_muse: Option<String>,
    /// Hermes gateway. Empty means the Windows default
    /// `http://127.0.0.1:8642/v1`. The key is written only when the user saves it.
    pub hermes_base_url: String,
    pub hermes_model: String,
    pub hermes_api_key: String,
    /// `free`, `treg`, or `hermes`. Empty means the desk settings file decides.
    pub take_provider: String,
    /// WSL distro/user tried when an agent is not on the Windows PATH.
    pub wsl_distro: String,
    pub wsl_user: String,
    pub config_dir: PathBuf,
    /// Optional Supabase project URL. Empty means sync is off. Not a secret.
    pub supabase_url: String,
    /// Email used for optional Supabase Auth. Not a secret. The password is never stored.
    pub supabase_email: String,
}

impl Default for XConfig {
    fn default() -> Self {
        Self {
            background_art: true,
            background_opacity: DEFAULT_BACKGROUND_OPACITY,
            default_limit: 10,
            // Auto: detect Premium from the OpenCLI session. Jeremy's install
            // also writes char_limit=25000 into x.toml as a hard override.
            char_limit: None,
            provider: None,
            official_enabled: false,
            client_id_env: "TERMY_X_CLIENT_ID".to_string(),
            bearer_token_env: "TERMY_X_BEARER_TOKEN".to_string(),
            redirect_uri: "http://127.0.0.1:8737/callback".to_string(),
            token_store: "keychain".to_string(),
            ai_base_url_env: "TERMY_X_AI_BASE_URL".to_string(),
            ai_model_env: "TERMY_X_AI_MODEL".to_string(),
            ai_key_env: "TERMY_X_AI_API_KEY".to_string(),
            ai_base_url: String::new(),
            ai_model: String::new(),
            ai_api_key: String::new(),
            ai_explicit: false,
            min_interval_ms: 1500,
            bulk_cap: crate::bulk::DEFAULT_BULK_CAP,
            list_sort: ResultSort::Popular,
            post_read_usd: 0.005,
            user_read_usd: 0.010,
            post_write_usd: 0.010,
            opencli_bin: "opencli".to_string(),
            twitter_bin: "twitter".to_string(),
            agent_reach_bin: "agent-reach".to_string(),
            youtube_bin: "yt-dlp".to_string(),
            agent_agy: None,
            agent_hermes: None,
            agent_grok: None,
            agent_muse: None,
            hermes_base_url: String::new(),
            hermes_model: String::new(),
            hermes_api_key: String::new(),
            take_provider: String::new(),
            wsl_distro: "Ubuntu".to_string(),
            wsl_user: String::new(),
            config_dir: default_config_dir(),
            supabase_url: String::new(),
            supabase_email: String::new(),
        }
    }
}

impl XConfig {
    pub fn load(dir: &Path) -> Self {
        let mut config = Self {
            config_dir: dir.to_path_buf(),
            ..Self::default()
        };
        let path = dir.join("x.toml");
        if let Ok(text) = fs::read_to_string(path) {
            config.apply_text(&text);
        }
        config.resolve_env();
        config
    }

    pub fn apply_text(&mut self, text: &str) {
        let mut section = String::new();
        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim().to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = unquote(value.trim());
            let full = if section.is_empty() {
                key.to_string()
            } else {
                format!("{section}.{key}")
            };
            self.set_key(&full, &value);
        }
    }

    pub fn set_key(&mut self, key: &str, value: &str) {
        match key {
            "background_art" => self.background_art = parse_bool(value),
            "background_opacity" => {
                if let Ok(opacity) = value.parse::<f32>() {
                    self.background_opacity = opacity.clamp(0.0, 1.0);
                }
            }
            "default_limit" => {
                if let Ok(limit) = value.parse::<usize>() {
                    self.default_limit = limit.clamp(1, 100);
                }
            }
            "char_limit" => {
                self.char_limit = parse_char_limit(value);
            }
            "provider" => {
                self.provider = if value.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    ProviderId::parse(value)
                };
            }
            "official_api.enabled" => self.official_enabled = parse_bool(value),
            "official_api.client_id_env" => self.client_id_env = value.to_string(),
            "official_api.bearer_token_env" => self.bearer_token_env = value.to_string(),
            "official_api.redirect_uri" => self.redirect_uri = value.to_string(),
            "official_api.token_store" => self.token_store = value.to_string(),
            "official_api.post_read_usd" | "official_api.prices_usd.post_read" => {
                if let Ok(price) = value.parse() {
                    self.post_read_usd = price;
                }
            }
            "official_api.user_read_usd" | "official_api.prices_usd.user_read" => {
                if let Ok(price) = value.parse() {
                    self.user_read_usd = price;
                }
            }
            "official_api.post_write_usd" | "official_api.prices_usd.post_write" => {
                if let Ok(price) = value.parse() {
                    self.post_write_usd = price;
                }
            }
            "ai.base_url_env" => self.ai_base_url_env = value.to_string(),
            "ai.model_env" => self.ai_model_env = value.to_string(),
            "ai.api_key_env" => self.ai_key_env = value.to_string(),
            "ai.base_url" => {
                self.ai_base_url = value.to_string();
                self.ai_explicit = true;
            }
            "ai.model" => self.ai_model = value.to_string(),
            "ai.api_key" => self.ai_api_key = value.to_string(),
            // `ai.preset = "xai"` (or "grok"): Grok via xAI's OpenAI-compatible API.
            "ai.preset" if matches!(value.trim(), "xai" | "grok") => {
                self.ai_base_url = crate::reply_guy::XAI_BASE_URL.to_string();
                self.ai_explicit = true;
                if self.ai_model.trim().is_empty() {
                    self.ai_model = crate::reply_guy::DEFAULT_GROK_MODEL.to_string();
                }
            }
            "shell.min_interval_ms" => {
                if let Ok(ms) = value.parse() {
                    self.min_interval_ms = ms;
                }
            }
            "bulk_cap" | "thread.bulk_cap" => {
                if let Ok(n) = value.parse::<usize>() {
                    self.bulk_cap = crate::bulk::clamp_bulk_cap(n);
                }
            }
            "list_sort" | "ui.list_sort" => {
                if let Some(sort) = ResultSort::parse(value) {
                    self.list_sort = sort;
                }
            }
            "shell.opencli_bin" => self.opencli_bin = value.to_string(),
            "shell.twitter_bin" => self.twitter_bin = value.to_string(),
            "shell.agent_reach_bin" => self.agent_reach_bin = value.to_string(),
            "research.youtube_bin" => self.youtube_bin = value.to_string(),
            "agents.agy" | "agent_agy" => self.agent_agy = nonempty_owned(value),
            "agents.hermes" | "agent_hermes" => self.agent_hermes = nonempty_owned(value),
            "agents.grok" | "agent_grok" => self.agent_grok = nonempty_owned(value),
            "agents.muse" | "agent_muse" => self.agent_muse = nonempty_owned(value),
            "agents.hermes_base_url" => self.hermes_base_url = value.to_string(),
            "agents.hermes_model" => self.hermes_model = value.to_string(),
            "agents.hermes_api_key" => self.hermes_api_key = value.to_string(),
            "desk.provider" | "takes.provider" => self.take_provider = value.to_string(),
            "agents.wsl_distro" => self.wsl_distro = value.to_string(),
            "agents.wsl_user" => self.wsl_user = value.to_string(),
            "supabase_url" | "supabase.url" => self.supabase_url = value.to_string(),
            "supabase_email" | "supabase.email" => self.supabase_email = value.to_string(),
            _ => {}
        }
    }

    pub fn resolve_env(&mut self) {
        let termy_base = nonempty_var(&self.ai_base_url_env);
        let openai_base = nonempty_var("OPENAI_BASE_URL");
        let termy_model = nonempty_var(&self.ai_model_env);
        self.apply_ai_sources(
            termy_base.as_deref(),
            openai_base.as_deref(),
            termy_model.as_deref(),
        );
        if let Some(limit) = nonempty_var("TERMY_X_CHAR_LIMIT").and_then(|v| parse_char_limit(&v)) {
            self.char_limit = Some(limit);
        }
    }

    /// `TERMY_X_CHAR_LIMIT` when set (also folded into `char_limit` by resolve_env).
    pub fn env_char_limit(&self) -> Option<usize> {
        nonempty_var("TERMY_X_CHAR_LIMIT").and_then(|v| parse_char_limit(&v))
    }

    /// Explicit `char_limit` from x.toml (or env already applied).
    pub fn explicit_char_limit(&self) -> Option<usize> {
        self.char_limit
    }

    /// Used when OpenCLI detection cannot run and no override is set.
    pub fn fallback_char_limit(&self) -> usize {
        self.char_limit.unwrap_or(STANDARD_CHAR_LIMIT)
    }

    /// Best-effort limit without spawning OpenCLI (env/config/cache/fallback).
    pub fn char_limit_or_fallback(&self) -> usize {
        crate::premium::resolve_char_limit_simple(self)
    }

    /// `TERMY_X_AI_BASE_URL` wins, then `OPENAI_BASE_URL`.
    pub fn apply_ai_sources(
        &mut self,
        termy_base: Option<&str>,
        openai_base: Option<&str>,
        termy_model: Option<&str>,
    ) {
        if let Some(value) = nonempty(termy_base).or_else(|| nonempty(openai_base)) {
            self.ai_base_url = value.to_string();
            self.ai_explicit = true;
        }
        if let Some(value) = nonempty(termy_model) {
            self.ai_model = value.to_string();
        }
    }

    /// True when the user pointed drafting at an endpoint explicitly.
    pub fn ai_configured(&self) -> bool {
        !self.ai_base_url.trim().is_empty()
    }

    /// Endpoint used for AI drafts: the configured one, else the free local
    /// default (Ollama's OpenAI-compatible API on 127.0.0.1:11434).
    pub fn ai_endpoint(&self) -> (String, String) {
        let base = if self.ai_base_url.trim().is_empty() {
            DEFAULT_AI_BASE_URL.to_string()
        } else {
            self.ai_base_url.trim().to_string()
        };
        let model = if self.ai_model.trim().is_empty() {
            DEFAULT_AI_MODEL.to_string()
        } else {
            self.ai_model.trim().to_string()
        };
        (base, model)
    }

    pub fn api_key(&self) -> Option<String> {
        nonempty_var(&self.ai_key_env)
            .or_else(|| nonempty_owned(&self.ai_api_key))
            .or_else(|| {
                if self.ai_base_url.contains("api.x.ai") {
                    crate::reply_guy::load_secret(self, crate::reply_guy::Secret::Grok)
                } else {
                    nonempty_var("OPENAI_API_KEY")
                }
            })
    }

    /// The user's own X developer bearer token, from their shell env only.
    /// Never read from x.toml, never logged.
    pub fn bearer_token(&self) -> Option<String> {
        nonempty_var(&self.bearer_token_env)
    }

    pub fn client_id(&self) -> Option<String> {
        std::env::var(&self.client_id_env)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    /// x.toml override for an Ask here agent, if any.
    pub fn agent_command(&self, agent: crate::agents::Agent) -> Option<&str> {
        use crate::agents::Agent;
        match agent {
            Agent::Agy => self.agent_agy.as_deref(),
            Agent::Hermes => self.agent_hermes.as_deref(),
            Agent::Grok => self.agent_grok.as_deref(),
            Agent::Muse => self.agent_muse.as_deref(),
        }
    }

    pub fn save_background_art(&self) -> Result<(), String> {
        self.upsert_config_assignment(
            "background_art",
            &format!("background_art = {}", self.background_art),
        )
    }

    pub fn save_supabase_url(&self, url: &str) -> Result<(), String> {
        self.upsert_config_assignment("supabase_url", &format!("supabase_url = \"{url}\""))
    }

    pub fn save_supabase_email(&self, email: &str) -> Result<(), String> {
        self.upsert_config_assignment("supabase_email", &format!("supabase_email = \"{email}\""))
    }

    /// Writes the free-model endpoint into this config directory only.
    pub fn save_ai(&self, base: &str, model: &str, key: &str) -> Result<(), String> {
        let base = toml_string(base);
        let model = toml_string(model);
        let key = toml_string(key);
        self.upsert_config_assignment("ai.base_url", &format!("ai.base_url = {base}"))?;
        self.upsert_config_assignment("ai.model", &format!("ai.model = {model}"))?;
        if !key.trim_matches('"').is_empty() {
            self.upsert_config_assignment("ai.api_key", &format!("ai.api_key = {key}"))?;
        }
        Ok(())
    }

    /// Writes the takes provider into this config directory only.
    pub fn save_take_provider(&self, provider: &str) -> Result<(), String> {
        let provider = provider.trim();
        fs::create_dir_all(&self.config_dir).map_err(|error| error.to_string())?;
        let path = self.config_dir.join("x.toml");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let text = upsert_root_assignment(
            &text,
            "desk.provider",
            &format!("desk.provider = \"{provider}\""),
        );
        fs::write(path, text).map_err(|error| error.to_string())
    }

    pub fn save_list_sort(&self) -> Result<(), String> {
        self.upsert_config_assignment(
            "list_sort",
            &format!("list_sort = \"{}\"", self.list_sort.as_str()),
        )
    }

    fn upsert_config_assignment(&self, key: &str, line: &str) -> Result<(), String> {
        fs::create_dir_all(&self.config_dir).map_err(|error| error.to_string())?;
        let path = self.config_dir.join("x.toml");
        let mut text = if path.exists() {
            fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };
        upsert_assignment(&mut text, key, line);
        fs::write(path, text).map_err(|error| error.to_string())
    }

    /// Files this config is allowed to create or rewrite.
    pub fn writable_paths(&self) -> Vec<PathBuf> {
        vec![
            self.config_dir.join("x.toml"),
            self.drafts_path(),
            self.inbox_path(),
            self.feeds_path(),
            self.followers_path(),
            self.queue_path(),
            self.token_path(),
            self.action_state_path(),
        ]
    }

    pub fn drafts_path(&self) -> PathBuf {
        self.config_dir.join("x-drafts.json")
    }

    pub fn inbox_path(&self) -> PathBuf {
        self.config_dir.join("x-inbox.json")
    }

    pub fn feeds_path(&self) -> PathBuf {
        self.config_dir.join("x-feeds.json")
    }

    pub fn followers_path(&self) -> PathBuf {
        self.config_dir.join("x-followers.json")
    }

    pub fn queue_path(&self) -> PathBuf {
        self.config_dir.join("x-reply-queue.json")
    }

    pub fn token_path(&self) -> PathBuf {
        self.config_dir.join("x-tokens.json")
    }

    /// Likes, follows, and post receipts for the signed-in account.
    /// Not tokens and not the VLC bundle.
    pub fn action_state_path(&self) -> PathBuf {
        self.config_dir.join(crate::action_state::FILE_NAME)
    }
}

pub fn default_config_dir() -> PathBuf {
    // Primary source is `dirs` on every platform: %APPDATA% on Windows,
    // $XDG_CONFIG_HOME (or ~/.config) on Linux, ~/Library on macOS.
    // Only when that finds nothing do we use the explicit cfg'd
    // fallback in `platform`, instead of writing config into the CWD.
    dirs::config_dir()
        .or_else(crate::platform::config_base_fallback)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("termy")
}

fn nonempty_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_char_limit(value: &str) -> Option<usize> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }
    if trimmed.eq_ignore_ascii_case("premium")
        || trimmed.eq_ignore_ascii_case("premium+")
        || trimmed.eq_ignore_ascii_case("long")
    {
        return Some(PREMIUM_CHAR_LIMIT);
    }
    if trimmed.eq_ignore_ascii_case("standard")
        || trimmed.eq_ignore_ascii_case("free")
        || trimmed.eq_ignore_ascii_case("basic")
    {
        return Some(STANDARD_CHAR_LIMIT);
    }
    trimmed
        .parse::<usize>()
        .ok()
        .map(|n| n.clamp(1, PREMIUM_CHAR_LIMIT))
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn toml_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value.trim().replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn nonempty_owned(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Keep a root key out of a later `[section]`.
fn upsert_root_assignment(text: &str, key: &str, line: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    let mut in_section = false;
    for raw in text.lines() {
        let code = raw.split('#').next().unwrap_or("").trim();
        if code.starts_with('[') && code.ends_with(']') {
            in_section = true;
        }
        let matches = !in_section
            && code
                .split_once('=')
                .is_some_and(|(left, _)| left.trim() == key);
        if matches {
            out.push_str(line);
            found = true;
        } else {
            out.push_str(raw);
        }
        out.push('\n');
    }
    if found {
        return out;
    }
    let mut prefixed = format!("{line}\n");
    prefixed.push_str(&out);
    prefixed
}

fn upsert_assignment(text: &mut String, key: &str, line: &str) {
    let mut out = String::new();
    let mut found = false;
    for raw in text.lines() {
        let code = raw.split('#').next().unwrap_or("").trim();
        let matches = code
            .split_once('=')
            .is_some_and(|(left, _)| left.trim() == key);
        if matches {
            out.push_str(line);
            found = true;
        } else {
            out.push_str(raw);
        }
        out.push('\n');
    }
    if !found {
        if !text.is_empty() && !text.ends_with('\n') && out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    *text = out;
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

pub fn example_config() -> &'static str {
    r#"# Termy X. Free paths are the default. No X API credits are required.
background_art = true       # set false to hide the bird entirely
background_opacity = 0.18   # bird strength behind lists (splash is always 1.0)
default_limit = 10
# Post length: 280 (standard), 25000 (Premium), or "auto" to detect from OpenCLI.
char_limit = 25000
provider = "auto"

[official_api]
enabled = false
client_id_env = "TERMY_X_CLIENT_ID"
redirect_uri = "http://127.0.0.1:8737/callback"
token_store = "keychain"
# Your own developer bearer token: set TERMY_X_BEARER_TOKEN in your shell env only.
# Never paste a real token into this file.
post_read_usd = 0.005
user_read_usd = 0.010
post_write_usd = 0.010

[ai]
# Any OpenAI-compatible /chat/completions endpoint. Env vars win over these.
# Free default when nothing is set: local Ollama at http://127.0.0.1:11434/v1
# (model llama3.2). If it is not running, drafts fall back to offline templates.
# Other free options (see README): LM Studio (http://127.0.0.1:1234/v1),
# OpenRouter ":free" models, Groq's free tier, llama.cpp server.
# OPENAI_BASE_URL and OPENAI_API_KEY are also accepted.
base_url_env = "TERMY_X_AI_BASE_URL"
model_env = "TERMY_X_AI_MODEL"
api_key_env = "TERMY_X_AI_API_KEY"

# Popular (likes, then engagement) or Newest (created_at desc). Restored on launch.
list_sort = "popular"

[thread]
# Like-all / follow-all cap per run. Clamped to 25..=50. Default 30.
bulk_cap = 30

[shell]
min_interval_ms = 1500
opencli_bin = "opencli"
twitter_bin = "twitter"
agent_reach_bin = "agent-reach"

# Optional. Leave empty to keep action state only on this machine.
# supabase_url = "https://YOUR_PROJECT.supabase.co"
# supabase_email = "you@example.com"
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_the_official_api_off_and_the_bird_on() {
        let config = XConfig::default();
        assert!(!config.official_enabled);
        assert!(config.background_art);
        assert_eq!(config.default_limit, 10);
        assert!(config.provider.is_none());
        assert!(!config.ai_configured());
        assert!(!config.ai_explicit);
        assert!((config.background_opacity - 0.18).abs() < f32::EPSILON);
        assert_eq!(config.ai_endpoint().0, DEFAULT_AI_BASE_URL);
        assert_eq!(config.ai_endpoint().1, DEFAULT_AI_MODEL);
    }

    #[test]
    fn openai_base_url_is_accepted_when_termy_url_is_unset() {
        let mut config = XConfig::default();
        config.apply_ai_sources(None, Some("http://127.0.0.1:1234/v1"), None);
        assert!(config.ai_explicit);
        assert_eq!(config.ai_endpoint().0, "http://127.0.0.1:1234/v1");
        assert_eq!(config.ai_endpoint().1, DEFAULT_AI_MODEL);
        config.apply_ai_sources(
            Some("http://127.0.0.1:11434/v1"),
            Some("http://127.0.0.1:1234/v1"),
            Some("llama3.2"),
        );
        assert_eq!(config.ai_endpoint().0, "http://127.0.0.1:11434/v1");
        assert_eq!(config.ai_endpoint().1, "llama3.2");
    }

    #[test]
    fn background_opacity_is_parsed_and_clamped() {
        let mut config = XConfig::default();
        config.apply_text("background_opacity = 0.15");
        assert!((config.background_opacity - 0.15).abs() < 1e-6);
        config.apply_text("background_opacity = 4");
        assert!((config.background_opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn config_text_can_disable_the_background_and_enable_the_api() {
        let mut config = XConfig::default();
        config.apply_text(
            r#"
            background_art = false
            provider = "mock"
            [official_api]
            enabled = true
            "#,
        );
        assert!(!config.background_art);
        assert_eq!(config.provider, Some(ProviderId::Mock));
        assert!(config.official_enabled);
    }

    #[test]
    fn char_limit_parses_premium_aliases() {
        assert_eq!(parse_char_limit("25000"), Some(PREMIUM_CHAR_LIMIT));
        assert_eq!(parse_char_limit("premium"), Some(PREMIUM_CHAR_LIMIT));
        assert_eq!(parse_char_limit("280"), Some(STANDARD_CHAR_LIMIT));
        assert_eq!(parse_char_limit("standard"), Some(STANDARD_CHAR_LIMIT));
        assert_eq!(parse_char_limit("auto"), None);
        let mut config = XConfig::default();
        config.apply_text("char_limit = 25000\n");
        assert_eq!(config.char_limit, Some(PREMIUM_CHAR_LIMIT));
        config.apply_text("char_limit = auto\n");
        assert_eq!(config.char_limit, None);
    }

    #[test]
    fn bulk_cap_parses_and_clamps_to_25_50() {
        let mut config = XConfig::default();
        assert_eq!(config.bulk_cap, crate::bulk::DEFAULT_BULK_CAP);
        config.apply_text("thread.bulk_cap = 40\n");
        assert_eq!(config.bulk_cap, 40);
        config.apply_text("bulk_cap = 10\n");
        assert_eq!(config.bulk_cap, 25);
        config.apply_text("[thread]\nbulk_cap = 99\n");
        assert_eq!(config.bulk_cap, 50);
    }

    #[test]
    fn list_sort_parses_and_round_trips_through_x_toml() {
        let mut config = XConfig::default();
        assert_eq!(config.list_sort, ResultSort::Popular);
        config.apply_text("list_sort = \"newest\"\n");
        assert_eq!(config.list_sort, ResultSort::Newest);
        config.apply_text("ui.list_sort = top\n");
        assert_eq!(config.list_sort, ResultSort::Popular);
        let dir = tempfile::tempdir().expect("temp dir");
        config.config_dir = dir.path().to_path_buf();
        config.list_sort = ResultSort::Newest;
        config.save_list_sort().expect("write list_sort");
        let loaded = XConfig::load(dir.path());
        assert_eq!(loaded.list_sort, ResultSort::Newest);
        config.list_sort = ResultSort::Popular;
        config.save_list_sort().expect("rewrite list_sort");
        let loaded = XConfig::load(dir.path());
        assert_eq!(loaded.list_sort, ResultSort::Popular);
    }

    #[test]
    fn writes_stay_inside_the_given_root() {
        let dir = tempfile::tempdir().unwrap();
        let config = XConfig::load(dir.path());
        let termy = default_config_dir();
        assert_ne!(dir.path(), termy.as_path());
        for path in config.writable_paths() {
            assert!(
                path.starts_with(dir.path()),
                "{} is outside the XL root",
                path.display()
            );
            assert!(
                !path.starts_with(&termy),
                "{} would write the Termy X config",
                path.display()
            );
        }
    }

    #[test]
    fn save_ai_writes_the_xl_file_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let config = XConfig::load(dir.path());
        config
            .save_ai("http://127.0.0.1:11434/v1", "qwen2.5", "secret-key")
            .unwrap();
        let loaded = XConfig::load(dir.path());
        assert_eq!(loaded.ai_base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(loaded.ai_model, "qwen2.5");
        assert_eq!(loaded.ai_api_key, "secret-key");
        assert!(loaded.ai_explicit);
        let text = std::fs::read_to_string(dir.path().join("x.toml")).unwrap();
        assert!(text.contains("ai.base_url"));
        assert!(!text.contains("TERMY"));
    }
}
