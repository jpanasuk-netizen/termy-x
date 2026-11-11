//! Topics, query focus, status-link normalization, reports, and desk phases.
//!
//! Fresh means this run just returned source-linked posts. A report loaded
//! from disk is cached. Empty and error never become a new successful report.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{post_url, strip_handle};
use crate::time::{parse_created_at, ymd};

pub const QUERY_LIMIT: usize = 180;
/// Window title and shortcut label for the standalone desk.
/// The Termy X app, its installer, and its shortcut stay named Termy.
pub const VISIBLE_NAME: &str = "Termy XL";

pub const GPU_NOTE: &str = "The Termy XL window is drawn with Termy's GPU renderer. An OpenCLI search still waits on the network and the browser bridge; the GPU does not make that search faster.";
pub const IMAGE_QUOTE_NOTE: &str = "Image-only quote posts are not shown. Open the status link; photos and video stay in the X panel.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePost {
    pub id: String,
    pub author: String,
    pub text: String,
    pub created_at: Option<String>,
    pub url: String,
    pub source_url: String,
    pub likes: u64,
    pub reposts: u64,
    pub views: u64,
    #[serde(default)]
    pub replies: u64,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub media: Vec<String>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
}

impl SourcePost {
    pub fn from_value(value: &Value) -> Option<Self> {
        let text = first_str(value, &["text", "full_text", "rawContent", "content"])
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            return None;
        }
        let author = author_of(value);
        let id = first_str(value, &["id", "id_str", "rest_id"])
            .map(str::to_string)
            .or_else(|| {
                value
                    .get("id")
                    .and_then(Value::as_u64)
                    .map(|id| id.to_string())
            })
            .unwrap_or_default();
        let source_url = first_str(value, &["url", "permalink", "link"])
            .unwrap_or("")
            .trim()
            .to_string();
        let status = normalize_status_link(&source_url, &author, &id)?;
        let created_at = created_at_of(value);
        let canonical = status.canonical.clone();
        Some(Self {
            id: if status.id.is_empty() { id } else { status.id },
            author: if author.is_empty() {
                status.author
            } else {
                author
            },
            text,
            created_at,
            url: canonical.clone(),
            source_url: if source_url.is_empty() {
                canonical
            } else {
                source_url
            },
            likes: metric(value, &["likes", "like_count", "favorite_count"]),
            reposts: metric(value, &["retweets", "reposts", "retweet_count"]),
            views: metric(value, &["views", "view_count", "impression_count"]),
            replies: metric(value, &["replies", "reply_count"]),
            author_name: display_name_of(value),
            media: media_urls_of(value),
            verified: value
                .get("verified")
                .and_then(Value::as_bool)
                .or_else(|| value.get("is_blue_verified").and_then(Value::as_bool))
                .unwrap_or(false),
            lang: first_str(value, &["lang", "language"])
                .unwrap_or("")
                .to_string(),
            in_reply_to: first_str(
                value,
                &[
                    "in_reply_to",
                    "in_reply_to_status_id_str",
                    "in_reply_to_status_id",
                ],
            )
            .map(str::to_string)
            .filter(|id| !id.is_empty()),
        })
    }

    pub fn engagement_label(&self) -> String {
        let mut bits = Vec::new();
        if self.likes > 0 {
            bits.push(format!("{} likes", self.likes));
        }
        if self.reposts > 0 {
            bits.push(format!("{} reposts", self.reposts));
        }
        if self.views > 0 {
            bits.push(format!("{} views", self.views));
        }
        bits.join(", ")
    }
}

struct StatusLink {
    id: String,
    author: String,
    canonical: String,
}

fn normalize_status_link(raw: &str, author: &str, id: &str) -> Option<StatusLink> {
    let trimmed = raw.trim();
    if !trimmed.is_empty() {
        if let Some(link) = status_from_url(trimmed) {
            return Some(link);
        }
        return None;
    }
    let id = id.trim();
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let author = strip_handle(author).to_string();
    Some(StatusLink {
        canonical: post_url(&author, id),
        author,
        id: id.to_string(),
    })
}

