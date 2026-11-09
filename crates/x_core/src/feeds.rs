//! User-defined keyword/topic feeds with local filters.
//! Filters persist as JSON. Fetching is OpenCLI search; nothing posts.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::drafts::new_id;
use crate::model::{Post, strip_handle};
use crate::time::parse_created_at;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngageFeed {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub max_age_hours: Option<u32>,
    #[serde(default)]
    pub muted_words: Vec<String>,
    #[serde(default)]
    pub muted_accounts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeedStore {
    #[serde(default)]
    pub feeds: Vec<EngageFeed>,
}

pub fn load_feeds(path: &Path) -> FeedStore {
    let Ok(text) = fs::read_to_string(path) else {
        return FeedStore::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_feeds(path: &Path, store: &FeedStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(store).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn add_feed(store: &mut FeedStore, mut feed: EngageFeed) -> EngageFeed {
    if feed.id.trim().is_empty() {
        feed.id = new_id();
    }
    feed.query = feed.query.trim().to_string();
    feed.lang = feed
        .lang
        .take()
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    feed.muted_words = normalize_list(&feed.muted_words);
    feed.muted_accounts = feed
        .muted_accounts
        .iter()
        .map(|a| strip_handle(a).to_ascii_lowercase())
        .filter(|a| !a.is_empty())
        .collect();
    store.feeds.push(feed.clone());
    feed
}

pub fn remove_feed(store: &mut FeedStore, id: &str) -> bool {
    let before = store.feeds.len();
    store
        .feeds
        .retain(|feed| feed.id != id && !feed.id.starts_with(id));
    store.feeds.len() < before
}

pub fn find_feed<'a>(store: &'a FeedStore, id: &str) -> Option<&'a EngageFeed> {
    store
        .feeds
        .iter()
        .find(|feed| feed.id == id || feed.id.starts_with(id))
}

/// Query sent to OpenCLI search (language operator folded in).
pub fn search_query(feed: &EngageFeed) -> String {
    let mut q = feed.query.trim().to_string();
    if let Some(lang) = &feed.lang {
        let token = format!("lang:{lang}");
        if !q.to_ascii_lowercase().contains(&token) {
            if !q.is_empty() {
                q.push(' ');
            }
            q.push_str(&token);
        }
    }
    q
}

pub fn filter_posts(posts: &[Post], feed: &EngageFeed, now: i64) -> Vec<Post> {
    let muted_words: Vec<String> = feed
        .muted_words
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let muted_accounts: Vec<String> = feed
        .muted_accounts
        .iter()
        .map(|a| strip_handle(a).to_ascii_lowercase())
        .collect();
    let max_age = feed.max_age_hours.map(|h| i64::from(h) * 3600);
    posts
        .iter()
        .filter(|post| {
            if muted_accounts
                .iter()
                .any(|a| strip_handle(&post.author_handle).eq_ignore_ascii_case(a))
            {
                return false;
            }
            let hay = post.card_text().to_ascii_lowercase();
            if muted_words.iter().any(|w| hay.contains(w)) {
                return false;
            }
            if let Some(max) = max_age {
                match post.created_at.as_deref().and_then(parse_created_at) {
                    Some(ts) if now.saturating_sub(ts) > max => return false,
                    _ => {}
                }
            }
            true
        })
        .cloned()
        .collect()
}

fn normalize_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect()
}

/// Parse a compact filter line: `lang:en age:24 mute:spam,bot @acct`.
pub fn parse_filter_line(line: &str) -> (Option<String>, Option<u32>, Vec<String>, Vec<String>) {
    let mut lang = None;
    let mut age = None;
    let mut words = Vec::new();
    let mut accounts = Vec::new();
    for token in line.split_whitespace() {
        if let Some(value) = token.strip_prefix("lang:") {
            lang = Some(value.trim().to_ascii_lowercase()).filter(|s| !s.is_empty());
        } else if let Some(value) = token.strip_prefix("age:") {
            age = value.parse().ok();
        } else if let Some(value) = token.strip_prefix("mute:") {
            words.extend(normalize_list(&[value.to_string()]));
        } else if let Some(handle) = token.strip_prefix('@')
            && !handle.is_empty()
        {
            accounts.push(handle.to_string());
        }
    }
    (lang, age, words, accounts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::sample_posts;

    #[test]
    fn persist_add_remove_and_search_query() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-feeds.json");
        let mut store = FeedStore::default();
        let feed = add_feed(
            &mut store,
            EngageFeed {
                id: String::new(),
                query: " rust lang ".into(),
                lang: Some(" EN ".into()),
                max_age_hours: Some(24),
                muted_words: vec!["spam".into(), " bot,ads ".into()],
                muted_accounts: vec!["@Nuisance".into()],
            },
        );
        assert!(!feed.id.is_empty());
        assert_eq!(feed.lang.as_deref(), Some("en"));
        assert_eq!(search_query(&feed), "rust lang lang:en");
        save_feeds(&path, &store).unwrap();
        let loaded = load_feeds(&path);
        assert_eq!(loaded.feeds.len(), 1);
        assert!(remove_feed(&mut store, &feed.id[..4]));
        assert!(store.feeds.is_empty());
    }

    #[test]
    fn filter_drops_muted_and_old() {
        let mut posts = sample_posts();
        posts[0].created_at = Some("2026-10-06T13:00:00Z".into());
        posts[1].created_at = Some("2020-01-01T00:00:00Z".into());
        posts[1].text = "this is spam from a bot".into();
        posts[2].author_handle = "nuisance".into();
        let feed = EngageFeed {
            id: "f1".into(),
            query: "panel".into(),
            lang: None,
            max_age_hours: Some(48),
            muted_words: vec!["spam".into()],
            muted_accounts: vec!["nuisance".into()],
        };
        let now = crate::time::parse_created_at("2026-10-06T14:00:00Z").unwrap();
        let kept = filter_posts(&posts, &feed, now);
        let ids: Vec<&str> = kept.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"1001"));
        assert!(!ids.contains(&"1002"), "muted word");
        assert!(!ids.contains(&"1003"), "muted account");
    }

    #[test]
    fn filter_line_parses_lang_age_mute_and_handles() {
        let (lang, age, words, accounts) =
            parse_filter_line("lang:en age:12 mute:spam,bot @Nuisance");
        assert_eq!(lang.as_deref(), Some("en"));
        assert_eq!(age, Some(12));
        assert_eq!(words, vec!["spam".to_string(), "bot".to_string()]);
        assert_eq!(accounts, vec!["Nuisance".to_string()]);
    }
}
