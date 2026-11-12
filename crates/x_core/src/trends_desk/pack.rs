//! Termy XL extras: heat, tone, variants, filters, hit list, receipts, keys.
//!
//! Nothing here posts. A key never maps to send. Treg failures stay errors.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::domain::SourcePost;
use super::{TakeProvider, char_count};
use crate::time::parse_created_at;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeTone {
    Spicy,
    Funny,
    Data,
    Contrarian,
    Supportive,
    Hype,
}

impl TakeTone {
    pub fn all() -> [Self; 6] {
        [
            Self::Spicy,
            Self::Funny,
            Self::Data,
            Self::Contrarian,
            Self::Supportive,
            Self::Hype,
        ]
    }

    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "funny" => Self::Funny,
            "data" => Self::Data,
            "contrarian" => Self::Contrarian,
            "supportive" => Self::Supportive,
            "hype" => Self::Hype,
            _ => Self::Spicy,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spicy => "Spicy",
            Self::Funny => "Funny",
            Self::Data => "Data",
            Self::Contrarian => "Contrarian",
            Self::Supportive => "Supportive",
            Self::Hype => "Hype",
        }
    }
}

/// Engagement per hour from counts already on the post. Missing time is not a rate.
pub fn heat_score(post: &SourcePost, now_unix: i64) -> Option<f64> {
    let posted = parse_created_at(post.created_at.as_deref()?)?;
    let elapsed = (now_unix - posted).max(0) as f64;
    let hours = (elapsed / 3600.0).max(1.0 / 60.0);
    let engagement = post.likes as f64
        + post.reposts as f64 * 2.0
        + post.replies as f64
        + post.views as f64 / 100.0;
    Some(engagement / hours)
}

/// Hide a zero or missing rate. "# 0/h" is not a badge.
pub fn heat_badge(post: &SourcePost, now_unix: i64) -> Option<String> {
    let score = heat_score(post, now_unix)?;
    if score < 1.0 {
        return None;
    }
    Some(heat_line(post, now_unix))
}

pub fn heat_line(post: &SourcePost, now_unix: i64) -> String {
    match heat_score(post, now_unix) {
        Some(score) => {
            let bars = if score >= 400.0 {
                5
            } else if score >= 150.0 {
                4
            } else if score >= 40.0 {
                3
            } else if score >= 8.0 {
                2
            } else if score > 0.0 {
                1
            } else {
                0
            };
            format!("{} {score:.0}/h", "#".repeat(bars))
        }
        None => "heat n/a".into(),
    }
}

