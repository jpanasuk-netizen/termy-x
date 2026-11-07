//! Account-scoped memory of likes, reposts, follows, and post receipts.
//!
//! The file is JSON next to the other Termy X config (`x-action-state.json`
//! inside the termy config directory). It stores handles, tweet ids, and
//! short receipts. It does not store passwords, tokens, or API keys.
//!
//! A confirmed value is shown immediately on startup. It is labeled saved
//! until a post loaded in this session carries a real like or follow flag,
//! or until a write just succeeded. There is no poll. Sync never follows,
//! likes, or posts. A payload that omits a flag does not erase the saved
//! row and is not treated as a live reading.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::model::{Post, strip_handle};

/// Posts inspected on one login sync. One home page, not the whole account.
pub const SYNC_POST_CAP: usize = 100;

pub const FILE_NAME: &str = "x-action-state.json";

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

pub fn database_location_note() -> &'static str {
    "x-action-state.json in the Termy config directory (%APPDATA%\\termy on Windows, ~/.config/termy on Linux). Handles, tweet ids, and short receipts only."
}

impl ActionDb {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            let mut db = Self::default();
            db.session_started_ms = now_ms();
            return Ok(db);
        }
        let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut db: Self = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        db.session_started_ms = now_ms();
        db.settle_abandoned();
        Ok(db)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &text).map_err(|error| error.to_string())?;
        fs::rename(&tmp, path).or_else(|_| fs::write(path, text).map_err(|error| error.to_string()))
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
    }
}
