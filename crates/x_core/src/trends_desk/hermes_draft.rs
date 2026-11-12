//! Hermes gateway drafts. The key is read here and never logged.

use crate::io::Http;

use super::agents::{HERMES_MODEL, hermes_auth, hermes_key_resolved};
use super::cards::TakeDraft;
use super::domain::SourcePost;

/// Hermes gateway draft. A 401 or 402 becomes three different fallback lines
/// and a one-line note. This does not post.
pub fn hermes_take(
    config: &crate::config::XConfig,
    http: &dyn Http,
    post: &SourcePost,
) -> TakeDraft {
    hermes_take_toned(config, http, post, "Reply")
}

pub fn hermes_take_toned(
    config: &crate::config::XConfig,
    http: &dyn Http,
    post: &SourcePost,
    tone: &str,
) -> TakeDraft {
    let mut config = config.clone();
    config.hermes_model = HERMES_MODEL.to_string();
    config.hermes_api_key = hermes_key_resolved(&config.hermes_api_key);
    let auth = hermes_auth(
        &config.hermes_base_url,
        &config.hermes_model,
        &config.hermes_api_key,
        "",
    );
    let (lines, note) =
        super::agents::draft_hermes_variants(&auth, http, &post.author, &post.text, tone, 3);
    let text = lines.first().cloned().unwrap_or_default();
    if note.is_empty() && !text.is_empty() {
        TakeDraft {
            text,
            source: "hermes",
            note: format!("Hermes draft ({}). Not posted.", auth.base),
            variants: lines,
        }
    } else {
        TakeDraft {
            text,
            source: "offline",
            note,
            variants: lines,
        }
    }
}
