//! Non-interactive Termy XL commands for Grok Bot and Hermes.
//!
//! Every result says `posted: false`. Queueing a reply writes a draft in the
//! Termy XL data folder. Nothing here publishes.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use crate::drafts::{self, replace_draft_for_target};
use crate::io::{CommandRunner, Http};
use crate::model::{self, Draft};

use super::domain::{SourcePost, Topic};
use super::fetch::OpenCliDesk;
use super::store::DeskStore;
use super::{TakeProvider, draft_take, isolated_opencli_config};

pub fn topic_by_subject<'a>(topics: &'a [Topic], subject: &str) -> Option<&'a Topic> {
    let subject = subject.trim();
    let lower = subject.to_ascii_lowercase();
    topics
        .iter()
        .find(|topic| topic.id.eq_ignore_ascii_case(&lower))
        .or_else(|| {
            topics
                .iter()
                .find(|topic| topic.name.eq_ignore_ascii_case(subject))
        })
}

pub fn search_subject(data_root: &Path, subject: &str, runner: Arc<dyn CommandRunner>) -> Value {
    let store = match DeskStore::open_isolated(data_root) {
        Ok(store) => store,
        Err(error) => return fail(&error),
    };
    let topics = store.load_topics();
    let Some(topic) = topic_by_subject(&topics, subject) else {
        return fail(&format!("no Termy XL column named {subject}"));
    };
    let settings = store.load_settings();
    let desk = OpenCliDesk::new(&isolated_opencli_config(data_root), runner).with_hermes_dir(None);
    let outcome = desk.run_topic(topic, &settings.focus, &AtomicBool::new(false));
    if outcome.kind.writes_report() {
        let _ = store.write_fresh(&outcome, now_unix_fallback());
    }
    json!({
        "ok": outcome.kind != super::domain::OutcomeKind::Error,
        "posted": false,
        "subject": topic.name,
        "topic": topic.id,
        "kind": outcome.kind.as_str(),
        "message": outcome.message,
        "posts": outcome.posts.iter().map(post_json).collect::<Vec<_>>(),
    })
}

fn post_json(post: &SourcePost) -> Value {
    json!({
        "id": post.id,
        "author": post.author,
        "text": post.text,
        "url": post.url,
        "likes": post.likes,
    })
}

pub fn find_cached_post(data_root: &Path, tweet: &str) -> Result<SourcePost, String> {
    let store = DeskStore::open_isolated(data_root)?;
    let wanted = model::post_id_from_input(tweet);
    store
        .all_reports()
        .into_iter()
        .flat_map(|report| report.posts)
        .find(|post| {
            post.id == wanted
                || post.url == tweet.trim()
                || post.url.ends_with(&format!("/{wanted}"))
        })
        .ok_or_else(|| format!("tweet {wanted} is not in the Termy XL desk cache"))
}

pub fn takes_for_tweet(
    data_root: &Path,
    tweet: &str,
    provider: &str,
    n: usize,
    http: &dyn Http,
) -> Value {
    let post = match find_cached_post(data_root, tweet) {
        Ok(post) => post,
        Err(error) => return fail(&error),
    };
    let provider = TakeProvider::parse(provider);
    let mut config = isolated_opencli_config(data_root);
    if config.take_provider.trim().is_empty() {
        if let Ok(store) = DeskStore::open_isolated(data_root) {
            config.take_provider = store.load_settings().take_provider;
        }
    }
    let draft = match draft_take(provider, &config, &post, http) {
        Ok(draft) => draft,
        Err(error) => return fail(&error),
    };
    let mut variants = if draft.variants.is_empty() {
        super::distinct_variants(&post.text, &[draft.text.clone()])
    } else {
        draft.variants.clone()
    };
    let n = n.clamp(1, 3);
    variants.truncate(n);
    json!({
        "ok": true,
        "posted": false,
        "provider": provider.as_str(),
        "tweet": post.id,
        "url": post.url,
        "note": draft.note,
        "variants": variants,
    })
}

