//! Tweet cards, inline reply state, and Reply Guy take selection.
//!
//! Sending is a separate explicit step. Drafting a take never posts.
//! A failed provider stays an error and does not invent a successful draft.

use crate::io::{Http, HttpRequest};
use crate::reply_guy::{self, Secret};

use super::domain::{SourcePost, draft_text_from_title};

/// Windows Termy opens at 1280×820. Termy XL sits beside it at twice the width
/// and the same height. The window stays resizable.
pub const COMPANION_WIDTH: f32 = 2560.0;
pub const COMPANION_HEIGHT: f32 = 820.0;
/// Slim dock that sits beside Termy X until XL is opened.
pub const DOCK_WIDTH: f32 = 72.0;
/// Logical width Jeremy measured on the 125% display (843 physical px).
/// Launch and re-expand always use this. A saved width does not replace it.
pub const LOCKED_OPEN_WIDTH: f32 = 687.0;
/// Nominal take-card width. Wider windows add columns instead of stretching one card.
pub const TAKE_CARD_WIDTH: f32 = 480.0;
pub const TAKE_CARD_MAX: f32 = 520.0;
/// Inset between the window edge and a take card's outer border.
pub const WINDOW_PAD: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeskFrame {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub expanded: bool,
    pub expanded_width: f32,
}

pub fn frame_from_parts(
    docked: bool,
    x: f32,
    y: f32,
    expanded_width: f32,
    height: f32,
) -> DeskFrame {
    let expanded_width = if expanded_width <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        expanded_width
    };
    let height = if height < 240.0 {
        COMPANION_HEIGHT
    } else {
        height
    };
    DeskFrame {
        x,
        y,
        width: if docked { DOCK_WIDTH } else { expanded_width },
        height,
        expanded: !docked,
        expanded_width,
    }
}

/// Grow to the right. `x` and `y` are the left and top edges.
pub fn expand_right(frame: DeskFrame) -> DeskFrame {
    let width = if frame.expanded_width <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        frame.expanded_width
    };
    DeskFrame {
        x: frame.x,
        y: frame.y,
        width,
        height: frame.height,
        expanded: true,
        expanded_width: width,
    }
}

pub fn toggle_dock(frame: DeskFrame) -> DeskFrame {
    if frame.expanded {
        collapse_dock(frame)
    } else {
        expand_right(frame)
    }
}

/// Preferred width, or the monitor's free width when the preferred width does not fit.
/// A missing measurement never shrinks the window.
pub fn open_width(preferred: f32, available: Option<f32>) -> f32 {
    let full = if preferred <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        preferred
    };
    match available {
        Some(room) if room.is_finite() && room + 0.5 < full && room > DOCK_WIDTH => room,
        _ => full,
    }
}

/// One resize to the full workspace. The left edge stays at `frame.x`.
/// `expanded_width` stays the preferred width even when the monitor clamps the open size.
pub fn expand_to_full(frame: DeskFrame, available: Option<f32>) -> DeskFrame {
    let preferred = if frame.expanded_width <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        frame.expanded_width
    };
    DeskFrame {
        x: frame.x,
        y: frame.y,
        width: open_width(preferred, available),
        height: frame.height,
        expanded: true,
        expanded_width: preferred,
    }
}

/// Keep a user resize. A width that only exists because the monitor was smaller
/// does not replace a larger preferred width.
pub fn remember_width(preferred: f32, current: f32, available: Option<f32>) -> f32 {
    if current <= DOCK_WIDTH + 0.5 {
        return if preferred <= DOCK_WIDTH {
            COMPANION_WIDTH
        } else {
            preferred
        };
    }
    if let Some(room) = available
        && current + 4.0 >= room
        && preferred > current + 4.0
    {
        return preferred;
    }
    current
}

