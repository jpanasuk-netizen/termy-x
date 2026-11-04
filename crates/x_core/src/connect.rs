//! Who is connected, in plain words. One happy path (OpenCLI using the
//! Chrome session the user already logged into), one alternative (the
//! user's own X developer token), and errors that say the next action.
//! Never stores a password and never prints a token.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::auth::{TokenSet, load_access_token, store_token};
use crate::config::XConfig;
use crate::io::{CommandRunner, Http, HttpRequest};
use crate::providers::opencli_read_pub;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connection {
    Connected { handle: String, via: String },
    NotConnected { reason: String, next: String },
}

impl Connection {
    pub fn handle(&self) -> Option<&str> {
        match self {
            Self::Connected { handle, .. } => Some(handle),
            Self::NotConnected { .. } => None,
        }
    }
    /// One-line label for the panel header / CLI.
    pub fn label(&self) -> String {
        match self {
            Self::Connected { handle, via } => format!("Connected as @{handle} · {via}"),
            Self::NotConnected { reason, .. } => format!("Not connected — {reason}"),
        }
    }
}

const NEXT_OPENCLI: &str = "1. Open Chrome.\n2. Go to x.com and log in as yourself (your normal login — Termy X never sees your password).\n3. Press “Connect my X” again.";
const NEXT_NO_OPENCLI: &str = "1. Install OpenCLI: npm install -g @jackwener/opencli\n2. Add its Browser Bridge extension to Chrome (opencli doctor shows the link).\n3. Open Chrome, log in to x.com as yourself, then press “Connect my X” again.";

/// Read-only detection. Tries the user's own developer token (users/me)
/// when one is saved, otherwise ONE OpenCLI `twitter profile` call with no
/// username (defaults to the logged-in user — reliable on OpenCLI 1.8.8
/// and 2.x). Deliberately no `whoami` fallback: see below.
pub fn detect_connection(
    config: &XConfig,
    runner: &dyn CommandRunner,
    http: &dyn Http,
) -> Connection {
    // Path 2 first when a token is actually present: it is unambiguous.
    if let Some(token) = config
        .bearer_token()
        .or_else(|| load_access_token(config).ok())
    {
        if let Ok(body) = http.request(HttpRequest {
            method: "GET",
            url: "https://api.x.com/2/users/me".into(),
            body: None,
            bearer: Some(token),
            content_type: None,
        }) {
            let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            if let Some(handle) = value.pointer("/data/username").and_then(|v| v.as_str()) {
                let handle = handle.trim().trim_start_matches('@').to_string();
                let _ = save_connected_handle(config, &handle);
                return Connection::Connected {
                    handle,
                    via: "your own X developer key".into(),
                };
            }
            // App-only bearer tokens cannot name a person; fall through
            // to OpenCLI instead of declaring the user broken.
        } else {
            // Token did not work; OpenCLI may still be fine. Fall through
            // and, if that also fails, the reason below names the token.
        }
    }

    if !runner.exists(&config.opencli_bin) {
        return Connection::NotConnected {
            reason: "OpenCLI is not installed yet".into(),
            next: NEXT_NO_OPENCLI.into(),
        };
    }

    // ONE probe only. `twitter profile` with no username = the logged-in
    // user, and it runs as an ephemeral background session (see
    // `opencli_read_pub`). The old second probe, `twitter whoami`, is a
    // *persistent* session in OpenCLI's manifest: it forces its tab to stay
    // open on x.com and made Connect visibly cycle x.com pages a second
    // time before parking on about:blank. One call, in-panel progress,
    // done — and a hard 30s ceiling so Connect can never spin forever.
    let profile_args = opencli_read_pub(&["twitter".into(), "profile".into()]);
    let last_detail =
        match runner.run_timeout(&config.opencli_bin, &profile_args, Duration::from_secs(30)) {
            Ok(output) => {
                if output.status == 0
                    && let Ok(handle) = parse_profile_self(&output.stdout)
                {
                    let _ = save_connected_handle(config, &handle);
                    return Connection::Connected {
                        handle,
                        via: "OpenCLI · your Chrome session".into(),
                    };
                }
                format!("{}\n{}", output.stdout, output.stderr)
            }
            Err(error) => error,
        };

    let (reason, next) = classify_opencli_failure(&last_detail, config);
    Connection::NotConnected { reason, next }
}