pub fn list_drafts(data_root: &Path) -> Value {
    let path = super::drafts_file(data_root);
    let drafts = drafts::load_drafts(&path);
    json!({
        "ok": true,
        "posted": false,
        "drafts": drafts.iter().map(draft_json).collect::<Vec<_>>(),
    })
}

pub fn save_draft(data_root: &Path, text: &str, tweet: Option<&str>) -> Value {
    queue_reply(data_root, tweet.unwrap_or(""), text)
}

/// Write one reply into the Termy XL draft file. Does not post.
pub fn queue_reply(data_root: &Path, tweet: &str, text: &str) -> Value {
    let text = text.trim();
    if text.is_empty() {
        return fail("draft text is empty");
    }
    if let Err(error) = DeskStore::open_isolated(data_root) {
        return fail(&error);
    }
    let reply_to = {
        let id = model::post_id_from_input(tweet);
        if id.is_empty() { None } else { Some(id) }
    };
    let draft = Draft {
        id: drafts::new_id(),
        text: text.to_string(),
        tone: Some("reply".into()),
        source: "termy-xl-queue".into(),
        created_at: drafts::timestamp(),
        reply_to: reply_to.clone(),
    };
    let path = super::drafts_file(data_root);
    match replace_draft_for_target(&path, draft) {
        Ok(kept) => json!({
            "ok": true,
            "posted": false,
            "id": kept.id,
            "text": kept.text,
            "reply_to": kept.reply_to,
        }),
        Err(error) => fail(&error),
    }
}

fn draft_json(draft: &Draft) -> Value {
    json!({
        "id": draft.id,
        "text": draft.text,
        "reply_to": draft.reply_to,
        "source": draft.source,
        "created_at": draft.created_at,
    })
}

fn fail(error: &str) -> Value {
    json!({
        "ok": false,
        "posted": false,
        "error": error,
    })
}