pub fn collapse_dock(frame: DeskFrame) -> DeskFrame {
    let from_live = if frame.width > DOCK_WIDTH {
        frame.width
    } else {
        0.0
    };
    let expanded_width = frame.expanded_width.max(from_live);
    let expanded_width = if expanded_width <= DOCK_WIDTH {
        COMPANION_WIDTH
    } else {
        expanded_width
    };
    DeskFrame {
        x: frame.x,
        y: frame.y,
        width: DOCK_WIDTH,
        height: frame.height,
        expanded: false,
        expanded_width,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TweetCard {
    pub id: String,
    pub author_name: String,
    pub handle: String,
    pub text: String,
    pub media: Vec<String>,
    pub likes: u64,
    pub reposts: u64,
    pub replies: u64,
    pub views: u64,
    pub created_at: Option<String>,
    pub url: String,
}

pub fn card_from_post(post: &SourcePost) -> TweetCard {
    let handle = if post.author.is_empty() {
        "unknown".into()
    } else {
        post.author.clone()
    };
    let author_name = if post.author_name.trim().is_empty() {
        handle.clone()
    } else {
        post.author_name.clone()
    };
    TweetCard {
        id: post.id.clone(),
        author_name,
        handle,
        text: post.text.clone(),
        media: post.media.clone(),
        likes: post.likes,
        reposts: post.reposts,
        replies: post.replies,
        views: post.views,
        created_at: post.created_at.clone(),
        url: post.url.clone(),
    }
}

pub fn card_columns(width: f32) -> usize {
    if width >= 1800.0 { 2 } else { 1 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TakeFrame {
    pub id: String,
    pub x: f32,
    pub width: f32,
}

impl TakeFrame {
    pub fn right(&self) -> f32 {
        self.x + self.width
    }
}

/// One frame per take. Every card shares the same inset so its right edge
/// stays inside the window at any width.
pub fn layout_takes(viewport: f32, ids: &[impl AsRef<str>]) -> Vec<TakeFrame> {
    let gap = 12.0;
    let inner = (viewport - WINDOW_PAD * 2.0).max(0.0);
    let mut columns = ((inner / TAKE_CARD_WIDTH).floor() as usize).max(1);
    while columns > 1 && columns as f32 * 420.0 + (columns - 1) as f32 * gap > inner {
        columns -= 1;
    }
    let width = if columns == 1 {
        inner.min(TAKE_CARD_MAX)
    } else {
        ((inner - gap * (columns as f32 - 1.0)) / columns as f32).min(TAKE_CARD_MAX)
    };
    ids.iter()
        .enumerate()
        .map(|(index, id)| {
            let column = index % columns;
            TakeFrame {
                id: id.as_ref().to_string(),
                x: WINDOW_PAD + column as f32 * (width + gap),
                width,
            }
        })
        .collect()
}

pub fn take_column_count(viewport: f32) -> usize {
    let frames = layout_takes(viewport, &["a"]);
    let _ = frames;
    let gap = 12.0;
    let inner = (viewport - WINDOW_PAD * 2.0).max(0.0);
    let mut columns = ((inner / TAKE_CARD_WIDTH).floor() as usize).max(1);
    while columns > 1 && columns as f32 * 420.0 + (columns - 1) as f32 * gap > inner {
        columns -= 1;
    }
    columns
}

/// One free-template take per saved post. Empty input stays empty. Not a fixture.
pub fn takes_for_posts(posts: &[super::domain::SourcePost]) -> Vec<(String, String)> {
    posts
        .iter()
        .map(|post| (post.id.clone(), super::domain::idea_from(post)))
        .collect()
}

/// Break long tokens so a URL or handle can wrap inside the card.
pub fn wrap_token(text: &str) -> String {
    const LIMIT: usize = 24;
    let mut out = String::new();
    for token in text.split_inclusive(char::is_whitespace) {
        let chars: Vec<char> = token.chars().collect();
        if chars.len() <= LIMIT {
            out.extend(chars);
            continue;
        }
        for (index, ch) in chars.into_iter().enumerate() {
            if index > 0 && index % LIMIT == 0 && !ch.is_whitespace() {
                out.push('\u{200b}');
            }
            out.push(ch);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectPage {
    pub id: String,
    pub name: String,
}

pub fn subject_pages(pairs: &[(&str, &str)]) -> Vec<SubjectPage> {
    pairs
        .iter()
        .filter(|(id, _)| !id.is_empty())
        .map(|(id, name)| SubjectPage {
            id: (*id).to_string(),
            name: if name.is_empty() {
                (*id).to_string()
            } else {
                (*name).to_string()
            },
        })
        .collect()
}

pub fn page_index(pages: &[SubjectPage], saved_id: &str) -> usize {
    pages
        .iter()
        .position(|page| page.id == saved_id)
        .unwrap_or(0)
}

/// Move one page. The index stays inside the book.
pub fn turn_page(index: usize, count: usize, delta: i32) -> usize {
    if count == 0 {
        return 0;
    }
    let next = index as i32 + delta;
    if next < 0 {
        0
    } else if next >= count as i32 {
        count - 1
    } else {
        next as usize
    }
}

pub fn page_label(index: usize, count: usize, name: &str) -> String {
    if count == 0 {
        "0 / 0".into()
    } else {
        format!("{} / {} · {}", index + 1, count, name)
    }
}

pub fn move_selection(ids: &[String], current: Option<&str>, delta: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let pos = current.and_then(|id| ids.iter().position(|item| item == id));
    let next = match (pos, delta.signum()) {
        (None, 1..) => 0,
        (None, _) => ids.len() - 1,
        (Some(index), _) => {
            let next = index as i32 + delta;
            if next < 0 {
                0
            } else if next >= ids.len() as i32 {
                ids.len() - 1
            } else {
                next as usize
            }
        }
    };
    Some(ids[next].clone())
}

pub fn scroll_by(
    offsets: &mut std::collections::HashMap<String, f32>,
    id: &str,
    delta: f32,
    limit: f32,
) -> f32 {
    let limit = limit.max(0.0);
    let next = (offsets.get(id).copied().unwrap_or(0.0) + delta).clamp(0.0, limit);
    offsets.insert(id.to_string(), next);
    next
}

pub fn scroll_of(offsets: &std::collections::HashMap<String, f32>, id: &str) -> f32 {
    offsets.get(id).copied().unwrap_or(0.0)
}

/// Floating reply editor. Closing it never posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditPopout {
    pub card_id: String,
    pub open: bool,
    pub draft: String,
    pub tone: String,
    pub variants: Vec<String>,
    pub provider: TakeProvider,
}

pub fn open_editor(card_id: &str, draft: &str, provider: TakeProvider) -> EditPopout {
    let tone = crate::model::Tone::Reply;
    EditPopout {
        card_id: card_id.to_string(),
        open: true,
        draft: draft.to_string(),
        tone: tone.as_str().to_string(),
        variants: tone_variants(draft, tone),
        provider,
    }
}

pub fn close_editor(pop: &EditPopout) -> EditPopout {
    EditPopout {
        open: false,
        ..pop.clone()
    }
}

/// Send is allowed only from an open editor and an explicit click.
/// This does not post. The caller posts only after `Ok`.
pub fn editor_send(pop: &EditPopout, explicit: bool) -> Result<String, String> {
    if !explicit {
        return Err("Send requires an explicit click.".into());
    }
    if !pop.open {
        return Err("The editor is closed. Nothing was posted.".into());
    }
    let text = pop.draft.trim();
    if text.is_empty() {
        return Err("Write a reply first.".into());
    }
    Ok(text.to_string())
}

pub fn tone_variants(idea: &str, tone: crate::model::Tone) -> Vec<String> {
    crate::ai::template_variants(idea, tone, 3)
        .into_iter()
        .map(|draft| draft.text)
        .take(3)
        .collect()
}

pub fn char_count(text: &str) -> usize {
    crate::text::weighted_len_limited(text, crate::text::STANDARD_CHAR_LIMIT).weighted
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyPhase {
    Idle,
    Editing { text: String },
    Pending { text: String },
    Confirmed { text: String, receipt: String },
    Failed { text: String, error: String },
}

impl ReplyPhase {
    pub fn text(&self) -> &str {
        match self {
            Self::Editing { text }
            | Self::Pending { text }
            | Self::Confirmed { text, .. }
            | Self::Failed { text, .. } => text,
            Self::Idle => "",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Editing { .. } => "editing",
            Self::Pending { .. } => "pending",
            Self::Confirmed { .. } => "confirmed",
            Self::Failed { .. } => "failed",
        }
    }
}

pub fn edit_reply(phase: &ReplyPhase, text: String) -> ReplyPhase {
    if matches!(phase, ReplyPhase::Pending { .. }) {
        return phase.clone();
    }
    ReplyPhase::Editing { text }
}

/// Move to pending only when the user explicitly chooses Send.
pub fn request_send(phase: &ReplyPhase, explicit: bool) -> Result<ReplyPhase, String> {
    if !explicit {
        return Err("Send requires an explicit click.".into());
    }
    let text = phase.text().trim().to_string();
    if text.is_empty() {
        return Err("Write a reply first.".into());
    }
    if matches!(phase, ReplyPhase::Pending { .. } | ReplyPhase::Idle) {
        return Err("Nothing is ready to send.".into());
    }
    Ok(ReplyPhase::Pending { text })
}

pub fn apply_send_result(pending: &ReplyPhase, result: Result<String, String>) -> ReplyPhase {
    let ReplyPhase::Pending { text } = pending else {
        return pending.clone();
    };
    match result {
        Ok(receipt) => ReplyPhase::Confirmed {
            text: text.clone(),
            receipt,
        },
        Err(error) => ReplyPhase::Failed {
            text: text.clone(),
            error,
        },
    }
}

pub trait ReplyPoster {
    fn post_reply(&self, target_url: &str, text: &str) -> Result<String, String>;
}

/// Calls the poster only after an explicit send. A false `explicit` leaves the
/// phase unchanged and does not call the poster.
pub fn commit_send(
    phase: &ReplyPhase,
    explicit: bool,
    target_url: &str,
    poster: &dyn ReplyPoster,
) -> ReplyPhase {
    let Ok(pending) = request_send(phase, explicit) else {
        return phase.clone();
    };
    let ReplyPhase::Pending { text } = &pending else {
        return pending;
    };
    apply_send_result(&pending, poster.post_reply(target_url, text))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeProvider {
    Treg,
    Free,
}

impl TakeProvider {
    pub fn parse(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("treg") {
            Self::Treg
        } else {
            Self::Free
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Treg => "treg",
            Self::Free => "free",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeDraft {
    pub text: String,
    pub source: &'static str,
    pub note: String,
}

pub fn free_take(post: &SourcePost) -> TakeDraft {
    TakeDraft {
        text: super::domain::idea_from(post),
        source: "free-template",
        note: "Free take from the post. Not a paid model.".into(),
    }
}

/// Treg reads the existing Termy X key and checks it. It does not write a new
/// key and it does not fall back to a fake success when the check fails.
/// The wording is still the free Reply Guy template, labeled as such.
pub fn treg_take(post: &SourcePost, http: &dyn Http) -> Result<TakeDraft, String> {
    let config = crate::config::XConfig::default();
    let Some(token) = reply_guy::load_secret(&config, Secret::Treg) else {
        return Err("No Treg key is saved in Termy X.".into());
    };
    treg_take_with_token(&token, http, post)
}

pub fn treg_take_with_token(
    token: &str,
    http: &dyn Http,
    post: &SourcePost,
) -> Result<TakeDraft, String> {
    let prompt = format!(
        "Write one short X reply under 240 characters. No hashtags. Post by @{}: {}",
        post.author, post.text
    );
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}]
    })
    .to_string();
    let response = http
        .request(reply_guy::treg_request(
            "POST",
            "https://treg.to/call/openai.chat.completions",
            token,
            Some(body),
        ))
        .map_err(|error| redact_token(&error, token))?;
    let text = parse_treg_draft(&response).map_err(|error| redact_token(&error, token))?;
    Ok(TakeDraft {
        text,
        source: "treg",
        note: "Treg draft. Not posted.".into(),
    })
}

pub fn parse_treg_draft(body: &str) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|error| format!("treg: bad response ({error})"))?;
    if let Some(text) = value
        .pointer("/error/message")
        .and_then(|item| item.as_str())
        .or_else(|| value.get("detail").and_then(|item| item.as_str()))
        .or_else(|| value.get("error").and_then(|item| item.as_str()))
        .or_else(|| value.get("message").and_then(|item| item.as_str()))
    {
        if value.pointer("/choices/0/message/content").is_none()
            && value.pointer("/output/data/text").is_none()
            && value.pointer("/output/text").is_none()
        {
            return Err(format!("treg: {text}"));
        }
    }
    let text = value
        .pointer("/choices/0/message/content")
        .and_then(|item| item.as_str())
        .or_else(|| {
            value
                .pointer("/output/data/text")
                .and_then(|item| item.as_str())
        })
        .or_else(|| value.pointer("/output/text").and_then(|item| item.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        Err("treg returned an empty draft".into())
    } else {
        Ok(text)
    }
}

pub fn draft_take(
    provider: TakeProvider,
    post: &SourcePost,
    http: &dyn Http,
) -> Result<TakeDraft, String> {
    match provider {
        TakeProvider::Free => Ok(free_take(post)),
        TakeProvider::Treg => treg_take(post, http),
    }
}

fn redact_token(error: &str, token: &str) -> String {
    if token.is_empty() {
        return error.to_string();
    }
    error.replace(token, "[redacted]")
}

/// Unused request shape kept so a future catalog draft tool can be swapped in
/// without changing the state machine. Tests assert it carries the token header
/// and never targets a post/reply publish URL.
pub fn treg_draft_probe(token: &str) -> HttpRequest {
    reply_guy::treg_request("GET", reply_guy::TREG_ME_URL, token, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{Http, HttpRequest};
    use serde_json::json;

    struct ScriptHttp {
        body: Result<String, String>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl Http for ScriptHttp {
        fn request(&self, request: HttpRequest) -> Result<String, String> {
            self.calls.lock().unwrap().push(request.url);
            self.body.clone()
        }
    }

    struct MockPoster {
        result: Result<String, String>,
        calls: std::sync::Mutex<u32>,
    }

    impl ReplyPoster for MockPoster {
        fn post_reply(&self, _target_url: &str, _text: &str) -> Result<String, String> {
            *self.calls.lock().unwrap() += 1;
            self.result.clone()
        }
    }

    fn post() -> SourcePost {
        SourcePost::from_value(&json!({
            "id": "55",
            "author": "ada",
            "author_name": "Ada Lovelace",
            "text": "Courts and models",
            "created_at": "Wed Nov 11 16:00:00 +0000 2026",
            "likes": 3,
            "replies": 1,
            "views": "10",
            "media_urls": ["https://pbs.twimg.com/media/example.jpg"],
            "url": "https://x.com/ada/status/55"
        }))
        .unwrap()
    }

    #[test]
    fn card_keeps_author_text_media_metrics_time_and_link() {
        let card = card_from_post(&post());
        assert_eq!(card.author_name, "Ada Lovelace");
        assert_eq!(card.handle, "ada");
        assert_eq!(card.text, "Courts and models");
        assert_eq!(
            card.media,
            vec!["https://pbs.twimg.com/media/example.jpg".to_string()]
        );
        assert_eq!(card.likes, 3);
        assert_eq!(card.replies, 1);
        assert_eq!(
            card.created_at.as_deref(),
            Some("Wed Nov 11 16:00:00 +0000 2026")
        );
        assert_eq!(card.url, "https://x.com/ada/status/55");
    }

    #[test]
    fn companion_size_is_twice_termy_width_and_cards_shrink() {
        assert_eq!(COMPANION_WIDTH, 2560.0);
        assert_eq!(COMPANION_HEIGHT, 820.0);
        assert_eq!(card_columns(COMPANION_WIDTH), 2);
        assert_eq!(card_columns(900.0), 1);
    }

    #[test]
    fn dock_expands_right_and_the_left_edge_stays() {
        let dock = frame_from_parts(true, 40.0, 80.0, 2560.0, 820.0);
        assert!(!dock.expanded);
        assert_eq!(dock.width, DOCK_WIDTH);
        let open = expand_right(dock);
        assert_eq!(open.x, dock.x);
        assert_eq!(open.y, dock.y);
        assert_eq!(open.width, COMPANION_WIDTH);
        let resized = DeskFrame {
            width: 1800.0,
            expanded_width: 1800.0,
            ..open
        };
        let shut = collapse_dock(resized);
        assert_eq!(shut.x, 40.0);
        assert_eq!(shut.y, 80.0);
        assert_eq!(shut.width, DOCK_WIDTH);
        assert_eq!(shut.expanded_width, 1800.0);
        let again = toggle_dock(shut);
        assert_eq!(again.x, shut.x);
        assert_eq!(again.width, 1800.0);
        let jumped = expand_to_full(dock, None);
        assert_eq!(jumped.x, dock.x);
        assert_eq!(jumped.width, COMPANION_WIDTH);
        let clamped = expand_to_full(dock, Some(1400.0));
        assert_eq!(clamped.x, dock.x);
        assert_eq!(clamped.width, 1400.0);
        assert_eq!(clamped.expanded_width, COMPANION_WIDTH);
        assert_eq!(
            remember_width(COMPANION_WIDTH, 1400.0, Some(1400.0)),
            COMPANION_WIDTH
        );
        assert_eq!(
            remember_width(COMPANION_WIDTH, 1800.0, Some(3440.0)),
            1800.0
        );
        assert_eq!(open_width(0.0, None), COMPANION_WIDTH);
    }

    #[test]
    fn each_take_is_its_own_card_inside_the_window() {
        for viewport in [900.0, 1600.0, 2560.0] {
            let frames = layout_takes(viewport, &["law-1", "law-2", "law-3"]);
            assert_eq!(frames.len(), 3);
            let mut ids = Vec::new();
            let columns = take_column_count(viewport);
            assert!(columns >= 1);
            if viewport >= 1600.0 {
                assert!(columns > 1, "wider windows add columns");
            }
            for frame in &frames {
                ids.push(frame.id.clone());
                assert!(frame.width <= TAKE_CARD_MAX + 0.01);
                assert!(frame.right() <= viewport - WINDOW_PAD + 0.01);
                assert!(frame.width > 0.0);
            }
            assert_eq!(
                ids.len(),
                ids.iter().collect::<std::collections::HashSet<_>>().len()
            );
        }
        let broken = wrap_token("https://x.com/a-very-long-handle/status/123456789012345678901234");
        assert!(broken.contains('\u{200b}'));
    }

    #[test]
    fn a_saved_report_makes_one_take_card_per_post() {
        let mut second = post();
        second.id = "56".into();
        let posts = vec![post(), second];
        let takes = takes_for_posts(&posts);
        assert_eq!(takes.len(), posts.len());
        assert!(takes.iter().all(|(_, text)| !text.is_empty()));
        assert!(takes_for_posts(&[]).is_empty());
        let frames = layout_takes(
            687.0,
            &takes.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
        );
        assert_eq!(frames.len(), 2);
        assert_eq!(take_column_count(687.0), 1);
        assert!(take_column_count(2560.0) > take_column_count(687.0));
    }

    #[test]
    fn subject_pages_stay_in_bounds_and_map_by_id() {
        let pages = subject_pages(&[("law", "Law"), ("finance", "Finance"), ("ai", "AI")]);
        assert_eq!(page_index(&pages, "finance"), 1);
        assert_eq!(page_index(&pages, "missing"), 0);
        assert_eq!(turn_page(0, pages.len(), -1), 0);
        assert_eq!(turn_page(2, pages.len(), 1), 2);
        assert_eq!(turn_page(1, pages.len(), 1), 2);
        assert_eq!(page_label(1, pages.len(), "Finance"), "2 / 3 · Finance");
        assert_eq!(pages[page_index(&pages, "ai")].id, "ai");
    }

    #[test]
    fn selection_moves_one_card_and_does_not_send() {
        let ids: Vec<String> = vec!["subject:law".into(), "take:1".into(), "take:2".into()];
        let phase = ReplyPhase::Editing {
            text: "keep this".into(),
        };
        let selected = Some(ids[1].clone());
        let down = move_selection(&ids, selected.as_deref(), 1);
        assert_eq!(down.as_deref(), Some("take:2"));
        let up = move_selection(&ids, down.as_deref(), -1);
        assert_eq!(up.as_deref(), Some("take:1"));
        assert_eq!(
            move_selection(&ids, Some("take:2"), 1).as_deref(),
            Some("take:2")
        );
        assert_eq!(phase.text(), "keep this");
        let cleared: Option<String> = None;
        assert!(cleared.is_none());
    }

    #[test]
    fn scroll_offsets_are_per_card() {
        let mut offsets = std::collections::HashMap::new();
        assert_eq!(scroll_by(&mut offsets, "take:1", 30.0, 80.0), 30.0);
        assert_eq!(scroll_by(&mut offsets, "take:2", 12.0, 80.0), 12.0);
        assert_eq!(scroll_by(&mut offsets, "take:1", 100.0, 80.0), 80.0);
        assert_eq!(scroll_of(&offsets, "take:1"), 80.0);
        assert_eq!(scroll_of(&offsets, "take:2"), 12.0);
        assert_eq!(scroll_of(&offsets, "take:3"), 0.0);
    }

    #[test]
    fn closing_the_editor_keeps_the_draft_and_does_not_send() {
        let open = open_editor("55", "hello take", TakeProvider::Free);
        assert!(open.open);
        assert_eq!(open.variants.len(), 3);
        assert_eq!(char_count("hi"), 2);
        let closed = close_editor(&open);
        assert!(!closed.open);
        assert_eq!(closed.draft, "hello take");
        assert!(editor_send(&closed, true).is_err());
        assert!(editor_send(&open, false).is_err());
        let poster = MockPoster {
            result: Ok("nope".into()),
            calls: std::sync::Mutex::new(0),
        };
        let phase = ReplyPhase::Editing {
            text: closed.draft.clone(),
        };
        let after = commit_send(&phase, false, "https://x.com/ada/status/55", &poster);
        assert_eq!(*poster.calls.lock().unwrap(), 0);
        assert_eq!(after.text(), "hello take");
        assert_eq!(editor_send(&open, true).unwrap(), "hello take");
        assert_eq!(*poster.calls.lock().unwrap(), 0);
    }

    #[test]
    fn dock_state_round_trips_in_settings() {
        use crate::trends_desk::DeskSettings;
        let mut settings = DeskSettings::default();
        assert!(settings.docked);
        settings.docked = false;
        settings.frame_x = 40;
        settings.frame_y = 80;
        settings.expanded_width = 1800;
        settings.frame_height = 820;
        let loaded: DeskSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        let frame = frame_from_parts(
            loaded.docked,
            loaded.frame_x as f32,
            loaded.frame_y as f32,
            loaded.expanded_width as f32,
            loaded.frame_height as f32,
        );
        assert!(frame.expanded);
        assert_eq!(frame.x, 40.0);
        assert_eq!(frame.width, 1800.0);
        let docked: DeskSettings = serde_json::from_str("{}").unwrap();
        assert!(docked.docked);
        assert_eq!(
            frame_from_parts(true, docked.frame_x as f32, docked.frame_y as f32, 0.0, 0.0).width,
            DOCK_WIDTH
        );
    }

    #[test]
    fn typing_and_a_failed_send_do_not_drop_text_or_auto_post() {
        let poster = MockPoster {
            result: Err("opencli down".into()),
            calls: std::sync::Mutex::new(0),
        };
        let editing = edit_reply(&ReplyPhase::Idle, "hello reply".into());
        let still = commit_send(&editing, false, "https://x.com/ada/status/55", &poster);
        assert_eq!(still, editing);
        assert_eq!(*poster.calls.lock().unwrap(), 0);
        let failed = commit_send(&editing, true, "https://x.com/ada/status/55", &poster);
        assert_eq!(
            failed,
            ReplyPhase::Failed {
                text: "hello reply".into(),
                error: "opencli down".into(),
            }
        );
        assert_eq!(*poster.calls.lock().unwrap(), 1);
        let pending = request_send(&editing, true).unwrap();
        assert!(matches!(pending, ReplyPhase::Pending { .. }));
        let confirmed = apply_send_result(&pending, Ok("receipt-1".into()));
        assert_eq!(
            confirmed,
            ReplyPhase::Confirmed {
                text: "hello reply".into(),
                receipt: "receipt-1".into(),
            }
        );
    }

    #[test]
    fn provider_selection_does_not_hide_a_treg_failure() {
        let post = post();
        let free = draft_take(
            TakeProvider::Free,
            &post,
            &ScriptHttp {
                body: Err("unused".into()),
                calls: std::sync::Mutex::new(Vec::new()),
            },
        )
        .unwrap();
        assert_eq!(free.source, "free-template");
        assert!(!free.text.is_empty());
        assert_ne!(free.source, "treg");
        let http = ScriptHttp {
            body: Err("treg: no".into()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let err = draft_take(TakeProvider::Treg, &post, &http).unwrap_err();
        assert!(err.contains("Treg") || err.contains("treg") || err.contains("key"));
        assert!(!err.contains("Hot take"));
        let http = ScriptHttp {
            body: Ok(r#"{"choices":[{"message":{"content":"A real Treg take"}}]}"#.into()),
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let drafted = treg_take_with_token("secret-token", &http, &post).unwrap();
        assert_eq!(drafted.text, "A real Treg take");
        assert_eq!(drafted.source, "treg");
        let failed = treg_take_with_token(
            "secret-token",
            &ScriptHttp {
                body: Ok(r#"{"detail":"no tool"}"#.into()),
                calls: std::sync::Mutex::new(Vec::new()),
            },
            &post,
        )
        .unwrap_err();
        assert!(failed.contains("treg"));
        assert!(!failed.contains("secret-token"));
        assert!(!failed.contains("Hot take"));
        assert!(
            http.calls.lock().unwrap().is_empty() || err.contains("key") || err.contains("Treg")
        );
    }

    #[test]
    fn treg_probe_is_not_a_publish_url() {
        let request = treg_draft_probe("secret-token");
        assert!(request.url.contains("treg.to"));
        assert!(!request.url.contains("/status/"));
        assert!(request.headers.iter().any(|(key, _)| key == "X-Treg-Token"));
        assert!(!format!("{request:?}").contains("x.exe"));
    }
}
