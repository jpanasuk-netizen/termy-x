use std::fs;
use std::path::{Path, PathBuf};

use crate::model::ProviderId;

#[derive(Debug, Clone, PartialEq)]
pub struct XConfig {
    pub background_art: bool,
    pub default_limit: usize,
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
            default_limit: 10,
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
            "default_limit" => {
                if let Ok(limit) = value.parse::<usize>() {
                    self.default_limit = limit.clamp(1, 100);
                }
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
            "ai.base_url" => self.ai_base_url = value.to_string(),
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
        if let Ok(value) = std::env::var(&self.ai_base_url_env) {
            if !value.trim().is_empty() {
                self.ai_base_url = value;
            }
        }
        if let Ok(value) = std::env::var(&self.ai_model_env) {
            if !value.trim().is_empty() {
                self.ai_model = value;
            }
        }
    }

    pub fn ai_configured(&self) -> bool {
        !self.ai_base_url.trim().is_empty() && !self.ai_model.trim().is_empty()
    }

    pub fn api_key(&self) -> Option<String> {
        std::env::var(&self.ai_key_env)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
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
                .map(|index| start + index)
                .unwrap_or(text.len());
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
background_art = true
default_limit = 10
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
# Leave empty and set the env vars, or fill a free OpenAI-compatible server.
# Ollama:  base_url = "http://127.0.0.1:11434/v1"  model = "llama3.2"
# Groq:     base_url = "https://api.groq.com/openai/v1"
# OpenRouter free models: base_url = "https://openrouter.ai/api/v1"
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
}