fn classify_opencli_failure(detail: &str, config: &XConfig) -> (String, String) {
    let lower = detail.to_ascii_lowercase();
    let token_note = if config.bearer_token().is_some() || load_access_token(config).is_ok() {
        "\n(Your saved developer key did not answer either — check it, or just use the Chrome steps above.)"
    } else {
        ""
    };
    if lower.contains("browser bridge")
        || lower.contains("browser_connect")
        || lower.contains("extension is not connected")
        || lower.contains("extension not connected")
        || lower.contains("not connected")
    {
        (
            "Chrome is closed, or the OpenCLI extension is not connected".into(),
            format!("{NEXT_OPENCLI}{token_note}"),
        )
    } else if lower.contains("auth_required")
        || lower.contains("logged-in")
        || lower.contains("log in")
        || lower.contains("logged_in\":false")
    {
        (
            "X is not logged in in Chrome yet".into(),
            format!("{NEXT_OPENCLI}{token_note}"),
        )
    } else if lower.contains("timed out") || lower.contains("timeout") {
        (
            "OpenCLI took too long to answer".into(),
            format!(
                "1. Open Chrome and open x.com once, so X finishes loading.\n2. Leave Chrome open.\n3. Press “Connect my X” again.{token_note}"
            ),
        )
    } else if detail.trim().is_empty() {
        (
            "OpenCLI did not answer".into(),
            format!("{NEXT_OPENCLI}{token_note}"),
        )
    } else {
        (
            "OpenCLI could not confirm your account".into(),
            format!("{NEXT_OPENCLI}{token_note}"),
        )
    }
}

pub fn parse_whoami(text: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| {
        "the account check returned unreadable output. Run `x doctor` and try again.".to_string()
    })?;
    if value.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        let msg = value
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("not logged in");
        return Err(format!(
            "not connected: {msg}. Open X in Chrome, log in as yourself, then run `x connect --check` again."
        ));
    }
    let node = value.get("data").unwrap_or(&value);
    let node = node
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node);
    if node.get("logged_in").and_then(|v| v.as_bool()) == Some(false) {
        return Err("not connected: X says you are logged out. Log in to X in Chrome as yourself, then run `x connect --check` again.".into());
    }
    for key in ["username", "screen_name", "handle"] {
        if let Some(handle) = node.get(key).and_then(|v| v.as_str())
            && !handle.trim().is_empty()
        {
            return Ok(handle.trim().trim_start_matches('@').to_string());
        }
    }
    Err("not connected: could not find your username. Log in to X in Chrome as yourself, then run `x connect --check` again.".into())
}

/// Handle from `opencli twitter profile` (no username): a JSON array or
/// object whose first row carries `screen_name` / `username`.
pub fn parse_profile_self(text: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|_| "profile check returned unreadable output".to_string())?;
    if value.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Err("profile check said not connected".into());
    }
    let node = value.get("data").unwrap_or(&value);
    let node = node
        .as_array()
        .and_then(|rows| rows.first())
        .unwrap_or(node);
    for key in ["screen_name", "username", "handle"] {
        if let Some(handle) = node.get(key).and_then(|v| v.as_str())
            && !handle.trim().is_empty()
        {
            return Ok(handle.trim().trim_start_matches('@').to_string());
        }
    }
    Err("profile check did not return a username".into())
}

pub fn profile_path(config: &XConfig) -> PathBuf {
    config.config_dir.join("x-profile.json")
}

/// Last confirmed handle (not a secret) so the panel can greet the user
/// and default Lookup to their own profile before the first live check.
pub fn save_connected_handle(config: &XConfig, handle: &str) -> Result<(), String> {
    fs::create_dir_all(&config.config_dir).map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "handle": handle.trim().trim_start_matches('@') }).to_string();
    fs::write(profile_path(config), body).map_err(|e| e.to_string())
}

