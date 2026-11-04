//! Zero-copy borrowed parsing for provider payloads (hot path).
//!
//! Every string field in the response structs below is an explicit
//! `&'a str` pointing into the raw payload buffer, never a `String`.
//! serde_json only borrows a `&str` when the JSON string needs no
//! unescaping; an escaped string makes deserialization fail and the
//! caller falls back to the legacy `Value`-tree parser in `providers`,
//! so there is no silent `String` allocation on hot fields and exotic
//! payloads keep byte-for-byte legacy behaviour.
//!
//! Audit (borrowed struct -> final `Post`):
//!   RawPost.text/id/created_at       &'a str -> String (Post outlives buffer)
//!   RawPost author/user handle+name  &'a str -> String (same; also '@'-trimmed)
//!   RawPost.media_urls/posters       Vec<&'a str> -> String (same; a photo
//!                                    poster may be a computed copy of the URL)
//!   RawPost counts (likes/...)       u64 (copied, no allocation either way)
//!   RawPost.retweeted                Box<RawPost<'a>> -> Box<Post> (recursive)
//!   Post.url                         -- (computed by `post_url`, never parsed)
//!   RawEval.next_cursor/error        &'a str -> String (stored / surfaced)
//!
//! Fields stay `String` only in `Post`/`MediaItem` themselves, because
//! those values are stored, merged across pages and rendered long after
//! the payload buffer is dropped — borrowing there would not compile
//! honestly, so it is not forced.

use serde::Deserialize;

use crate::model::{MediaItem, MediaKind, Post, post_url};
use crate::providers::TimelinePage;

/// One included user: (id, username, display name), all borrowed.
type RawUsers<'a> = Vec<(&'a str, &'a str, &'a str)>;

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawUser<'a> {
    #[serde(borrow)]
    id: Option<&'a str>,
    #[serde(borrow, alias = "screen_name")]
    username: Option<&'a str>,
    #[serde(borrow)]
    name: Option<&'a str>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawAuthorObj<'a> {
    #[serde(borrow, alias = "screen_name", alias = "screenName", alias = "handle")]
    username: Option<&'a str>,
    #[serde(borrow, alias = "display_name")]
    name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawAuthor<'a> {
    Str(&'a str),
    Obj(RawAuthorObj<'a>),
}

#[derive(Debug, Deserialize, Default, Clone, Copy)]
#[serde(default)]
struct RawMetrics {
    #[serde(alias = "likes", alias = "favorite_count")]
    like_count: u64,
    #[serde(alias = "reposts", alias = "retweets")]
    retweet_count: u64,
    #[serde(alias = "replies")]
    reply_count: u64,
    #[serde(alias = "bookmarks")]
    bookmark_count: u64,
    #[serde(alias = "views", alias = "view_count")]
    impression_count: u64,
}

/// X API / OpenCLI / GraphQL-eval tweet row, borrowed from the payload.
/// (The GraphQL eval scripts already normalize GraphQL into this shape
/// in-page; this struct parses that normalized row, and the official-API
/// envelope below, without building a `Value` tree.)
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawPost<'a> {
    #[serde(borrow, alias = "full_text", alias = "rawContent", alias = "content")]
    text: Option<&'a str>,
    #[serde(borrow, alias = "id_str", alias = "rest_id")]
    id: Option<&'a str>,
    #[serde(borrow)]
    user: Option<RawAuthor<'a>>,
    #[serde(borrow)]
    author: Option<RawAuthor<'a>>,
    #[serde(borrow, alias = "screen_name", alias = "screenName")]
    username: Option<&'a str>,
    #[serde(borrow)]
    author_id: Option<&'a str>,
    #[serde(borrow, alias = "createdAt", alias = "time")]
    created_at: Option<&'a str>,
    public_metrics: Option<RawMetrics>,
    metrics: Option<RawMetrics>,
    // Inline metric keys, used only when neither nested object exists —
    // mirroring `post_from_value`, where the metrics node falls back to
    // the row itself.
    #[serde(rename = "like_count", alias = "likes", alias = "favorite_count")]
    inline_like_count: u64,
    #[serde(rename = "retweet_count", alias = "reposts", alias = "retweets")]
    inline_retweet_count: u64,
    #[serde(rename = "reply_count", alias = "replies")]
    inline_reply_count: u64,
    #[serde(rename = "bookmark_count", alias = "bookmarks")]
    inline_bookmark_count: u64,
    #[serde(rename = "impression_count", alias = "views", alias = "view_count")]
    inline_impression_count: u64,
    #[serde(borrow)]
    media_urls: Option<Vec<&'a str>>,
    #[serde(borrow)]
    media_posters: Option<Vec<&'a str>>,
    // Shapes the borrowed media path does not model (`media`/`images`
    // arrays): their presence disqualifies the fast path for the whole
    // payload, which then takes the legacy parser unchanged.
    media: Option<serde::de::IgnoredAny>,
    images: Option<serde::de::IgnoredAny>,
    #[serde(borrow, alias = "retweeted_status", alias = "retweeted_tweet")]
    retweeted: Option<Box<RawPost<'a>>>,
}

fn raw_post_supported(raw: &RawPost<'_>) -> bool {
    if raw.media_urls.is_none() && (raw.media.is_some() || raw.images.is_some()) {
        return false;
    }
    raw.retweeted.as_deref().is_none_or(raw_post_supported)
}

/// Media kind from a URL, case-insensitively, without allocating a
/// lowercased copy of the URL. Needles are ASCII, matching the previous
/// `to_ascii_lowercase` behaviour for these checks.
pub(crate) fn media_kind(url: &str) -> MediaKind {
    let bytes = url.as_bytes();
    let contains_ci = |needle: &str| {
        let n = needle.as_bytes();
        !n.is_empty() && bytes.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
    };
    let ends_ci = |suffix: &str| {
        let n = suffix.as_bytes();
        bytes.len() >= n.len() && bytes[bytes.len() - n.len()..].eq_ignore_ascii_case(n)
    };
    if contains_ci("/tweet_video/") || ends_ci(".gif") {
        MediaKind::Gif
    } else if contains_ci("video.twimg.com") || contains_ci(".mp4") || contains_ci(".m3u8") {
        MediaKind::Video
    } else {
        MediaKind::Photo
    }
}

fn raw_media_to_items(raw: &RawPost<'_>) -> Vec<MediaItem> {
    let Some(urls) = &raw.media_urls else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        if url.is_empty() {
            continue;
        }
        let kind = media_kind(url);
        let poster = raw
            .media_posters
            .as_ref()
            .and_then(|p| p.get(i).copied())
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .or_else(|| (kind == MediaKind::Photo).then(|| url.to_string()));
        let (width, height) = match crate::model::dims_from_media_url(url) {
            Some((w, h)) => (Some(w), Some(h)),
            None => (None, None),
        };
        out.push(MediaItem {
            kind,
            url: url.to_string(),
            poster,
            width,
            height,
        });
    }
    out
}

