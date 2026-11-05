//! Reply Guy: fresh X posts from Treg (the user's own Treg token) plus
//! Grok-drafted reply options (the user's own xAI key). Drafts only:
//! posting always goes through the X panel's Confirm screen.
//!
//! Auth, both official and per user (no shared house key):
//! - Treg: the user's own token from `treg login` or the dashboard's
//!   API keys page, sent as `X-Treg-Token` (Treg's documented header).
//! - Grok: the user's own xAI API key from console.x.ai, sent as a
//!   Bearer token to the OpenAI-compatible `https://api.x.ai/v1`.
//!
//! Secrets live in the OS keychain (service `termy-x`), else a private
//! file in the config dir. They are never written to x.toml or logged.

use std::fs;

use crate::config::XConfig;
use crate::io::{Http, HttpRequest};
use crate::model::Post;

pub const TREG_BASE: &str = "https://treg.to";
/// Where a user signs in to Treg and creates their own API key.
pub const TREG_KEYS_URL: &str = "https://treg.to/login";
/// Treg catalog tool `anyapi.x.search.posts`, called by id. (The
/// `/call/<upstream url>` form only works for a team's own tools; for a
/// catalog tool it returns 404 `{"detail":"Not Found"}`.)
pub const TREG_SEARCH_URL: &str = "https://treg.to/call/anyapi.x.search.posts";
/// Treg catalog tool `anyapi.twitter.trends`, called by id.
pub const TREG_TRENDS_URL: &str = "https://treg.to/call/anyapi.twitter.trends";
/// Free identity check for a Treg token.
pub const TREG_ME_URL: &str = "https://treg.to/auth/me";
/// Catalog cost blocks (p90 observed, USD per successful call). The
/// response's own `costUsd` is shown instead whenever it is present.
pub const TREG_SEARCH_COST_USD: f64 = 0.00075;
pub const TREG_TRENDS_COST_USD: f64 = 0.00075;

pub const XAI_BASE_URL: &str = "https://api.x.ai/v1";
/// Where a user signs in to xAI and creates their own API key.
pub const XAI_KEYS_URL: &str = "https://console.x.ai/";
pub const DEFAULT_GROK_MODEL: &str = "grok-4";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    Grok,
    Treg,
}

impl Secret {
    fn account(self) -> &'static str {
        match self {
            Self::Grok => "grok-api-key",
            Self::Treg => "treg-token",
        }
    }
    fn env_vars(self) -> &'static [&'static str] {
        match self {
            Self::Grok => &["TERMY_X_GROK_API_KEY", "XAI_API_KEY"],
            Self::Treg => &["TERMY_X_TREG_TOKEN", "TREG_TOKEN"],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Grok => "Grok (xAI) API key",
            Self::Treg => "Treg API key",
        }
    }
    pub fn sign_in_url(self) -> &'static str {
        match self {
            Self::Grok => XAI_KEYS_URL,
            Self::Treg => TREG_KEYS_URL,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Popular,
    Newest,
}

impl Sort {
    pub fn query_type(self) -> &'static str {
        match self {
            Self::Popular => "Top",
            Self::Newest => "Latest",
        }
    }
}

fn secrets_path(config: &XConfig) -> std::path::PathBuf {
    config.config_dir.join("reply-guy-secrets.json")
}

fn read_file_secrets(config: &XConfig) -> serde_json::Map<String, serde_json::Value> {
    fs::read_to_string(secrets_path(config))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// Saves a secret. Keychain first (when `token_store = "keychain"`),
/// else a private file. Never prints the value.
pub fn store_secret(config: &XConfig, which: Secret, value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty key".into());
    }
    if config.token_store == "keychain"
        && let Ok(entry) = keyring::Entry::new("termy-x", which.account())
        && entry.set_password(value).is_ok()
        && entry.get_password().is_ok_and(|stored| stored == value)
    {
        return Ok(());
    }
    let mut map = read_file_secrets(config);
    map.insert(
        which.account().into(),
        serde_json::Value::String(value.into()),
    );
    let json = serde_json::Value::Object(map).to_string();
    crate::auth::write_private_file(&secrets_path(config), &json)
}

pub fn load_secret(config: &XConfig, which: Secret) -> Option<String> {
    for var in which.env_vars() {
        if let Ok(value) = std::env::var(var)
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_string());
        }
    }
    if config.token_store == "keychain"
        && let Ok(entry) = keyring::Entry::new("termy-x", which.account())
        && let Ok(value) = entry.get_password()
        && !value.trim().is_empty()
    {
        return Some(value);
    }
    read_file_secrets(config)
        .get(which.account())
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

