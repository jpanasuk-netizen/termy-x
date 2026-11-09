//! Multi-source Treg fetch + quote-to-X helpers for Reply Guy.
//! Tool ids are catalog call-by-id only (see notes/treg-multi-source-tools.md).

use crate::io::Http;
use crate::model::Post;
use crate::reply_guy::{
    Sort, TREG_BASE, TREG_SEARCH_COST_USD, fetch_posts, fetch_trends, treg_request,
};

/// Catalog tool ids Reply Guy calls by id.
pub const TOOL_REDDIT_SEARCH: &str = "anyapi.reddit.search";
pub const TOOL_REDDIT_TRENDING: &str = "anyapi.reddit.trending_posts";
pub const TOOL_TIKTOK_SEARCH: &str = "anyapi.tiktok.search.videos";
pub const TOOL_TIKTOK_HASHTAGS: &str = "anyapi.tiktok.trending_hashtags";
pub const TOOL_IG_REELS: &str = "anyapi.instagram.search.reels";
pub const TOOL_YT_SEARCH: &str = "anyapi.youtube.search";
pub const TOOL_YT_SHORTS: &str = "anyapi.youtube.trending_shorts";
pub const TOOL_LINKEDIN_SEARCH: &str = "anyapi.linkedin.search.posts";
pub const TOOL_FACEBOOK_SEARCH: &str = "anyapi.facebook.search_posts";

pub const TREG_COST_VARIES: &str = "varies";

/// Platform a Reply Guy hit came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    X,
    Reddit,
    TikTok,
    Instagram,
    YouTube,
    LinkedIn,
    Facebook,
}

impl Source {
    pub const ALL: [Source; 7] = [
        Source::X,
        Source::Reddit,
        Source::TikTok,
        Source::Instagram,
        Source::YouTube,
        Source::LinkedIn,
        Source::Facebook,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Reddit => "Reddit",
            Self::TikTok => "TikTok",
            Self::Instagram => "Instagram",
            Self::YouTube => "YouTube",
            Self::LinkedIn => "LinkedIn",
            Self::Facebook => "Facebook",
        }
    }

    pub fn tab(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Reddit => "Reddit",
            Self::TikTok => "TikTok",
            Self::Instagram => "IG",
            Self::YouTube => "YT",
            Self::LinkedIn => "in",
            Self::Facebook => "FB",
        }
    }

    pub fn is_x(self) -> bool {
        matches!(self, Self::X)
    }

    pub fn accent_rgb(self) -> u32 {
        match self {
            Self::X => 0xe7e9ea,
            Self::Reddit => 0xff4500,
            Self::TikTok => 0x25f4ee,
            Self::Instagram => 0xe1306c,
            Self::YouTube => 0xff0000,
            Self::LinkedIn => 0x0a66c2,
            Self::Facebook => 0x1877f2,
        }
    }
}

/// Sources asked for but not callable on this Treg org.
pub const MISSING_SOURCES: &[&str] = &["Hacker News", "Product Hunt", "Threads", "Bluesky"];

#[derive(Clone, Debug, PartialEq)]
pub struct RgHit {
    pub source: Source,
    pub post: Post,
}

