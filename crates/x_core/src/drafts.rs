use std::fs;
use std::path::Path;

use crate::model::Draft;

pub fn load_drafts(path: &Path) -> Vec<Draft> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_drafts(path: &Path, drafts: &[Draft]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(drafts).map_err(|error| error.to_string())?;
    fs::write(path, text).map_err(|error| error.to_string())
}

pub fn upsert_drafts(path: &Path, new_drafts: Vec<Draft>) -> Result<Vec<Draft>, String> {
    let mut drafts = load_drafts(path);
    drafts.extend(new_drafts);
    save_drafts(path, &drafts)?;
    Ok(drafts)
}

/// One draft per reply target. A second save for the same post replaces the text.
pub fn replace_draft_for_target(path: &Path, draft: Draft) -> Result<Draft, String> {
    let mut drafts = load_drafts(path);
    if let Some(target) = draft.reply_to.clone().filter(|value| !value.is_empty())
        && let Some(existing) = drafts
            .iter_mut()
            .find(|item| item.reply_to.as_ref() == Some(&target))
    {
        existing.text = draft.text;
        existing.tone = draft.tone;
        existing.created_at = draft.created_at;
        let kept = existing.clone();
        save_drafts(path, &drafts)?;
        return Ok(kept);
    }
    let kept = draft.clone();
    drafts.push(draft);
    save_drafts(path, &drafts)?;
    Ok(kept)
}

pub fn find_draft<'a>(drafts: &'a [Draft], id: &str) -> Option<&'a Draft> {
    drafts
        .iter()
        .find(|draft| draft.id == id || draft.id.starts_with(id))
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
}

pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format!("{secs}")
}
