//! Follower snapshots. A snapshot is taken only on an explicit button /
//! CLI call. Diff is latest vs previous. Nothing follows or unfollows.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::strip_handle;
use crate::time::unix_now;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Follower {
    pub handle: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowerSnapshot {
    pub taken_at: String,
    pub handle: String,
    pub followers: Vec<Follower>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FollowerLog {
    #[serde(default)]
    pub snapshots: Vec<FollowerSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowerDiff {
    pub handle: String,
    pub latest_at: String,
    pub previous_at: Option<String>,
    pub new: Vec<Follower>,
    pub gone: Vec<Follower>,
    pub latest_count: usize,
    pub previous_count: usize,
}

pub fn load_log(path: &Path) -> FollowerLog {
    let Ok(text) = fs::read_to_string(path) else {
        return FollowerLog::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_log(path: &Path, log: &FollowerLog) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(log).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn append_snapshot(log: &mut FollowerLog, snapshot: FollowerSnapshot) {
    log.snapshots.push(snapshot);
}

pub fn make_snapshot(
    handle: &str,
    followers: Vec<Follower>,
    taken_at: Option<i64>,
) -> FollowerSnapshot {
    FollowerSnapshot {
        taken_at: taken_at.unwrap_or_else(unix_now).to_string(),
        handle: strip_handle(handle).to_string(),
        followers,
    }
}

pub fn diff_latest(log: &FollowerLog) -> Option<FollowerDiff> {
    let latest = log.snapshots.last()?;
    let previous = if log.snapshots.len() >= 2 {
        log.snapshots.get(log.snapshots.len() - 2)
    } else {
        None
    };
    Some(diff_pair(latest, previous))
}

pub fn diff_pair(latest: &FollowerSnapshot, previous: Option<&FollowerSnapshot>) -> FollowerDiff {
    let latest_set: HashSet<&str> = latest.followers.iter().map(|f| f.handle.as_str()).collect();
    let prev_set: HashSet<&str> = previous
        .map(|p| p.followers.iter().map(|f| f.handle.as_str()).collect())
        .unwrap_or_default();
    let new: Vec<Follower> = latest
        .followers
        .iter()
        .filter(|f| !prev_set.contains(f.handle.as_str()))
        .cloned()
        .collect();
    let gone: Vec<Follower> = previous
        .map(|p| {
            p.followers
                .iter()
                .filter(|f| !latest_set.contains(f.handle.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    FollowerDiff {
        handle: latest.handle.clone(),
        latest_at: latest.taken_at.clone(),
        previous_at: previous.map(|p| p.taken_at.clone()),
        latest_count: latest.followers.len(),
        previous_count: previous.map_or(0, |p| p.followers.len()),
        new,
        gone,
    }
}

/// Parse OpenCLI `twitter followers -f json` (array or `{data:[…]}`).
pub fn parse_followers(text: &str) -> Result<Vec<Follower>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let rows = array_of(&value);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in rows {
        let Some(handle) = first_str(item, &["username", "screen_name", "screenName", "handle"])
        else {
            continue;
        };
        let handle = strip_handle(handle).to_string();
        if handle.is_empty() || !seen.insert(handle.clone()) {
            continue;
        }
        let name = first_str(item, &["name", "display_name"])
            .unwrap_or(&handle)
            .to_string();
        out.push(Follower { handle, name });
    }
    Ok(out)
}

fn array_of(value: &Value) -> Vec<&Value> {
    if let Some(rows) = value.as_array() {
        return rows.iter().collect();
    }
    for key in ["data", "users", "followers"] {
        if let Some(rows) = value.get(key).and_then(Value::as_array) {
            return rows.iter().collect();
        }
    }
    if value.is_object() {
        vec![value]
    } else {
        Vec::new()
    }
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(Value::as_str) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(handle: &str) -> Follower {
        Follower {
            handle: handle.into(),
            name: handle.into(),
        }
    }

    #[test]
    fn snapshot_diff_finds_new_and_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-followers.json");
        let mut log = FollowerLog::default();
        append_snapshot(
            &mut log,
            make_snapshot("me", vec![f("ada"), f("bob")], Some(100)),
        );
        append_snapshot(
            &mut log,
            make_snapshot("me", vec![f("bob"), f("cam")], Some(200)),
        );
        save_log(&path, &log).unwrap();
        let loaded = load_log(&path);
        let diff = diff_latest(&loaded).unwrap();
        assert_eq!(
            diff.new
                .iter()
                .map(|x| x.handle.as_str())
                .collect::<Vec<_>>(),
            vec!["cam"]
        );
        assert_eq!(
            diff.gone
                .iter()
                .map(|x| x.handle.as_str())
                .collect::<Vec<_>>(),
            vec!["ada"]
        );
        assert_eq!(diff.latest_count, 2);
        assert_eq!(diff.previous_count, 2);
        assert_eq!(diff.previous_at.as_deref(), Some("100"));
    }

    #[test]
    fn first_snapshot_lists_everyone_as_new() {
        let snap = make_snapshot("me", vec![f("ada")], Some(1));
        let diff = diff_pair(&snap, None);
        assert_eq!(diff.new.len(), 1);
        assert!(diff.gone.is_empty());
        assert!(diff.previous_at.is_none());
    }

    #[test]
    fn parse_followers_accepts_array_and_data_wrap() {
        let rows =
            parse_followers(r#"[{"username":"@Ada","name":"Ada Lovelace"},{"screen_name":"bob"}]"#)
                .unwrap();
        assert_eq!(rows[0].handle, "Ada");
        assert_eq!(rows[0].name, "Ada Lovelace");
        assert_eq!(rows[1].handle, "bob");
        let wrapped =
            parse_followers(r#"{"data":[{"handle":"cam","display_name":"Cam"}]}"#).unwrap();
        assert_eq!(wrapped[0].handle, "cam");
        assert!(parse_followers("").unwrap().is_empty());
        assert!(parse_followers("not json").is_err());
    }
}