pub fn clear_secret(config: &XConfig, which: Secret) -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new("termy-x", which.account()) {
        let _ = entry.delete_credential();
    }
    let mut map = read_file_secrets(config);
    if map.remove(which.account()).is_some() {
        let json = serde_json::Value::Object(map).to_string();
        crate::auth::write_private_file(&secrets_path(config), &json)?;
    }
    Ok(())
}

/// Display form of a secret: never more than the last 4 characters.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.trim().chars().collect();
    if chars.len() <= 8 {
        return "\u{2022}\u{2022}\u{2022}\u{2022}".into();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("\u{2022}\u{2022}\u{2022}\u{2022}{tail}")
}

/// Off until both keys are set.
pub fn ready(grok: Option<&str>, treg: Option<&str>) -> Result<(), String> {
    match (grok.is_some(), treg.is_some()) {
        (true, true) => Ok(()),
        (false, true) => Err("Add your Grok (xAI) API key to turn Reply Guy on.".into()),
        (true, false) => Err("Add your Treg API key to turn Reply Guy on.".into()),
        (false, false) => {
            Err("Reply Guy is off. Add your Treg and Grok (xAI) keys to turn it on.".into())
        }
    }
}

fn treg_request(method: &'static str, url: &str, token: &str, body: Option<String>) -> HttpRequest {
    HttpRequest {
        method,
        url: url.to_string(),
        body,
        bearer: None,
        content_type: Some("application/json".into()),
        headers: vec![("X-Treg-Token".into(), token.to_string())],
    }
}

/// Free check: does Treg accept this token?
pub fn test_treg(http: &dyn Http, token: &str) -> Result<String, String> {
    let body = http.request(treg_request("GET", TREG_ME_URL, token, None))?;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let who = ["/email", "/user/email", "/name", "/user/name", "/login"]
        .iter()
        .find_map(|path| value.pointer(path).and_then(|v| v.as_str()))
        .unwrap_or("connected");
    Ok(format!("Treg OK \u{b7} {who}"))
}

/// Free check: does xAI accept this key? Lists models, generates nothing.
pub fn test_grok(http: &dyn Http, key: &str) -> Result<String, String> {
    let body = http.request(HttpRequest {
        method: "GET",
        url: format!("{XAI_BASE_URL}/models"),
        body: None,
        bearer: Some(key.to_string()),
        content_type: None,
        headers: Vec::new(),
    })?;
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let count = value
        .get("data")
        .and_then(|d| d.as_array())
        .map_or(0, Vec::len);
    Ok(format!("Grok OK \u{b7} {count} models"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fetched {
    pub posts: Vec<Post>,
    /// Exact charge from the response when Treg/AnyAPI reports it.
    pub cost_usd: Option<f64>,
}

/// Fresh posts for a topic/niche. One Treg call (~$0.00075).
pub fn fetch_posts(
    http: &dyn Http,
    token: &str,
    topic: &str,
    sort: Sort,
    limit: usize,
) -> Result<Fetched, String> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err("Type a topic or niche first.".into());
    }
    let body = serde_json::json!({
        "query": format!("{topic} -filter:replies lang:en"),
        "limit": limit.clamp(1, 50),
        "queryType": sort.query_type(),
    })
    .to_string();
    let response = http.request(treg_request("POST", TREG_SEARCH_URL, token, Some(body)))?;
    parse_search(&response)
}

/// Treg/provider error text when a body is not a result envelope
/// (`{"detail":"Not Found"}`, `{"error":...}`), so a failed call is shown
/// as an error and never as "no posts".
fn envelope_error(value: &serde_json::Value) -> Option<String> {
    if value
        .pointer("/output/data/items")
        .is_some_and(|v| v.is_array())
    {
        return None;
    }
    let text = ["/detail", "/error/message", "/error", "/message", "/hint"]
        .iter()
        .find_map(|path| {
            value.pointer(path).and_then(|v| match v {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Null => None,
                other => Some(other.to_string()),
            })
        })
        .unwrap_or_else(|| "unexpected response shape".into());
    Some(format!(
        "treg: {}",
        text.chars().take(300).collect::<String>()
    ))
}

pub fn parse_search(body: &str) -> Result<Fetched, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("treg: bad response ({e})"))?;
    if let Some(error) = envelope_error(&value) {
        return Err(error);
    }
    let items = value
        .pointer("/output/data/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let posts = items.iter().filter_map(item_to_post).collect();
    Ok(Fetched {
        posts,
        cost_usd: value.get("costUsd").and_then(|v| v.as_f64()),
    })
}