/// Convert a borrowed row into an owned `Post`: the one place `&'a str`
/// fields become `String`s, because `Post` outlives the buffer.
/// Mirrors `providers::post_from_value` field-for-field.
fn raw_post_to_post(raw: RawPost<'_>, users: &[(&str, &str, &str)]) -> Option<Post> {
    let text = raw.text?.to_string();
    let id = raw.id.unwrap_or("0").to_string();
    let (mut handle, mut name) = match raw.user.as_ref().or(raw.author.as_ref()) {
        Some(RawAuthor::Str(s)) => (s.trim().trim_start_matches('@').to_string(), String::new()),
        Some(RawAuthor::Obj(o)) => (
            o.username.unwrap_or("").trim_start_matches('@').to_string(),
            o.name.unwrap_or("").to_string(),
        ),
        None => (String::new(), String::new()),
    };
    if handle.is_empty() {
        let author_str = match &raw.author {
            Some(RawAuthor::Str(s)) => Some(*s),
            _ => None,
        };
        handle = raw
            .username
            .or(author_str)
            .unwrap_or("")
            .trim_start_matches('@')
            .to_string();
    }
    if handle.is_empty()
        && let Some(author_id) = raw.author_id
        && let Some((_, username, display)) = users.iter().find(|(uid, _, _)| *uid == author_id)
    {
        handle = username.to_string();
        name = display.to_string();
    }
    if name.is_empty() {
        name = handle.clone();
    }
    let (likes, reposts, replies, bookmarks, views) = match raw.public_metrics.or(raw.metrics) {
        Some(m) => (
            m.like_count,
            m.retweet_count,
            m.reply_count,
            m.bookmark_count,
            m.impression_count,
        ),
        None => (
            raw.inline_like_count,
            raw.inline_retweet_count,
            raw.inline_reply_count,
            raw.inline_bookmark_count,
            raw.inline_impression_count,
        ),
    };
    Some(Post {
        url: post_url(&handle, &id),
        author_handle: handle,
        author_name: name,
        created_at: raw.created_at.map(str::to_string),
        likes,
        reposts,
        replies,
        bookmarks,
        views,
        text,
        id,
        media: raw_media_to_items(&raw),
        retweeted: raw
            .retweeted
            .and_then(|inner| raw_post_to_post(*inner, &[]))
            .map(Box::new),
    })
}