pub fn load_connected_handle(config: &XConfig) -> Option<String> {
    let text = fs::read_to_string(profile_path(config)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("handle")
        .and_then(|v| v.as_str())
        .map(|h| h.trim().trim_start_matches('@').to_string())
        .filter(|h| !h.is_empty())
}

/// Store the user's own developer token in the OS keychain, falling back
/// to a 0600 file (see auth::store_token). Never written to x.toml.
pub fn store_bearer_token(config: &XConfig, token: &str) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("the key was empty — paste your own X developer token and try again".into());
    }
    store_token(
        config,
        &TokenSet {
            access_token: token.to_string(),
            refresh_token: None,
        },
    )
}

/// Forget the saved developer token and the remembered handle. This does
/// not log Chrome out of X — switching the Chrome account is the user's
/// step, and the panel says so.
pub fn disconnect(config: &XConfig) -> Result<(), String> {
    if config.token_store == "keychain"
        && let Ok(entry) = keyring::Entry::new("termy-x", "x-oauth")
    {
        let _ = entry.delete_credential();
    }
    let token_file = config.token_path();
    if token_file.exists() {
        fs::remove_file(&token_file).map_err(|e| e.to_string())?;
    }
    let profile = profile_path(config);
    if profile.exists() {
        fs::remove_file(&profile).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{MapHttp, ScriptedRunner};
    use std::sync::Arc;

    fn test_config(dir: &std::path::Path) -> XConfig {
        XConfig {
            config_dir: dir.to_path_buf(),
            min_interval_ms: 0,
            ..XConfig::default()
        }
    }

    #[test]
    fn profile_self_parses_opencli_18_and_20_shapes() {
        assert_eq!(
            parse_profile_self(r#"[{"screen_name":"ada","name":"Ada"}]"#).unwrap(),
            "ada"
        );
        assert_eq!(
            parse_profile_self(r#"{"ok":true,"data":[{"screen_name":"@ada"}]}"#).unwrap(),
            "ada"
        );
        assert!(parse_profile_self(r#"{"ok":false}"#).is_err());
        assert!(parse_profile_self("nope").is_err());
    }

    #[test]
    fn detects_connected_via_profile_when_whoami_is_broken() {
        // OpenCLI 1.8.8 on Windows: whoami says AUTH_REQUIRED while
        // timeline/profile work. Profile-with-no-name must win.
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "profile"],
            r#"[{"screen_name":"ada","name":"Ada"}]"#,
        );
        let conn = detect_connection(&config, &runner, &MapHttp::default());
        // Exactly one silent probe: no whoami second navigation.
        {
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "{calls:?}");
            assert!(calls[0].1.iter().any(|a| a == "ephemeral"), "{calls:?}");
            assert!(calls[0].1.iter().any(|a| a == "background"), "{calls:?}");
        }
        assert_eq!(
            conn,
            Connection::Connected {
                handle: "ada".into(),
                via: "OpenCLI · your Chrome session".into()
            }
        );
        assert_eq!(load_connected_handle(&config).as_deref(), Some("ada"));
    }

    #[test]
    fn not_connected_says_next_action_when_chrome_is_closed() {
        let dir = tempfile::tempdir().unwrap();
        let config = test_config(dir.path());
        let runner = ScriptedRunner::new();
        runner.fail(
            "opencli",
            &["twitter", "profile"],
            "BROWSER_CONNECT Browser Bridge extension not connected",
        );
        let conn = detect_connection(&config, &runner, &MapHttp::default());
        match conn {
            Connection::NotConnected { reason, next } => {
                assert!(reason.contains("Chrome is closed"), "{reason}");
                assert!(next.contains("Open Chrome"), "{next}");
                assert!(next.contains("Connect my X"), "{next}");
            }
            other => panic!("expected NotConnected, got {other:?}"),
        }
    }

    #[test]
    fn bearer_token_stores_and_disconnect_forgets() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = test_config(dir.path());
        config.token_store = "file".into(); // deterministic in tests
        store_bearer_token(&config, "user-own-token").unwrap();
        assert_eq!(load_access_token(&config).unwrap(), "user-own-token");
        save_connected_handle(&config, "ada").unwrap();
        disconnect(&config).unwrap();
        assert!(load_access_token(&config).is_err());
        assert!(load_connected_handle(&config).is_none());
        let _ = Arc::new(config);
    }
}