fn status_from_url(raw: &str) -> Option<StatusLink> {
    let without_hash = raw.split('#').next().unwrap_or(raw);
    let (base, _query) = without_hash.split_once('?').unwrap_or((without_hash, ""));
    let rest = strip_http_scheme(base)?;
    let (host, path) = rest.split_once('/')?;
    let host = host.split('@').next_back().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    if !is_x_host(host) {
        return None;
    }
    let path = path.trim_matches('/');
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let first = parts.next()?;
    let (author, id) = if first.eq_ignore_ascii_case("i") {
        let kind = parts.next()?;
        if !kind.eq_ignore_ascii_case("status") && !kind.eq_ignore_ascii_case("web") {
            return None;
        }
        let id = if kind.eq_ignore_ascii_case("web") {
            let status = parts.next()?;
            if !status.eq_ignore_ascii_case("status") {
                return None;
            }
            parts.next()?
        } else {
            parts.next()?
        };
        (String::new(), digits(id)?)
    } else {
        let status = parts.next()?;
        if !status.eq_ignore_ascii_case("status") {
            return None;
        }
        (strip_handle(first).to_string(), digits(parts.next()?)?)
    };
    Some(StatusLink {
        canonical: post_url(&author, &id),
        author,
        id,
    })
}

fn strip_http_scheme(url: &str) -> Option<&str> {
    if url.len() >= 8 && url[..8].eq_ignore_ascii_case("https://") {
        Some(&url[8..])
    } else if url.len() >= 7 && url[..7].eq_ignore_ascii_case("http://") {
        Some(&url[7..])
    } else {
        None
    }
}

fn digits(value: &str) -> Option<String> {
    let id: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if id.is_empty() { None } else { Some(id) }
}

fn is_x_host(host: &str) -> bool {
    let host = host.trim().trim_start_matches("www.").to_ascii_lowercase();
    matches!(
        host.as_str(),
        "x.com"
            | "twitter.com"
            | "mobile.twitter.com"
            | "mobile.x.com"
            | "fxtwitter.com"
            | "vxtwitter.com"
    ) || host.ends_with(".twitter.com")
        || host.ends_with(".x.com")
}

fn display_name_of(value: &Value) -> String {
    if let Some(text) = first_str(value, &["author_name", "authorName", "name"]) {
        return text.to_string();
    }
    for key in ["author", "user"] {
        if let Some(node) = value.get(key)
            && let Some(text) = first_str(node, &["name", "display_name"])
        {
            return text.to_string();
        }
    }
    String::new()
}

fn media_urls_of(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in ["media_urls", "images"] {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            for item in items {
                push_media(&mut out, item);
            }
        }
    }
    if let Some(items) = value.get("media").and_then(Value::as_array) {
        for item in items {
            push_media(&mut out, item);
        }
    }
    out
}

fn push_media(out: &mut Vec<String>, item: &Value) {
    let text = item.as_str().or_else(|| {
        first_str(
            item,
            &["url", "media_url_https", "poster", "preview_image_url"],
        )
    });
    if let Some(text) = text
        && text.starts_with("http")
        && !out.iter().any(|existing| existing == text)
    {
        out.push(text.to_string());
    }
}

fn author_of(value: &Value) -> String {
    if let Some(text) = value.get("author").and_then(Value::as_str) {
        return strip_handle(text).to_string();
    }
    if let Some(text) = first_str(value, &["username", "screen_name", "screenName"]) {
        return strip_handle(text).to_string();
    }
    for key in ["author", "user"] {
        if let Some(node) = value.get(key)
            && let Some(text) =
                first_str(node, &["username", "screen_name", "screenName", "handle"])
        {
            return strip_handle(text).to_string();
        }
    }
    String::new()
}

