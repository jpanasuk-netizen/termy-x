//! Local Trends Desk files under the Termy config directory.
//!
//! Reports are written only for a fresh OpenCLI run. History is never deleted
//! by a failed run, and nothing here is a credential.

use std::fs;
use std::path::{Path, PathBuf};

use super::domain::{
    DeskSettings, OutcomeKind, RunOutcome, StoredReport, Topic, default_topics, topic_id_ok,
};

pub const DIR_NAME: &str = "trends-desk";

#[derive(Debug, Clone)]
pub struct DeskStore {
    root: PathBuf,
}

impl DeskStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            root: config_dir.join(DIR_NAME),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load_settings(&self) -> DeskSettings {
        let path = self.root.join("settings.json");
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &DeskSettings) -> Result<(), String> {
        self.ensure()?;
        write_json(&self.root.join("settings.json"), settings)
    }

    pub fn load_topics(&self) -> Vec<Topic> {
        let path = self.root.join("topics.json");
        let Some(text) = fs::read_to_string(path).ok() else {
            return default_topics();
        };
        serde_json::from_str(&text).unwrap_or_else(|_| default_topics())
    }

    pub fn save_topics(&self, topics: &[Topic]) -> Result<(), String> {
        if topics.is_empty() {
            return Err("keep at least one column".into());
        }
        if topics.iter().any(|topic| !topic_id_ok(&topic.id)) {
            return Err("column id must be a short lowercase slug".into());
        }
        self.ensure()?;
        write_json(&self.root.join("topics.json"), &topics)
    }

    pub fn latest(&self, topic_id: &str) -> Option<StoredReport> {
        self.history(topic_id).into_iter().next()
    }

    pub fn history(&self, topic_id: &str) -> Vec<StoredReport> {
        if !topic_id_ok(topic_id) {
            return Vec::new();
        }
        let mut reports = self.all_reports();
        reports.retain(|report| report.topic_id == topic_id);
        reports
    }

    pub fn all_reports(&self) -> Vec<StoredReport> {
        let dir = self.root.join("reports");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut reports = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(report) = serde_json::from_str::<StoredReport>(&text) {
                reports.push(report);
            }
        }
        reports.sort_by_key(|report| std::cmp::Reverse(report.fetched_at_unix));
        reports
    }

    /// Persist a fresh run. Empty and error outcomes are refused.
    pub fn write_fresh(
        &self,
        outcome: &RunOutcome,
        fetched_at_unix: i64,
    ) -> Result<StoredReport, String> {
        if outcome.kind != OutcomeKind::Fresh {
            return Err("only a fresh OpenCLI run is saved".into());
        }
        if !topic_id_ok(&outcome.topic_id) {
            return Err("refusing to write a report outside the desk directory".into());
        }
        if outcome.posts.is_empty() || !outcome.markdown.contains("https://x.com/") {
            return Err("refusing to save a report without a source link".into());
        }
        self.ensure()?;
        let file = format!("{}-{fetched_at_unix}.json", outcome.topic_id);
        let report = StoredReport {
            file: file.clone(),
            topic_id: outcome.topic_id.clone(),
            fetched_at_unix,
            query: outcome.query.clone(),
            focus: outcome.focus.clone(),
            markdown: outcome.markdown.clone(),
            posts: outcome.posts.clone(),
        };
        let path = self.root.join("reports").join(&file);
        write_json(&path, &report)?;
        let markdown_path = self
            .root
            .join("reports")
            .join(format!("{}-{fetched_at_unix}.md", outcome.topic_id));
        fs::write(markdown_path, &outcome.markdown).map_err(|error| error.to_string())?;
        Ok(report)
    }

    fn ensure(&self) -> Result<(), String> {
        fs::create_dir_all(self.root.join("reports")).map_err(|error| error.to_string())
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends_desk::domain::{Topic, outcome_error, outcome_from_posts};
    use serde_json::json;

    fn sample_outcome() -> RunOutcome {
        let post = crate::trends_desk::domain::SourcePost::from_value(&json!({
            "id": "1",
            "author": "alice",
            "text": "courts and AI",
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "url": "https://x.com/alice/status/1"
        }))
        .unwrap();
        outcome_from_posts(&Topic::law(), "AI", vec![post])
    }

    #[test]
    fn fresh_run_is_saved_and_reloads_as_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeskStore::new(dir.path());
        let outcome = sample_outcome();
        let saved = store.write_fresh(&outcome, 1_700_000_000).unwrap();
        assert_eq!(saved.topic_id, "law");
        let loaded = store.latest("law").unwrap();
        assert_eq!(
            loaded.posts[0].created_at.as_deref(),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert_eq!(loaded.posts[0].url, "https://x.com/alice/status/1");
        assert!(loaded.markdown.contains("https://x.com/alice/status/1"));
    }

    #[test]
    fn failure_does_not_write_or_replace_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeskStore::new(dir.path());
        let fresh = sample_outcome();
        store.write_fresh(&fresh, 10).unwrap();
        let error = outcome_error(&Topic::law(), "AI", "OpenCLI search returned no posts");
        assert!(store.write_fresh(&error, 11).is_err());
        let empty = outcome_from_posts(&Topic::law(), "AI", Vec::new());
        assert!(store.write_fresh(&empty, 12).is_err());
        let history = store.history("law");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].fetched_at_unix, 10);
        assert!(!history[0].markdown.starts_with("# Fixture"));
    }

    #[test]
    fn preferences_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeskStore::new(dir.path());
        let mut settings = DeskSettings::default();
        settings.focus = "Finance".into();
        settings.reply_as = "@YourHandle".into();
        settings.daily_at = "06:30".into();
        store.save_settings(&settings).unwrap();
        let loaded = store.load_settings();
        assert_eq!(loaded.focus, "Finance");
        assert_eq!(loaded.reply_as, "@YourHandle");
        assert_eq!(loaded.daily_at, "06:30");
        let topics = vec![
            Topic::law(),
            Topic {
                id: "custom".into(),
                name: "Custom".into(),
                desc: "local column".into(),
            },
        ];
        store.save_topics(&topics).unwrap();
        assert_eq!(store.load_topics().len(), 2);
        assert!(
            store
                .save_topics(&[Topic {
                    id: "../escape".into(),
                    name: "nope".into(),
                    desc: "nope".into(),
                }])
                .is_err()
        );
    }
}
