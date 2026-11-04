//! Local reply queue. Each item is sent only when the user clicks Send
//! (GUI confirm screen) or types yes (CLI). Nothing auto-posts.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::drafts::{new_id, timestamp};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedReply {
    pub id: String,
    pub text: String,
    pub reply_to: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub source_text: Option<String>,
}

pub fn load_queue(path: &Path) -> Vec<QueuedReply> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_queue(path: &Path, items: &[QueuedReply]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(items).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn enqueue(items: &mut Vec<QueuedReply>, mut item: QueuedReply) -> Result<(), String> {
    item.text = item.text.trim().to_string();
    item.reply_to = item.reply_to.trim().to_string();
    if item.text.is_empty() {
        return Err("queue item has no text".into());
    }
    if item.reply_to.is_empty() {
        return Err("queue item needs a reply_to tweet id".into());
    }
    if item.id.trim().is_empty() {
        item.id = new_id();
    }
    if item.created_at.trim().is_empty() {
        item.created_at = timestamp();
    }
    items.push(item);
    Ok(())
}

pub fn make_item(
    text: &str,
    reply_to: &str,
    author: &str,
    source_text: Option<String>,
) -> QueuedReply {
    QueuedReply {
        id: new_id(),
        text: text.trim().to_string(),
        reply_to: reply_to.trim().to_string(),
        author: crate::model::strip_handle(author).to_string(),
        created_at: timestamp(),
        source_text,
    }
}

pub fn find_item<'a>(items: &'a [QueuedReply], id: &str) -> Option<&'a QueuedReply> {
    items
        .iter()
        .find(|item| item.id == id || item.id.starts_with(id))
}

pub fn remove_item(items: &mut Vec<QueuedReply>, id: &str) -> Option<QueuedReply> {
    let idx = items
        .iter()
        .position(|item| item.id == id || item.id.starts_with(id))?;
    Some(items.remove(idx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_persist_find_remove() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-reply-queue.json");
        let mut items = Vec::new();
        let item = make_item("thanks", "1001", "@ada", Some("hello".into()));
        enqueue(&mut items, item.clone()).unwrap();
        save_queue(&path, &items).unwrap();
        let loaded = load_queue(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(find_item(&loaded, &item.id[..4]).unwrap().text, "thanks");
        let mut loaded = loaded;
        assert!(remove_item(&mut loaded, &item.id).is_some());
        assert!(loaded.is_empty());
        assert!(enqueue(&mut items, make_item("  ", "1", "a", None)).is_err());
        assert!(enqueue(&mut items, make_item("hi", "", "a", None)).is_err());
    }
}