impl RgHit {
    pub fn is_x(&self) -> bool {
        self.source.is_x()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MultiFetched {
    pub hits: Vec<RgHit>,
    pub cost_usd: Option<f64>,
    pub notes: Vec<String>,
}

impl MultiFetched {
    fn push_cost(&mut self, part: Option<f64>) {
        match (self.cost_usd, part) {
            (None, Some(c)) => self.cost_usd = Some(c),
            (Some(a), Some(b)) => self.cost_usd = Some(a + b),
            _ => {}
        }
    }
}

fn call_json(
    http: &dyn Http,
    token: &str,
    tool_id: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = format!("{TREG_BASE}/call/{tool_id}");
    let response = http.request(treg_request("POST", &url, token, Some(body.to_string())))?;
    let value: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("treg: bad response ({e})"))?;
    if let Some(error) = envelope_error_flex(&value) {
        return Err(error);
    }
    Ok(value)
}

fn envelope_error_flex(value: &serde_json::Value) -> Option<String> {
    let data = value.pointer("/output/data");
    if data.is_some_and(|d| {
        ["items", "posts", "videos", "reels", "shorts"]
            .iter()
            .any(|k| d.get(*k).is_some_and(|v| v.is_array()))
    }) {
        return None;
    }
    if value.pointer("/output/found").and_then(|v| v.as_bool()) == Some(true) {
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

fn cost_of(value: &serde_json::Value) -> Option<f64> {
    value.get("costUsd").and_then(|v| v.as_f64())
}

fn s(item: &serde_json::Value, key: &str) -> String {
    item.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn n(item: &serde_json::Value, key: &str) -> u64 {
    item.get(key)
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_f64().map(|f| f as u64))
                .or_else(|| v.as_i64().map(|i| i.max(0) as u64))
        })
        .unwrap_or(0)
}

fn created(item: &serde_json::Value) -> Option<String> {
    item.get("createdUtc")
        .and_then(|v| v.as_u64())
        .map(|t| t.to_string())
}

/// Search every wired Treg source for `topic`. One failure does not cancel others.
pub fn fetch_multi(
    http: &dyn Http,
    token: &str,
    topic: &str,
    sort: Sort,
    limit: usize,
) -> Result<MultiFetched, String> {
    let topic = topic.trim();
    if topic.is_empty() {
        return Err("Type a topic or niche first.".into());
    }
    let mut out = MultiFetched::default();
    let limit = limit.clamp(1, 50);

    match fetch_posts(http, token, topic, sort, limit) {
        Ok(fetched) => {
            out.push_cost(fetched.cost_usd);
            for post in fetched.posts {
                out.hits.push(RgHit {
                    source: Source::X,
                    post,
                });
            }
        }
        Err(e) => out.notes.push(format!("X: {e}")),
    }

    push_parsed(
        &mut out,
        "Reddit",
        Source::Reddit,
        call_json(
            http,
            token,
            TOOL_REDDIT_SEARCH,
            &serde_json::json!({ "query": topic }),
        ),
        parse_reddit,
    );
    push_parsed(
        &mut out,
        "TikTok",
        Source::TikTok,
        call_json(
            http,
            token,
            TOOL_TIKTOK_SEARCH,
            &serde_json::json!({ "query": topic }),
        ),
        parse_tiktok_videos,
    );
    push_parsed(
        &mut out,
        "Instagram",
        Source::Instagram,
        call_json(
            http,
            token,
            TOOL_IG_REELS,
            &serde_json::json!({ "query": topic }),
        ),
        |v| parse_instagram(v, "reels"),
    );
    push_parsed(
        &mut out,
        "YouTube",
        Source::YouTube,
        call_json(
            http,
            token,
            TOOL_YT_SEARCH,
            &serde_json::json!({ "query": topic }),
        ),
        |v| parse_youtube(v, "videos"),
    );
    push_parsed(
        &mut out,
        "LinkedIn",
        Source::LinkedIn,
        call_json(
            http,
            token,
            TOOL_LINKEDIN_SEARCH,
            &serde_json::json!({ "query": topic }),
        ),
        parse_linkedin,
    );
    push_parsed(
        &mut out,
        "Facebook",
        Source::Facebook,
        call_json(
            http,
            token,
            TOOL_FACEBOOK_SEARCH,
            &serde_json::json!({ "query": topic, "limit": 10 }),
        ),
        parse_facebook,
    );

    if out.hits.is_empty() && !out.notes.is_empty() {
        return Err(out.notes.join(" · "));
    }
    let _ = TREG_SEARCH_COST_USD; // keep estimate symbol referenced for callers
    Ok(out)
}

fn push_parsed<F>(
    out: &mut MultiFetched,
    label: &str,
    source: Source,
    result: Result<serde_json::Value, String>,
    parse: F,
) where
    F: FnOnce(&serde_json::Value) -> Result<(Vec<Post>, Option<f64>), String>,
{
    match result {
        Ok(value) => match parse(&value) {
            Ok((posts, cost)) => {
                out.push_cost(cost);
                for post in posts {
                    out.hits.push(RgHit { source, post });
                }
            }
            Err(e) => out.notes.push(format!("{label}: {e}")),
        },
        Err(e) => out.notes.push(format!("{label}: {e}")),
    }
}

/// Trends: X name chips + Reddit trending + TikTok hashtag chips + YT shorts hits.
pub fn fetch_trends_multi(
    http: &dyn Http,
    token: &str,
) -> Result<(Vec<String>, MultiFetched), String> {
    let mut chips = Vec::new();
    let mut out = MultiFetched::default();

    match fetch_trends(http, token, "") {
        Ok((names, cost)) => {
            out.push_cost(cost);
            chips.extend(names);
        }
        Err(e) => out.notes.push(format!("X trends: {e}")),
    }

    push_parsed(
        &mut out,
        "Reddit trending",
        Source::Reddit,
        call_json(http, token, TOOL_REDDIT_TRENDING, &serde_json::json!({})),
        parse_reddit,
    );

    match call_json(http, token, TOOL_TIKTOK_HASHTAGS, &serde_json::json!({})) {
        Ok(value) => {
            out.push_cost(cost_of(&value));
            let tags = value
                .pointer("/output/data/items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for item in tags.iter().take(12) {
                if let Some(tag) = item
                    .get("hashtag")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    chips.push(format!("#{tag}"));
                }
            }
        }
        Err(e) => out.notes.push(format!("TikTok hashtags: {e}")),
    }

