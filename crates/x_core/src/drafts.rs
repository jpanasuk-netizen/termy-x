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

pub fn find_draft<'a>(drafts: &'a [Draft], id: &str) -> Option<&'a Draft> {
    drafts.iter().find(|draft| draft.id == id || draft.id.starts_with(id))
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string().replace('-', "")[..8].to_string()
}

pub fn timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}
