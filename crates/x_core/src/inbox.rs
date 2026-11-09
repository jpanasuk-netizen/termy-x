//! Reply inbox: mentions/replies that still need a response.
//! Waiting/Done is local JSON. Nothing here posts.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::{Post, strip_handle};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InboxState {
    #[default]
    Waiting,
    Done,
}

impl InboxState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Done => "done",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "waiting" | "open" | "todo" => Some(Self::Waiting),
            "done" | "closed" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InboxStore {
    #[serde(default)]
    pub items: HashMap<String, InboxState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    pub post: Post,
    pub state: InboxState,
}

pub fn load_inbox(path: &Path) -> InboxStore {
    let Ok(text) = fs::read_to_string(path) else {
        return InboxStore::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_inbox(path: &Path, store: &InboxStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(store).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn mark(store: &mut InboxStore, id: &str, state: InboxState) {
    let id = id.trim();
    if id.is_empty() {
        return;
    }
    store.items.insert(id.to_string(), state);
}

pub fn state_of(store: &InboxStore, id: &str) -> InboxState {
    store.items.get(id).copied().unwrap_or(InboxState::Waiting)
}

pub fn apply_states(posts: &[Post], store: &InboxStore) -> Vec<InboxItem> {
    posts
        .iter()
        .cloned()
        .map(|post| InboxItem {
            state: state_of(store, &post.id),
            post,
        })
        .collect()
}

pub fn filter_items(items: &[InboxItem], want: Option<InboxState>) -> Vec<InboxItem> {
    match want {
        None => items.to_vec(),
        Some(state) => items
            .iter()
            .filter(|item| item.state == state)
            .cloned()
            .collect(),
    }
}

/// OpenCLI search for replies directed at the connected account.
pub fn to_me_query(me: &str) -> String {
    format!("to:{}", strip_handle(me))
}

/// OpenCLI search for posts that mention the connected account.
pub fn mentions_query(me: &str) -> String {
    format!("@{}", strip_handle(me))
}

/// Merge notification rows with `to:me` search, newest first, unique by id.
pub fn merge_inbox_posts(notifications: Vec<Post>, directed: Vec<Post>) -> Vec<Post> {
    let mut posts =
        crate::providers::dedupe_posts(notifications.into_iter().chain(directed).collect());
    crate::time::sort_newest(&mut posts, |post| post.created_at.as_deref());
    posts
}

/// Draft idea for the existing `ai::draft_variants` path (Tone::Reply).
pub fn draft_idea(post: &Post) -> String {
    let handle = strip_handle(&post.author_handle);
    let text = post.card_text().trim();
    if text.is_empty() {
        format!("reply to @{handle}")
    } else {
        format!("reply to @{handle}: {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::sample_posts;

    #[test]
    fn queries_strip_at_and_round_trip_store() {
        assert_eq!(to_me_query("@Ada"), "to:Ada");
        assert_eq!(mentions_query("Ada"), "@Ada");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-inbox.json");
        let mut store = InboxStore::default();
        mark(&mut store, "1001", InboxState::Done);
        mark(&mut store, "", InboxState::Done);
        save_inbox(&path, &store).unwrap();
        let loaded = load_inbox(&path);
        assert_eq!(state_of(&loaded, "1001"), InboxState::Done);
        assert_eq!(state_of(&loaded, "nope"), InboxState::Waiting);
        assert_eq!(InboxState::parse("DONE"), Some(InboxState::Done));
        assert!(InboxState::parse("maybe").is_none());
    }

    #[test]
    fn apply_and_filter_waiting_vs_done() {
        let posts = sample_posts();
        let mut store = InboxStore::default();
        mark(&mut store, &posts[0].id, InboxState::Done);
        let items = apply_states(&posts, &store);
        assert_eq!(items[0].state, InboxState::Done);
        assert_eq!(items[1].state, InboxState::Waiting);
        assert_eq!(filter_items(&items, Some(InboxState::Done)).len(), 1);
        assert_eq!(
            filter_items(&items, Some(InboxState::Waiting)).len(),
            items.len() - 1
        );
        assert_eq!(filter_items(&items, None).len(), items.len());
    }

    #[test]
    fn merge_dedupes_and_orders_newest() {
        let mut a = sample_posts();
        let mut b = sample_posts();
        a[0].created_at = Some("2026-10-01T00:00:00Z".into());
        a[1].created_at = Some("2026-10-06T00:00:00Z".into());
        b.truncate(2);
        let merged = merge_inbox_posts(a, b);
        assert!(merged.len() >= 2);
        let ids: Vec<&str> = merged.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|id| **id == "1001").count(), 1);
        let pos_1002 = ids.iter().position(|id| *id == "1002").expect("1002");
        let pos_1001 = ids.iter().position(|id| *id == "1001").expect("1001");
        assert!(pos_1002 < pos_1001, "newer 1002 sorts before 1001");
    }

    #[test]
    fn draft_idea_names_the_author() {
        let post = &sample_posts()[0];
        let idea = draft_idea(post);
        assert!(idea.contains("@termy"));
        assert!(idea.contains("Termy X"));
    }
}
