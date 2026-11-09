//! Email sign-in for optional Supabase sync.
//!
//! The password is sent once and never stored. A refresh token stays in
//! the secret store so the next launch can renew the session. This module
//! does not post, like, or follow on X.

use serde_json::{Value, json};

use crate::config::XConfig;
use crate::io::{Http, HttpRequest};
use crate::supabase_defaults::{self, DEFAULT_URL};
use crate::supabase_sync::{self, SyncOutcome, SyncPhase, SyncStatus};

pub const CONFIRM_EMAIL: &str = "Check your email to confirm, then Sign in";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthOutcome {
    SignedIn,
    ConfirmEmail,
    ResetSent,
    SignedOut,
    Message(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CliAction {
    Status,
    Configure { url: String },
    SignIn { email: String },
    SignUp { email: String },
    SignOut,
}

pub fn project_url(config: &XConfig) -> String {
    let custom = config.supabase_url.trim();
    if custom.is_empty() {
        return DEFAULT_URL.to_string();
    }
    supabase_sync::normalize_project_url(custom).unwrap_or_else(|_| DEFAULT_URL.to_string())
}

pub fn anon_key(config: &XConfig) -> Option<String> {
    supabase_sync::saved_anon(config).or_else(|| supabase_defaults::builtin_anon_key().map(str::to_string))
}

pub fn has_session(config: &XConfig) -> bool {
    supabase_sync::saved_access(config).is_some()
}

pub fn password_session(
    http: &dyn Http,
    url: &str,
    anon_key: &str,
    email: &str,
    password: &str,
) -> Result<(String, Option<String>), String> {
    let url = supabase_sync::normalize_project_url(url)?;
    let anon_key = anon_key.trim();
    if anon_key.is_empty() || email.trim().is_empty() || password.is_empty() {
        return Err("Enter an email and a password. Nothing was sent.".into());
    }
    let body = json!({ "email": email.trim(), "password": password }).to_string();
    let response = http
        .request(api_request(
            "POST",
            &format!("{url}/auth/v1/token?grant_type=password"),
            anon_key,
            None,
            Some(body),
        ))
        .map_err(|error| friendly(&error, &[anon_key, password]))?;
    session_tokens(&response, anon_key, password)
}

pub fn sign_up(config: &XConfig, http: &dyn Http, email: &str, password: &str) -> Result<AuthOutcome, String> {
    let (url, anon_key) = endpoint(config)?;
    if email.trim().is_empty() || password.is_empty() {
        return Err("Enter an email and a password. Nothing was sent.".into());
    }
    let body = json!({ "email": email.trim(), "password": password }).to_string();
    let response = http
        .request(api_request(
            "POST",
            &format!("{url}/auth/v1/signup"),
            &anon_key,
            None,
            Some(body),
        ))
        .map_err(|error| friendly(&error, &[&anon_key, password]))?;
    if needs_confirmation(&response) {
        let _ = config.save_supabase_email(email.trim());
        return Ok(AuthOutcome::ConfirmEmail);
    }
    let (access, refresh) = session_tokens(&response, &anon_key, password)?;
    supabase_sync::store_session(config, &access, refresh.as_deref())?;
    let _ = config.save_supabase_email(email.trim());
    Ok(AuthOutcome::SignedIn)
}

pub fn sign_in_store(config: &XConfig, http: &dyn Http, email: &str, password: &str) -> Result<AuthOutcome, String> {
    let (url, anon_key) = endpoint(config)?;
    let (access, refresh) = password_session(http, &url, &anon_key, email, password)?;
    supabase_sync::store_session(config, &access, refresh.as_deref())?;
    let _ = config.save_supabase_email(email.trim());
    Ok(AuthOutcome::SignedIn)
}

pub fn recover(config: &XConfig, http: &dyn Http, email: &str) -> Result<AuthOutcome, String> {
    let (url, anon_key) = endpoint(config)?;
    if email.trim().is_empty() {
        return Err("Enter the email for that account. Nothing was sent.".into());
    }
    let body = json!({ "email": email.trim() }).to_string();
    http.request(api_request(
        "POST",
        &format!("{url}/auth/v1/recover"),
        &anon_key,
        None,
        Some(body),
    ))
    .map_err(|error| friendly(&error, &[&anon_key]))?;
    Ok(AuthOutcome::ResetSent)
}

pub fn sign_out(config: &XConfig, http: &dyn Http) -> AuthOutcome {
    if let (Some(anon_key), Some(access)) = (anon_key(config), supabase_sync::saved_access(config)) {
        let url = project_url(config);
        let _ = http.request(api_request(
            "POST",
            &format!("{url}/auth/v1/logout"),
            &anon_key,
            Some(&access),
            Some("{}".into()),
        ));
    }
    let _ = supabase_sync::clear_session(config);
    AuthOutcome::SignedOut
}

/// Renew the access token when a refresh token is saved. No refresh token is success.
pub fn refresh_session(config: &XConfig, http: &dyn Http) -> Result<(), String> {
    let Some(refresh) = supabase_sync::saved_refresh(config) else {
        return Ok(());
    };
    let (url, anon_key) = endpoint(config)?;
    let body = json!({ "refresh_token": refresh }).to_string();
    let response = http
        .request(api_request(
            "POST",
            &format!("{url}/auth/v1/token?grant_type=refresh_token"),
            &anon_key,
            None,
            Some(body),
        ))
        .map_err(|error| friendly(&error, &[&anon_key, &refresh]))?;
    let (access, next) = session_tokens(&response, &anon_key, &refresh)?;
    supabase_sync::store_session(config, &access, next.as_deref().or(Some(refresh.as_str())))?;
    Ok(())
}

pub fn refresh_and_sync(
    config: &XConfig,
    http: &dyn Http,
    db: &crate::action_state::ActionDb,
    pull: bool,
) -> SyncOutcome {
    if anon_key(config).is_none() || supabase_sync::saved_access(config).is_none() && supabase_sync::saved_refresh(config).is_none()
    {
        return supabase_sync::sync(config, http, db, false);
    }
    if let Err(error) = refresh_session(config, http) {
        let status = SyncStatus {
            phase: SyncPhase::Error,
            at_ms: crate::action_state::now_ms(),
            message: error,
        };
        supabase_sync::record_status(config, &status);
        return SyncOutcome {
            status,
            toggles: Vec::new(),
            receipts: Vec::new(),
        };
    }
    supabase_sync::sync(config, http, db, pull)
}

pub fn cli(action: CliAction, config: &mut XConfig, http: &dyn Http) -> i32 {
    match action {
        CliAction::Status => {
            println!("{}", supabase_sync::settings_summary(config));
            0
        }
        CliAction::Configure { url } => configure(config, &url),
        CliAction::SignIn { email } => {
            let Some(password) = password_from_env() else {
                return 1;
            };
            finish_auth(sign_in_store(config, http, &email, &password))
        }
        CliAction::SignUp { email } => {
            let Some(password) = password_from_env() else {
                return 1;
            };
            finish_auth(sign_up(config, http, &email, &password))
        }
        CliAction::SignOut => {
            let _ = sign_out(config, http);
            println!("Signed out of account sync on this PC. Likes and follows on this PC were kept.");
            0
        }
    }
}

pub fn outcome_text(outcome: &AuthOutcome) -> String {
    match outcome {
        AuthOutcome::SignedIn => "Signed in. Account sync can run on this PC. The password was not stored.".into(),
        AuthOutcome::ConfirmEmail => CONFIRM_EMAIL.into(),
        AuthOutcome::ResetSent => "Password reset email sent. Check your inbox, then Sign in.".into(),
        AuthOutcome::SignedOut => "Signed out of account sync on this PC.".into(),
        AuthOutcome::Message(text) => text.clone(),
    }
}

fn configure(config: &mut XConfig, url: &str) -> i32 {
    if let Err(error) = supabase_sync::save_url(config, url) {
        eprintln!("{error}");
        return 1;
    }
    config.supabase_url = url.trim().trim_end_matches('/').to_string();
    match std::env::var("TERMY_X_SUPABASE_ANON_KEY") {
        Ok(key) if !key.trim().is_empty() => {
            if let Err(error) = supabase_sync::store_anon_key(config, &key) {
                eprintln!("{error}");
                return 1;
            }
            println!("Sync URL saved. Anon key override saved in the local secret store. It was not printed.");
        }
        _ => {
            println!("Sync URL saved. The hosted anon key is used unless TERMY_X_SUPABASE_ANON_KEY is set. Keys are not written to x.toml.");
        }
    }
    0
}

fn password_from_env() -> Option<String> {
    match std::env::var("TERMY_X_SUPABASE_PASSWORD") {
        Ok(password) if !password.is_empty() => Some(password),
        _ => {
            eprintln!("Set TERMY_X_SUPABASE_PASSWORD for this command. The password is not stored and not printed.");
            None
        }
    }
}

fn finish_auth(result: Result<AuthOutcome, String>) -> i32 {
    match result {
        Ok(outcome) => {
            println!("{}", outcome_text(&outcome));
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn endpoint(config: &XConfig) -> Result<(String, String), String> {
    let Some(anon_key) = anon_key(config) else {
        return Err("Account sync is not available in this build. Nothing was sent.".into());
    };
    Ok((project_url(config), anon_key))
}

fn api_request(
    method: &'static str,
    url: &str,
    anon_key: &str,
    bearer: Option<&str>,
    body: Option<String>,
) -> HttpRequest {
    HttpRequest {
        method,
        url: url.to_string(),
        body,
        bearer: Some(bearer.unwrap_or(anon_key).to_string()),
        content_type: Some("application/json".into()),
        headers: vec![("apikey".into(), anon_key.to_string())],
    }
}

fn session_tokens(body: &str, anon_key: &str, secret: &str) -> Result<(String, Option<String>), String> {
    let value: Value = serde_json::from_str(body).map_err(|_| {
        "Account sync did not return a session. Nothing was stored.".to_string()
    })?;
    let access = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "That email or password was not accepted. Nothing was changed.".to_string())?;
    let refresh = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    if access.contains(secret) {
        return Err("Account sync returned an unusable session.".into());
    }
    let _ = anon_key;
    Ok((access.to_string(), refresh))
}

fn needs_confirmation(body: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    if value.get("access_token").and_then(Value::as_str).is_some_and(|token| !token.is_empty()) {
        return false;
    }
    if value
        .pointer("/session/access_token")
        .and_then(Value::as_str)
        .is_some_and(|token| !token.is_empty())
    {
        return false;
    }
    value.get("confirmation_sent_at").is_some()
        || value.pointer("/user/confirmation_sent_at").is_some()
        || value.get("session").is_some_and(Value::is_null)
        || (value.get("id").is_some() && value.get("access_token").is_none())
}

fn friendly(error: &str, secrets: &[&str]) -> String {
    let error = supabase_sync::redact(error, secrets);
    let lower = error.to_ascii_lowercase();
    if lower.contains("invalid login")
        || lower.contains("invalid_grant")
        || lower.contains("invalid_credentials")
        || lower.contains("wrong")
    {
        return "That email or password was not accepted. Nothing was changed.".into();
    }
    if lower.contains("already") && lower.contains("register") {
        return "That email already has an account. Sign in instead.".into();
    }
    if lower.contains("password") && (lower.contains("weak") || lower.contains("short") || lower.contains("least")) {
        return "That password was not accepted. Try a longer one. Nothing was stored.".into();
    }
    "Account sync could not reach the server. Likes and follows on this PC are unchanged.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Rec {
        calls: Mutex<Vec<(String, String)>>,
        mode: Mutex<&'static str>,
    }

    impl Http for Rec {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            let body = request.body.clone().unwrap_or_default();
            self.calls.lock().unwrap().push((request.url.clone(), body.clone()));
            let mode = *self.mode.lock().unwrap();
            if request.url.contains("grant_type=password") {
                if body.contains("wrong-password") {
                    return Err("http 400: invalid login credentials".into());
                }
                return Ok(r#"{"access_token":"access-one","refresh_token":"refresh-one"}"#.into());
            }
            if request.url.ends_with("/auth/v1/signup") {
                return match mode {
                    "confirm" => Ok(
                        r#"{"id":"user-1","email":"ada@example.com","confirmation_sent_at":"2026-11-09T12:00:00Z"}"#
                            .into(),
                    ),
                    _ => Ok(r#"{"access_token":"access-new","refresh_token":"refresh-new"}"#.into()),
                };
            }
            if request.url.contains("grant_type=refresh_token") {
                return Ok(r#"{"access_token":"access-two","refresh_token":"refresh-two"}"#.into());
            }
            if request.url.ends_with("/auth/v1/logout") {
                return Ok("{}".into());
            }
            if request.url.ends_with("/auth/v1/recover") {
                return Ok("{}".into());
            }
            Ok("[]".into())
        }
    }

    fn config(dir: &std::path::Path) -> XConfig {
        XConfig {
            config_dir: dir.to_path_buf(),
            token_store: "file".into(),
            ..XConfig::default()
        }
    }

    #[test]
    fn sign_up_confirm_required_does_not_store_a_session_or_password() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        supabase_sync::store_anon_key(&config, "anon-key-value").unwrap();
        let http = Rec { calls: Mutex::new(Vec::new()), mode: Mutex::new("confirm") };
        let outcome = sign_up(&config, &http, "ada@example.com", "correct-password").unwrap();
        assert_eq!(outcome, AuthOutcome::ConfirmEmail);
        assert_eq!(outcome_text(&outcome), CONFIRM_EMAIL);
        assert!(!has_session(&config));
        let saved = std::fs::read_to_string(config.config_dir.join("supabase-secrets.json")).unwrap();
        assert!(!saved.contains("correct-password"));
    }

    #[test]
    fn sign_up_and_sign_in_store_the_refresh_token_only() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        supabase_sync::store_anon_key(&config, "anon-key-value").unwrap();
        let http = Rec { calls: Mutex::new(Vec::new()), mode: Mutex::new("session") };
        assert_eq!(
            sign_up(&config, &http, "ada@example.com", "correct-password").unwrap(),
            AuthOutcome::SignedIn
        );
        assert!(has_session(&config));
        assert_eq!(supabase_sync::saved_refresh(&config).as_deref(), Some("refresh-new"));
        let saved = std::fs::read_to_string(config.config_dir.join("supabase-secrets.json")).unwrap();
        assert!(!saved.contains("correct-password"));
    }

    #[test]
    fn wrong_password_is_a_plain_error() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        supabase_sync::store_anon_key(&config, "anon-key-value").unwrap();
        let http = Rec { calls: Mutex::new(Vec::new()), mode: Mutex::new("session") };
        let error = sign_in_store(&config, &http, "ada@example.com", "wrong-password").unwrap_err();
        assert_eq!(error, "That email or password was not accepted. Nothing was changed.");
        assert!(!error.contains("wrong-password"));
        assert!(!error.contains("anon-key-value"));
        assert!(!has_session(&config));
    }

    #[test]
    fn refresh_replaces_the_session_and_sign_out_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        supabase_sync::store_anon_key(&config, "anon-key-value").unwrap();
        supabase_sync::store_session(&config, "access-one", Some("refresh-one")).unwrap();
        let http = Rec { calls: Mutex::new(Vec::new()), mode: Mutex::new("session") };
        refresh_session(&config, &http).unwrap();
        assert_eq!(supabase_sync::saved_access(&config).as_deref(), Some("access-two"));
        assert_eq!(supabase_sync::saved_refresh(&config).as_deref(), Some("refresh-two"));
        assert_eq!(sign_out(&config, &http), AuthOutcome::SignedOut);
        assert!(!has_session(&config));
        assert!(supabase_sync::saved_refresh(&config).is_none());
        let calls = http.calls.lock().unwrap();
        assert!(calls.iter().any(|(url, _)| url.ends_with("/auth/v1/logout")));
    }

    #[test]
    fn recover_sends_the_email_and_does_not_store_a_password() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        supabase_sync::store_anon_key(&config, "anon-key-value").unwrap();
        let http = Rec { calls: Mutex::new(Vec::new()), mode: Mutex::new("session") };
        assert_eq!(
            recover(&config, &http, "ada@example.com").unwrap(),
            AuthOutcome::ResetSent
        );
        assert!(http.calls.lock().unwrap().iter().any(|(url, body)| {
            url.ends_with("/auth/v1/recover") && body.contains("ada@example.com") && !body.contains("password")
        }));
    }
}
