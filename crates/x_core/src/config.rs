use crate::text::{PREMIUM_CHAR_LIMIT, STANDARD_CHAR_LIMIT};

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ProviderId;

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
    pub redirect_uri: String,
    pub token_store: String,
    pub ai_base_url_env: String,
    pub ai_model_env: String,
    pub ai_key_env: String,
    pub ai_base_url: String,
    pub ai_model: String,
    /// Set when a config file or env var chose the endpoint. The free local
    /// default stays implicit so a dead Ollama falls back to templates.
    pub ai_explicit: bool,
    pub min_interval_ms: u64,
    pub post_read_usd: f64,
    pub user_read_usd: f64,
    pub post_write_usd: f64,
    pub opencli_bin: String,
    pub twitter_bin: String,
    pub agent_reach_bin: String,
    pub youtube_bin: String,
    pub config_dir: PathBuf,
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
            redirect_uri: "http://127.0.0.1:8737/callback".to_string(),
            token_store: "keychain".to_string(),
            ai_base_url_env: "TERMY_X_AI_BASE_URL".to_string(),
            ai_model_env: "TERMY_X_AI_MODEL".to_string(),
            ai_key_env: "TERMY_X_AI_API_KEY".to_string(),
            ai_base_url: String::new(),
            ai_model: String::new(),
            ai_explicit: false,
            min_interval_ms: 1500,
            post_read_usd: 0.005,
            user_read_usd: 0.010,
            post_write_usd: 0.010,
            opencli_bin: "opencli".to_string(),
            twitter_bin: "twitter".to_string(),
            agent_reach_bin: "agent-reach".to_string(),
            youtube_bin: "yt-dlp".to_string(),
            config_dir: default_config_dir(),
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
            "shell.min_interval_ms" => {
                if let Ok(ms) = value.parse() {
                    self.min_interval_ms = ms;
                }
            }
            "shell.opencli_bin" => self.opencli_bin = value.to_string(),
            "shell.twitter_bin" => self.twitter_bin = value.to_string(),
            "shell.agent_reach_bin" => self.agent_reach_bin = value.to_string(),
            "research.youtube_bin" => self.youtube_bin = value.to_string(),
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
        nonempty_var(&self.ai_key_env).or_else(|| nonempty_var("OPENAI_API_KEY"))
    }

    pub fn client_id(&self) -> Option<String> {
        std::env::var(&self.client_id_env)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn save_background_art(&self) -> Result<(), String> {
        fs::create_dir_all(&self.config_dir).map_err(|error| error.to_string())?;
        let path = self.config_dir.join("x.toml");
        let mut text = if path.exists() {
            fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };
        let line = format!("background_art = {}", self.background_art);
        if let Some(start) = text.find("background_art") {
            let end = text[start..]
                .find('\n')
                .map_or(text.len(), |index| start + index);
            text.replace_range(start..end, &line);
        } else {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&line);
            text.push('\n');
        }
        fs::write(path, text).map_err(|error| error.to_string())
    }

    pub fn drafts_path(&self) -> PathBuf {
        self.config_dir.join("x-drafts.json")
    }

    pub fn token_path(&self) -> PathBuf {
        self.config_dir.join("x-tokens.json")
    }
}

pub fn default_config_dir() -> PathBuf {
    dirs::config_dir()
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

[shell]
min_interval_ms = 1500
opencli_bin = "opencli"
twitter_bin = "twitter"
agent_reach_bin = "agent-reach"
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
}