fn raw_posts_to_posts(rows: Vec<RawPost<'_>>, users: &[(&str, &str, &str)]) -> Option<Vec<Post>> {
    if rows.iter().any(|r| !raw_post_supported(r)) {
        return None;
    }
    let posts: Vec<Post> = rows
        .into_iter()
        .filter_map(|r| raw_post_to_post(r, users))
        .collect();
    // An empty result defers to the legacy parser, which owns the
    // "no posts" error and any shape this path might have missed.
    (!posts.is_empty()).then_some(posts)
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawIncludes<'a> {
    #[serde(borrow)]
    users: Option<Vec<RawUser<'a>>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawEnvelope<'a> {
    #[serde(borrow)]
    data: Option<Vec<RawPost<'a>>>,
    #[serde(borrow)]
    tweets: Option<Vec<RawPost<'a>>>,
    #[serde(borrow)]
    items: Option<Vec<RawPost<'a>>>,
    #[serde(borrow)]
    rows: Option<Vec<RawPost<'a>>>,
    #[serde(borrow)]
    results: Option<Vec<RawPost<'a>>>,
    #[serde(borrow)]
    timeline: Option<Vec<RawPost<'a>>>,
    includes: Option<RawIncludes<'a>>,
}

fn raw_users<'a>(includes: Option<RawIncludes<'a>>) -> RawUsers<'a> {
    includes
        .and_then(|i| i.users)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|u| Some((u.id?, u.username?, u.name.unwrap_or(""))))
        .collect()
}

/// Borrowed fast path for `providers::parse_posts`. `None` means "not a
/// shape this path models exactly — use the legacy parser".
pub(crate) fn parse_posts_borrowed(text: &str) -> Option<Vec<Post>> {
    if text.trim_start().starts_with('[') {
        let rows: Vec<RawPost<'_>> = serde_json::from_str(text).ok()?;
        return raw_posts_to_posts(rows, &[]);
    }
    let envelope: RawEnvelope<'_> = serde_json::from_str(text).ok()?;
    let users = raw_users(envelope.includes);
    // Key precedence mirrors `array_of`: data, tweets, items, rows,
    // results, timeline — first present array wins.
    let rows = envelope
        .data
        .or(envelope.tweets)
        .or(envelope.items)
        .or(envelope.rows)
        .or(envelope.results)
        .or(envelope.timeline);
    if let Some(rows) = rows {
        return raw_posts_to_posts(rows, &users);
    }
    // Single-tweet object (legacy `array_of` treats it as a one-row list).
    let single: RawPost<'_> = serde_json::from_str(text).ok()?;
    if !raw_post_supported(&single) {
        return None;
    }
    raw_post_to_post(single, &users).map(|p| vec![p])
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RawEval<'a> {
    #[serde(borrow)]
    posts: Option<Vec<RawPost<'a>>>,
    #[serde(borrow, rename = "nextCursor")]
    next_cursor: Option<&'a str>,
    #[serde(borrow)]
    error: Option<&'a str>,
    #[serde(borrow)]
    pinned: Option<RawPost<'a>>,
    diag: Option<serde::de::IgnoredAny>,
}

type ProfilePosts = (Vec<Post>, Vec<Post>);

struct EvalParts {
    posts: Vec<Post>,
    pinned: Vec<Post>,
    next_cursor: Option<String>,
    error: Option<String>,
}

fn eval_posts(raw: RawEval<'_>) -> Option<EvalParts> {
    let posts = match raw.posts {
        Some(rows) => {
            if rows.iter().any(|r| !raw_post_supported(r)) {
                return None;
            }
            rows.into_iter()
                .filter_map(|r| raw_post_to_post(r, &[]))
                .collect::<Vec<_>>()
        }
        None => Vec::new(),
    };
    let pinned: Vec<Post> = match raw.pinned {
        Some(row) => {
            if !raw_post_supported(&row) {
                return None;
            }
            raw_post_to_post(row, &[]).into_iter().collect()
        }
        None => Vec::new(),
    };
    Some(EvalParts {
        posts,
        pinned,
        next_cursor: raw.next_cursor.map(str::to_string),
        error: raw.error.map(str::to_string),
    })
}

pub(crate) fn parse_timeline_eval_borrowed(stdout: &str) -> Option<Result<TimelinePage, String>> {
    let raw: RawEval<'_> = serde_json::from_str(stdout).ok()?;
    let EvalParts {
        posts,
        next_cursor,
        error,
        ..
    } = eval_posts(raw)?;
    if let Some(error) = &error
        && posts.is_empty()
    {
        return Some(Err(if error == "http-429" {
            "X rate-limited the session (HTTP 429) — wait a few minutes and try Load more again"
                .into()
        } else {
            format!("timeline session: {error}")
        }));
    }
    Some(Ok(TimelinePage { posts, next_cursor }))
}

pub(crate) fn parse_profile_eval_borrowed(stdout: &str) -> Option<Result<ProfilePosts, String>> {
    let raw: RawEval<'_> = serde_json::from_str(stdout).ok()?;
    // The legacy path logs `diag`; keep that side effect by deferring.
    if raw.diag.is_some() {
        return None;
    }
    let EvalParts {
        posts,
        pinned,
        error,
        ..
    } = eval_posts(raw)?;
    if let Some(error) = error
        && posts.is_empty()
        && pinned.is_empty()
    {
        return Some(Err(error));
    }
    Some(Ok((posts, pinned)))
}