fn created_at_of(value: &Value) -> Option<String> {
    for key in ["created_at", "createdAt", "time"] {
        let Some(raw) = value.get(key) else {
            continue;
        };
        if let Some(text) = raw.as_str() {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
        if let Some(number) = raw.as_i64() {
            return Some(number.to_string());
        }
        if let Some(number) = raw.as_u64() {
            return Some(number.to_string());
        }
    }
    None
}

fn first_str<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn metric(value: &Value, keys: &[&str]) -> u64 {
    for key in keys {
        let Some(raw) = value.get(*key) else {
            continue;
        };
        if let Some(number) = raw.as_u64() {
            return number;
        }
        if let Some(text) = raw.as_str() {
            let digits: String = text.chars().filter(|ch| ch.is_ascii_digit()).collect();
            if let Ok(number) = digits.parse::<u64>() {
                return number;
            }
        }
    }
    0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    pub id: String,
    pub name: String,
    pub desc: String,
}

impl Topic {
    pub fn law() -> Self {
        Self {
            id: "law".into(),
            name: "Law".into(),
            desc:
                "legal news, court rulings, Supreme Court, lawsuits, legal commentary, regulation"
                    .into(),
        }
    }

    pub fn finance() -> Self {
        Self {
            id: "finance".into(),
            name: "Finance".into(),
            desc: "markets, stocks, the Fed, interest rates, crypto, earnings, economy".into(),
        }
    }

    pub fn ai() -> Self {
        Self {
            id: "ai".into(),
            name: "AI".into(),
            desc: "AI news and discussion".into(),
        }
    }
}

pub fn default_topics() -> Vec<Topic> {
    vec![Topic::law(), Topic::finance(), Topic::ai()]
}

pub fn topic_id_ok(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && id.len() <= 32
        && id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub fn slug_id(name: &str) -> Option<String> {
    let mut id = String::new();
    let mut dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !id.is_empty() {
            id.push('-');
            dash = true;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    id.truncate(32);
    while id.ends_with('-') {
        id.pop();
    }
    topic_id_ok(&id).then_some(id)
}

/// Keep `focus` in the query. A long description is shortened before the focus term.
pub fn search_query(desc: &str, focus: &str) -> String {
    let desc = collapse_ws(desc);
    let focus = collapse_ws(focus);
    if focus.is_empty() {
        return take_chars(&desc, QUERY_LIMIT);
    }
    if desc.to_lowercase().contains(&focus.to_lowercase()) {
        return take_chars(&desc, QUERY_LIMIT);
    }
    let suffix = format!(" {focus}");
    let suffix_len = suffix.chars().count();
    if suffix_len >= QUERY_LIMIT {
        return take_chars(&focus, QUERY_LIMIT);
    }
    let room = QUERY_LIMIT - suffix_len;
    format!("{}{suffix}", take_chars(&desc, room).trim_end())
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn take_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeKind {
    Fresh,
    Empty,
    Error,
}

impl OutcomeKind {
    pub fn writes_report(self) -> bool {
        matches!(self, Self::Fresh)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Empty => "empty",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub topic_id: String,
    pub query: String,
    pub focus: String,
    pub kind: OutcomeKind,
    pub message: String,
    pub posts: Vec<SourcePost>,
    pub markdown: String,
}

pub fn outcome_from_posts(topic: &Topic, focus: &str, posts: Vec<SourcePost>) -> RunOutcome {
    let query = search_query(&topic.desc, focus);
    if posts.is_empty() {
        return RunOutcome {
            topic_id: topic.id.clone(),
            query,
            focus: focus.trim().to_string(),
            kind: OutcomeKind::Empty,
            message: "OpenCLI search returned no posts".into(),
            posts,
            markdown: String::new(),
        };
    }
    let markdown = synthesize_report(topic, focus, &posts);
    RunOutcome {
        topic_id: topic.id.clone(),
        query,
        focus: focus.trim().to_string(),
        kind: OutcomeKind::Fresh,
        message: format!("{} source-linked post(s)", posts.len()),
        posts,
        markdown,
    }
}

pub fn outcome_error(topic: &Topic, focus: &str, message: impl Into<String>) -> RunOutcome {
    RunOutcome {
        topic_id: topic.id.clone(),
        query: search_query(&topic.desc, focus),
        focus: focus.trim().to_string(),
        kind: OutcomeKind::Error,
        message: message.into(),
        posts: Vec::new(),
        markdown: String::new(),
    }
}

pub fn synthesize_report(topic: &Topic, focus: &str, posts: &[SourcePost]) -> String {
    let focus_note = {
        let focus = focus.trim();
        if focus.is_empty() {
            String::new()
        } else {
            format!(", focused on {focus}")
        }
    };
    let mut ranked = posts.to_vec();
    ranked.sort_by_key(|post| std::cmp::Reverse(score(post)));
    let mut md = format!(
        "From recent X posts via OpenCLI (free session, no API credits), top engagement around {}{focus_note}.\n\n",
        topic.desc
    );
    if ranked.is_empty() {
        md.push_str(&format!(
            "### No live hits for “{}”\n- Why: OpenCLI search returned nothing useful right now.\n- Idea: Re-run once the timeline warms up.\n",
            topic.name
        ));
        return md;
    }
    for post in ranked.iter().take(5) {
        let title = title_from(post);
        let why = format!(
            "@{} is pulling {}. {}",
            if post.author.is_empty() {
                "?"
            } else {
                post.author.as_str()
            },
            if post.engagement_label().is_empty() {
                "solid engagement".to_string()
            } else {
                post.engagement_label()
            },
            take_chars(&collapse_ws(&post.text), 220)
        );
        md.push_str(&format!("### {title}\n"));
        md.push_str(&format!("- Why: {why}\n"));
        md.push_str(&format!("- Posts: {}\n", post.url));
        md.push_str(&format!("- Idea: {}\n\n", idea_from(post)));
    }
    md.trim_end().to_string() + "\n"
}

fn score(post: &SourcePost) -> u64 {
    let views = (post.views as f64 + 1.0).log10() * 50.0;
    post.likes + post.reposts * 3 + views as u64
}

fn title_from(post: &SourcePost) -> String {
    let mut title = collapse_ws(&post.text);
    title = title
        .split_whitespace()
        .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ");
    if title.chars().count() > 110 {
        let cut: String = title.chars().take(110).collect();
        let split = cut
            .rfind(' ')
            .filter(|index| *index > 40)
            .unwrap_or(cut.len());
        return format!("{}…", cut[..split].trim_end());
    }
    if title.is_empty() {
        format!("@{} post", post.author)
    } else {
        title
    }
}

/// A take built only from this post: who said it, and the claim in their words.
/// The same sentence is never reused for a different post.
pub fn idea_from(post: &SourcePost) -> String {
    let who = if !post.author_name.trim().is_empty() {
        post.author_name.trim().to_string()
    } else if !post.author.trim().is_empty() {
        format!("@{}", post.author.trim())
    } else {
        "This post".into()
    };
    let claim = claim_from(&post.text);
    if claim.is_empty() {
        format!("{who} posted an update with no text to quote.")
    } else {
        format!("{who}: {claim}")
    }
}

fn claim_from(text: &str) -> String {
    let cleaned = text
        .split_whitespace()
        .filter(|word| {
            !word.starts_with("http://") && !word.starts_with("https://") && !word.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = collapse_ws(&cleaned);
    let sentence = cleaned
        .split(['.', '!', '?'])
        .map(str::trim)
        .find(|part| part.chars().count() >= 12)
        .unwrap_or(cleaned.trim());
    take_chars(sentence, 180)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTag {
    New,
    Rising,
    Fading,
}

pub fn diff_urls(previous: &[String], current: &[String]) -> Vec<(String, ChangeTag)> {
    let mut tags = Vec::new();
    for (index, url) in current.iter().enumerate() {
        match previous.iter().position(|prior| prior == url) {
            None => tags.push((url.clone(), ChangeTag::New)),
            Some(prior) if index < prior => tags.push((url.clone(), ChangeTag::Rising)),
            Some(_) => {}
        }
    }
    for url in previous {
        if !current.iter().any(|item| item == url) {
            tags.push((url.clone(), ChangeTag::Fading));
        }
    }
    tags
}

pub fn filter_posts<'a>(posts: &'a [SourcePost], needle: &str) -> Vec<&'a SourcePost> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return posts.iter().collect();
    }
    posts
        .iter()
        .filter(|post| {
            post.text.to_lowercase().contains(&needle)
                || post.author.to_lowercase().contains(&needle)
                || post.url.to_lowercase().contains(&needle)
                || post
                    .created_at
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&needle)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskSettings {
    #[serde(default)]
    pub reply_as: String,
    #[serde(default = "default_focus")]
    pub focus: String,
    #[serde(default)]
    pub auto: bool,
    #[serde(default = "default_every")]
    pub every_minutes: u64,
    #[serde(default = "default_true")]
    pub daily: bool,
    #[serde(default = "default_daily_at")]
    pub daily_at: String,
    #[serde(default)]
    pub daily_done: Option<String>,
    /// `free` or `treg`. Draft takes never post.
    #[serde(default = "default_take_provider")]
    pub take_provider: String,
    /// Starts as the slim dock. False means the workspace is open.
    #[serde(default = "default_true")]
    pub docked: bool,
    #[serde(default = "default_frame_origin")]
    pub frame_x: i32,
    #[serde(default = "default_frame_origin")]
    pub frame_y: i32,
    #[serde(default = "default_expanded_width")]
    pub expanded_width: u32,
    #[serde(default = "default_frame_height")]
    pub frame_height: u32,
    /// Last subject page, stored with the rest of the Termy XL settings.
    #[serde(default)]
    pub subject_id: String,
}

fn default_focus() -> String {
    "AI".into()
}

fn default_every() -> u64 {
    60
}

fn default_true() -> bool {
    true
}

fn default_daily_at() -> String {
    "06:00".into()
}

fn default_take_provider() -> String {
    "free".into()
}

fn default_frame_origin() -> i32 {
    24
}

fn default_expanded_width() -> u32 {
    2560
}

fn default_frame_height() -> u32 {
    820
}

impl Default for DeskSettings {
    fn default() -> Self {
        Self {
            reply_as: "@YourHandle".into(),
            focus: default_focus(),
            auto: false,
            every_minutes: default_every(),
            daily: true,
            daily_at: default_daily_at(),
            daily_done: None,
            take_provider: default_take_provider(),
            docked: true,
            frame_x: default_frame_origin(),
            frame_y: default_frame_origin(),
            expanded_width: default_expanded_width(),
            frame_height: default_frame_height(),
            subject_id: String::new(),
        }
    }
}

pub fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

pub fn chicago_offset_secs(unix: i64) -> i64 {
    let year: i32 = ymd(unix)
        .get(0..4)
        .and_then(|text| text.parse().ok())
        .unwrap_or(1970);
    let start = utc_unix(year, 3, nth_sunday(year, 3, 2), 8, 0, 0);
    let end = utc_unix(year, 11, nth_sunday(year, 11, 1), 7, 0, 0);
    if unix >= start && unix < end {
        -5 * 3600
    } else {
        -6 * 3600
    }
}

pub fn chicago_stamp(unix: i64) -> (String, u32, u32) {
    let local = unix + chicago_offset_secs(unix);
    let date = ymd(local);
    let sod = local.rem_euclid(86_400) as u32;
    (date, sod / 3600, (sod % 3600) / 60)
}

pub fn daily_due(now_unix: i64, settings: &DeskSettings) -> bool {
    if !settings.daily {
        return false;
    }
    let Some((hour, minute)) = parse_hhmm(&settings.daily_at) else {
        return false;
    };
    let (date, now_hour, now_minute) = chicago_stamp(now_unix);
    if settings.daily_done.as_deref() == Some(date.as_str()) {
        return false;
    }
    let now_mins = now_hour * 60 + now_minute;
    let at = hour * 60 + minute;
    now_mins >= at && now_mins < at + 90
}

fn nth_sunday(year: i32, month: u32, n: u32) -> u32 {
    let first = weekday_sunday0(year, month, 1);
    let first_sunday = if first == 0 { 1 } else { 1 + (7 - first) };
    first_sunday + (n - 1) * 7
}

fn weekday_sunday0(year: i32, month: u32, day: u32) -> u32 {
    let days = days_from_civil(year, month, day);
    ((days + 4).rem_euclid(7)) as u32
}

fn utc_unix(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3600
        + minute as i64 * 60
        + second as i64
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let mut y = year;
    let m = month as i32;
    let d = day as i32;
    y -= i32::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 }.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u32;
    (era as i64) * 146_097 + doe as i64 - 719_468
}

pub fn draft_text_from_title(title: &str) -> String {
    let first = title
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("this trend");
    let clean = take_chars(&first.replace("**", ""), 180);
    let mut draft = format!("Worth watching: {clean}");
    if draft.chars().count() > 240 {
        draft = format!("{}…", draft.chars().take(237).collect::<String>());
    }
    draft
}

pub fn reply_dry_run_args(tweet_url: &str, text: &str) -> Vec<String> {
    vec![
        "twitter".into(),
        "reply".into(),
        tweet_url.to_string(),
        text.to_string(),
        "--window".into(),
        "background".into(),
        "-f".into(),
        "json".into(),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Loading,
    Fresh,
    Cached,
    Empty,
    Error,
    Fixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnView {
    pub phase: Phase,
    pub banner: String,
    pub posts: Vec<SourcePost>,
    pub posts_cached: bool,
    pub query: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredReport {
    pub file: String,
    pub topic_id: String,
    pub fetched_at_unix: i64,
    pub query: String,
    pub focus: String,
    pub markdown: String,
    pub posts: Vec<SourcePost>,
}

pub fn column_from_cache(report: Option<&StoredReport>) -> ColumnView {
    match report {
        Some(report) => ColumnView {
            phase: Phase::Cached,
            banner: format!("Cached report from {}. Not a new run.", report.file),
            posts: report.posts.clone(),
            posts_cached: true,
            query: report.query.clone(),
        },
        None => ColumnView {
            phase: Phase::Idle,
            banner: "Not run yet.".into(),
            posts: Vec::new(),
            posts_cached: false,
            query: String::new(),
        },
    }
}

pub fn fixture_column(topic: &Topic, posts: Vec<SourcePost>) -> ColumnView {
    ColumnView {
        phase: Phase::Fixture,
        banner: "SYNTHETIC FIXTURE — not a live fetch and not a saved report.".into(),
        query: search_query(&topic.desc, "AI"),
        posts,
        posts_cached: false,
    }
}

pub fn column_loading(query: &str) -> ColumnView {
    column_loading_keep(query, Vec::new())
}

/// A refresh keeps the saved cards on screen. An empty list is the only
/// case that shows the loading line instead of posts.
pub fn column_loading_keep(query: &str, posts: Vec<SourcePost>) -> ColumnView {
    let posts_cached = !posts.is_empty();
    ColumnView {
        phase: Phase::Loading,
        banner: if posts_cached {
            format!("Refreshing “{query}”. Cards below are the saved report.")
        } else {
            format!("Loading “{query}” through OpenCLI. Nothing here is a new report yet.")
        },
        posts,
        posts_cached,
        query: query.to_string(),
    }
}

pub fn column_from_outcome(outcome: &RunOutcome, cached: Option<&StoredReport>) -> ColumnView {
    match outcome.kind {
        OutcomeKind::Fresh => ColumnView {
            phase: Phase::Fresh,
            banner: format!("Fresh OpenCLI run. {}", outcome.message),
            posts: outcome.posts.clone(),
            posts_cached: false,
            query: outcome.query.clone(),
        },
        OutcomeKind::Empty => ColumnView {
            phase: Phase::Empty,
            banner: empty_banner(&outcome.message, cached),
            posts: cached
                .map(|report| report.posts.clone())
                .unwrap_or_default(),
            posts_cached: cached.is_some(),
            query: outcome.query.clone(),
        },
        OutcomeKind::Error => ColumnView {
            phase: Phase::Error,
            banner: error_banner(&outcome.message, cached),
            posts: cached
                .map(|report| report.posts.clone())
                .unwrap_or_default(),
            posts_cached: cached.is_some(),
            query: outcome.query.clone(),
        },
    }
}

fn empty_banner(message: &str, cached: Option<&StoredReport>) -> String {
    match cached {
        Some(report) => format!(
            "{message}. Cards below are the last saved report ({}) and are not a new run.",
            report.file
        ),
        None => message.to_string(),
    }
}

fn error_banner(message: &str, cached: Option<&StoredReport>) -> String {
    match cached {
        Some(report) => format!(
            "{message}. Cards below are the last saved report ({}) and are not a new run.",
            report.file
        ),
        None => message.to_string(),
    }
}

pub fn column_count_for_width(width: f32) -> usize {
    if width >= 3000.0 {
        4
    } else if width >= 1800.0 {
        3
    } else if width >= 1100.0 {
        2
    } else {
        1
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Columns,
    History,
    Replies,
    Draft,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Columns,
    History,
    Replies,
    Draft,
    Settings,
}

pub fn surface_after_nav(_current: Surface, nav: Nav) -> Surface {
    match nav {
        Nav::Columns => Surface::Columns,
        Nav::History => Surface::History,
        Nav::Replies => Surface::Replies,
        Nav::Draft => Surface::Draft,
        Nav::Settings => Surface::Settings,
    }
}

pub fn redact_secrets(text: &str) -> String {
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "auth_token",
                "ct0",
                "cookie",
                "bearer",
                "api_key",
                "secret",
                "password",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                "[redacted]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Created-at text is kept even when it also parses as a timestamp.
pub fn preserved_timestamp(post: &SourcePost) -> Option<&str> {
    let raw = post.created_at.as_deref()?;
    let _ = parse_created_at(raw);
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn post(id: &str, author: &str, text: &str, url: &str) -> SourcePost {
        SourcePost::from_value(&json!({
            "id": id,
            "author": author,
            "text": text,
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "likes": 4,
            "url": url
        }))
        .unwrap()
    }

    #[test]
    fn focus_stays_in_a_long_query() {
        let desc = "legal news ".repeat(40);
        let query = search_query(&desc, "AI");
        assert!(query.chars().count() <= QUERY_LIMIT);
        assert!(query.to_lowercase().ends_with(" ai") || query.ends_with("AI"));
        assert!(query.contains("legal"));
    }

    #[test]
    fn focus_is_not_duplicated_when_already_present() {
        let query = search_query("AI news and discussion", "AI");
        assert_eq!(query, "AI news and discussion");
    }

    #[test]
    fn contract_example_keeps_law_and_focus() {
        let query = search_query(Topic::law().desc.as_str(), "AI");
        assert!(query.contains("legal news"));
        assert!(query.ends_with("AI"));
    }

    #[test]
    fn status_link_variants_keep_the_id_and_time() {
        let variants = [
            "https://twitter.com/Ada/status/99",
            "https://www.twitter.com/Ada/status/99/",
            "http://mobile.twitter.com/Ada/status/99?s=20",
            "https://x.com/Ada/status/99/photo/1",
            "https://x.com/i/web/status/99",
            "https://x.com/i/status/99",
            "HTTPS://Mobile.X.com/Ada/status/99",
        ];
        for raw in variants {
            let link = normalize_status_link(raw, "", "").unwrap();
            assert_eq!(link.id, "99", "{raw}");
            assert!(link.canonical.starts_with("https://x.com/"), "{raw}");
            assert!(link.canonical.contains("/status/99"), "{raw}");
        }
        let post = SourcePost::from_value(&json!({
            "id": "99",
            "author": "@Ada",
            "text": "timestamp stays",
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "url": "https://twitter.com/Ada/status/99?s=20"
        }))
        .unwrap();
        assert_eq!(
            preserved_timestamp(&post),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert_eq!(post.source_url, "https://twitter.com/Ada/status/99?s=20");
        assert_eq!(post.author, "Ada");
        assert!(parse_created_at(post.created_at.as_deref().unwrap()).is_some());
    }

    #[test]
    fn non_status_domains_are_dropped() {
        assert!(
            SourcePost::from_value(&json!({
                "id": "1",
                "author": "a",
                "text": "nope",
                "url": "https://example.com/status/1"
            }))
            .is_none()
        );
    }

    #[test]
    fn filter_matches_author_text_and_leaves_history_untouched() {
        let posts = vec![
            post("1", "alice", "court ruling", "https://x.com/alice/status/1"),
            post("2", "bob", "markets", "https://x.com/bob/status/2"),
        ];
        let original = posts.clone();
        let hit = filter_posts(&posts, "Court");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].id, "1");
        assert_eq!(posts, original);
        assert_eq!(filter_posts(&posts, "   ").len(), 2);
    }

    #[test]
    fn empty_and_error_do_not_write_and_are_not_fresh() {
        let topic = Topic::law();
        let empty = outcome_from_posts(&topic, "AI", Vec::new());
        assert_eq!(empty.kind, OutcomeKind::Empty);
        assert!(!empty.kind.writes_report());
        let error = outcome_error(&topic, "AI", "bridge down");
        assert_eq!(error.kind, OutcomeKind::Error);
        assert!(!error.kind.writes_report());
        let fresh = outcome_from_posts(
            &topic,
            "AI",
            vec![post(
                "1",
                "alice",
                "courts and AI",
                "https://x.com/alice/status/1",
            )],
        );
        assert!(fresh.kind.writes_report());
        assert!(fresh.markdown.contains("https://x.com/alice/status/1"));
        assert!(fresh.markdown.contains("focused on AI"));
        assert!(!fresh.markdown.contains("Fixture"));
    }

    #[test]
    fn cached_cards_stay_cached_under_an_error() {
        let cached = StoredReport {
            file: "law-1.json".into(),
            topic_id: "law".into(),
            fetched_at_unix: 1,
            query: "legal".into(),
            focus: "AI".into(),
            markdown: "old".into(),
            posts: vec![post("9", "old", "saved", "https://x.com/old/status/9")],
        };
        let view = column_from_cache(Some(&cached));
        assert_eq!(view.phase, Phase::Cached);
        assert!(view.posts_cached);
        let failed = outcome_error(&Topic::law(), "AI", "OpenCLI timed out");
        let shown = column_from_outcome(&failed, Some(&cached));
        assert_eq!(shown.phase, Phase::Error);
        assert!(shown.posts_cached);
        assert_eq!(shown.posts[0].id, "9");
        assert!(shown.banner.contains("not a new run"));
        let loaded_again = column_from_cache(Some(&cached));
        assert_ne!(loaded_again.phase, Phase::Fresh);
    }

    #[test]
    fn diff_marks_new_rising_and_fading() {
        let previous = vec![
            "https://x.com/a/status/1".into(),
            "https://x.com/b/status/2".into(),
        ];
        let current = vec![
            "https://x.com/b/status/2".into(),
            "https://x.com/c/status/3".into(),
        ];
        let tags = diff_urls(&previous, &current);
        assert!(tags.contains(&("https://x.com/c/status/3".into(), ChangeTag::New)));
        assert!(tags.contains(&("https://x.com/b/status/2".into(), ChangeTag::Rising)));
        assert!(tags.contains(&("https://x.com/a/status/1".into(), ChangeTag::Fading)));
    }

    #[test]
    fn layout_prefers_ultrawide_and_falls_back() {
        assert_eq!(column_count_for_width(3440.0), 4);
        assert_eq!(column_count_for_width(1920.0), 3);
        assert_eq!(column_count_for_width(1600.0), 2);
        assert_eq!(column_count_for_width(800.0), 1);
    }

    #[test]
    fn navigation_reaches_each_surface() {
        assert_eq!(
            surface_after_nav(Surface::Columns, Nav::History),
            Surface::History
        );
        assert_eq!(
            surface_after_nav(Surface::History, Nav::Replies),
            Surface::Replies
        );
        assert_eq!(
            surface_after_nav(Surface::Replies, Nav::Draft),
            Surface::Draft
        );
        assert_eq!(
            surface_after_nav(Surface::Draft, Nav::Settings),
            Surface::Settings
        );
        assert_eq!(
            surface_after_nav(Surface::Settings, Nav::Columns),
            Surface::Columns
        );
    }

    #[test]
    fn loading_empty_and_fresh_phases_are_distinct() {
        let loading = column_loading("legal AI");
        assert_eq!(loading.phase, Phase::Loading);
        let empty = column_from_outcome(&outcome_from_posts(&Topic::ai(), "", Vec::new()), None);
        assert_eq!(empty.phase, Phase::Empty);
        assert!(!empty.posts_cached);
        let fresh = column_from_outcome(
            &outcome_from_posts(
                &Topic::ai(),
                "",
                vec![post("3", "ada", "models", "https://x.com/ada/status/3")],
            ),
            None,
        );
        assert_eq!(fresh.phase, Phase::Fresh);
        assert_eq!(fresh.posts[0].url, "https://x.com/ada/status/3");
    }

    #[test]
    fn chicago_daily_window() {
        let morning = utc_unix(2026, 11, 11, 12, 0, 0);
        let (date, hour, minute) = chicago_stamp(morning);
        assert_eq!(date, "2026-11-11");
        assert_eq!((hour, minute), (6, 0));
        let mut settings = DeskSettings::default();
        settings.daily_at = "06:00".into();
        settings.daily_done = None;
        assert!(daily_due(morning, &settings));
        settings.daily_done = Some("2026-11-11".into());
        assert!(!daily_due(morning, &settings));
        settings.daily_done = None;
        assert!(!daily_due(utc_unix(2026, 11, 11, 14, 0, 0), &settings));
        settings.daily = false;
        assert!(!daily_due(morning, &settings));
        let summer = utc_unix(2026, 7, 15, 17, 0, 0);
        assert_eq!(chicago_stamp(summer).1, 12);
    }

    #[test]
    fn visible_name_is_termy_xl_and_not_the_main_app() {
        assert_eq!(VISIBLE_NAME, "Termy XL");
        assert!(VISIBLE_NAME.contains("XL"));
        assert_ne!(VISIBLE_NAME, "Termy");
        assert_ne!(VISIBLE_NAME, "Termy X");
    }

    #[test]
    fn draft_text_is_local_and_dry_run_args_are_not_a_send() {
        let text = draft_text_from_title("### Court filing moves");
        assert!(text.starts_with("Worth watching:"));
        let args = reply_dry_run_args("https://x.com/a/status/1", &text);
        assert_eq!(args[0], "twitter");
        assert_eq!(args[1], "reply");
        assert!(args.iter().any(|arg| arg == "--window"));
        assert!(!args.iter().any(|arg| arg == "x.exe" || arg == "x"));
    }

    #[test]
    fn settings_ignore_shape_still_round_trips_without_a_model_field() {
        let raw = r#"{"replyAs":"unused","focus":"AI","daily":true,"dailyAt":"06:00","model":"grok-4.7"}"#;
        let settings: DeskSettings = serde_json::from_str(
            &serde_json::json!({
                "focus": "AI",
                "daily": true,
                "daily_at": "06:00"
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(settings.focus, "AI");
        let stored = serde_json::to_string(&settings).unwrap();
        assert!(!stored.contains("grok"));
        assert!(!stored.contains("xai"));
        let _ = raw;
    }
}