/// Used when a fresh search should be stamped even if the clock is odd.
pub fn now_unix_fallback() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::XConfig;
    use crate::io::{HttpRequest, ScriptedRunner};
    use crate::trends_desk::domain::outcome_from_posts;
    use serde_json::json;

    struct ScriptHttp {
        body: Result<String, String>,
        urls: std::sync::Mutex<Vec<String>>,
    }

    impl Http for ScriptHttp {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            self.urls.lock().unwrap().push(request.url);
            if request.bearer.as_deref() == Some("secret-key") {
                return Err("http 402 payment required for secret-key".into());
            }
            self.body.clone()
        }
    }

    fn sample(id: &str, text: &str) -> SourcePost {
        SourcePost::from_value(&json!({
            "id": id,
            "author": "ada",
            "text": text,
            "url": format!("https://x.com/ada/status/{id}"),
            "likes": 3
        }))
        .unwrap()
    }

    #[test]
    fn subject_match_is_by_name_or_id() {
        let topics = vec![Topic::law(), Topic::finance()];
        assert_eq!(topic_by_subject(&topics, "Law").unwrap().id, "law");
        assert_eq!(
            topic_by_subject(&topics, "finance").unwrap().name,
            "Finance"
        );
        assert!(topic_by_subject(&topics, "Sports").is_none());
    }

    #[test]
    fn search_json_does_not_post_and_queue_only_writes_a_draft() {
        let dir = std::env::temp_dir().join(format!("xl-desk-cmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let runner = ScriptedRunner::new();
        runner.script(
            "opencli",
            &["twitter", "search"],
            json!({"ok": true, "data": [{
                "id": "77",
                "author": "ada",
                "text": "Courts are testing models",
                "url": "https://x.com/ada/status/77",
                "likes": 4
            }]})
            .to_string(),
        );
        let store = DeskStore::open_isolated(&dir).unwrap();
        store.save_topics(&[Topic::law()]).unwrap();
        let searched = search_subject(&dir, "Law", Arc::new(runner));
        assert_eq!(searched["posted"], false);
        assert_eq!(searched["ok"], true);
        assert_eq!(searched["posts"][0]["id"], "77");
        assert!(!searched.to_string().contains("receipt"));
        let queued = queue_reply(&dir, "77", "A queued reply that is not a post");
        assert_eq!(queued["posted"], false);
        assert_eq!(queued["ok"], true);
        assert_eq!(queued["reply_to"], "77");
        let listed = list_drafts(&dir);
        assert_eq!(listed["posted"], false);
        assert_eq!(
            listed["drafts"][0]["text"],
            "A queued reply that is not a post"
        );
        let saved = save_draft(&dir, "second draft", Some("77"));
        assert_eq!(saved["posted"], false);
        assert_eq!(list_drafts(&dir)["drafts"].as_array().unwrap().len(), 1);
        assert_eq!(list_drafts(&dir)["drafts"][0]["text"], "second draft");
        let empty = queue_reply(&dir, "77", "  ");
        assert_eq!(empty["ok"], false);
        assert_eq!(empty["posted"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hermes_takes_are_three_distinct_lines_and_402_stays_a_fallback() {
        let dir = std::env::temp_dir().join(format!("xl-desk-takes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = XConfig::load(&dir);
        config.hermes_base_url = "http://127.0.0.1:9/v1".into();
        config.hermes_model = "hermes-agent".into();
        config.hermes_api_key = "local-key".into();
        config.take_provider = "hermes".into();
        config.save_take_provider("hermes").unwrap();
        let _ = config.save_ai("", "", "");
        // save_ai is unrelated; write hermes fields the same way the app does.
        let text = std::fs::read_to_string(dir.join("x.toml")).unwrap_or_default();
        let text = super::super::write_hermes_settings(
            &text,
            "http://127.0.0.1:9/v1",
            "hermes-agent",
            "local-key",
        );
        std::fs::write(dir.join("x.toml"), text).unwrap();
        let store = DeskStore::open_isolated(&dir).unwrap();
        let post = sample("77", "Courts are testing whether models can write opinions");
        let outcome = outcome_from_posts(&Topic::law(), "", vec![post]);
        store.write_fresh(&outcome, 1).unwrap();
        let http = ScriptHttp {
            body: Ok(
                r#"{"choices":[{"message":{"content":"same\n---\nsame\n---\nsame"}}]}"#.into(),
            ),
            urls: std::sync::Mutex::new(Vec::new()),
        };
        let body = takes_for_tweet(&dir, "https://x.com/ada/status/77", "hermes", 3, &http);
        assert_eq!(body["posted"], false);
        assert_eq!(body["provider"], "hermes");
        let lines: Vec<String> = serde_json::from_value(body["variants"].clone()).unwrap();
        assert_eq!(lines.len(), 3);
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[1], lines[2]);
        assert_ne!(lines[0], lines[2]);
        assert!(http.urls.lock().unwrap()[0].starts_with("http://127.0.0.1:9/v1/chat/completions"));
        let blocked = ScriptHttp {
            body: Err("unused".into()),
            urls: std::sync::Mutex::new(Vec::new()),
        };
        // The saved key is local-key, not secret-key, so this client is unused.
        // Swap the file key to the one the fake treats as 402.
        let text = std::fs::read_to_string(dir.join("x.toml")).unwrap();
        let text = text.replace("local-key", "secret-key");
        std::fs::write(dir.join("x.toml"), text).unwrap();
        let failed = takes_for_tweet(&dir, "77", "hermes", 3, &blocked);
        assert_eq!(failed["posted"], false);
        assert_eq!(failed["ok"], true);
        assert!(failed["note"].as_str().unwrap().contains("402"));
        assert!(!failed["note"].as_str().unwrap().contains("secret-key"));
        let lines: Vec<String> = serde_json::from_value(failed["variants"].clone()).unwrap();
        assert_eq!(lines.len(), 3);
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[1], lines[2]);
        let missing = takes_for_tweet(&dir, "999", "hermes", 3, &blocked);
        assert_eq!(missing["ok"], false);
        assert_eq!(missing["posted"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