pub fn sort_by_heat(posts: &[&SourcePost], now_unix: i64) -> Vec<String> {
    let mut rows: Vec<(String, f64)> = posts
        .iter()
        .map(|post| (post.id.clone(), heat_score(post, now_unix).unwrap_or(0.0)))
        .collect();
    rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    rows.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
pub fn three_variants(text: &str, tone: TakeTone) -> Vec<String> {
    let name = tone.label();
    fallback_angles(text)
        .into_iter()
        .map(|line| format!("{name} · {line}"))
        .collect()
}

/// Three fallback replies. Test-only. Cards and the compose box never call this.
#[cfg(test)]
pub fn fallback_angles(text: &str) -> Vec<String> {
    let claim = text
        .split_whitespace()
        .filter(|word| !word.starts_with("http://") && !word.starts_with("https://"))
        .collect::<Vec<_>>()
        .join(" ");
    let claim: String = claim.chars().take(140).collect();
    let claim = claim.trim();
    let claim = if claim.is_empty() { "this post" } else { claim };
    vec![
        format!("Agree, and one thing to add: {claim}"),
        format!("Question: what changes if \"{claim}\" is only half true?"),
        format!("Contrarian: {claim} still skips who pays the cost."),
    ]
}

/// Keep model lines that differ. Templates are not added to fill a short list.
pub fn distinct_variants(_seed: &str, generated: &[String]) -> Vec<String> {
    super::takes::accept_lines(generated)
}

pub fn flip_variant(index: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let next = index as i32 + delta;
    if next < 0 {
        len - 1
    } else if next >= len as i32 {
        0
    } else {
        next as usize
    }
}

/// Treg must already have a result. Free, Hermes, and Grok do not invent
/// template replies; the take chain asks a model instead.
pub fn provider_drafts(
    text: &str,
    tone: TakeTone,
    provider: TakeProvider,
    treg: Result<String, String>,
) -> Result<Vec<String>, String> {
    let _ = (text, tone);
    match provider {
        TakeProvider::Free | TakeProvider::Hermes | TakeProvider::Grok => {
            Err("model reply required".into())
        }
        TakeProvider::Treg => {
            let draft = treg?;
            if draft.trim().is_empty() {
                return Err("Treg returned an empty draft.".into());
            }
            Ok(vec![format!("{} · {draft}", tone.label())])
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerRead {
    pub weighted: usize,
    pub limit: usize,
    pub over: bool,
    pub mentions: usize,
    pub links: usize,
}

pub fn read_composer(text: &str) -> ComposerRead {
    let weighted = char_count(text);
    let mentions = text
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '@' && ch != '_')
        .filter(|token| {
            let rest = token.trim_start_matches('@');
            token.starts_with('@')
                && rest.len() >= 2
                && rest
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        })
        .count();
    let links = text.matches("http://").count() + text.matches("https://").count();
    ComposerRead {
        weighted,
        limit: 280,
        over: weighted > 280,
        mentions,
        links,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HitItem {
    pub id: String,
    pub url: String,
    pub author: String,
    pub text: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyMark {
    pub card_id: String,
    pub url: String,
    pub at_unix: i64,
    pub status: String,
    #[serde(default)]
    pub text: String,
}

pub fn replied_badge(marks: &[ReplyMark], card_id: &str) -> Option<String> {
    marks
        .iter()
        .rev()
        .find(|mark| mark.card_id == card_id && mark.status == "confirmed")
        .map(|mark| format!("replied · {}", mark.at_unix))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardFilter {
    #[serde(default)]
    pub min_likes: u64,
    #[serde(default)]
    pub last_hours: Option<u64>,
    #[serde(default)]
    pub has_media: bool,
    #[serde(default)]
    pub verified_only: bool,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub mute_words: Vec<String>,
    #[serde(default)]
    pub mute_accounts: Vec<String>,
}

pub fn apply_filters<'a>(
    posts: &[&'a SourcePost],
    filter: &CardFilter,
    now_unix: i64,
) -> Vec<&'a SourcePost> {
    posts
        .iter()
        .copied()
        .filter(|post| post.likes >= filter.min_likes)
        .filter(|post| !filter.has_media || !post.media.is_empty())
        .filter(|post| !filter.verified_only || post.verified)
        .filter(|post| {
            let want = filter.language.trim();
            want.is_empty() || post.lang.eq_ignore_ascii_case(want)
        })
        .filter(|post| match filter.last_hours {
            None => true,
            Some(hours) => parse_created_at(post.created_at.as_deref().unwrap_or(""))
                .is_some_and(|posted| now_unix.saturating_sub(posted) <= hours as i64 * 3600),
        })
        .filter(|post| {
            let text = post.text.to_lowercase();
            !filter
                .mute_words
                .iter()
                .any(|word| !word.trim().is_empty() && text.contains(&word.to_lowercase()))
        })
        .filter(|post| {
            let author = post.author.trim_start_matches('@').to_lowercase();
            !filter.mute_accounts.iter().any(|name| {
                name.trim()
                    .trim_start_matches('@')
                    .eq_ignore_ascii_case(&author)
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFile {
    #[serde(default)]
    pub hits: Vec<HitItem>,
    #[serde(default)]
    pub receipts: Vec<ReplyMark>,
    #[serde(default)]
    pub filters: CardFilter,
    #[serde(default = "default_tone")]
    pub tone: String,
    #[serde(default)]
    pub sort_heat: bool,
    #[serde(default = "default_true")]
    pub refresh_on: bool,
    #[serde(default = "default_refresh_minutes")]
    pub refresh_minutes: u64,
    #[serde(default)]
    pub last_refresh: Option<i64>,
    #[serde(default)]
    pub seen: HashMap<String, Vec<String>>,
}

fn default_tone() -> String {
    "spicy".into()
}
fn default_true() -> bool {
    true
}
fn default_refresh_minutes() -> u64 {
    10
}

impl Default for PackFile {
    fn default() -> Self {
        Self {
            hits: Vec::new(),
            receipts: Vec::new(),
            filters: CardFilter::default(),
            tone: default_tone(),
            sort_heat: false,
            refresh_on: true,
            refresh_minutes: 10,
            last_refresh: None,
            seen: HashMap::new(),
        }
    }
}

pub fn load_pack(root: &Path) -> PackFile {
    fs::read_to_string(root.join("pack.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_pack(root: &Path, pack: &PackFile) -> Result<(), String> {
    if super::within_termy_x_config(root) {
        return Err("refusing to write the Termy X config tree".into());
    }
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(pack).map_err(|error| error.to_string())?;
    fs::write(root.join("pack.json"), text).map_err(|error| error.to_string())
}

pub fn new_ids(seen: &[String], current: &[String]) -> Vec<String> {
    current
        .iter()
        .filter(|id| !seen.iter().any(|old| old == *id))
        .cloned()
        .collect()
}

pub fn refresh_due(last: Option<i64>, now: i64, enabled: bool, minutes: u64) -> bool {
    if !enabled || minutes == 0 {
        return false;
    }
    match last {
        None => true,
        Some(at) => now.saturating_sub(at) >= minutes as i64 * 60,
    }
}

/// A stopped free-model session does not block the read-only post search.
pub fn should_auto_refresh(
    last: Option<i64>,
    now: i64,
    enabled: bool,
    minutes: u64,
    busy: bool,
    model_halted: bool,
) -> bool {
    let _ = model_halted;
    !busy && refresh_due(last, now, enabled, minutes)
}

pub fn refresh_is_search_only(args: &[String]) -> bool {
    args.first().map(String::as_str) == Some("twitter")
        && args.get(1).map(String::as_str) == Some("search")
        && !args.iter().any(|arg| arg == "reply" || arg == "post")
}

pub fn local_offset_secs(now: i64) -> i32 {
    #[cfg(windows)]
    {
        let _ = now;
        #[repr(C)]
        struct SystemTime {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            milliseconds: u16,
        }
        #[repr(C)]
        struct TzInfo {
            bias: i32,
            standard_name: [u16; 32],
            standard_date: SystemTime,
            standard_bias: i32,
            daylight_name: [u16; 32],
            daylight_date: SystemTime,
            daylight_bias: i32,
        }
        unsafe extern "system" {
            fn GetTimeZoneInformation(info: *mut TzInfo) -> u32;
        }
        let mut info = std::mem::MaybeUninit::<TzInfo>::zeroed();
        let kind = unsafe { GetTimeZoneInformation(info.as_mut_ptr()) };
        let info = unsafe { info.assume_init() };
        let extra = match kind {
            2 => info.daylight_bias,
            1 => info.standard_bias,
            _ => 0,
        };
        -(info.bias + extra) * 60
    }
    #[cfg(not(windows))]
    {
        unix_local_offset(now)
    }
}

#[cfg(not(windows))]
fn unix_local_offset(now: i64) -> i32 {
    #[cfg(unix)]
    {
        #[repr(C)]
        struct Tm {
            tm_sec: i32,
            tm_min: i32,
            tm_hour: i32,
            tm_mday: i32,
            tm_mon: i32,
            tm_year: i32,
            tm_wday: i32,
            tm_yday: i32,
            tm_isdst: i32,
            tm_gmtoff: i64,
            tm_zone: *const i8,
        }
        unsafe extern "C" {
            fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
        }
        let mut tm = std::mem::MaybeUninit::<Tm>::zeroed();
        let ptr = unsafe { localtime_r(&now, tm.as_mut_ptr()) };
        if ptr.is_null() {
            0
        } else {
            unsafe { tm.assume_init().tm_gmtoff as i32 }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = now;
        0
    }
}

pub fn format_post_time(raw: &str, now: i64, offset_secs: i32) -> String {
    let Some(unix) = crate::time::parse_created_at(raw) else {
        return raw.to_string();
    };
    let age = now.saturating_sub(unix);
    if now >= unix && age < 60 {
        return "just now".into();
    }
    if now >= unix && age < 3600 {
        return format!("{}m ago", (age / 60).max(1));
    }
    if now >= unix && age < 36 * 3600 {
        return format!("{}h ago", (age / 3600).max(1));
    }
    let local = unix + offset_secs as i64;
    let date = crate::time::ymd(local);
    let month = match date.get(5..7).unwrap_or("01") {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        _ => "Dec",
    };
    let day = date.get(8..10).unwrap_or("01").trim_start_matches('0');
    format!("{month} {day}, {}", format_clock(unix, offset_secs))
}

pub fn next_refresh_text(
    last: Option<i64>,
    now: i64,
    enabled: bool,
    minutes: u64,
    offset_secs: i32,
) -> String {
    if !enabled {
        return "auto-refresh off".into();
    }
    let Some(last) = last else {
        return "next refresh now".into();
    };
    let due = last.saturating_add(minutes as i64 * 60);
    if now >= due {
        "next refresh now".into()
    } else {
        format!("next refresh {}", format_clock(due, offset_secs))
    }
}

pub fn format_clock(unix: i64, offset_secs: i32) -> String {
    let local = unix + offset_secs as i64;
    let rem = local.rem_euclid(86_400);
    let hour24 = (rem / 3600) as u32;
    let minute = (rem % 3600) / 60;
    let (hour, suffix) = match hour24 {
        0 => (12, "AM"),
        1..=11 => (hour24, "AM"),
        12 => (12, "PM"),
        _ => (hour24 - 12, "PM"),
    };
    format!("{hour}:{minute:02} {suffix}")
}

pub fn hud_line(
    sent_today: usize,
    drafts: usize,
    provider: &str,
    last_refresh: Option<i64>,
    offset_secs: i32,
) -> String {
    let refresh =
        last_refresh.map_or_else(|| "never".to_string(), |at| format_clock(at, offset_secs));
    format!("{sent_today} sent today · {drafts} drafts · {provider} · refresh {refresh}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskKey {
    Select(i32),
    Page(i32),
    Draft,
    Reply,
    Open,
    Star,
    Palette,
    Unbound,
}

pub fn desk_key(key: &str, ctrl: bool, shift: bool) -> DeskKey {
    if ctrl && !shift && key == "k" {
        return DeskKey::Palette;
    }
    if ctrl || shift {
        return DeskKey::Unbound;
    }
    match key {
        "j" => DeskKey::Select(1),
        "k" => DeskKey::Select(-1),
        "h" | "left" | "up" | "pageup" | "page_up" => DeskKey::Page(-1),
        "l" | "right" | "down" | "pagedown" | "page_down" => DeskKey::Page(1),
        "d" => DeskKey::Draft,
        "r" => DeskKey::Reply,
        "o" => DeskKey::Open,
        "s" => DeskKey::Star,
        _ => DeskKey::Unbound,
    }
}

pub fn search_label(editing: bool, text: &str) -> String {
    if editing {
        format!("{text}|")
    } else if text.is_empty() {
        "Search: click to edit".into()
    } else {
        format!("Search: {text}")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SearchEdit {
    Text(String),
    Submit(String),
    Close,
    Ignore,
}

pub fn search_edit(text: &str, key: &str, ch: Option<&str>) -> SearchEdit {
    match key {
        "enter" => SearchEdit::Submit(text.trim().to_string()),
        "escape" => SearchEdit::Close,
        "backspace" => {
            let mut next: String = text.chars().collect();
            next.pop();
            SearchEdit::Text(next)
        }
        _ => {
            let Some(ch) = ch.filter(|value| !value.is_empty()) else {
                return SearchEdit::Ignore;
            };
            if ch.chars().any(|item| item.is_control()) {
                return SearchEdit::Ignore;
            }
            let mut next = text.to_string();
            if next.chars().count() < 240 {
                next.push_str(ch);
            }
            SearchEdit::Text(next)
        }
    }
}

pub fn search_page_name(term: &str) -> String {
    format!("Search: {term}")
}

/// Page keys move the subject. Escape is not one of them.
pub fn page_after_key(index: usize, count: usize, key: &str) -> Option<usize> {
    match desk_key(key, false, false) {
        DeskKey::Page(delta) => Some(super::turn_page(index, count, delta)),
        _ => None,
    }
}

pub fn sends_on_key(key: &str, ctrl: bool, shift: bool) -> bool {
    match desk_key(key, ctrl, shift) {
        DeskKey::Select(_)
        | DeskKey::Page(_)
        | DeskKey::Draft
        | DeskKey::Reply
        | DeskKey::Open
        | DeskKey::Star
        | DeskKey::Palette
        | DeskKey::Unbound => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn post(id: &str, likes: u64, hours_ago: i64, extra: serde_json::Value) -> SourcePost {
        let mut value = json!({
            "id": id,
            "author": "ada",
            "text": format!("note {id}"),
            "likes": likes,
            "reposts": 1,
            "replies": 0,
            "views": 100,
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "url": format!("https://x.com/ada/status/{id}")
        });
        if let Some(map) = extra.as_object() {
            for (key, item) in map {
                value[key] = item.clone();
            }
        }
        let post = SourcePost::from_value(&value).unwrap();
        let posted = parse_created_at(post.created_at.as_deref().unwrap()).unwrap();
        let _ = hours_ago;
        let _ = posted;
        post
    }

    #[test]
    fn heat_uses_engagement_per_hour_and_sorts() {
        let now = parse_created_at("Wed Nov 11 18:00:00 +0000 2026").unwrap();
        let hot = post("1", 100, 2, json!({}));
        let cold = post("2", 1, 2, json!({}));
        let missing = SourcePost::from_value(&json!({
            "id": "3",
            "author": "ada",
            "text": "no time",
            "likes": 50,
            "url": "https://x.com/ada/status/3"
        }))
        .unwrap();
        let hot_score = heat_score(&hot, now).unwrap();
        let cold_score = heat_score(&cold, now).unwrap();
        assert!(hot_score > cold_score);
        assert!(heat_score(&missing, now).is_none());
        assert!(heat_line(&missing, now).contains("n/a"));
        let order = sort_by_heat(&[&cold, &hot], now);
        assert_eq!(order, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn tone_reaches_free_variants_and_treg_errors_stay_errors() {
        let spicy = three_variants("rates moved", TakeTone::Spicy);
        assert_eq!(spicy.len(), 3);
        assert!(spicy.iter().all(|line| line.contains("Spicy")));
        assert_ne!(spicy[0], spicy[1]);
        assert_ne!(spicy[1], spicy[2]);
        assert_ne!(spicy[0], spicy[2]);
        let fallback = fallback_angles("Courts are testing whether models can write opinions");
        assert_ne!(fallback[0], fallback[1]);
        assert_ne!(fallback[1], fallback[2]);
        assert_ne!(fallback[0], fallback[2]);
        let repeated = distinct_variants(
            "Courts are testing whether models can write opinions",
            &["same line".into(), "same line".into(), "same line".into()],
        );
        assert_eq!(repeated, vec!["same line".to_string()]);
        let funny = provider_drafts(
            "rates",
            TakeTone::Funny,
            TakeProvider::Free,
            Err("no".into()),
        )
        .unwrap_err();
        assert!(funny.contains("model reply required"));
        assert!(!funny.contains("Agree, and one thing to add"));
        let err = provider_drafts(
            "rates",
            TakeTone::Data,
            TakeProvider::Treg,
            Err("treg: no key".into()),
        )
        .unwrap_err();
        assert!(err.contains("treg"));
        assert!(!err.contains("Funny"));
        assert_eq!(flip_variant(2, 1, 3), 0);
        assert_eq!(flip_variant(0, -1, 3), 2);
    }

    #[test]
    fn composer_counts_limit_mentions_and_links() {
        let read = read_composer("hi @ada see https://x.com/ada/status/1");
        assert_eq!(read.limit, 280);
        assert_eq!(read.mentions, 1);
        assert_eq!(read.links, 1);
        assert!(!read.over);
        let long = "a".repeat(300);
        assert!(read_composer(&long).over);
    }

    #[test]
    fn hit_list_and_reply_badge_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut pack = PackFile::default();
        pack.hits.push(HitItem {
            id: "9".into(),
            url: "https://x.com/ada/status/9".into(),
            author: "ada".into(),
            text: "pinned".into(),
            note: "watch this".into(),
        });
        pack.receipts.push(ReplyMark {
            card_id: "9".into(),
            url: "https://x.com/me/status/10".into(),
            at_unix: 1_700_000_000,
            status: "confirmed".into(),
            text: "reply".into(),
        });
        pack.receipts.push(ReplyMark {
            card_id: "8".into(),
            url: "https://x.com/me/status/11".into(),
            at_unix: 1,
            status: "failed".into(),
            text: "nope".into(),
        });
        save_pack(dir.path(), &pack).unwrap();
        let loaded = load_pack(dir.path());
        assert_eq!(loaded.hits[0].note, "watch this");
        assert!(
            replied_badge(&loaded.receipts, "9")
                .unwrap()
                .contains("replied")
        );
        assert!(replied_badge(&loaded.receipts, "8").is_none());
        assert!(save_pack(crate::config::default_config_dir().as_path(), &pack).is_err());
    }

    #[test]
    fn filters_and_mute_drop_posts() {
        let now = parse_created_at("Wed Nov 11 18:00:00 +0000 2026").unwrap();
        let keep = post(
            "1",
            10,
            1,
            json!({"text": "clean markets", "verified": true, "lang": "en", "media_urls": ["https://img.example/a.jpg"]}),
        );
        let quiet = post("2", 0, 1, json!({"text": "noise word", "lang": "en"}));
        let other = post(
            "3",
            12,
            1,
            json!({"text": "hola", "lang": "es", "author": "bob"}),
        );
        let filter = CardFilter {
            min_likes: 5,
            last_hours: Some(6),
            has_media: true,
            verified_only: true,
            language: "en".into(),
            mute_words: vec!["noise".into()],
            mute_accounts: vec!["bob".into()],
        };
        let ids: Vec<_> = apply_filters(&[&keep, &quiet, &other], &filter, now)
            .into_iter()
            .map(|post| post.id.clone())
            .collect();
        assert_eq!(ids, vec!["1".to_string()]);
    }

    #[test]
    fn keymap_has_no_send_and_refresh_is_search_only() {
        for key in ["enter", "return", "s", "d", "r", "o", "j", "k"] {
            assert!(!sends_on_key(key, false, false));
            assert!(!sends_on_key(key, true, false));
            assert!(!sends_on_key(key, true, true));
        }
        assert_eq!(desk_key("s", false, false), DeskKey::Star);
        assert_eq!(desk_key("d", false, false), DeskKey::Draft);
        assert_eq!(desk_key("enter", false, false), DeskKey::Unbound);
        assert_eq!(desk_key("k", true, false), DeskKey::Palette);
        assert_eq!(desk_key("page_up", false, false), DeskKey::Page(-1));
        assert_eq!(desk_key("right", false, false), DeskKey::Page(1));
        let mut index = 1usize;
        for key in ["right", "pagedown", "down"] {
            assert_ne!(key, "escape");
            index = page_after_key(index, 4, key).unwrap();
        }
        assert_eq!(index, 3);
        assert_eq!(page_after_key(3, 4, "left"), Some(2));
        assert_eq!(page_after_key(2, 4, "pageup"), Some(1));
        assert_eq!(page_after_key(1, 4, "up"), Some(0));
        assert!(page_after_key(1, 4, "escape").is_none());
        assert_eq!(search_label(true, ""), "|");
        assert!(!search_label(true, "rates").contains("click to edit"));
        match search_edit("rat", "e", Some("e")) {
            SearchEdit::Text(text) => assert_eq!(text, "rate"),
            other => panic!("{other:?}"),
        }
        match search_edit("rates", "enter", None) {
            SearchEdit::Submit(term) => assert_eq!(search_page_name(&term), "Search: rates"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            next_refresh_text(Some(0), 10, false, 15, 0),
            "auto-refresh off"
        );
        assert!(next_refresh_text(Some(0), 10, true, 15, -6 * 3600).contains("next refresh"));
        let args = super::super::fetch::search_args("ai", 20);
        assert!(refresh_is_search_only(&args));
        assert!(refresh_due(None, 100, true, 10));
        assert!(!refresh_due(Some(100), 200, true, 10));
        assert!(!refresh_due(Some(0), 10_000, false, 10));
        let last = 1_700_000_000;
        assert!(
            !should_auto_refresh(Some(last), last + 9 * 60, true, 10, false, true),
            "nine minutes is early"
        );
        assert!(
            should_auto_refresh(Some(last), last + 10 * 60, true, 10, false, true),
            "a stopped free model still lets the search run"
        );
        assert!(
            !should_auto_refresh(Some(last), last + 10 * 60, true, 10, true, false),
            "a search already in flight is not started again"
        );
        assert_eq!(new_ids(&["a".into()], &["a".into(), "b".into()]).len(), 1);
        let line = hud_line(2, 1, "free", None, 0);
        let stamped = hud_line(0, 0, "free", Some(1_794_432_658), 0);
        assert!(!stamped.contains("1794432658"));
        assert!(stamped.contains(':'));
        let utc_2150 = 86_400 + 21 * 3600 + 50 * 60;
        assert_eq!(format_clock(utc_2150, 0), "9:50 PM");
        assert_eq!(format_clock(utc_2150, -6 * 3600), "3:50 PM");
        let raw = "Tue Nov 10 16:32:51 +0000 2026";
        let posted = crate::time::parse_created_at(raw).unwrap();
        assert_eq!(
            format_post_time(raw, posted + 2 * 3600, -6 * 3600),
            "2h ago"
        );
        let older = format_post_time(raw, posted + 50 * 3600, -6 * 3600);
        assert!(older.contains("10:32 AM"), "{older}");
        assert!(!older.contains("+0000"));
        assert!(line.contains("2 sent"));
        assert!(line.contains("free"));
    }
}