fn item_to_post(item: &serde_json::Value) -> Option<Post> {
    let s = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    let n = |key: &str| item.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let id = s("id");
    if id.is_empty() {
        return None;
    }
    let handle = s("authorUsername");
    Some(Post {
        id: id.to_string(),
        url: if s("url").is_empty() {
            format!("https://x.com/{handle}/status/{id}")
        } else {
            s("url").to_string()
        },
        author_handle: handle.to_string(),
        author_name: s("authorName").to_string(),
        text: s("text").to_string(),
        created_at: item
            .get("createdUtc")
            .and_then(|v| v.as_u64())
            .map(|t| t.to_string()),
        likes: n("likeCount"),
        reposts: n("retweetCount"),
        replies: n("replyCount"),
        bookmarks: n("bookmarkCount"),
        views: n("viewCount"),
        quotes: item.get("quoteCount").and_then(|v| v.as_u64()),
        ..Default::default()
    })
}

/// Current X trends (names) for a location (empty = worldwide).
pub fn fetch_trends(
    http: &dyn Http,
    token: &str,
    location: &str,
) -> Result<(Vec<String>, Option<f64>), String> {
    let mut body = serde_json::json!({"limit": 20});
    if !location.trim().is_empty() {
        body["location"] = serde_json::Value::String(location.trim().into());
    }
    let response = http.request(treg_request(
        "POST",
        TREG_TRENDS_URL,
        token,
        Some(body.to_string()),
    ))?;
    let value: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("treg: bad response ({e})"))?;
    if let Some(error) = envelope_error(&value) {
        return Err(error);
    }
    let names = value
        .pointer("/output/data/items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok((names, value.get("costUsd").and_then(|v| v.as_f64())))
}

/// Grok drafts `n` reply options for one post. Never posts.
pub fn draft_replies(
    http: &dyn Http,
    key: &str,
    model: &str,
    post: &Post,
    niche: &str,
    n: usize,
) -> Result<Vec<String>, String> {
    let n = n.clamp(1, 5);
    let model = if model.trim().is_empty() {
        DEFAULT_GROK_MODEL
    } else {
        model.trim()
    };
    let system = "You write short, sharp, genuinely useful X replies that add something: a fact, a question, a concrete take. No hashtags, no emojis unless the post uses them, no flattery, no 'Great post'. Under 240 characters each. You only draft; a human reviews and posts.";
    let user = format!(
        "Niche: {niche}\nPost by @{} :\n\"\"\"{}\"\"\"\n\nWrite {n} different reply options. Separate them with a line containing only ---. Output only the replies.",
        post.author_handle,
        post.text.trim()
    );
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.8,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ]
    })
    .to_string();
    let response = http.request(HttpRequest {
        method: "POST",
        url: format!("{XAI_BASE_URL}/chat/completions"),
        body: Some(body),
        bearer: Some(key.to_string()),
        content_type: Some("application/json".into()),
        headers: Vec::new(),
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("grok: bad response ({e})"))?;
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "grok: no message content".to_string())?;
    let options: Vec<String> = text
        .split("\n---")
        .map(|part| part.trim().trim_matches('-').trim().to_string())
        .filter(|part| !part.is_empty())
        .take(n)
        .collect();
    if options.is_empty() {
        return Err("grok returned no drafts".into());
    }
    Ok(options)
}