    push_parsed(
        &mut out,
        "YouTube shorts",
        Source::YouTube,
        call_json(http, token, TOOL_YT_SHORTS, &serde_json::json!({})),
        |v| parse_youtube(v, "shorts"),
    );

    Ok((chips, out))
}

fn parse_reddit(value: &serde_json::Value) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = value
        .pointer("/output/data/posts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            if id.is_empty() {
                return None;
            }
            let author = s(item, "author");
            let title = s(item, "title");
            let body = s(item, "selftext");
            let text = if body.is_empty() {
                title
            } else if title.is_empty() {
                body
            } else {
                format!("{title}\n\n{body}")
            };
            let permalink = s(item, "permalink");
            let url = {
                let u = s(item, "url");
                if u.starts_with("http") {
                    u
                } else if !permalink.is_empty() {
                    format!("https://www.reddit.com{permalink}")
                } else {
                    format!("https://www.reddit.com/comments/{id}")
                }
            };
            let sub = s(item, "subreddit");
            Some(Post {
                id: format!("reddit:{id}"),
                url,
                author_handle: if sub.is_empty() {
                    author.clone()
                } else {
                    format!("r/{sub}")
                },
                author_name: author,
                text,
                created_at: created(item),
                likes: n(item, "score"),
                replies: n(item, "numComments"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

fn parse_tiktok_videos(value: &serde_json::Value) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = value
        .pointer("/output/data/videos")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            if id.is_empty() {
                return None;
            }
            let handle = s(item, "author");
            let url = {
                let u = s(item, "url");
                if u.is_empty() {
                    format!("https://www.tiktok.com/@{handle}/video/{id}")
                } else {
                    u
                }
            };
            Some(Post {
                id: format!("tiktok:{id}"),
                url,
                author_handle: handle,
                author_name: s(item, "authorName"),
                text: s(item, "caption"),
                created_at: created(item),
                likes: n(item, "likes"),
                reposts: n(item, "shares"),
                replies: n(item, "comments"),
                views: n(item, "views"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

fn parse_instagram(
    value: &serde_json::Value,
    key: &str,
) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = value
        .pointer(&format!("/output/data/{key}"))
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| {
            value
                .pointer("/output/data/posts")
                .and_then(|v| v.as_array())
                .cloned()
        })
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            let short = s(item, "shortcode");
            if id.is_empty() && short.is_empty() {
                return None;
            }
            let key_id = if id.is_empty() { short.clone() } else { id };
            let handle = s(item, "username");
            let url = {
                let u = s(item, "url");
                if !u.is_empty() {
                    u
                } else if !short.is_empty() {
                    format!("https://www.instagram.com/p/{short}/")
                } else {
                    String::new()
                }
            };
            Some(Post {
                id: format!("ig:{key_id}"),
                url,
                author_handle: handle,
                author_name: s(item, "displayName"),
                text: s(item, "caption"),
                created_at: created(item),
                likes: n(item, "likes"),
                replies: n(item, "comments"),
                views: n(item, "views"),
                reposts: n(item, "reshares"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

fn parse_youtube(value: &serde_json::Value, key: &str) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = value
        .pointer(&format!("/output/data/{key}"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            if id.is_empty() {
                return None;
            }
            let channel = {
                let c = s(item, "channel");
                if c.is_empty() {
                    s(item, "authorHandle")
                } else {
                    c
                }
            };
            let url = {
                let u = s(item, "url");
                if u.is_empty() {
                    format!("https://www.youtube.com/watch?v={id}")
                } else {
                    u
                }
            };
            Some(Post {
                id: format!("yt:{id}"),
                url,
                author_handle: channel.clone(),
                author_name: channel,
                text: s(item, "title"),
                created_at: created(item).or_else(|| {
                    let t = s(item, "publishedTime");
                    (!t.is_empty()).then_some(t)
                }),
                views: n(item, "views"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

fn parse_linkedin(value: &serde_json::Value) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = value
        .pointer("/output/data/posts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            if id.is_empty() {
                return None;
            }
            let name = s(item, "authorName");
            Some(Post {
                id: format!("li:{id}"),
                url: s(item, "url"),
                author_handle: name.clone(),
                author_name: name,
                text: s(item, "text"),
                created_at: created(item),
                likes: n(item, "reactionCount"),
                replies: n(item, "commentCount"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

fn parse_facebook(value: &serde_json::Value) -> Result<(Vec<Post>, Option<f64>), String> {
    let items = ["posts", "items", "results"]
        .iter()
        .find_map(|key| {
            value
                .pointer(&format!("/output/data/{key}"))
                .and_then(|v| v.as_array())
                .cloned()
        })
        .or_else(|| {
            value
                .pointer("/output/data")
                .and_then(|v| v.as_array())
                .cloned()
        })
        .unwrap_or_default();
    let posts = items
        .iter()
        .filter_map(|item| {
            let id = s(item, "id");
            if id.is_empty() {
                return None;
            }
            let text = {
                let t = s(item, "text");
                if t.is_empty() { s(item, "message") } else { t }
            };
            let author = {
                let a = s(item, "authorName");
                if a.is_empty() { s(item, "author") } else { a }
            };
            Some(Post {
                id: format!("fb:{id}"),
                url: s(item, "url"),
                author_handle: author.clone(),
                author_name: author,
                text,
                created_at: created(item),
                likes: n(item, "likes").max(n(item, "reactionCount")),
                replies: n(item, "comments").max(n(item, "commentCount")),
                reposts: n(item, "shares"),
                ..Default::default()
            })
        })
        .collect();
    Ok((posts, cost_of(value)))
}

/// Prefill for quoting a non-X hit into a new X post. Never posts.
pub fn quote_to_x_body(hit: &RgHit) -> String {
    let raw = hit.post.text.trim();
    let clipped: String = if raw.chars().count() > 220 {
        let mut out: String = raw.chars().take(217).collect();
        out.push('\u{2026}');
        out
    } else {
        raw.to_string()
    };
    let via = hit.source.label();
    let url = hit.post.url.trim();
    if url.is_empty() {
        format!("{clipped}\n\nvia {via}")
    } else {
        format!("{clipped}\n\nvia {via} — {url}")
    }
}

pub fn multi_cost_label(cost: Option<f64>) -> String {
    match cost {
        Some(c) => format!("Treg charged ${c:.5} (multi)"),
        None => format!("Treg multi-source · cost {TREG_COST_VARIES}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_includes_source_and_url() {
        let hit = RgHit {
            source: Source::Reddit,
            post: Post {
                id: "reddit:1".into(),
                url: "https://reddit.com/r/x/1".into(),
                text: "hello world".into(),
                ..Default::default()
            },
        };
        let body = quote_to_x_body(&hit);
        assert!(body.contains("via Reddit"));
        assert!(body.contains("https://reddit.com/r/x/1"));
        assert!(body.contains("hello world"));
    }

    #[test]
    fn parse_reddit_sample() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"output":{"data":{"posts":[{"id":"abc","author":"u1","title":"T","selftext":"B","permalink":"/r/rust/comments/abc/t/","score":9,"numComments":2,"subreddit":"rust"}]}},"costUsd":0.0003}"#,
        )
        .unwrap();
        let (posts, cost) = parse_reddit(&v).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, "reddit:abc");
        assert_eq!(posts[0].author_handle, "r/rust");
        assert!(posts[0].text.contains("T"));
        assert_eq!(cost, Some(0.0003));
    }
}
