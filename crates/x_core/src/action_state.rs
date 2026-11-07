//! Account-scoped memory of likes, reposts, follows, and post receipts.
//!
//! Stored in an embedded SQLite file, `x-action-state.db`, in the Termy
//! config directory. No separate database server is installed or required.
//! An older `x-action-state.json` in that directory is copied in once.
//! The database stores handles, tweet ids, and short receipts. It does
//! not store passwords, tokens, or API keys.
//!
//! A confirmed value is shown immediately on startup. It is labeled saved
//! until a post loaded in this session carries a real like or follow flag,
//! or until a write just succeeded. There is no poll. Sync never follows,
//! likes, or posts. A payload that omits a flag does not erase the saved
//! row and is not treated as a live reading.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::model::{Post, strip_handle};

/// Posts inspected on one login sync. One home page, not the whole account.
pub const SYNC_POST_CAP: usize = 100;

pub const FILE_NAME: &str = "x-action-state.db";
/// Previous store. Read once, then renamed so a later start does not import it again.
pub const LEGACY_JSON_NAME: &str = "x-action-state.json";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToggleKind {
    Like,
    Repost,
    Follow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptKind {
    Reply,
    QTweet,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSource {
    LocalWrite,
    LiveRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Last confirmed by a write or a flag during this session.
    Live,
    /// Loaded from disk and not rechecked yet.
    Saved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Cached,
    Checking,
    Checked,
    Offline,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFinish {
    Ignored,
    Report(SyncReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub phase: SyncPhase,
    pub account: String,
    pub checked: usize,
    pub like_flags: usize,
    pub follow_flags: usize,
    pub truncated: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishResult {
    Applied,
    Stale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingWrite {
    seq: u64,
    desired: bool,
    at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ToggleState {
    #[serde(default)]
    confirmed: Option<bool>,
    #[serde(default)]
    pending: Option<PendingWrite>,
    #[serde(default)]
    last_error: Option<String>,
    #[serde(default)]
    source: Option<RecordSource>,
    #[serde(default)]
    verified_at_ms: u64,
    #[serde(default)]
    seq: u64,
    /// Set only after an OpenCLI success message was checked. Older rows stay empty.
    #[serde(default)]
    proof: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Receipt {
    kind: ReceiptKind,
    tweet_id: String,
    text: String,
    seq: u64,
    at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct AccountBucket {
    #[serde(default)]
    likes: HashMap<String, ToggleState>,
    #[serde(default)]
    reposts: HashMap<String, ToggleState>,
    #[serde(default)]
    follows: HashMap<String, ToggleState>,
    #[serde(default)]
    receipts: Vec<Receipt>,
    #[serde(default)]
    last_sync_ms: Option<u64>,
    #[serde(default)]
    last_sync_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionDb {
    pub version: u32,
    #[serde(default)]
    accounts: HashMap<String, AccountBucket>,
    /// Set when the file is loaded. Not written. Older rows are Saved.
    #[serde(skip)]
    session_started_ms: u64,
    #[serde(skip)]
    sync_generation: u64,
    #[serde(skip)]
    sync_account: Option<String>,
}

impl Default for ActionDb {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: HashMap::new(),
            session_started_ms: 0,
            sync_generation: 0,
            sync_account: None,
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn account_key(handle: &str) -> String {
    let stripped = strip_handle(handle).to_ascii_lowercase();
    if stripped.is_empty() {
        "signed-out".to_string()
    } else {
        stripped
    }
}

pub fn tweet_key(id: &str) -> String {
    id.trim().to_string()
}

/// Follow and like on a repost use the original author and id.
/// A quote uses the quote tweet itself, not the quoted author.
pub fn follow_subject(post: &Post) -> &str {
    post.engage_target().author_handle.as_str()
}

pub fn remembered_or_payload(remembered: Option<bool>, payload: Option<bool>) -> Option<bool> {
    remembered.or(payload)
}

/// What a login sync can actually see. Not a promise of full history.
pub fn sync_coverage() -> &'static str {
    "Login checks the posts that load for this account, up to one page. A like or follow flag is applied only when that payload includes it. This provider has no call for the full like history or the full following list, so anything older stays saved until a loaded post carries the flag. The sync does not follow, like, or post."
}

/// Read saved action rows. Does not write the database and does not call X.
pub fn export_sanitized(path: &Path) -> Result<String, String> {
    if !path.exists() {
        return Ok(format!(
            "No action database at {}. Nothing was changed.",
            path.display()
        ));
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("action database: {error}"))?;
    let has_proof = toggles_have_column(&conn, "proof")?;
    let mut lines = vec![
        "action export (read-only). No passwords or keys. Nothing was sent to X.".to_string(),
        "columns: account, kind, target, confirmed, source, time_utc, error, proof".to_string(),
        "provider is not stored on these rows. A retweet click uses OpenCLI when that posting path is available.".to_string(),
        "proof=legacy-unverified means confirmed was saved before the OpenCLI message check. It is not fresh X proof.".to_string(),
    ];
    let sql = if has_proof {
        "SELECT account, kind, item_key, confirmed, source, verified_at_ms, last_error, proof FROM toggles ORDER BY verified_at_ms, account, kind, item_key"
    } else {
        "SELECT account, kind, item_key, confirmed, source, verified_at_ms, last_error, NULL FROM toggles ORDER BY verified_at_ms, account, kind, item_key"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|error| format!("action database: {error}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|error| format!("action database: {error}"))?;
    for row in rows {
        let (account, kind, target, confirmed, source, at_ms, error, proof) =
            row.map_err(|error| format!("action database: {error}"))?;
        let confirmed_label = match confirmed {
            Some(1) => "yes",
            Some(0) => "no",
            _ => "unknown",
        };
        let proof_label = match proof.as_deref() {
            Some(text) if !text.is_empty() => text.to_string(),
            _ if source.as_deref() == Some("live_read") => "live-flag".into(),
            _ if confirmed == Some(1) => "legacy-unverified".into(),
            _ => "-".into(),
        };
        lines.push(format!(
            "{account}\t{kind}\t{target}\t{confirmed_label}\t{}\t{}\t{}\t{proof_label}",
            source.unwrap_or_else(|| "-".into()),
            utc_stamp(at_ms as u64),
            error.unwrap_or_else(|| "-".into())
        ));
    }
    if lines.len() == 4 {
        lines.push("no rows".into());
    }
    Ok(lines.join("\n"))
}

fn utc_stamp(ms: u64) -> String {
    if ms == 0 {
        return "-".into();
    }
    let secs = (ms / 1000) as i64;
    let rem = secs.rem_euclid(86_400) as u64;
    format!(
        "{} {:02}:{:02}:{:02}Z",
        crate::time::ymd(secs),
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

pub fn database_location_note() -> &'static str {
    "SQLite file x-action-state.db in the Termy config directory (%APPDATA%\\termy on Windows, ~/.config/termy on Linux). Handles, tweet ids, and short receipts only. No separate database server."
}

/// A second follow click does nothing while one follow write is in flight.
pub fn follow_write_allowed(follow_in_flight: bool) -> bool {
    !follow_in_flight
}

impl ActionDb {
    pub fn load(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        if path.exists() {
            if !sqlite_header_ok(path)? {
                return Err(
                    "action database is not a readable SQLite file. It was left in place."
                        .into(),
                );
            }
            let mut db = read_sqlite(path)?;
            db.session_started_ms = now_ms();
            db.settle_abandoned();
            db.save(path)?;
            return Ok(db);
        }
        let json_path = legacy_json_path(path);
        if json_path.exists() {
            let text = fs::read_to_string(&json_path).map_err(|error| error.to_string())?;
            let mut db: Self = serde_json::from_str(&text)
                .map_err(|error| format!("saved action file is unreadable: {error}"))?;
            db.session_started_ms = now_ms();
            db.settle_abandoned();
            db.save(path)?;
            let parked = json_path.with_file_name("x-action-state.json.migrated");
            fs::rename(&json_path, &parked).map_err(|error| error.to_string())?;
            return Ok(db);
        }
        let mut db = Self::default();
        db.session_started_ms = now_ms();
        db.save(path)?;
        Ok(db)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = open_conn(path)?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("action database: {error}"))?;
        ensure_schema(&tx)?;
        tx.execute("DELETE FROM receipts", [])
            .map_err(|error| format!("action database: {error}"))?;
        tx.execute("DELETE FROM toggles", [])
            .map_err(|error| format!("action database: {error}"))?;
        tx.execute("DELETE FROM accounts", [])
            .map_err(|error| format!("action database: {error}"))?;
        for (account, bucket) in &self.accounts {
            tx.execute(
                "INSERT INTO accounts(account, last_sync_ms, last_sync_error) VALUES(?1, ?2, ?3)",
                params![account, bucket.last_sync_ms, bucket.last_sync_error],
            )
            .map_err(|error| format!("action database: {error}"))?;
            write_toggles(&tx, account, ToggleKind::Like, &bucket.likes)?;
            write_toggles(&tx, account, ToggleKind::Repost, &bucket.reposts)?;
            write_toggles(&tx, account, ToggleKind::Follow, &bucket.follows)?;
            for receipt in &bucket.receipts {
                tx.execute(
                    "INSERT INTO receipts(account, kind, tweet_id, text, seq, at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        account,
                        receipt_kind_str(receipt.kind),
                        receipt.tweet_id,
                        receipt.text,
                        receipt.seq,
                        receipt.at_ms as i64
                    ],
                )
                .map_err(|error| format!("action database: {error}"))?;
            }
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|error| format!("action database: {error}"))?;
        tx.commit()
            .map_err(|error| format!("action database: {error}"))?;
        Ok(())
    }

    pub fn begin_sync(&mut self, account: &str) -> u64 {
        self.sync_generation = self.sync_generation.wrapping_add(1);
        self.sync_account = Some(account_key(account));
        self.sync_generation
    }

    pub fn sync_generation(&self) -> u64 {
        self.sync_generation
    }

    pub fn cached_summary(&self, account: &str) -> String {
        let key = account_key(account);
        let when = self
            .accounts
            .get(&key)
            .and_then(|bucket| bucket.last_sync_ms)
            .map(sync_date);
        match when {
            Some(date) => format!(
                "Showing saved likes and follows for @{key}. Last successful check {date}. Not a live reading yet."
            ),
            None => format!(
                "Showing saved likes and follows for @{key}. They have not been checked against X yet."
            ),
        }
    }

    pub fn checking_summary(account: &str) -> String {
        format!(
            "Checking the posts that load for @{}. Saved state stays on screen.",
            account_key(account)
        )
    }

    /// Apply one bounded page, or record a failed read. A mismatched
    /// account or generation is ignored so an older response cannot
    /// overwrite a newer write or a switched account.
    pub fn apply_sync_page(
        &mut self,
        account: &str,
        generation: u64,
        posts: &[Post],
        error: Option<&str>,
        snapshot_ms: u64,
        now: u64,
    ) -> SyncFinish {
        let key = account_key(account);
        if self.sync_account.as_deref() != Some(key.as_str()) || self.sync_generation != generation
        {
            return SyncFinish::Ignored;
        }
        if let Some(message) = error {
            let previous = self
                .accounts
                .get(&key)
                .and_then(|bucket| bucket.last_sync_ms);
            self.bucket(&key).last_sync_error = Some(clip(message));
            let summary = match previous.map(sync_date) {
                Some(date) => format!(
                    "Could not refresh @{key}: {}. Showing saved state. Last successful check {date}.",
                    clip(message)
                ),
                None => format!(
                    "Could not refresh @{key}: {}. Showing saved state. There is no successful check yet.",
                    clip(message)
                ),
            };
            return SyncFinish::Report(SyncReport {
                phase: SyncPhase::Offline,
                account: key,
                checked: 0,
                like_flags: 0,
                follow_flags: 0,
                truncated: false,
                summary,
            });
        }
        let truncated = posts.len() > SYNC_POST_CAP;
        let slice = if truncated {
            &posts[..SYNC_POST_CAP]
        } else {
            posts
        };
        let mut like_flags = 0usize;
        let mut follow_flags = 0usize;
        for post in slice {
            walk_post(post, &mut |node| {
                if node.viewer_liked.is_some() {
                    like_flags += 1;
                }
                if node.viewer_follows_author.is_some() {
                    follow_flags += 1;
                }
                self.reconcile_like(&key, &node.id, node.viewer_liked, snapshot_ms, now);
                self.reconcile_follow(
                    &key,
                    &node.author_handle,
                    node.viewer_follows_author,
                    snapshot_ms,
                    now,
                );
            });
        }
        let bucket = self.bucket(&key);
        bucket.last_sync_ms = Some(now);
        bucket.last_sync_error = None;
        let summary = format!(
            "Checked {} posts for @{key}. {like_flags} had a like flag and {follow_flags} had a follow flag from X. Saved rows without a new flag were kept. {}",
            slice.len(),
            sync_coverage()
        );
        let _ = truncated;
        SyncFinish::Report(SyncReport {
            phase: SyncPhase::Checked,
            account: key,
            checked: slice.len(),
            like_flags,
            follow_flags,
            truncated,
            summary: if truncated {
                format!("{summary} Stopped at {SYNC_POST_CAP} posts.")
            } else {
                summary
            },
        })
    }

    pub fn begin_toggle(
        &mut self,
        account: &str,
        kind: ToggleKind,
        id: &str,
        desired: bool,
        seq: u64,
        now: u64,
    ) {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        if key.is_empty() || key == "signed-out" && kind == ToggleKind::Follow {
            return;
        }
        let toggle = self.toggle_mut(account, kind, &key);
        toggle.pending = Some(PendingWrite {
            seq,
            desired,
            at_ms: now,
        });
        toggle.seq = seq;
        toggle.last_error = None;
    }

    pub fn finish_toggle(
        &mut self,
        account: &str,
        kind: ToggleKind,
        id: &str,
        seq: u64,
        ok: bool,
        error: Option<&str>,
        now: u64,
    ) -> FinishResult {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        let Some(toggle) = self.toggle_ref_mut(account, kind, &key) else {
            return FinishResult::Stale;
        };
        let Some(pending) = toggle.pending.clone() else {
            return FinishResult::Stale;
        };
        if pending.seq != seq {
            return FinishResult::Stale;
        }
        toggle.pending = None;
        if ok {
            toggle.confirmed = Some(pending.desired);
            toggle.last_error = None;
            toggle.source = Some(RecordSource::LocalWrite);
            toggle.verified_at_ms = now;
            toggle.seq = seq;
        } else {
            toggle.last_error = Some(clip(error.unwrap_or("the write failed")));
        }
        FinishResult::Applied
    }

    pub fn force_confirm(
        &mut self,
        account: &str,
        kind: ToggleKind,
        id: &str,
        value: bool,
        now: u64,
    ) {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        let toggle = self.toggle_mut(account, kind, &key);
        toggle.pending = None;
        toggle.confirmed = Some(value);
        toggle.last_error = None;
        toggle.source = Some(RecordSource::LocalWrite);
        toggle.verified_at_ms = now;
    }

    /// Mark a row only after this process checked an OpenCLI success message.
    /// Older rows stay empty. This does not change `confirmed`.
    pub fn note_opencli_contract(&mut self, account: &str, kind: ToggleKind, id: &str) {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        let Some(toggle) = self.toggle_ref_mut(account, kind, &key) else {
            return;
        };
        if toggle.confirmed.is_some() {
            toggle.proof = Some("opencli-contract".into());
        }
    }

    pub fn record_receipt(
        &mut self,
        account: &str,
        kind: ReceiptKind,
        tweet_id: &str,
        text: &str,
        seq: u64,
        now: u64,
    ) {
        let bucket = self.bucket(&account_key(account));
        bucket.receipts.push(Receipt {
            kind,
            tweet_id: tweet_key(tweet_id),
            text: clip(text),
            seq,
            at_ms: now,
        });
    }

    pub fn confirmed(
        &self,
        account: &str,
        kind: ToggleKind,
        id: &str,
    ) -> Option<bool> {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        self.toggle_ref(account, kind, &key)
            .and_then(|toggle| toggle.confirmed)
    }

    pub fn is_saved(&self, account: &str, kind: ToggleKind, id: &str) -> bool {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        let Some(toggle) = self.toggle_ref(account, kind, &key) else {
            return false;
        };
        toggle.confirmed.is_some() && toggle.verified_at_ms < self.session_started_ms
    }

    pub fn true_ids(&self, account: &str, kind: ToggleKind) -> Vec<String> {
        let Some(bucket) = self.accounts.get(&account_key(account)) else {
            return Vec::new();
        };
        let map = match kind {
            ToggleKind::Like => &bucket.likes,
            ToggleKind::Repost => &bucket.reposts,
            ToggleKind::Follow => &bucket.follows,
        };
        let mut ids: Vec<String> = map
            .iter()
            .filter(|(_, toggle)| toggle.confirmed == Some(true))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn follow_map(&self, account: &str) -> HashMap<String, bool> {
        let Some(bucket) = self.accounts.get(&account_key(account)) else {
            return HashMap::new();
        };
        bucket
            .follows
            .iter()
            .filter_map(|(handle, toggle)| toggle.confirmed.map(|value| (handle.clone(), value)))
            .collect()
    }

    pub fn receipt_ids(&self, account: &str, kind: ReceiptKind) -> Vec<String> {
        let Some(bucket) = self.accounts.get(&account_key(account)) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = bucket
            .receipts
            .iter()
            .filter(|receipt| receipt.kind == kind && !receipt.tweet_id.is_empty())
            .map(|receipt| receipt.tweet_id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }

    pub fn pending_desired(
        &self,
        account: &str,
        kind: ToggleKind,
        id: &str,
    ) -> Option<bool> {
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        self.toggle_ref(account, kind, &key)
            .and_then(|toggle| toggle.pending.as_ref().map(|pending| pending.desired))
    }

    fn reconcile_like(
        &mut self,
        account: &str,
        id: &str,
        live: Option<bool>,
        snapshot_ms: u64,
        now: u64,
    ) {
        self.reconcile(account, ToggleKind::Like, id, live, snapshot_ms, now);
    }

    fn reconcile_follow(
        &mut self,
        account: &str,
        handle: &str,
        live: Option<bool>,
        snapshot_ms: u64,
        now: u64,
    ) {
        self.reconcile(account, ToggleKind::Follow, handle, live, snapshot_ms, now);
    }

    fn reconcile(
        &mut self,
        account: &str,
        kind: ToggleKind,
        id: &str,
        live: Option<bool>,
        snapshot_ms: u64,
        now: u64,
    ) {
        let Some(live) = live else {
            return;
        };
        let key = match kind {
            ToggleKind::Follow => account_key(id),
            _ => tweet_key(id),
        };
        if key.is_empty() || key == "signed-out" {
            return;
        }
        let toggle = self.toggle_mut(account, kind, &key);
        if toggle.pending.is_some() {
            return;
        }
        if toggle.source == Some(RecordSource::LocalWrite) && toggle.verified_at_ms > snapshot_ms {
            return;
        }
        toggle.confirmed = Some(live);
        toggle.source = Some(RecordSource::LiveRead);
        toggle.verified_at_ms = now;
        toggle.last_error = None;
        toggle.proof = None;
    }

    fn settle_abandoned(&mut self) {
        for bucket in self.accounts.values_mut() {
            for toggle in bucket
                .likes
                .values_mut()
                .chain(bucket.reposts.values_mut())
                .chain(bucket.follows.values_mut())
            {
                if toggle.pending.take().is_some() {
                    toggle.last_error = Some(
                        "Termy closed before X confirmed this. It was not marked successful."
                            .into(),
                    );
                }
            }
        }
    }

    fn bucket(&mut self, account: &str) -> &mut AccountBucket {
        self.accounts
            .entry(account_key(account))
            .or_default()
    }

    fn toggle_mut(&mut self, account: &str, kind: ToggleKind, key: &str) -> &mut ToggleState {
        let bucket = self.bucket(account);
        let map = match kind {
            ToggleKind::Like => &mut bucket.likes,
            ToggleKind::Repost => &mut bucket.reposts,
            ToggleKind::Follow => &mut bucket.follows,
        };
        map.entry(key.to_string()).or_default()
    }

    fn toggle_ref(&self, account: &str, kind: ToggleKind, key: &str) -> Option<&ToggleState> {
        let bucket = self.accounts.get(&account_key(account))?;
        let map = match kind {
            ToggleKind::Like => &bucket.likes,
            ToggleKind::Repost => &bucket.reposts,
            ToggleKind::Follow => &bucket.follows,
        };
        map.get(key)
    }

    fn toggle_ref_mut(
        &mut self,
        account: &str,
        kind: ToggleKind,
        key: &str,
    ) -> Option<&mut ToggleState> {
        let bucket = self.accounts.get_mut(&account_key(account))?;
        let map = match kind {
            ToggleKind::Like => &mut bucket.likes,
            ToggleKind::Repost => &mut bucket.reposts,
            ToggleKind::Follow => &mut bucket.follows,
        };
        map.get_mut(key)
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    account TEXT PRIMARY KEY,
    last_sync_ms INTEGER,
    last_sync_error TEXT
);
CREATE TABLE IF NOT EXISTS toggles (
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    item_key TEXT NOT NULL,
    confirmed INTEGER,
    pending_seq INTEGER,
    pending_desired INTEGER,
    pending_at_ms INTEGER,
    last_error TEXT,
    source TEXT,
    verified_at_ms INTEGER NOT NULL DEFAULT 0,
    seq INTEGER NOT NULL DEFAULT 0,
    proof TEXT,
    PRIMARY KEY (account, kind, item_key)
);
CREATE TABLE IF NOT EXISTS receipts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    tweet_id TEXT NOT NULL,
    text TEXT NOT NULL,
    seq INTEGER NOT NULL,
    at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_receipts_account ON receipts(account, kind);
";

fn legacy_json_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LEGACY_JSON_NAME)
}

fn sqlite_header_ok(path: &Path) -> Result<bool, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() < 16 {
        return Ok(false);
    }
    Ok(&bytes[..16] == b"SQLite format 3\0")
}

fn open_conn(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|error| format!("action database: {error}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|error| format!("action database: {error}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| format!("action database: {error}"))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| format!("action database: {error}"))?;
    Ok(conn)
}

fn ensure_schema(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| format!("action database: {error}"))?;
    if version != 0 && version != SCHEMA_VERSION {
        return Err(format!(
            "action database version {version} is not supported. The file was left in place."
        ));
    }
    conn.execute_batch(SCHEMA)
        .map_err(|error| format!("action database: {error}"))?;
    if !toggles_have_column(conn, "proof")? {
        conn.execute("ALTER TABLE toggles ADD COLUMN proof TEXT", [])
            .map_err(|error| format!("action database: {error}"))?;
    }
    Ok(())
}

fn toggles_have_column(conn: &Connection, name: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(toggles)")
        .map_err(|error| format!("action database: {error}"))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("action database: {error}"))?;
    for name_row in names {
        let found = name_row.map_err(|error| format!("action database: {error}"))?;
        if found == name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn read_sqlite(path: &Path) -> Result<ActionDb, String> {
    let conn = open_conn(path)?;
    ensure_schema(&conn)?;
    let mut db = ActionDb::default();
    let mut accounts = conn
        .prepare("SELECT account, last_sync_ms, last_sync_error FROM accounts")
        .map_err(|error| format!("action database: {error}"))?;
    let rows = accounts
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| format!("action database: {error}"))?;
    for row in rows {
        let (account, last_sync_ms, last_sync_error) =
            row.map_err(|error| format!("action database: {error}"))?;
        let bucket = db.bucket(&account);
        bucket.last_sync_ms = last_sync_ms.map(|ms| ms as u64);
        bucket.last_sync_error = last_sync_error;
    }
    let mut toggles = conn
        .prepare(
            "SELECT account, kind, item_key, confirmed, pending_seq, pending_desired, pending_at_ms, last_error, source, verified_at_ms, seq, proof FROM toggles",
        )
        .map_err(|error| format!("action database: {error}"))?;
    let toggle_rows = toggles
        .query_map([], |row| {
            Ok(ToggleRow {
                account: row.get(0)?,
                kind: row.get(1)?,
                item_key: row.get(2)?,
                confirmed: row.get(3)?,
                pending_seq: row.get(4)?,
                pending_desired: row.get(5)?,
                pending_at_ms: row.get(6)?,
                last_error: row.get(7)?,
                source: row.get(8)?,
                verified_at_ms: row.get(9)?,
                seq: row.get(10)?,
                proof: row.get(11)?,
            })
        })
        .map_err(|error| format!("action database: {error}"))?;
    for row in toggle_rows {
        let row = row.map_err(|error| format!("action database: {error}"))?;
        let kind = toggle_kind(&row.kind).ok_or_else(|| {
            format!("action database has an unknown toggle kind {}", row.kind)
        })?;
        let toggle = db.toggle_mut(&row.account, kind, &row.item_key);
        *toggle = row.into_state()?;
    }
    let mut receipts = conn
        .prepare("SELECT account, kind, tweet_id, text, seq, at_ms FROM receipts ORDER BY id")
        .map_err(|error| format!("action database: {error}"))?;
    let receipt_rows = receipts
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|error| format!("action database: {error}"))?;
    for row in receipt_rows {
        let (account, kind, tweet_id, text, seq, at_ms) =
            row.map_err(|error| format!("action database: {error}"))?;
        let kind = receipt_kind(&kind)
            .ok_or_else(|| format!("action database has an unknown receipt kind {kind}"))?;
        db.bucket(&account).receipts.push(Receipt {
            kind,
            tweet_id,
            text,
            seq: seq as u64,
            at_ms: at_ms as u64,
        });
    }
    Ok(db)
}

struct ToggleRow {
    account: String,
    kind: String,
    item_key: String,
    confirmed: Option<i64>,
    pending_seq: Option<i64>,
    pending_desired: Option<i64>,
    pending_at_ms: Option<i64>,
    last_error: Option<String>,
    source: Option<String>,
    verified_at_ms: i64,
    seq: i64,
    proof: Option<String>,
}

impl ToggleRow {
    fn into_state(self) -> Result<ToggleState, String> {
        let pending = match (self.pending_seq, self.pending_desired, self.pending_at_ms) {
            (Some(seq), Some(desired), Some(at_ms)) => Some(PendingWrite {
                seq: seq as u64,
                desired: desired != 0,
                at_ms: at_ms as u64,
            }),
            (None, None, None) => None,
            _ => {
                return Err(
                    "action database has a partial pending follow or like. The file was left in place."
                        .into(),
                );
            }
        };
        Ok(ToggleState {
            confirmed: self.confirmed.map(|value| value != 0),
            pending,
            last_error: self.last_error,
            source: self.source.as_deref().and_then(record_source),
            verified_at_ms: self.verified_at_ms as u64,
            seq: self.seq as u64,
            proof: self.proof.filter(|text| !text.is_empty()),
        })
    }
}

fn write_toggles(
    conn: &Connection,
    account: &str,
    kind: ToggleKind,
    map: &HashMap<String, ToggleState>,
) -> Result<(), String> {
    for (item_key, toggle) in map {
        let (pending_seq, pending_desired, pending_at_ms) = match &toggle.pending {
            Some(pending) => (
                Some(pending.seq as i64),
                Some(i64::from(pending.desired)),
                Some(pending.at_ms as i64),
            ),
            None => (None, None, None),
        };
        conn.execute(
            "INSERT INTO toggles(account, kind, item_key, confirmed, pending_seq, pending_desired, pending_at_ms, last_error, source, verified_at_ms, seq, proof) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                account,
                toggle_kind_str(kind),
                item_key,
                toggle.confirmed.map(i64::from),
                pending_seq,
                pending_desired,
                pending_at_ms,
                toggle.last_error,
                toggle.source.map(source_str),
                toggle.verified_at_ms as i64,
                toggle.seq as i64,
                toggle.proof
            ],
        )
        .map_err(|error| format!("action database: {error}"))?;
    }
    Ok(())
}

fn toggle_kind_str(kind: ToggleKind) -> &'static str {
    match kind {
        ToggleKind::Like => "like",
        ToggleKind::Repost => "repost",
        ToggleKind::Follow => "follow",
    }
}

fn toggle_kind(value: &str) -> Option<ToggleKind> {
    match value {
        "like" => Some(ToggleKind::Like),
        "repost" => Some(ToggleKind::Repost),
        "follow" => Some(ToggleKind::Follow),
        _ => None,
    }
}

fn receipt_kind_str(kind: ReceiptKind) -> &'static str {
    match kind {
        ReceiptKind::Reply => "reply",
        ReceiptKind::QTweet => "q_tweet",
        ReceiptKind::Post => "post",
    }
}

fn receipt_kind(value: &str) -> Option<ReceiptKind> {
    match value {
        "reply" => Some(ReceiptKind::Reply),
        "q_tweet" => Some(ReceiptKind::QTweet),
        "post" => Some(ReceiptKind::Post),
        _ => None,
    }
}

fn source_str(source: RecordSource) -> &'static str {
    match source {
        RecordSource::LocalWrite => "local_write",
        RecordSource::LiveRead => "live_read",
    }
}

fn record_source(value: &str) -> Option<RecordSource> {
    match value {
        "local_write" => Some(RecordSource::LocalWrite),
        "live_read" => Some(RecordSource::LiveRead),
        _ => None,
    }
}

fn walk_post(post: &Post, visit: &mut dyn FnMut(&Post)) {
    visit(post);
    if let Some(inner) = &post.retweeted {
        walk_post(inner, visit);
    }
    if let Some(inner) = &post.quoted {
        walk_post(inner, visit);
    }
}

fn clip(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(180).collect()
}

fn sync_date(ms: u64) -> String {
    crate::time::ymd((ms / 1000) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Post;

    fn post(id: &str, handle: &str, liked: Option<bool>, follows: Option<bool>) -> Post {
        Post {
            id: id.into(),
            author_handle: handle.into(),
            viewer_liked: liked,
            viewer_follows_author: follows,
            ..Post::default()
        }
    }

    #[test]
    fn restart_keeps_confirmed_follow_and_drops_abandoned_pending() {
        let dir = std::env::temp_dir().join(format!("termy-actions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        let mut db = ActionDb::default();
        db.force_confirm("Ada", ToggleKind::Follow, "@Jack", true, 50);
        db.begin_toggle("ada", ToggleKind::Like, "1", true, 1, 60);
        db.save(&path).unwrap();
        let loaded = ActionDb::load(&path).unwrap();
        assert_eq!(
            loaded.confirmed("ADA", ToggleKind::Follow, "jack"),
            Some(true)
        );
        assert_eq!(loaded.confirmed("ada", ToggleKind::Like, "1"), None);
        assert!(loaded.is_saved("ada", ToggleKind::Follow, "jack"));
        assert!(loaded.pending_desired("ada", ToggleKind::Like, "1").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_flag_overrides_saved_follow_and_silence_does_not() {
        let mut db = ActionDb::default();
        db.force_confirm("ada", ToggleKind::Follow, "jack", true, 100);
        db.reconcile("ada", ToggleKind::Follow, "jack", Some(false), 200, 250);
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(false));
        assert!(!db.is_saved("ada", ToggleKind::Follow, "jack"));
        db.reconcile("ada", ToggleKind::Follow, "jack", None, 300, 350);
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(false));
    }

    #[test]
    fn newer_local_write_beats_an_older_sync_page() {
        let mut db = ActionDb::default();
        db.session_started_ms = 0;
        let generation = db.begin_sync("ada");
        db.force_confirm("ada", ToggleKind::Like, "9", true, 400);
        let mut page = vec![post("9", "bob", Some(false), None)];
        page[0].viewer_liked = Some(false);
        let finish = db.apply_sync_page("ada", generation, &page, None, 100, 500);
        assert!(matches!(finish, SyncFinish::Report(_)));
        assert_eq!(db.confirmed("ada", ToggleKind::Like, "9"), Some(true));
    }

    #[test]
    fn account_switch_ignores_the_previous_sync_and_isolates_rows() {
        let mut db = ActionDb::default();
        db.force_confirm("ada", ToggleKind::Follow, "jack", true, 10);
        db.force_confirm("bob", ToggleKind::Follow, "jack", false, 10);
        let old = db.begin_sync("ada");
        let new = db.begin_sync("bob");
        assert_ne!(old, new);
        let page = vec![post("1", "jack", None, Some(false))];
        assert!(matches!(
            db.apply_sync_page("ada", old, &page, None, 20, 30),
            SyncFinish::Ignored
        ));
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(true));
        assert_eq!(db.confirmed("bob", ToggleKind::Follow, "jack"), Some(false));
        let applied = db.apply_sync_page("bob", new, &page, None, 20, 30);
        assert!(matches!(applied, SyncFinish::Report(_)));
        assert_eq!(db.confirmed("bob", ToggleKind::Follow, "jack"), Some(false));
    }

    #[test]
    fn failure_and_pending_do_not_become_followed() {
        let mut db = ActionDb::default();
        db.force_confirm("ada", ToggleKind::Follow, "jack", false, 1);
        db.begin_toggle("ada", ToggleKind::Follow, "jack", true, 2, 2);
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(false));
        assert_eq!(
            db.pending_desired("ada", ToggleKind::Follow, "jack"),
            Some(true)
        );
        assert_eq!(
            db.finish_toggle(
                "ada",
                ToggleKind::Follow,
                "jack",
                2,
                false,
                Some("no follow path"),
                3
            ),
            FinishResult::Applied
        );
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(false));
        assert!(db.pending_desired("ada", ToggleKind::Follow, "jack").is_none());
    }

    #[test]
    fn an_older_response_does_not_confirm_over_a_newer_request() {
        let mut db = ActionDb::default();
        db.begin_toggle("ada", ToggleKind::Like, "5", true, 1, 1);
        db.begin_toggle("ada", ToggleKind::Like, "5", false, 2, 2);
        assert_eq!(
            db.finish_toggle("ada", ToggleKind::Like, "5", 1, true, None, 3),
            FinishResult::Stale
        );
        assert_eq!(db.confirmed("ada", ToggleKind::Like, "5"), None);
        assert_eq!(
            db.finish_toggle("ada", ToggleKind::Like, "5", 2, false, Some("nope"), 4),
            FinishResult::Applied
        );
        assert_eq!(db.confirmed("ada", ToggleKind::Like, "5"), None);
    }

    #[test]
    fn offline_sync_keeps_the_saved_row_and_the_last_success() {
        let mut db = ActionDb::default();
        db.force_confirm("ada", ToggleKind::Follow, "jack", true, 10);
        let generation = db.begin_sync("ada");
        let first = db.apply_sync_page("ada", generation, &[], None, 20, 1_700_000_000_000);
        assert!(matches!(first, SyncFinish::Report(report) if report.phase == SyncPhase::Checked));
        let again = db.begin_sync("ada");
        let failed = db.apply_sync_page("ada", again, &[], Some("network down"), 30, 40);
        let SyncFinish::Report(report) = failed else {
            panic!("expected a report");
        };
        assert_eq!(report.phase, SyncPhase::Offline);
        assert!(report.summary.contains("network down"));
        assert!(report.summary.contains("saved"));
        assert_eq!(db.confirmed("ada", ToggleKind::Follow, "jack"), Some(true));
    }

    #[test]
    fn sync_page_is_capped_and_does_not_claim_full_history() {
        let mut db = ActionDb::default();
        let generation = db.begin_sync("ada");
        let posts: Vec<Post> = (0..SYNC_POST_CAP + 5)
            .map(|n| post(&n.to_string(), "zoe", Some(true), None))
            .collect();
        let SyncFinish::Report(report) = db.apply_sync_page("ada", generation, &posts, None, 1, 2)
        else {
            panic!("expected a report");
        };
        assert!(report.truncated);
        assert_eq!(report.checked, SYNC_POST_CAP);
        assert!(report.summary.contains("full like history"));
        assert!(sync_coverage().contains("does not follow"));
    }

    #[test]
    fn repost_follow_subject_is_the_original_and_a_quote_is_itself() {
        let mut repost = post("wrap", "reposter", None, None);
        repost.retweeted = Some(Box::new(post("orig", "Original", None, None)));
        assert_eq!(follow_subject(&repost), "Original");
        let mut quote = post("q", "quoter", None, None);
        quote.quoted = Some(Box::new(post("inner", "quoted", None, None)));
        assert_eq!(follow_subject(&quote), "quoter");
        assert_eq!(
            remembered_or_payload(None, Some(true)),
            Some(true),
            "a payload follow flag counts when nothing is saved"
        );
        assert_eq!(remembered_or_payload(Some(false), Some(true)), Some(false));
    }

    #[test]
    fn receipts_are_not_toggles() {
        let mut db = ActionDb::default();
        db.record_receipt("ada", ReceiptKind::QTweet, "9", "Q-tweet sent", 1, 5);
        db.record_receipt("ada", ReceiptKind::Reply, "9", "reply sent", 2, 6);
        db.record_receipt("ada", ReceiptKind::Post, "", "1 part posted", 3, 7);
        assert_eq!(db.receipt_ids("ada", ReceiptKind::Reply), vec!["9".to_string()]);
        assert_eq!(db.confirmed("ada", ToggleKind::Repost, "9"), None);
        assert!(database_location_note().contains(FILE_NAME));
        assert!(!database_location_note().to_ascii_lowercase().contains("token"));
        assert!(!follow_write_allowed(true));
        assert!(follow_write_allowed(false));
    }

    #[test]
    fn json_rows_move_into_sqlite_once_and_survive_restart() {
        let dir = std::env::temp_dir().join(format!("termy-actions-mig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mut db = ActionDb::default();
        db.force_confirm("Ada", ToggleKind::Follow, "@Jack", true, 50);
        db.record_receipt("ada", ReceiptKind::Reply, "9", "Reply sent", 2, 60);
        let json = serde_json::to_string(&db).unwrap();
        fs::write(dir.join(LEGACY_JSON_NAME), json).unwrap();
        let db_path = dir.join(FILE_NAME);
        let loaded = ActionDb::load(&db_path).unwrap();
        assert_eq!(loaded.confirmed("ada", ToggleKind::Follow, "jack"), Some(true));
        assert_eq!(loaded.receipt_ids("ada", ReceiptKind::Reply), vec!["9".to_string()]);
        assert!(!dir.join(LEGACY_JSON_NAME).exists());
        assert!(dir.join("x-action-state.json.migrated").is_file());
        let again = ActionDb::load(&db_path).unwrap();
        assert_eq!(again.receipt_ids("ada", ReceiptKind::Reply), vec!["9".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_database_is_left_in_place() {
        let dir = std::env::temp_dir().join(format!("termy-actions-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join(FILE_NAME);
        fs::write(&db_path, b"this is not sqlite").unwrap();
        let error = ActionDb::load(&db_path).unwrap_err();
        assert!(error.contains("left in place"));
        assert_eq!(fs::read(&db_path).unwrap(), b"this is not sqlite");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sqlite_columns_do_not_hold_secrets() {
        let dir = std::env::temp_dir().join(format!("termy-actions-cols-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join(FILE_NAME);
        let mut db = ActionDb::default();
        db.record_receipt("ada", ReceiptKind::Post, "", "1 part posted", 1, 5);
        db.save(&db_path).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(receipts)").unwrap();
        let names: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap().map(|name| name.unwrap()).collect();
        for banned in ["token", "password", "secret", "cookie", "authorization"] {
            assert!(!names.iter().any(|name| name.contains(banned)), "{names:?}");
        }
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(version, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_is_read_only_and_has_no_secrets() {
        let dir = std::env::temp_dir().join(format!("termy-actions-export-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join(FILE_NAME);
        let mut db = ActionDb::default();
        db.force_confirm("Jasper_Black", ToggleKind::Repost, "555", true, 1_762_000_000_000);
        db.save(&db_path).unwrap();
        let before = fs::read(&db_path).unwrap();
        let text = export_sanitized(&db_path).unwrap();
        assert_eq!(fs::read(&db_path).unwrap(), before);
        assert!(text.contains("jasper_black"));
        assert!(text.contains("repost"));
        assert!(text.contains("555"));
        assert!(text.contains("read-only"));
        let rows: String = text.lines().skip(4).collect();
        assert!(rows.contains("legacy-unverified") || rows.contains("opencli-contract"));
        assert!(!rows.to_ascii_lowercase().contains("token"));
        assert!(!rows.to_ascii_lowercase().contains("password"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_confirmed_rows_stay_unverified_and_are_not_rewritten_by_export() {
        let dir = std::env::temp_dir().join(format!("termy-actions-legacy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join(FILE_NAME);
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE toggles (
                account TEXT NOT NULL,
                kind TEXT NOT NULL,
                item_key TEXT NOT NULL,
                confirmed INTEGER,
                pending_seq INTEGER,
                pending_desired INTEGER,
                pending_at_ms INTEGER,
                last_error TEXT,
                source TEXT,
                verified_at_ms INTEGER NOT NULL DEFAULT 0,
                seq INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account, kind, item_key)
            );
            INSERT INTO toggles(account, kind, item_key, confirmed, source, verified_at_ms, seq)
            VALUES('jasper_black', 'repost', '2119037065459568858', 1, 'local_write', 1762513834000, 1);
            PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(conn);
        let before = fs::read(&db_path).unwrap();
        let text = export_sanitized(&db_path).unwrap();
        assert_eq!(fs::read(&db_path).unwrap(), before);
        assert!(text.contains("2119037065459568858"));
        assert!(text.contains("legacy-unverified"));
        assert!(!text.contains("opencli-contract"));
        let loaded = ActionDb::load(&db_path).unwrap();
        assert_eq!(
            loaded.confirmed("jasper_black", ToggleKind::Repost, "2119037065459568858"),
            Some(true)
        );
        let after = export_sanitized(&db_path).unwrap();
        assert!(after.contains("legacy-unverified"));
        assert!(!after.lines().any(|line| line.contains("2119037065459568858") && line.contains("opencli-contract")));
        let _ = fs::remove_dir_all(&dir);
    }
}