pub fn cost_label(cost: Option<f64>, estimate: f64) -> String {
    match cost {
        Some(c) => format!("Treg charged ${c:.5}"),
        None => format!("Treg est. ~${estimate:.5}/fetch"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn treg_errors_are_errors_not_empty_results() {
        let err = super::parse_search(r#"{"detail":"Not Found"}"#).unwrap_err();
        assert_eq!(err, "treg: Not Found");
        assert!(
            super::parse_search(r#"{"error":{"message":"insufficient balance"}}"#)
                .unwrap_err()
                .contains("insufficient balance")
        );
        let empty = super::parse_search(r#"{"output":{"data":{"items":[]}}}"#).unwrap();
        assert!(empty.posts.is_empty());
    }

    #[test]
    fn treg_calls_catalog_tools_by_id() {
        assert_eq!(
            super::TREG_SEARCH_URL,
            "https://treg.to/call/anyapi.x.search.posts"
        );
        assert_eq!(
            super::TREG_TRENDS_URL,
            "https://treg.to/call/anyapi.twitter.trends"
        );
    }

    use super::*;
    use crate::io::MapHttp;

    const SEARCH: &str = r#"{"output":{"data":{"items":[{"authorName":"A","authorUsername":"a1","id":"11","text":"rust is fast","likeCount":5,"retweetCount":1,"replyCount":2,"viewCount":90,"quoteCount":0,"createdUtc":1789811122,"url":"https://x.com/i/web/status/11"},{"id":"","text":"skip"}]}},"costUsd":0.0004}"#;

    #[test]
    fn ready_needs_both_keys() {
        assert!(ready(None, None).is_err());
        assert!(ready(Some("g"), None).is_err());
        assert!(ready(None, Some("t")).is_err());
        assert!(ready(Some("g"), Some("t")).is_ok());
    }

    #[test]
    fn mask_never_shows_more_than_four() {
        assert_eq!(mask("short"), "\u{2022}\u{2022}\u{2022}\u{2022}");
        let m = mask("xai-abcdefghijklmnop");
        assert!(m.ends_with("mnop"));
        assert!(!m.contains("abcd"));
    }

    #[test]
    fn sort_maps_to_query_type() {
        assert_eq!(Sort::Popular.query_type(), "Top");
        assert_eq!(Sort::Newest.query_type(), "Latest");
    }

    #[test]
    fn fetch_posts_uses_treg_search_and_parses_cost() {
        let http = MapHttp::default();
        http.routes
            .lock()
            .unwrap()
            .insert(TREG_SEARCH_URL.into(), Ok(SEARCH.into()));
        let got = fetch_posts(&http, "tok", "rust", Sort::Newest, 10).unwrap();
        assert_eq!(got.posts.len(), 1);
        let post = &got.posts[0];
        assert_eq!(post.id, "11");
        assert_eq!(post.author_handle, "a1");
        assert_eq!(post.likes, 5);
        assert_eq!(got.cost_usd, Some(0.0004));
        assert_eq!(http.calls.lock().unwrap()[0], TREG_SEARCH_URL);
    }

    #[test]
    fn fetch_requires_topic() {
        let http = MapHttp::default();
        assert!(fetch_posts(&http, "tok", "  ", Sort::Popular, 10).is_err());
        assert!(http.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn trends_parse_names() {
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            TREG_TRENDS_URL.into(),
            Ok(
                r#"{"output":{"data":{"items":[{"name":"Greenland","rank":1}]}},"costUsd":0.0001}"#
                    .into(),
            ),
        );
        let (names, cost) = fetch_trends(&http, "tok", "US").unwrap();
        assert_eq!(names, vec!["Greenland".to_string()]);
        assert_eq!(cost, Some(0.0001));
    }

    #[test]
    fn grok_drafts_split_and_never_post() {
        let http = MapHttp::default();
        http.routes.lock().unwrap().insert(
            format!("{XAI_BASE_URL}/chat/completions"),
            Ok(r#"{"choices":[{"message":{"content":"one\n---\ntwo\n---\nthree"}}]}"#.into()),
        );
        let post = Post {
            id: "1".into(),
            author_handle: "a".into(),
            text: "t".into(),
            ..Default::default()
        };
        let drafts = draft_replies(&http, "k", "", &post, "rust", 3).unwrap();
        assert_eq!(drafts, vec!["one", "two", "three"]);
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(
            calls
                .iter()
                .all(|u| !u.contains("x.com") && !u.contains("twitter"))
        );
    }

    #[test]
    fn connection_tests_hit_free_endpoints() {
        let http = MapHttp::default();
        http.routes
            .lock()
            .unwrap()
            .insert(TREG_ME_URL.into(), Ok(r#"{"email":"me@x.y"}"#.into()));
        http.routes.lock().unwrap().insert(
            format!("{XAI_BASE_URL}/models"),
            Ok(r#"{"data":[{"id":"grok-4"}]}"#.into()),
        );
        assert!(test_treg(&http, "t").unwrap().contains("me@x.y"));
        assert!(test_grok(&http, "k").unwrap().contains("1 models"));
    }

    #[test]
    fn file_secret_round_trip_without_keychain() {
        let dir = std::env::temp_dir().join(format!("rg-secret-{}", std::process::id()));
        let config = XConfig {
            config_dir: dir.clone(),
            token_store: "file".into(),
            ..Default::default()
        };
        store_secret(&config, Secret::Treg, "  tok-123456789  ").unwrap();
        // env override may be set on a dev box; only assert file contents.
        let text = std::fs::read_to_string(dir.join("reply-guy-secrets.json")).unwrap();
        assert!(text.contains("tok-123456789"));
        clear_secret(&config, Secret::Treg).unwrap();
        let text = std::fs::read_to_string(dir.join("reply-guy-secrets.json")).unwrap();
        assert!(!text.contains("tok-123456789"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cost_label_prefers_reported_charge() {
        assert!(cost_label(Some(0.0004), TREG_SEARCH_COST_USD).contains("charged"));
        assert!(cost_label(None, TREG_SEARCH_COST_USD).contains("est."));
    }
}
