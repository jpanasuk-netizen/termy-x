//! Optional Supabase sync for the local action database.
//!
//! The SQLite file stays the source of truth. When a Supabase URL, anon
//! key, and user session are saved, this module pushes toggles and
//! receipts with PostgREST upserts and pulls newer rows back. Nothing
//! here posts, likes, or follows on X. Keys are never written into the
//! status line.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::action_state::{
    ActionDb, CloudReceipt, CloudToggle, ReceiptKind, RecordSource, ToggleKind,
};
use crate::config::XConfig;
use crate::io::{Http, HttpRequest};

const ANON_ACCOUNT: &str = "supabase-anon";
const ACCESS_ACCOUNT: &str = "supabase-access";
const REFRESH_ACCOUNT: &str = "supabase-refresh";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncPhase {
    Off,
    Syncing,
    Synced,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatus {
    pub phase: SyncPhase,
    pub at_ms: u64,
    pub message: String,
}

impl SyncStatus {
    pub fn label(&self) -> String {
        match self.phase {
            SyncPhase::Off => "off".to_string(),
            SyncPhase::Syncing => "syncing".to_string(),
            SyncPhase::Synced => {
                if self.at_ms == 0 {
                    "synced".to_string()
                } else {
                    format!("synced at {}", hhmm_utc(self.at_ms))
                }
            }
            SyncPhase::Error => {
                if self.message.is_empty() {
                    "error".to_string()
                } else {
                    format!("error: {}", self.message)
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyncOutcome {
    pub status: SyncStatus,
    pub toggles: Vec<CloudToggle>,
    pub receipts: Vec<CloudReceipt>,
}

/// What Settings can show. The anon key and session are present or absent, never printed.
pub fn settings_summary(config: &XConfig) -> String {
    let url = crate::supabase_auth::project_url(config);
    let key = if load_secret(config, ANON_ACCOUNT).is_some() {
        "saved override"
    } else if crate::supabase_defaults::builtin_anon_key().is_some() {
        "built in"
    } else {
        "not in this build"
    };
    let session = if load_secret(config, ACCESS_ACCOUNT).is_some() {
        "signed in"
    } else {
        "not signed in"
    };
    let email = if config.supabase_email.trim().is_empty() {
        "not set".to_string()
    } else {
        config.supabase_email.trim().to_string()
    };
    let status = read_status(&status_path(config));
    format!(
        "Supabase sync: {}\nProject: {url}\nAnon key: {key}\nEmail: {email}\nSession: {session}\nThe local action database is still used when this is off.",
        status.label()
    )
}

pub fn save_url(config: &XConfig, url: &str) -> Result<(), String> {
    let url = normalize_project_url(url)?;
    config.save_supabase_url(&url)
}

pub fn store_anon_key(config: &XConfig, key: &str) -> Result<(), String> {
    store_secret(config, ANON_ACCOUNT, key)
}

pub fn store_session(config: &XConfig, access: &str, refresh: Option<&str>) -> Result<(), String> {
    store_secret(config, ACCESS_ACCOUNT, access)?;
    if let Some(refresh) = refresh.filter(|value| !value.trim().is_empty()) {
        store_secret(config, REFRESH_ACCOUNT, refresh)?;
    }
    Ok(())
}

/// Email sign-in against `{url}/auth/v1/token`. The password is not stored.
pub fn sign_in(
    http: &dyn Http,
    url: &str,
    anon_key: &str,
    email: &str,
    password: &str,
) -> Result<(String, Option<String>), String> {
    crate::supabase_auth::password_session(http, url, anon_key, email, password)
}

/// Push local rows and, when `pull` is set, merge newer remote rows.
/// Unconfigured installs return Off and do not use the network.
pub fn sync(config: &XConfig, http: &dyn Http, db: &ActionDb, pull: bool) -> SyncOutcome {
    let Some(creds) = credentials(config) else {
        let status = SyncStatus {
            phase: SyncPhase::Off,
            at_ms: 0,
            message: String::new(),
        };
        let _ = write_status(&status_path(config), &status);
        return SyncOutcome {
            status,
            toggles: Vec::new(),
            receipts: Vec::new(),
        };
    };
    let syncing = SyncStatus {
        phase: SyncPhase::Syncing,
        at_ms: crate::action_state::now_ms(),
        message: String::new(),
    };
    let _ = write_status(&status_path(config), &syncing);
    let mut toggles = Vec::new();
    let mut receipts = Vec::new();
    if pull {
        match pull_remote(http, &creds) {
            Ok((remote_toggles, remote_receipts)) => {
                toggles = remote_toggles;
                receipts = remote_receipts;
            }
            Err(error) => {
                let _ = remember_outbox(config, db);
                return finish_error(config, &creds, error);
            }
        }
    }
    if let Err(error) = push_local(http, &creds, db, &outbox_rows(config)) {
        let _ = remember_outbox(config, db);
        return finish_error(config, &creds, error);
    }
    let _ = fs::remove_file(outbox_path(config));
    let status = SyncStatus {
        phase: SyncPhase::Synced,
        at_ms: crate::action_state::now_ms(),
        message: String::new(),
    };
    let _ = write_status(&status_path(config), &status);
    SyncOutcome {
        status,
        toggles,
        receipts,
    }
}

/// Read the database from disk, sync, and save rows that the remote won.
/// Safe to call from a background thread. Does nothing when Supabase is off.
pub fn sync_saved(config: &XConfig, http: &dyn Http, pull: bool) -> SyncStatus {
    let path = config.action_state_path();
    let Ok(mut db) = ActionDb::load(&path) else {
        return SyncStatus {
            phase: SyncPhase::Error,
            at_ms: 0,
            message: "local action database could not be read".into(),
        };
    };
    let outcome = sync(config, http, &db, pull);
    let mut changed = false;
    for row in &outcome.toggles {
        changed |= db.merge_cloud_toggle(row);
    }
    for row in &outcome.receipts {
        changed |= db.merge_cloud_receipt(row);
    }
    if changed {
        let _ = db.save(&path);
    }
    outcome.status
}

pub fn push_saved_in_background() {
    let config = XConfig::load(&crate::config::default_config_dir());
    if !configured(&config) {
        return;
    }
    std::thread::spawn(|| {
        let config = XConfig::load(&crate::config::default_config_dir());
        let http = crate::io::UreqHttp::default();
        let _ = sync_saved(&config, &http, false);
    });
}

fn finish_error(config: &XConfig, creds: &Creds, error: String) -> SyncOutcome {
    let status = SyncStatus {
        phase: SyncPhase::Error,
        at_ms: crate::action_state::now_ms(),
        message: redact(&error, &[&creds.anon_key, &creds.access]),
    };
    let _ = write_status(&status_path(config), &status);
    SyncOutcome {
        status,
        toggles: Vec::new(),
        receipts: Vec::new(),
    }
}

struct Creds {
    url: String,
    anon_key: String,
    access: String,
}

fn configured(config: &XConfig) -> bool {
    credentials(config).is_some()
}

fn credentials(config: &XConfig) -> Option<Creds> {
    let url = crate::supabase_auth::project_url(config);
    let anon_key = crate::supabase_auth::anon_key(config)?;
    let access = load_secret(config, ACCESS_ACCOUNT)?;
    Some(Creds {
        url,
        anon_key,
        access,
    })
}

pub(crate) fn saved_access(config: &XConfig) -> Option<String> {
    load_secret(config, ACCESS_ACCOUNT)
}

pub(crate) fn saved_refresh(config: &XConfig) -> Option<String> {
    load_secret(config, REFRESH_ACCOUNT)
}

pub(crate) fn saved_anon(config: &XConfig) -> Option<String> {
    load_secret(config, ANON_ACCOUNT)
}

pub fn clear_session(config: &XConfig) -> Result<(), String> {
    for account in [ACCESS_ACCOUNT, REFRESH_ACCOUNT] {
        if let Ok(entry) = keyring::Entry::new("termy-x", account) {
            let _ = entry.delete_credential();
        }
    }
    let mut map = read_secret_file(config);
    let access_removed = map.remove(ACCESS_ACCOUNT).is_some();
    let refresh_removed = map.remove(REFRESH_ACCOUNT).is_some();
    let removed = access_removed || refresh_removed;
    if removed || secrets_path(config).exists() {
        crate::auth::write_private_file(&secrets_path(config), &Value::Object(map).to_string())?;
    }
    let status = SyncStatus {
        phase: SyncPhase::Off,
        at_ms: 0,
        message: String::new(),
    };
    write_status(&status_path(config), &status)
}

pub fn record_status(config: &XConfig, status: &SyncStatus) {
    let _ = write_status(&status_path(config), status);
}

fn pull_remote(
    http: &dyn Http,
    creds: &Creds,
) -> Result<(Vec<CloudToggle>, Vec<CloudReceipt>), String> {
    let toggles = get_json(
        http,
        creds,
        &format!(
            "{}/rest/v1/x_action_toggles?select=x_account,kind,item_key,confirmed,source,verified_at_ms,seq,proof,updated_at",
            creds.url
        ),
    )?;
    let receipts = get_json(
        http,
        creds,
        &format!(
            "{}/rest/v1/x_action_receipts?select=x_account,kind,tweet_id,text,seq,at_ms,updated_at",
            creds.url
        ),
    )?;
    Ok((parse_toggles(&toggles)?, parse_receipts(&receipts)?))
}

fn push_local(
    http: &dyn Http,
    creds: &Creds,
    db: &ActionDb,
    queued: &(Vec<CloudToggle>, Vec<CloudReceipt>),
) -> Result<(), String> {
    let mut toggles = db.cloud_toggles();
    toggles.extend(queued.0.iter().cloned());
    let toggle_body: Vec<Value> = toggles
        .iter()
        .map(|row| {
            json!({
                "x_account": row.account,
                "kind": row.kind.as_str(),
                "item_key": row.key,
                "confirmed": row.confirmed,
                "source": row.source.map(RecordSource::as_str),
                "verified_at_ms": row.verified_at_ms,
                "seq": row.seq,
                "proof": row.proof,
                "updated_at": rfc3339_from_ms(row.verified_at_ms),
            })
        })
        .collect();
    if !toggle_body.is_empty() {
        post_merge(
            http,
            creds,
            &format!(
                "{}/rest/v1/x_action_toggles?on_conflict=owner,x_account,kind,item_key",
                creds.url
            ),
            &toggle_body,
        )?;
    }
    let mut receipts = db.cloud_receipts();
    receipts.extend(queued.1.iter().cloned());
    let receipt_body: Vec<Value> = receipts
        .iter()
        .map(|row| {
            json!({
                "x_account": row.account,
                "kind": row.kind.as_str(),
                "tweet_id": row.tweet_id,
                "text": row.text,
                "seq": row.seq,
                "at_ms": row.at_ms,
                "updated_at": rfc3339_from_ms(row.at_ms),
            })
        })
        .collect();
    if !receipt_body.is_empty() {
        post_merge(
            http,
            creds,
            &format!(
                "{}/rest/v1/x_action_receipts?on_conflict=owner,x_account,kind,tweet_id,seq",
                creds.url
            ),
            &receipt_body,
        )?;
    }
    Ok(())
}

fn get_json(http: &dyn Http, creds: &Creds, url: &str) -> Result<Value, String> {
    let text = http
        .request(auth_request("GET", url, creds, None))
        .map_err(|error| redact(&error, &[&creds.anon_key, &creds.access]))?;
    serde_json::from_str(&text)
        .map_err(|_| "Supabase returned a row list that could not be read.".into())
}

fn post_merge(http: &dyn Http, creds: &Creds, url: &str, body: &[Value]) -> Result<(), String> {
    let payload = serde_json::to_string(body).map_err(|error| error.to_string())?;
    http.request(auth_request("POST", url, creds, Some(payload)))
        .map(|_| ())
        .map_err(|error| redact(&error, &[&creds.anon_key, &creds.access]))
}

fn auth_request(
    method: &'static str,
    url: &str,
    creds: &Creds,
    body: Option<String>,
) -> HttpRequest {
    HttpRequest {
        method,
        url: url.to_string(),
        body,
        bearer: Some(creds.access.clone()),
        content_type: Some("application/json".into()),
        headers: vec![
            ("apikey".into(), creds.anon_key.clone()),
            (
                "Prefer".into(),
                "resolution=merge-duplicates,return=minimal".into(),
            ),
        ],
    }
}

fn parse_toggles(value: &Value) -> Result<Vec<CloudToggle>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "Supabase toggle list was not a list.".to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .and_then(ToggleKind::parse);
        let Some(kind) = kind else {
            continue;
        };
        let account = text_field(row, "x_account");
        let key = text_field(row, "item_key");
        if account.is_empty() || key.is_empty() {
            continue;
        }
        out.push(CloudToggle {
            account,
            kind,
            key,
            confirmed: row.get("confirmed").and_then(Value::as_bool),
            source: row
                .get("source")
                .and_then(Value::as_str)
                .and_then(RecordSource::parse),
            verified_at_ms: row
                .get("verified_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| ms_from_timestamp(row.get("updated_at"))),
            seq: row.get("seq").and_then(Value::as_u64).unwrap_or(0),
            proof: row.get("proof").and_then(Value::as_str).map(str::to_string),
            pending: false,
        });
    }
    Ok(out)
}

fn parse_receipts(value: &Value) -> Result<Vec<CloudReceipt>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| "Supabase receipt list was not a list.".to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let kind = row
            .get("kind")
            .and_then(Value::as_str)
            .and_then(ReceiptKind::parse);
        let Some(kind) = kind else {
            continue;
        };
        out.push(CloudReceipt {
            account: text_field(row, "x_account"),
            kind,
            tweet_id: text_field(row, "tweet_id"),
            text: text_field(row, "text"),
            seq: row.get("seq").and_then(Value::as_u64).unwrap_or(0),
            at_ms: row
                .get("at_ms")
                .and_then(Value::as_u64)
                .unwrap_or_else(|| ms_from_timestamp(row.get("updated_at"))),
        });
    }
    Ok(out)
}

fn text_field(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn remember_outbox(config: &XConfig, db: &ActionDb) -> Result<(), String> {
    let body = json!({
        "toggles": db.cloud_toggles().iter().map(|row| json!({
            "account": row.account,
            "kind": row.kind.as_str(),
            "key": row.key,
            "confirmed": row.confirmed,
            "source": row.source.map(RecordSource::as_str),
            "verified_at_ms": row.verified_at_ms,
            "seq": row.seq,
            "proof": row.proof,
        })).collect::<Vec<_>>(),
        "receipts": db.cloud_receipts().iter().map(|row| json!({
            "account": row.account,
            "kind": row.kind.as_str(),
            "tweet_id": row.tweet_id,
            "text": row.text,
            "seq": row.seq,
            "at_ms": row.at_ms,
        })).collect::<Vec<_>>(),
    });
    crate::auth::write_private_file(&outbox_path(config), &body.to_string())
}

fn outbox_rows(config: &XConfig) -> (Vec<CloudToggle>, Vec<CloudReceipt>) {
    let Ok(text) = fs::read_to_string(outbox_path(config)) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return (Vec::new(), Vec::new());
    };
    let toggles = value
        .get("toggles")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let kind =
                        ToggleKind::parse(row.get("kind").and_then(Value::as_str).unwrap_or(""))?;
                    Some(CloudToggle {
                        account: text_field(row, "account"),
                        kind,
                        key: text_field(row, "key"),
                        confirmed: row.get("confirmed").and_then(Value::as_bool),
                        source: row
                            .get("source")
                            .and_then(Value::as_str)
                            .and_then(RecordSource::parse),
                        verified_at_ms: row
                            .get("verified_at_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        seq: row.get("seq").and_then(Value::as_u64).unwrap_or(0),
                        proof: row.get("proof").and_then(Value::as_str).map(str::to_string),
                        pending: false,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let receipts = value
        .get("receipts")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let kind =
                        ReceiptKind::parse(row.get("kind").and_then(Value::as_str).unwrap_or(""))?;
                    Some(CloudReceipt {
                        account: text_field(row, "account"),
                        kind,
                        tweet_id: text_field(row, "tweet_id"),
                        text: text_field(row, "text"),
                        seq: row.get("seq").and_then(Value::as_u64).unwrap_or(0),
                        at_ms: row.get("at_ms").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (toggles, receipts)
}

pub(crate) fn normalize_project_url(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err("Supabase URL is empty.".into());
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://127.0.0.1"))
        .ok_or_else(|| "Supabase URL must start with https://.".to_string())?;
    if rest.contains('@') || rest.contains(' ') {
        return Err("Supabase URL is not a project URL.".into());
    }
    Ok(url.to_string())
}

fn hhmm_utc(ms: u64) -> String {
    let tod = (ms / 1000) % 86_400;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    format!("{hour:02}:{minute:02}")
}

pub fn redact(message: &str, secrets: &[&str]) -> String {
    let mut out = message.to_string();
    for secret in secrets {
        let secret = secret.trim();
        if secret.len() >= 6 {
            out = out.replace(secret, "[redacted]");
        }
    }
    out.chars().take(240).collect()
}

fn secrets_path(config: &XConfig) -> PathBuf {
    config.config_dir.join("supabase-secrets.json")
}

fn status_path(config: &XConfig) -> PathBuf {
    config.config_dir.join("x-supabase-status.json")
}

fn outbox_path(config: &XConfig) -> PathBuf {
    config.config_dir.join("x-supabase-outbox.json")
}

fn store_secret(config: &XConfig, account: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty Supabase secret".into());
    }
    if config.token_store == "keychain"
        && let Ok(entry) = keyring::Entry::new("termy-x", account)
        && entry.set_password(value).is_ok()
    {
        return Ok(());
    }
    let mut map = read_secret_file(config);
    map.insert(account.to_string(), Value::String(value.to_string()));
    crate::auth::write_private_file(&secrets_path(config), &Value::Object(map).to_string())
}

fn load_secret(config: &XConfig, account: &str) -> Option<String> {
    if config.token_store == "keychain"
        && let Ok(entry) = keyring::Entry::new("termy-x", account)
        && let Ok(value) = entry.get_password()
        && !value.trim().is_empty()
    {
        return Some(value);
    }
    read_secret_file(config)
        .get(account)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn read_secret_file(config: &XConfig) -> serde_json::Map<String, Value> {
    fs::read_to_string(secrets_path(config))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

#[derive(Serialize, Deserialize)]
struct StatusFile {
    phase: String,
    at_ms: u64,
    message: String,
}

fn write_status(path: &Path, status: &SyncStatus) -> Result<(), String> {
    let phase = match status.phase {
        SyncPhase::Off => "off",
        SyncPhase::Syncing => "syncing",
        SyncPhase::Synced => "synced",
        SyncPhase::Error => "error",
    };
    let body = serde_json::to_string(&StatusFile {
        phase: phase.into(),
        at_ms: status.at_ms,
        message: status.message.clone(),
    })
    .map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, body).map_err(|error| error.to_string())
}

fn read_status(path: &Path) -> SyncStatus {
    let Ok(text) = fs::read_to_string(path) else {
        return SyncStatus {
            phase: SyncPhase::Off,
            at_ms: 0,
            message: String::new(),
        };
    };
    let Ok(file) = serde_json::from_str::<StatusFile>(&text) else {
        return SyncStatus {
            phase: SyncPhase::Off,
            at_ms: 0,
            message: String::new(),
        };
    };
    let phase = match file.phase.as_str() {
        "syncing" => SyncPhase::Syncing,
        "synced" => SyncPhase::Synced,
        "error" => SyncPhase::Error,
        _ => SyncPhase::Off,
    };
    SyncStatus {
        phase,
        at_ms: file.at_ms,
        message: file.message,
    }
}

fn rfc3339_from_ms(ms: u64) -> String {
    let secs = ms / 1000;
    let millis = ms % 1000;
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn ms_from_timestamp(value: Option<&Value>) -> u64 {
    let Some(text) = value.and_then(Value::as_str) else {
        return 0;
    };
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes[4] != b'-' || bytes[10] != b'T' {
        return 0;
    }
    let year: i64 = text[0..4].parse().unwrap_or(0);
    let month: i64 = text[5..7].parse().unwrap_or(0);
    let day: i64 = text[8..10].parse().unwrap_or(0);
    let hour: u64 = text[11..13].parse().unwrap_or(0);
    let minute: u64 = text[14..16].parse().unwrap_or(0);
    let second: u64 = text[17..19].parse().unwrap_or(0);
    if year < 1970 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return 0;
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return 0;
    }
    ((days as u64) * 86_400 + hour * 3600 + minute * 60 + second) * 1000
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    era * 146_097 + doe as i64 - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::Http;
    use std::sync::Mutex;

    struct RecHttp {
        calls: Mutex<Vec<HttpRequest>>,
        fail_push: Mutex<bool>,
    }

    impl RecHttp {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail_push: Mutex::new(false),
            }
        }
    }

    impl Http for RecHttp {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            let url = request.url.clone();
            let body = request.body.clone().unwrap_or_default();
            self.calls.lock().unwrap().push(request);
            if url.contains("/auth/v1/token") {
                if body.contains("wrong-password") {
                    return Err("http 400: invalid login".into());
                }
                return Ok(
                    r#"{"access_token":"user-jwt-token","refresh_token":"refresh-token"}"#.into(),
                );
            }
            if *self.fail_push.lock().unwrap()
                && url.contains("/rest/v1/")
                && !url.contains("select=")
            {
                return Err("http 503: offline secret-anon-key-value".into());
            }
            if url.contains("x_action_toggles") && url.contains("select=") {
                return Ok(
                    r#"[{"x_account":"jasper_black","kind":"like","item_key":"55","confirmed":true,"source":"live_read","verified_at_ms":50,"seq":2,"proof":"opencli-contract","updated_at":"1970-01-01T00:00:00.050Z"}]"#
                        .into(),
                );
            }
            if url.contains("x_action_receipts") && url.contains("select=") {
                return Ok(
                    r#"[{"x_account":"jasper_black","kind":"post","tweet_id":"9","text":"posted","seq":1,"at_ms":40,"updated_at":"1970-01-01T00:00:00.040Z"}]"#
                        .into(),
                );
            }
            Ok("[]".into())
        }
    }

    fn config(dir: &std::path::Path) -> XConfig {
        XConfig {
            config_dir: dir.to_path_buf(),
            token_store: "file".into(),
            supabase_url: "https://example.supabase.co".into(),
            ..XConfig::default()
        }
    }

    #[test]
    fn unconfigured_sync_stays_local_and_makes_no_request() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(dir.path());
        config.supabase_url.clear();
        let http = RecHttp::new();
        let outcome = sync(&config, &http, &ActionDb::default(), true);
        assert_eq!(outcome.status.phase, SyncPhase::Off);
        assert!(http.calls.lock().unwrap().is_empty());
        assert!(settings_summary(&config).contains("Supabase sync: off"));
    }

    #[test]
    fn push_upserts_and_a_newer_remote_row_merges() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        store_anon_key(&config, "secret-anon-key-value").unwrap();
        store_session(&config, "user-jwt-token", None).unwrap();
        let mut db = ActionDb::default();
        db.force_confirm("jasper_black", ToggleKind::Like, "55", false, 10);
        let http = RecHttp::new();
        let outcome = sync(&config, &http, &db, true);
        assert_eq!(
            outcome.status.phase,
            SyncPhase::Synced,
            "{}",
            outcome.status.label()
        );
        assert!(db.merge_cloud_toggle(&outcome.toggles[0]));
        assert_eq!(
            db.confirmed("jasper_black", ToggleKind::Like, "55"),
            Some(true)
        );
        let calls = http.calls.lock().unwrap();
        let post = calls
            .iter()
            .find(|call| {
                call.method == "POST"
                    && call
                        .url
                        .contains("on_conflict=owner,x_account,kind,item_key")
            })
            .expect("upsert");
        assert!(post.headers.iter().any(|(name, _)| name == "apikey"));
        assert!(
            post.headers
                .iter()
                .any(|(name, value)| name == "Prefer" && value.contains("merge-duplicates"))
        );
        assert!(!outcome.status.label().contains("secret-anon-key-value"));
        assert!(!outcome.status.label().contains("user-jwt-token"));
    }

    #[test]
    fn an_older_remote_row_and_a_pending_local_row_stay_put() {
        let mut db = ActionDb::default();
        db.force_confirm("jasper_black", ToggleKind::Follow, "ada", true, 80);
        let older = CloudToggle {
            account: "jasper_black".into(),
            kind: ToggleKind::Follow,
            key: "ada".into(),
            confirmed: Some(false),
            source: Some(RecordSource::LiveRead),
            verified_at_ms: 20,
            seq: 1,
            proof: None,
            pending: false,
        };
        assert!(!db.merge_cloud_toggle(&older));
        assert_eq!(
            db.confirmed("jasper_black", ToggleKind::Follow, "ada"),
            Some(true)
        );
        db.begin_toggle("jasper_black", ToggleKind::Like, "7", true, 3, 90);
        let remote = CloudToggle {
            account: "jasper_black".into(),
            kind: ToggleKind::Like,
            key: "7".into(),
            confirmed: Some(false),
            source: None,
            verified_at_ms: 200,
            seq: 9,
            proof: None,
            pending: false,
        };
        assert!(!db.merge_cloud_toggle(&remote));
        assert_eq!(db.confirmed("jasper_black", ToggleKind::Like, "7"), None);
    }

    #[test]
    fn offline_push_queues_and_the_error_hides_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        store_anon_key(&config, "secret-anon-key-value").unwrap();
        store_session(&config, "user-jwt-token", None).unwrap();
        let mut db = ActionDb::default();
        db.record_receipt("jasper_black", ReceiptKind::Post, "9", "hello", 1, 15);
        let http = RecHttp::new();
        *http.fail_push.lock().unwrap() = true;
        let outcome = sync(&config, &http, &db, false);
        assert_eq!(outcome.status.phase, SyncPhase::Error);
        assert!(outcome.status.label().contains("error"));
        assert!(!outcome.status.message.contains("secret-anon-key-value"));
        assert!(outbox_path(&config).exists());
        *http.fail_push.lock().unwrap() = false;
        let again = sync(&config, &http, &db, false);
        assert_eq!(
            again.status.phase,
            SyncPhase::Synced,
            "{}",
            again.status.label()
        );
        assert!(!outbox_path(&config).exists());
    }

    #[test]
    fn sign_in_stores_the_session_and_not_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path());
        let http = RecHttp::new();
        let err = sign_in(
            &http,
            "https://example.supabase.co",
            "secret-anon-key-value",
            "ada@example.com",
            "wrong-password",
        )
        .unwrap_err();
        assert!(!err.contains("wrong-password"));
        assert!(!err.contains("secret-anon-key-value"));
        let (access, refresh) = sign_in(
            &http,
            "https://example.supabase.co",
            "secret-anon-key-value",
            "ada@example.com",
            "correct-password",
        )
        .unwrap();
        store_session(&config, &access, refresh.as_deref()).unwrap();
        let summary = settings_summary(&config);
        assert!(summary.contains("signed in"));
        assert!(!summary.contains("user-jwt-token"));
        assert!(!summary.contains("correct-password"));
        let saved = fs::read_to_string(secrets_path(&config)).unwrap();
        assert!(!saved.contains("correct-password"));
    }
}
