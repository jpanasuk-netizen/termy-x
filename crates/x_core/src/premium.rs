//! Resolve the post character limit: Premium long-form (25k) or standard (280).
//!
//! Order: `TERMY_X_CHAR_LIMIT` env, then `char_limit` in x.toml, then a short
//! OpenCLI profile probe (`verified` = Premium / Premium+). Detection is
//! cached on disk for a few minutes so compose stays snappy.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::config::XConfig;
use crate::io::CommandRunner;
use crate::text::{PREMIUM_CHAR_LIMIT, STANDARD_CHAR_LIMIT};

const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

static MEMORY: Mutex<Option<CachedLimit>> = Mutex::new(None);

#[derive(Clone, Copy, Debug)]
struct CachedLimit {
    limit: usize,
    source: LimitSource,
    fetched_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitSource {
    Env,
    Config,
    DetectedPremium,
    DetectedStandard,
    Cache,
    Fallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedLimit {
    pub limit: usize,
    pub source: LimitSource,
}

pub fn resolve_char_limit(config: &XConfig, runner: &dyn CommandRunner) -> ResolvedLimit {
    if let Some(limit) = config.env_char_limit() {
        return remember(
            config,
            ResolvedLimit {
                limit,
                source: LimitSource::Env,
            },
        );
    }
    if let Some(limit) = config.explicit_char_limit() {
        return remember(
            config,
            ResolvedLimit {
                limit,
                source: LimitSource::Config,
            },
        );
    }
    if let Some(cached) = read_memory_fresh() {
        return ResolvedLimit {
            limit: cached.limit,
            source: LimitSource::Cache,
        };
    }
    if let Some(cached) = read_disk_fresh(&config.config_dir) {
        write_memory(cached);
        return ResolvedLimit {
            limit: cached.limit,
            source: LimitSource::Cache,
        };
    }
    match detect_via_opencli(config, runner) {
        Ok(true) => remember(
            config,
            ResolvedLimit {
                limit: PREMIUM_CHAR_LIMIT,
                source: LimitSource::DetectedPremium,
            },
        ),
        Ok(false) => remember(
            config,
            ResolvedLimit {
                limit: STANDARD_CHAR_LIMIT,
                source: LimitSource::DetectedStandard,
            },
        ),
        Err(_) => {
            // Prefer Jeremy's Premium install default when detection cannot run.
            let limit = config.fallback_char_limit();
            remember(
                config,
                ResolvedLimit {
                    limit,
                    source: LimitSource::Fallback,
                },
            )
        }
    }
}

/// Sync helper for UI / CLI when a runner is available as Arc.
pub fn resolve_char_limit_simple(config: &XConfig) -> usize {
    if let Some(limit) = config
        .env_char_limit()
        .or_else(|| config.explicit_char_limit())
    {
        return limit;
    }
    if let Some(cached) = read_memory_fresh().or_else(|| read_disk_fresh(&config.config_dir)) {
        return cached.limit;
    }
    config.fallback_char_limit()
}

fn detect_via_opencli(config: &XConfig, runner: &dyn CommandRunner) -> Result<bool, String> {
    let who = runner.run(
        &config.opencli_bin,
        &[
            "twitter".into(),
            "whoami".into(),
            "-f".into(),
            "json".into(),
            "--window".into(),
            "background".into(),
        ],
    )?;
    if who.status != 0 {
        return Err(who.stderr);
    }
    let username = extract_username(&who.stdout).ok_or_else(|| "no username".to_string())?;
    let profile = runner.run(
        &config.opencli_bin,
        &[
            "twitter".into(),
            "profile".into(),
            username,
            "-f".into(),
            "json".into(),
            "--window".into(),
            "background".into(),
        ],
    )?;
    if profile.status != 0 {
        return Err(profile.stderr);
    }
    Ok(profile_looks_premium(&profile.stdout))
}

pub fn profile_looks_premium(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let node = profile_node(&value);
    boolish(
        node,
        &[
            "verified",
            "is_blue_verified",
            "isBlueVerified",
            "premium",
            "is_premium",
        ],
    ) || string_premium(
        node,
        &["subscription", "subscription_type", "verified_type"],
    )
}

fn profile_node(value: &Value) -> &Value {
    if let Some(array) = value.as_array()
        && let Some(first) = array.first()
    {
        return first;
    }
    value
        .get("data")
        .or_else(|| value.get("user"))
        .unwrap_or(value)
}

fn extract_username(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let node = if let Some(array) = value.as_array() {
        array.first()?
    } else {
        &value
    };
    first_str(node, &["username", "screen_name", "screenName", "handle"])
        .map(|s| s.trim_start_matches('@').to_string())
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}

fn boolish(value: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| match value.get(*key) {
        Some(Value::Bool(true)) => true,
        Some(Value::String(s)) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on" | "premium" | "blue"
        ),
        Some(Value::Number(n)) => n.as_u64() == Some(1),
        _ => false,
    })
}

fn string_premium(value: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        value.get(*key).and_then(Value::as_str).is_some_and(|s| {
            let lower = s.to_ascii_lowercase();
            lower.contains("premium") || lower.contains("blue") || lower == "verified"
        })
    })
}

fn remember(config: &XConfig, resolved: ResolvedLimit) -> ResolvedLimit {
    let cached = CachedLimit {
        limit: resolved.limit,
        source: resolved.source,
        fetched_unix: now_unix(),
    };
    write_memory(cached);
    let _ = write_disk(&config.config_dir, cached);
    resolved
}

fn write_memory(cached: CachedLimit) {
    if let Ok(mut guard) = MEMORY.lock() {
        *guard = Some(cached);
    }
}

fn read_memory_fresh() -> Option<CachedLimit> {
    let guard = MEMORY.lock().ok()?;
    let cached = (*guard)?;
    if age_secs(cached.fetched_unix) <= CACHE_TTL.as_secs() {
        Some(cached)
    } else {
        None
    }
}

fn cache_path(dir: &Path) -> std::path::PathBuf {
    dir.join("x-char-limit.cache")
}

fn write_disk(dir: &Path, cached: CachedLimit) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let body = format!(
        "{}\n{}\n{}\n",
        cached.limit,
        cached.fetched_unix,
        match cached.source {
            LimitSource::DetectedPremium => "premium",
            LimitSource::DetectedStandard => "standard",
            LimitSource::Env => "env",
            LimitSource::Config => "config",
            LimitSource::Cache => "cache",
            LimitSource::Fallback => "fallback",
        }
    );
    fs::write(cache_path(dir), body).map_err(|e| e.to_string())
}

fn read_disk_fresh(dir: &Path) -> Option<CachedLimit> {
    let text = fs::read_to_string(cache_path(dir)).ok()?;
    let mut lines = text.lines();
    let limit: usize = lines.next()?.trim().parse().ok()?;
    let fetched_unix: u64 = lines.next()?.trim().parse().ok()?;
    if age_secs(fetched_unix) > CACHE_TTL.as_secs() {
        return None;
    }
    Some(CachedLimit {
        limit,
        source: LimitSource::Cache,
        fetched_unix,
    })
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn age_secs(fetched_unix: u64) -> u64 {
    now_unix().saturating_sub(fetched_unix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_profile_json_counts_as_premium() {
        let text = r#"[{"screen_name":"Jasper_Black","verified":true}]"#;
        assert!(profile_looks_premium(text));
        assert!(!profile_looks_premium(r#"{"verified":false}"#));
        assert!(profile_looks_premium(
            r#"{"user":{"username":"a","subscription_type":"Premium+"}}"#
        ));
    }
}
