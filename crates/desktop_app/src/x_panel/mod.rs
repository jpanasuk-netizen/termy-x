//! Termy X side panel. The geometric swift is an original SVG, drawn at
//! `background_opacity` behind lists and at full strength on the empty splash.

pub mod art;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{
    Context, FocusHandle, Image, ImageSource, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ObjectFit, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, Window, div, img, px, rgb, rgba,
};

macro_rules! x_button {
    ($id:expr, $label:expr, $handler:expr) => {
        x_button!($id, $label, false, $handler)
    };
    ($id:expr, $label:expr, $active:expr, $handler:expr) => {{
        let active = $active;
        div()
            .id($id)
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if active {
                rgb(0xff4fa3)
            } else {
                tint(0x7af6ff, 55)
            })
            .bg(if active { rgb(0x2a0f3a) } else { rgb(0x1636c9) })
            .text_size(px(12.0))
            .text_color(if active { rgb(0xffd0ea) } else { rgb(0xf8fbff) })
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(rgb(0x1f4fff))
                    .border_color(rgb(0x7af6ff))
                    .text_color(rgb(0xe9fdff))
            })
            .child($label)
            .on_mouse_down(MouseButton::Left, $handler)
    }};
}
use termy_x::XConfig;
use termy_x::drafts::new_id;
use termy_x::model::{Draft, Post, Profile, ResearchHit, Tone, Trend};
use termy_x::providers::{MockProvider, XProvider};
use termy_x::text::{split_thread_limited, weighted_len_limited};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XSection {
    Timeline,
    Search,
    Trends,
    Lookup,
    Compose,
    Research,
}

impl XSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Timeline => "Timeline",
            Self::Search => "Search",
            Self::Trends => "Trends",
            Self::Lookup => "Lookup",
            Self::Compose => "Compose",
            Self::Research => "Research",
        }
    }

    pub fn from_command(name: &str) -> Self {
        match name {
            "search" => Self::Search,
            "trends" => Self::Trends,
            "lookup" => Self::Lookup,
            "compose" => Self::Compose,
            "research" => Self::Research,
            _ => Self::Timeline,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiFocus {
    Compose,
    Clear,
    Attach,
    Suggest,
    Publish,
    Art,
    ConfirmCancel,
    ConfirmCopy,
    ConfirmNext,
    ConfirmBrowser,
}

/// Which text field owns the keyboard: the big Compose box, the Search
/// tab's query field, or an inline reply box under a tweet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputTarget {
    Compose,
    Search,
    Reply,
}

/// Pure predicate behind `XPanel::owns_edit_keys`: does a panel text
/// field own Ctrl+V/C/A right now? Kept free-standing so the routing
/// matrix is unit-testable without a GPUI context.
#[allow(clippy::too_many_arguments)]
fn edit_keys_owned(
    confirm: bool,
    bearer_mode: bool,
    input_target: InputTarget,
    section: XSection,
    reply_open: bool,
    compose_focused: bool,
    ui_focus_is_compose: bool,
) -> bool {
    if confirm {
        return false;
    }
    if bearer_mode {
        return true;
    }
    match input_target {
        InputTarget::Compose => {
            section == XSection::Compose && compose_focused && ui_focus_is_compose
        }
        InputTarget::Search => section == XSection::Search,
        InputTarget::Reply => reply_open,
    }
}

/// The two simple end-caret text fields (Search query, inline reply).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextField {
    Search,
    Reply,
}

/// Which post a pending image-picker result belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickTarget {
    Compose,
    Reply,
}

#[derive(Clone, PartialEq)]
enum ConnState {
    Checking,
    Connected { handle: String, via: String },
    NotConnected { reason: String, next: String },
}

pub struct XPanel {
    focus: FocusHandle,
    section: XSection,
    posts: Vec<Post>,
    search_posts: Vec<Post>,
    trends: Vec<Trend>,
    research: Vec<ResearchHit>,
    profile: Option<Profile>,
    query: String,
    lookup: String,
    compose: String,
    /// Local image paths attached to the next publish (max 4).
    media: Vec<PathBuf>,
    /// True while an in-app post is running in the background (prevents double-post).
    posting: bool,
    /// True while the native image picker is open on a background thread.
    picking: bool,
    /// Per-section single-flight guard: one OpenCLI/browser-session call at a time,
    /// so rapid clicks cannot spawn a storm of Chrome/OpenCLI processes.
    fetching: [bool; 5],
    /// Last explicit browser open, for debounce (never reopen in a burst).
    last_browser_open: Option<Instant>,
    variants: Vec<Draft>,
    drafts: Vec<Draft>,
    status: String,
    confirm: bool,
    intent_urls: Vec<String>,
    intent_index: usize,
    background_art: bool,
    background_opacity: f32,
    splash: bool,
    fill_window: bool,
    live: bool,
    art: Option<Arc<Image>>,
    notice: Option<String>,
    connection: ConnState,
    checking_conn: bool,
    bearer_mode: bool,
    bearer_input: String,
    queued_section: Option<XSection>,
    /// Per-section last successful fetch. Tab switches reuse data younger than 30s.
    fetched_at: SectionTimes,
    /// Monotonic id so a slow OpenCLI reply cannot overwrite a newer tab's data.
    fetch_seq: u64,
    inflight: [u64; 5],
    loading: bool,
    /// Focus the panel on next render (Compose needs keys; terminal steals focus).
    pending_focus: bool,
    /// Suggest variants is running in the background.
    suggesting: bool,
    /// Compose box owns keyboard input and shows a real caret.
    compose_focused: bool,
    /// UTF-8 byte caret inside `compose`.
    caret: usize,
    /// Selection anchor; range is between anchor and caret when set.
    sel_anchor: Option<usize>,
    /// Blinking caret phase.
    blink_visible: bool,
    /// Keyboard / Tab focus target for neon chrome.
    ui_focus: UiFocus,
    /// Blink loop started once per panel.
    blink_running: bool,
    /// Dedup KeyDown + action Paste within the same chord.
    last_edit_chord_at: Option<Instant>,
    /// Search tab: what the user typed, and the last query actually run
    /// (empty = never searched yet, so the tab shows a prompt, not posts).
    search_input: String,
    searched_query: String,
    /// Inline reply: the tweet expanded under the cursor, its draft, and
    /// images attached to that reply (validated like Compose media).
    reply_post_id: Option<String>,
    reply_author: String,
    reply_text: String,
    reply_media: Vec<PathBuf>,
    reply_posting: bool,
    replied_ids: Vec<String>,
    input_target: InputTarget,
    pick_target: PickTarget,
}

const FRESH_FOR: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Default)]
struct SectionTimes {
    timeline: Option<Instant>,
    search: Option<Instant>,
    trends: Option<Instant>,
    lookup: Option<Instant>,
    research: Option<Instant>,
}

#[derive(Clone)]
enum FetchPayload {
    Timeline {
        posts: Vec<Post>,
        status: String,
    },
    Search {
        posts: Vec<Post>,
        status: String,
    },
    Trends {
        trends: Vec<Trend>,
        status: String,
    },
    Lookup {
        profile: Option<Profile>,
        status: String,
    },
    Research {
        hits: Vec<ResearchHit>,
        status: String,
    },
}

fn section_slot(section: XSection) -> Option<usize> {
    match section {
        XSection::Timeline => Some(0),
        XSection::Search => Some(1),
        XSection::Trends => Some(2),
        XSection::Lookup => Some(3),
        XSection::Research => Some(4),
        XSection::Compose => None,
    }
}

fn log_switch(section: XSection, started: Instant, kind: &str) {
    let us = started.elapsed().as_micros();
    let line = format!("termy-x tab: {kind} {} in {us} us", section.label());
    eprintln!("{line}");
    let path = std::env::temp_dir().join("termy-x-tab-timing.txt");
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

impl XPanel {
    pub fn new(cx: &mut Context<Self>, fill_window: bool) -> Self {
        let config = if fill_window {
            XConfig::default()
        } else {
            termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
        };
        let saved_handle = if fill_window {
            None
        } else {
            termy_x::connect::load_connected_handle(&config)
        };
        let mut panel = Self {
            focus: cx.focus_handle(),
            section: XSection::Timeline,
            posts: Vec::new(),
            search_posts: Vec::new(),
            trends: Vec::new(),
            research: Vec::new(),
            profile: None,
            query: "gpui terminal".into(),
            lookup: if fill_window {
                "termy".into()
            } else {
                saved_handle.clone().unwrap_or_default()
            },
            media: Vec::new(),
            posting: false,
            picking: false,
            fetching: [false; 5],
            last_browser_open: None,
            compose: if fill_window {
                "Termy X counts this URL as 23: https://termy.sh".into()
            } else {
                String::new()
            },
            variants: Vec::new(),
            drafts: Vec::new(),
            status: "ready Â· free providers Â· no X API credits".into(),
            confirm: false,
            intent_urls: Vec::new(),
            intent_index: 0,
            background_art: config.background_art,
            background_opacity: config.background_opacity,
            splash: false,
            fill_window,
            live: !fill_window,
            art: None,
            notice: None,
            connection: ConnState::Checking,
            checking_conn: false,
            bearer_mode: false,
            bearer_input: String::new(),
            queued_section: None,
            fetched_at: SectionTimes::default(),
            fetch_seq: 0,
            inflight: [0; 5],
            loading: false,
            pending_focus: false,
            suggesting: false,
            compose_focused: false,
            caret: 0,
            sel_anchor: None,
            blink_visible: true,
            ui_focus: UiFocus::Compose,
            blink_running: false,
            last_edit_chord_at: None,
            search_input: String::new(),
            searched_query: String::new(),
            reply_post_id: None,
            reply_author: String::new(),
            reply_text: String::new(),
            reply_media: Vec::new(),
            reply_posting: false,
            replied_ids: Vec::new(),
            input_target: InputTarget::Compose,
            pick_target: PickTarget::Compose,
        };
        if fill_window {
            // Preview path fills via load_scene / open_section.
        } else {
            panel.prewarm_art(cx);
            // Who is connected comes first: real tabs must never sit
            // silently empty (or on fixtures) with no profile behind them.
            panel.status = "Checking your X connection\u{2026}".into();
            panel.start_connection_check(cx);
        }
        panel.caret = panel.compose.len();
        panel.start_caret_blink(cx);
        panel
    }

    pub fn preview(scene: &str, cx: &mut Context<Self>) -> Self {
        let mut panel = Self::new(cx, true);
        panel.background_art = true;
        panel.load_scene(scene, cx);
        panel
    }

    /// Tab / command open. Must return in microseconds: swap local state only,
    /// never call OpenCLI / twitter-cli on the UI thread.
    pub fn open_section(&mut self, section: XSection, cx: &mut Context<Self>) {
        let started = Instant::now();
        self.section = section;
        self.confirm = false;
        self.splash = false;

        if self.fill_window || !self.live {
            self.ensure_mock_lists();
            if section == XSection::Compose {
                self.loading = false;
                self.pending_focus = true;
                self.focus_compose_editor();
                // Preview keeps a demo draft; do not auto-run AI.
                if self.compose.is_empty() {
                    self.compose = "Termy X counts this URL as 23: https://termy.sh".into();
                    self.caret = self.compose.len();
                }
            }
            if section == XSection::Search && self.search_posts.is_empty() {
                self.search_posts = MockProvider
                    .search(&self.query, false, 10)
                    .map(|served| served.value)
                    .unwrap_or_default();
            }
            log_switch(section, started, "local");
            return;
        }

        if section == XSection::Lookup
            && self.lookup.trim().is_empty()
            && let ConnState::Connected { handle, .. } = &self.connection
        {
            self.lookup = handle.clone();
        }

        if section == XSection::Search {
            // Real search is user-driven: focus the query field and wait.
            // Never auto-run the stale default query behind their back.
            self.loading = false;
            self.input_target = InputTarget::Search;
            self.pending_focus = true;
            self.status = if self.searched_query.is_empty() {
                "search \u{b7} type a query and press Enter \u{b7} runs silently in the background"
                    .into()
            } else {
                format!(
                    "search \u{b7} results for \u{201c}{}\u{201d} \u{b7} type a new query and press Enter",
                    self.searched_query
                )
            };
            log_switch(section, started, "search-input");
            return;
        }

        if section == XSection::Compose {
            // Never block: no AI / OpenCLI on the open path. Suggest is a button.
            self.loading = false;
            self.pending_focus = true;
            self.input_target = InputTarget::Compose;
            self.focus_compose_editor();
            self.status =
                "compose · click the box · caret blinks · Publish opens confirm instantly".into();
            log_switch(section, started, "compose");
            return;
        }

        if self.is_fresh(section) {
            self.loading = false;
            self.status = format!("{} Â· cached (<30s) Â· no X API credits", section.label());
            log_switch(section, started, "cache");
            return;
        }

        if self.section_has_data(section) {
            self.status = format!("{} Â· refreshingâ€¦", section.label());
        } else {
            self.status = format!("loading {}â€¦", section.label());
        }
        self.loading = true;
        log_switch(section, started, "async");
        self.schedule_fetch(section, cx);
    }

    fn load_scene(&mut self, scene: &str, cx: &mut Context<Self>) {
        match scene {
            "search" => self.open_section(XSection::Search, cx),
            "compose" => self.open_section(XSection::Compose, cx),
            "confirm" => {
                self.open_section(XSection::Compose, cx);
                self.arm_confirm();
            }
            "splash" => {
                self.section = XSection::Timeline;
                self.posts.clear();
                self.search_posts.clear();
                self.trends.clear();
                self.research.clear();
                self.profile = None;
                self.variants.clear();
                self.splash = true;
                self.status = "splash Â· background art at full strength".into();
            }
            "mock" => self.load_mock_dashboard(),
            "trends" => self.open_section(XSection::Trends, cx),
            _ => self.open_section(XSection::Timeline, cx),
        }
    }

    fn load_mock_dashboard(&mut self) {
        self.splash = false;
        self.live = false;
        self.section = XSection::Timeline;
        self.ensure_mock_lists();
        self.suggest_templates();
        self.status = "mock Â· search, timeline, trends Â· no network Â· no X API credits".into();
    }

    /// Seed fixture lists without turning off live mode.
    #[allow(dead_code)]
    fn seed_fixtures(&mut self) {
        if self.posts.is_empty() {
            self.posts = MockProvider
                .timeline(8)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.search_posts.is_empty() {
            self.search_posts = MockProvider
                .search(&self.query, false, 4)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.trends.is_empty() {
            self.trends = MockProvider
                .trends(Some("worldwide"), 6)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.research.is_empty() {
            self.research = vec![ResearchHit {
                source: "fixture".into(),
                title: "GPUI terminals people actually use".into(),
                url: "https://example.com/gpui".into(),
                excerpt: "Fixture research for drafts. Live fetch runs in the background.".into(),
                session: None,
            }];
        }
        if self.profile.is_none() {
            self.profile = MockProvider
                .lookup(&self.lookup)
                .ok()
                .map(|served| served.value);
        }
        self.status = "fixtures Â· refreshing free providers in background".into();
    }

    fn ensure_mock_lists(&mut self) {
        self.live = false;
        if self.posts.is_empty() {
            self.posts = MockProvider
                .timeline(8)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.search_posts.is_empty() {
            self.search_posts = MockProvider
                .search(&self.query, false, 4)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.trends.is_empty() {
            self.trends = MockProvider
                .trends(Some("worldwide"), 6)
                .map(|served| served.value)
                .unwrap_or_default();
        }
        if self.research.is_empty() {
            self.research = vec![ResearchHit {
                source: "fixture".into(),
                title: "GPUI terminals people actually use".into(),
                url: "https://example.com/gpui".into(),
                excerpt: "Fixture research for drafts. No network call in the mock preview.".into(),
                session: None,
            }];
        }
    }

    fn is_fresh(&self, section: XSection) -> bool {
        let at = match section {
            XSection::Timeline => self.fetched_at.timeline,
            XSection::Search => self.fetched_at.search,
            XSection::Trends => self.fetched_at.trends,
            XSection::Lookup => self.fetched_at.lookup,
            XSection::Research => self.fetched_at.research,
            XSection::Compose => return true,
        };
        matches!(
            at,
            Some(instant) if instant.elapsed() < FRESH_FOR && self.section_has_data(section)
        )
    }

    fn section_has_data(&self, section: XSection) -> bool {
        match section {
            XSection::Timeline => !self.posts.is_empty(),
            XSection::Search => !self.search_posts.is_empty(),
            XSection::Trends => !self.trends.is_empty(),
            XSection::Lookup => self.profile.is_some(),
            XSection::Research => !self.research.is_empty(),
            XSection::Compose => true,
        }
    }

    /// Cycle every tab once. Used by `x_panel_preview --bench-tabs` to log switch times.
    pub fn bench_tabs(&mut self, cx: &mut Context<Self>) {
        for section in [
            XSection::Timeline,
            XSection::Search,
            XSection::Trends,
            XSection::Lookup,
            XSection::Compose,
            XSection::Research,
            XSection::Timeline,
        ] {
            self.open_section(section, cx);
        }
    }

    fn prewarm_art(&mut self, cx: &mut Context<Self>) {
        if !self.background_art || self.art.is_some() {
            return;
        }
        // Never rasterize on the UI hot path. Decode once in the background.
        cx.spawn(async move |this, cx| {
            let image = cx
                .background_executor()
                .spawn(async move { art::load().map(|art| art.image) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                if panel.art.is_none() {
                    panel.art = image;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn schedule_fetch(&mut self, section: XSection, cx: &mut Context<Self>) {
        let Some(slot) = section_slot(section) else {
            return;
        };
        if self.live && matches!(self.connection, ConnState::NotConnected { .. }) {
            self.loading = false;
            self.status = "Not connected — press Connect my X above first.".into();
            return;
        }
        if section == XSection::Search && self.searched_query.is_empty() {
            // No query yet: the Search tab is a prompt, not a feed.
            self.loading = false;
            self.status =
                "search \u{b7} type a query and press Enter \u{b7} runs silently in the background"
                    .into();
            return;
        }
        if self.live && matches!(self.connection, ConnState::Checking) {
            // Run this tab as soon as we know who is connected.
            self.queued_section = Some(section);
            self.loading = true;
            return;
        }
        // Single-flight: rapid tab/item clicks must not spawn parallel
        // OpenCLI calls (each can prod the Chrome session). One at a time,
        // but never drop the click: queue the latest tab and run it next.
        if self.fetching.iter().any(|f| *f) {
            self.queued_section = Some(section);
            self.status =
                "already loading — one X session call at a time (no extra browser windows)".into();
            return;
        }
        self.fetching[slot] = true;
        self.fetch_seq = self.fetch_seq.wrapping_add(1);
        let seq = self.fetch_seq;
        self.inflight[slot] = seq;

        let query = self.query.clone();
        let lookup = self.lookup.clone();
        let live = self.live;

        cx.spawn(async move |this, cx| {
            let payload = cx
                .background_executor()
                .spawn(async move { fetch_section(section, query, lookup, live) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.fetching[slot] = false;
                if panel.inflight[slot] != seq {
                    return;
                }
                panel.loading = false;
                panel.apply_fetch(section, payload);
                if let Some(next) = panel.queued_section.take()
                    && next != section
                {
                    panel.loading = true;
                    panel.schedule_fetch(next, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_fetch(&mut self, section: XSection, payload: FetchPayload) {
        let now = Instant::now();
        match payload {
            FetchPayload::Timeline { posts, status } => {
                self.posts = posts;
                self.status = status;
                self.fetched_at.timeline = Some(now);
            }
            FetchPayload::Search { posts, status } => {
                self.search_posts = posts;
                self.status = status;
                self.fetched_at.search = Some(now);
            }
            FetchPayload::Trends { trends, status } => {
                self.trends = trends;
                self.status = status;
                self.fetched_at.trends = Some(now);
            }
            FetchPayload::Lookup { profile, status } => {
                self.profile = profile;
                self.status = status;
                self.fetched_at.lookup = Some(now);
            }
            FetchPayload::Research { hits, status } => {
                self.research = hits;
                self.status = status;
                self.fetched_at.research = Some(now);
            }
        }
        // Keep status accurate if the user already left this tab.
        if self.section != section && !self.loading {
            // leave current tab's status alone if it has its own message
        }
        let _ = section;
    }

    fn service_free() -> termy_x::providers::Service {
        let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
        termy_x::providers::Service::free(
            &config,
            Arc::new(termy_x::io::SystemRunner),
            Arc::new(termy_x::io::UreqHttp::default()),
        )
    }

    fn start_connection_check(&mut self, cx: &mut Context<Self>) {
        if self.checking_conn {
            return;
        }
        self.checking_conn = true;
        self.connection = ConnState::Checking;
        self.status = "Connecting\u{2026} checking your Chrome session silently in the background \u{b7} no Chrome window should pop \u{b7} up to 30s".into();
        cx.spawn(async move |this, cx| {
            let conn = cx
                .background_executor()
                .spawn(async move {
                    let config =
                        termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
                    termy_x::connect::detect_connection(
                        &config,
                        &termy_x::io::SystemRunner,
                        &termy_x::io::UreqHttp::default(),
                    )
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.checking_conn = false;
                match conn {
                    termy_x::connect::Connection::Connected { handle, via } => {
                        if panel.lookup.trim().is_empty() || panel.lookup == "termy" {
                            panel.lookup = handle.clone();
                        }
                        panel.status = format!("Connected as @{handle} \u{b7} {via}");
                        // Clear any stale "Disconnected\u{2026}" notice from a
                        // previous disconnect_x: it is false once reconnected.
                        panel.notice = None;
                        panel.connection = ConnState::Connected { handle, via };
                        let next = panel.queued_section.take().unwrap_or(panel.section);
                        let next = if next == XSection::Compose {
                            XSection::Timeline
                        } else {
                            next
                        };
                        panel.loading = true;
                        panel.schedule_fetch(next, cx);
                    }
                    termy_x::connect::Connection::NotConnected { reason, next } => {
                        panel.status = format!("Not connected \u{2014} {reason}");
                        panel.connection = ConnState::NotConnected { reason, next };
                        panel.loading = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn disconnect_x(&mut self, cx: &mut Context<Self>) {
        let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
        let _ = termy_x::connect::disconnect(&config);
        self.posts.clear();
        self.search_posts.clear();
        self.trends.clear();
        self.research.clear();
        self.profile = None;
        self.fetched_at = SectionTimes::default();
        self.bearer_mode = false;
        self.bearer_input.clear();
        self.connection = ConnState::NotConnected {
            reason: "you disconnected on this machine".into(),
            next: "1. To use a different account, log in to X in Chrome as that account.\n2. Press \u{201c}Connect my X\u{201d}.\n(Termy X cannot log Chrome out for you, and never asks for your password.)".into(),
        };
        self.notice =
            Some("Disconnected on this machine. Your Chrome login was not touched.".into());
        self.status = "Not connected \u{2014} you disconnected on this machine".into();
        cx.notify();
    }

    fn begin_bearer(&mut self, cx: &mut Context<Self>) {
        self.bearer_mode = true;
        self.bearer_input.clear();
        self.notice = Some(
            "Paste your own X developer key (Ctrl+V), then press Enter to save it on this machine. Esc cancels. It is stored in your OS keychain and never shown or written to x.toml.".into(),
        );
        self.pending_focus = true;
        cx.notify();
    }

    fn save_bearer(&mut self, cx: &mut Context<Self>) {
        let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
        match termy_x::connect::store_bearer_token(&config, &self.bearer_input) {
            Ok(()) => {
                self.bearer_mode = false;
                self.bearer_input.clear();
                self.notice = Some("Key saved on this machine. Checking it now\u{2026}".into());
                self.start_connection_check(cx);
            }
            Err(error) => {
                self.notice = Some(error);
            }
        }
        cx.notify();
    }

    fn connection_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.fill_window {
            return div().into_any_element();
        }
        match &self.connection {
            ConnState::Checking => card(
                "Connecting\u{2026}".into(),
                "Checking your Chrome session silently in the background \u{b7} no Chrome window should pop. Usually 10\u{2013}30 seconds."
                    .into(),
            ),
            ConnState::Connected { handle, via } => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .flex_wrap()
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x8affc1))
                        .child(format!("\u{25cf} Connected as @{handle} \u{b7} {via}")),
                )
                .child(x_button!(
                    "x-conn-refresh",
                    "Check again",
                    cx.listener(|this, _, _, cx| {
                        this.start_connection_check(cx);
                        cx.stop_propagation();
                        cx.notify();
                    })
                ))
                .child(x_button!(
                    "x-disconnect",
                    "Disconnect / switch account",
                    cx.listener(|this, _, _, cx| {
                        this.disconnect_x(cx);
                        cx.stop_propagation();
                        cx.notify();
                    })
                ))
                .into_any_element(),
            ConnState::NotConnected { reason, next } => {
                let mut col = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(tint(0x2a1408, 220))
                    .border_1()
                    .border_color(rgb(0xffb36b));
                col = col.child(
                    div()
                        .text_size(px(13.0))
                        .text_color(rgb(0xffd9ad))
                        .child("Connect your X \u{2014} it takes about 10 seconds"),
                );
                col = col.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xf3e2cf))
                        .child(format!("Why you see this: {reason}.")),
                );
                for line in next.lines() {
                    col = col.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xe7eefc))
                            .child(line.to_string()),
                    );
                }
                col = col.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .flex_wrap()
                        .child(x_button!(
                            "x-connect",
                            "Connect my X",
                            cx.listener(|this, _, _, cx| {
                                this.start_connection_check(cx);
                                cx.stop_propagation();
                                cx.notify();
                            })
                        ))
                        .child(x_button!(
                            "x-bearer-mode",
                            "Use my own developer key instead",
                            cx.listener(|this, _, _, cx| {
                                this.begin_bearer(cx);
                                cx.stop_propagation();
                                cx.notify();
                            })
                        )),
                );
                if self.bearer_mode {
                    let masked = "\u{2022}".repeat(self.bearer_input.chars().count().min(24));
                    col = col.child(
                        div().text_size(px(12.0)).text_color(rgb(0x7af6ff))
                            .child(format!(
                                "Your key: {masked} ({} characters) \u{2014} Ctrl+V pastes, Enter saves, Esc cancels",
                                self.bearer_input.chars().count()
                            )),
                    );
                    col = col.child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .flex_wrap()
                            .child(x_button!(
                                "x-bearer-save",
                                "Save key on this machine",
                                cx.listener(|this, _, _, cx| {
                                    this.save_bearer(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                            .child(x_button!(
                                "x-bearer-cancel",
                                "Cancel",
                                cx.listener(|this, _, _, cx| {
                                    this.bearer_mode = false;
                                    this.bearer_input.clear();
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            )),
                    );
                }
                col.into_any_element()
            }
        }
    }

    fn suggest_templates(&mut self) {
        self.variants = termy_x::ai::template_variants(&self.compose, Tone::Punchy, 3);
        self.suggesting = false;
        self.notice = Some("Template drafts. Nothing was posted.".into());
    }

    /// Instant local templates, then optional AI in the background. Never blocks UI.
    fn suggest_async(&mut self, cx: &mut Context<Self>) {
        self.suggest_templates();
        self.suggesting = true;
        self.notice = Some("Drafting variantsâ€¦ UI stays live.".into());
        let idea = self.compose.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let config =
                        termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
                    let http = termy_x::io::UreqHttp::default();
                    termy_x::ai::draft_variants(&config, &http, &idea, Tone::Punchy, 3)
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.suggesting = false;
                match result {
                    Ok(variants) => {
                        panel.variants = variants;
                        panel.notice = Some("Variants are drafts. Nothing was posted.".into());
                    }
                    Err(error) => {
                        panel.notice = Some(format!(
                            "AI offline ({error}). Showing templates. UI stayed live."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn arm_confirm(&mut self) {
        let char_limit = termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
            .char_limit_or_fallback();
        let parts = split_thread_limited(&self.compose, char_limit);
        let parts = if parts.is_empty() {
            vec![self.compose.trim().to_string()]
        } else {
            parts
        };
        if parts.first().is_none_or(|p| p.is_empty()) {
            self.notice = Some("Type something before Publish.".into());
            self.confirm = false;
            return;
        }
        self.intent_urls = termy_x::publish::intent_urls(&parts, None);
        self.intent_index = 0;
        if self.media.is_empty() {
            self.notice = Some("Confirm exact text before anything posts.".into());
        } else {
            let names: Vec<String> = self
                .media
                .iter()
                .map(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("image")
                        .to_string()
                })
                .collect();
            self.notice = Some(format!(
                "Confirm text + {} image(s): {}",
                self.media.len(),
                names.join(", ")
            ));
        }
        self.confirm = true;
        self.compose_focused = false;
        self.ui_focus = UiFocus::ConfirmNext;
        self.pending_focus = true;
    }

    fn accept_variant(&mut self, index: usize) {
        if let Some(draft) = self.variants.get(index) {
            self.compose = draft.text.clone();
            self.caret = self.compose.len();
            self.sel_anchor = None;
            self.drafts.push(Draft {
                id: new_id(),
                text: draft.text.clone(),
                tone: draft.tone.clone(),
                source: draft.source.clone(),
                created_at: draft.created_at.clone(),
                reply_to: None,
            });
        }
    }

    fn toggle_art(&mut self) {
        self.background_art = !self.background_art;
        let config = XConfig {
            background_art: self.background_art,
            ..Default::default()
        };
        if !self.fill_window {
            let _ = config.save_background_art();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        if self.bearer_mode {
            if key == "escape" {
                self.bearer_mode = false;
                self.bearer_input.clear();
                self.notice = Some("Key entry cancelled. Nothing was saved.".into());
            } else if key == "enter" {
                self.save_bearer(cx);
            } else if key == "backspace" {
                self.bearer_input.pop();
            } else if (mods.control || mods.platform) && key.eq_ignore_ascii_case("v") {
                if let Some(item) = cx.read_from_clipboard()
                    && let Some(clip) = item.text()
                {
                    self.bearer_input.push_str(clip.trim());
                }
            } else if !(mods.control || mods.alt || mods.platform || mods.function)
                && let Some(ch) = event.keystroke.key_char.as_deref()
            {
                self.bearer_input.push_str(ch);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if key == "escape" {
            if self.reply_post_id.is_some() {
                self.close_reply();
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if self.confirm {
                self.confirm = false;
                self.focus_compose_editor();
                self.pending_focus = true;
                cx.notify();
            }
            // Never leak Esc into the terminal while the X panel has the key stream.
            cx.stop_propagation();
            return;
        }

        if key == "tab" && !mods.control && !mods.platform && !mods.alt {
            self.cycle_ui_focus(mods.shift);
            self.pending_focus = true;
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if self.confirm {
            if key == "enter" && !mods.modified() {
                match self.ui_focus {
                    UiFocus::ConfirmCancel => {
                        self.confirm = false;
                        self.focus_compose_editor();
                    }
                    UiFocus::ConfirmCopy => self.copy_current_part(cx),
                    // ONLY the explicit browser button may open Chrome/x.com.
                    UiFocus::ConfirmBrowser => self.open_current_intent(),
                    // Default/Enter posts IN-APP. It must never fall through
                    // to a browser intent (that was the reopen-Chrome bug).
                    _ => self.post_now_async(cx),
                }
                self.pending_focus = true;
                cx.notify();
            }
            // Confirm modal owns the keyboard; never leak into the terminal.
            cx.stop_propagation();
            return;
        }

        if self.input_target == InputTarget::Search && self.section == XSection::Search {
            self.on_text_input_key(event, TextField::Search, cx);
            return;
        }
        if self.input_target == InputTarget::Reply && self.reply_post_id.is_some() {
            self.on_text_input_key(event, TextField::Reply, cx);
            return;
        }

        if self.section != XSection::Compose {
            return;
        }

        // Activate focused chrome button with Enter.
        if key == "enter" && !mods.modified() && self.ui_focus != UiFocus::Compose {
            match self.ui_focus {
                UiFocus::Attach => {
                    self.attach_images(cx);
                }
                UiFocus::Clear => {
                    self.compose.clear();
                    self.caret = 0;
                    self.sel_anchor = None;
                    self.variants.clear();
                    self.notice = Some("Draft cleared.".into());
                    self.focus_compose_editor();
                }
                UiFocus::Suggest => {
                    self.suggest_async(cx);
                    self.focus_compose_editor();
                }
                UiFocus::Publish => self.arm_confirm(),
                UiFocus::Art => self.toggle_art(),
                _ => {}
            }
            self.pending_focus = true;
            cx.notify();
            return;
        }

        // Typing goes to the compose editor.
        if self.ui_focus != UiFocus::Compose {
            self.focus_compose_editor();
        }
        self.compose_focused = true;
        self.blink_visible = true;

        // Always stop_propagation for edit chords — even if clipboard is empty —
        // otherwise TerminalView's Paste/Copy/SelectAll actions still hit the shell.
        if (mods.control || mods.platform) && key.eq_ignore_ascii_case("v") {
            if let Some(item) = cx.read_from_clipboard()
                && let Some(clip) = item.text()
            {
                self.insert_text(&clip);
            }
            self.mark_edit_chord();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if (mods.control || mods.platform) && key.eq_ignore_ascii_case("a") {
            self.sel_anchor = Some(0);
            self.caret = self.compose.len();
            self.mark_edit_chord();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if (mods.control || mods.platform) && key.eq_ignore_ascii_case("c") {
            if let Some((a, b)) = self.selection_range() {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                    self.compose[a..b].to_string(),
                ));
            }
            self.mark_edit_chord();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if (mods.control || mods.platform) && key.eq_ignore_ascii_case("x") {
            if let Some((a, b)) = self.selection_range() {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                    self.compose[a..b].to_string(),
                ));
                self.compose.replace_range(a..b, "");
                self.caret = a;
                self.clear_selection();
                self.blink_visible = true;
            }
            self.mark_edit_chord();
            cx.stop_propagation();
            cx.notify();
            return;
        }

        if key == "left" {
            self.move_caret(CaretMove::Left, mods.shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "right" {
            self.move_caret(CaretMove::Right, mods.shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "up" || key == "home" {
            self.move_caret(CaretMove::LineStart, mods.shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "down" || key == "end" {
            self.move_caret(CaretMove::LineEnd, mods.shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        // Shift alone is NOT a blocking modifier — it produces capitals / symbols.
        // Only ctrl/alt/platform/function should suppress plain text insertion.
        // (Old `mods.modified()` included shift and ate the first capital letter.)
        let blocks_text = mods.control || mods.alt || mods.platform || mods.function;
        if blocks_text && key != "backspace" && key != "delete" {
            return;
        }
        if key == "backspace" {
            self.backspace();
        } else if key == "delete" {
            self.delete_forward();
        } else if key == "space" {
            self.insert_text(" ");
        } else if key == "enter" {
            self.insert_text("\n");
        } else if let Some(ch) = event.keystroke.key_char.as_deref() {
            // Prefer key_char so Shift+letter inserts "H" not "h", and Shift+1 inserts "!".
            if ch.is_empty() {
                return;
            }
            self.insert_text(ch);
        } else if key.chars().count() == 1 {
            self.insert_text(key);
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn focus_compose_editor(&mut self) {
        self.compose_focused = true;
        self.ui_focus = UiFocus::Compose;
        self.blink_visible = true;
        if self.compose.is_empty() {
            // Empty draft: caret at left (index 0), never a phantom selection.
            self.caret = 0;
            self.sel_anchor = None;
        } else {
            self.caret = self.caret.min(self.compose.len());
        }
    }

    /// True when Compose owns edit chords (Ctrl+V/A/C/X). TerminalView must
    /// route Paste/Copy/SelectAll here so they never hit the shell.
    pub fn owns_compose_keys(&self) -> bool {
        !self.confirm
            && self.section == XSection::Compose
            && self.compose_focused
            && self.ui_focus == UiFocus::Compose
    }

    /// True when ANY panel text field owns edit chords: Compose, the
    /// Search query, an open inline reply, or the bearer-key entry.
    /// GPUI dispatches Ctrl+V/C/A as TerminalView Paste/Copy/SelectAll
    /// actions *instead of* this panel's `on_key_down`, and Compose only
    /// worked because `owns_compose_keys` intercepted them. The reply and
    /// Search fields were not covered, so pasting a reply silently went
    /// into the terminal shell behind the panel (verified in the real
    /// GUI: clipboard text landed at the shell prompt, reply unchanged).
    /// TerminalView must route those actions here whenever this is true.
    pub fn owns_edit_keys(&self) -> bool {
        edit_keys_owned(
            self.confirm,
            self.bearer_mode,
            self.input_target,
            self.section,
            self.reply_post_id.is_some(),
            self.compose_focused,
            self.ui_focus == UiFocus::Compose,
        )
    }

    /// Paste clipboard text into whichever panel field is active.
    /// Mirrors the Compose path (chord dedup included) so the KeyDown
    /// handler and the TerminalView action cannot insert twice.
    pub fn paste_active_text(&mut self, text: &str) {
        if self.chord_already_applied() {
            return;
        }
        let clean = text.trim_end_matches(['\r', '\n']);
        if self.bearer_mode {
            self.bearer_input.push_str(clean.trim());
        } else {
            match self.input_target {
                InputTarget::Compose => {
                    self.focus_compose_editor();
                    self.insert_text(clean);
                }
                InputTarget::Search => self.search_input.push_str(clean),
                InputTarget::Reply => self.reply_text.push_str(clean),
            }
        }
        self.blink_visible = true;
        self.mark_edit_chord();
    }

    /// Text a Copy action should take, if the active field has a
    /// selection. Search/reply fields are end-caret only (no selection),
    /// so they return None; the action is still swallowed by the caller
    /// so the terminal selection is never copied over the clipboard.
    pub fn active_copy_text(&self) -> Option<String> {
        if self.input_target == InputTarget::Compose {
            self.compose_selection_text()
        } else {
            None
        }
    }

    /// SelectAll for the active field. Search/reply fields have no
    /// selection model, so this just marks the chord (the caller swallows
    /// the action, keeping it out of the terminal).
    pub fn select_all_active(&mut self) {
        if self.input_target == InputTarget::Compose && !self.bearer_mode {
            self.select_all_compose();
        } else if !self.chord_already_applied() {
            self.mark_edit_chord();
        }
        self.blink_visible = true;
    }

    pub fn focus_compose_editor_public(&mut self) {
        self.focus_compose_editor();
    }

    pub fn insert_compose_text(&mut self, text: &str) {
        if self.chord_already_applied() {
            return;
        }
        self.insert_text(text);
        self.mark_edit_chord();
    }

    pub fn compose_selection_text(&self) -> Option<String> {
        let (a, b) = self.selection_range()?;
        Some(self.compose[a..b].to_string())
    }

    pub fn select_all_compose(&mut self) {
        if self.chord_already_applied() {
            return;
        }
        self.sel_anchor = Some(0);
        self.caret = self.compose.len();
        self.blink_visible = true;
        self.mark_edit_chord();
    }

    fn mark_edit_chord(&mut self) {
        self.last_edit_chord_at = Some(Instant::now());
    }

    fn chord_already_applied(&self) -> bool {
        self.last_edit_chord_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(80))
    }

    fn start_caret_blink(&mut self, cx: &mut Context<Self>) {
        if self.blink_running {
            return;
        }
        self.blink_running = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(530))
                    .await;
                let keep = this
                    .update(cx, |panel, cx| {
                        if !panel.confirm
                            && (panel.input_target != InputTarget::Compose
                                || (panel.compose_focused && panel.section == XSection::Compose))
                        {
                            panel.blink_visible = !panel.blink_visible;
                            cx.notify();
                        } else {
                            panel.blink_visible = true;
                        }
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }

    /// Shared keyboard handling for the Search query and inline reply
    /// fields: plain text appends at the end, Backspace deletes, Ctrl+V
    /// pastes. Enter runs a search; in a reply Enter adds a line and
    /// Ctrl+Enter sends. Everything is swallowed so the terminal behind
    /// the panel never sees these keys.
    fn on_text_input_key(
        &mut self,
        event: &KeyDownEvent,
        field: TextField,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if key == "enter" && (mods.control || mods.platform) {
            if field == TextField::Reply {
                self.submit_reply(cx);
            } else {
                self.run_search(cx);
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "enter" && !mods.modified() {
            match field {
                TextField::Search => self.run_search(cx),
                TextField::Reply => self.reply_text.push('\n'),
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        // Read chord/clipboard state before borrowing the target string.
        let is_paste = (mods.control || mods.platform) && key.eq_ignore_ascii_case("v");
        let already_applied = is_paste && self.chord_already_applied();
        let clip_text = if is_paste && !already_applied {
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .map(|clip| clip.trim_end_matches(['\r', '\n']).to_string())
        } else {
            None
        };
        let mut pasted = false;
        let target = match field {
            TextField::Search => &mut self.search_input,
            TextField::Reply => &mut self.reply_text,
        };
        if key == "backspace" {
            target.pop();
        } else if is_paste {
            // If TerminalView's Paste action already inserted this chord
            // (it routes here via `paste_active_text`), do not insert twice.
            if let Some(text) = &clip_text {
                target.push_str(text);
                pasted = true;
            }
        } else if !(mods.control || mods.alt || mods.platform || mods.function)
            && let Some(ch) = event.keystroke.key_char.as_deref()
        {
            target.push_str(ch);
        } else {
            return;
        }
        if pasted {
            self.mark_edit_chord();
        }
        self.blink_visible = true;
        cx.stop_propagation();
        cx.notify();
    }

    /// Run the typed query through the active provider (OpenCLI silent
    /// background session). Empty query = a plain hint, never a fetch.
    fn run_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.trim().to_string();
        if query.is_empty() {
            self.notice = Some(
                "Type something to search first — a word, #tag, or from:someone — then press Enter."
                    .into(),
            );
            self.status = "search · nothing typed yet".into();
            cx.notify();
            return;
        }
        self.query = query.clone();
        self.searched_query = query.clone();
        self.search_posts.clear();
        self.loading = true;
        self.status = format!(
            "Searching for \u{201c}{query}\u{201d}\u{2026} silently in the background \u{b7} no Chrome window will pop"
        );
        self.schedule_fetch(XSection::Search, cx);
        cx.notify();
    }

    fn open_reply(&mut self, post: &Post) {
        if self.reply_post_id.as_deref() != Some(post.id.as_str()) {
            self.reply_text.clear();
            self.reply_media.clear();
        }
        self.reply_post_id = Some(post.id.clone());
        self.reply_author = post.author_handle.clone();
        self.input_target = InputTarget::Reply;
        self.pending_focus = true;
        self.status = format!(
            "reply to @{} \u{b7} type below, then Reply (or Ctrl+Enter)",
            post.author_handle
        );
    }

    fn close_reply(&mut self) {
        self.reply_post_id = None;
        self.reply_author.clear();
        self.reply_text.clear();
        self.reply_media.clear();
        self.input_target = if self.section == XSection::Search {
            InputTarget::Search
        } else {
            InputTarget::Compose
        };
        self.status = "reply closed \u{b7} nothing was posted".into();
    }

    /// Post the inline reply in-app via the active publisher, threaded to
    /// the selected tweet (`twitter reply <url> <text>` on OpenCLI, all
    /// `--window background`). Never opens Chrome. A real click on Reply
    /// is the confirmation — the draft and target are shown in place.
    fn submit_reply(&mut self, cx: &mut Context<Self>) {
        if self.reply_posting {
            self.notice = Some("Already sending that reply — wait for it.".into());
            cx.notify();
            return;
        }
        let Some(target_id) = self.reply_post_id.clone() else {
            return;
        };
        let text = self.reply_text.trim().to_string();
        if text.is_empty() {
            self.notice = Some("Type a reply first.".into());
            cx.notify();
            return;
        }
        if !self.live {
            self.notice = Some("Preview/mock never posts a reply.".into());
            cx.notify();
            return;
        }
        if let Err(error) = termy_x::publish::validate_media_paths(&self.reply_media) {
            self.notice = Some(error);
            cx.notify();
            return;
        }
        if self.reply_media.len() > 1 {
            self.notice = Some(
                "Replies can attach 1 image (OpenCLI limit) — remove the extra image first. Nothing was posted."
                    .into(),
            );
            cx.notify();
            return;
        }
        let media = self.reply_media.clone();
        let author = self.reply_author.clone();
        let target_id_for_update = target_id.clone();
        self.reply_posting = true;
        self.notice = Some(format!(
            "Replying to @{author} in-app, silently in the background\u{2026} no browser will open."
        ));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let service = XPanel::service_free();
                    let publishers = service.publishers;
                    let Some(publisher) = termy_x::publish::first_available(publishers.as_slice())
                    else {
                        return Err(
                            "No in-app posting path found. Connect X first, then try again."
                                .to_string(),
                        );
                    };
                    let char_limit =
                        termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
                            .char_limit_or_fallback();
                    match termy_x::publish::confirm_and_publish_with_media(
                        &text,
                        Some(&target_id),
                        termy_x::publish::ConfirmDecision::Yes,
                        false,
                        publisher,
                        char_limit,
                        &media,
                    ) {
                        termy_x::publish::GateOutcome::Sent { receipt, .. } => Ok(receipt),
                        termy_x::publish::GateOutcome::Rejected { plan } => Err(plan
                            .session
                            .unwrap_or_else(|| "the reply was rejected".to_string())),
                        termy_x::publish::GateOutcome::DryRun { .. } => {
                            Err("unexpected dry-run".to_string())
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.reply_posting = false;
                match result {
                    Ok(_receipt) => {
                        if !panel.replied_ids.contains(&target_id_for_update) {
                            panel.replied_ids.push(target_id_for_update.clone());
                        }
                        panel.reply_post_id = None;
                        panel.reply_text.clear();
                        panel.reply_media.clear();
                        panel.input_target = if panel.section == XSection::Search {
                            InputTarget::Search
                        } else {
                            InputTarget::Compose
                        };
                        panel.notice = Some(format!(
                            "Replied to @{author} \u{b7} posted in-app, no browser opened."
                        ));
                        panel.status = format!("replied to @{author}");
                    }
                    Err(error) => {
                        panel.notice = Some(format!(
                            "Could not reply: {error} Nothing was posted; no browser opened."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.sel_anchor?;
        let a = anchor.min(self.caret);
        let b = anchor.max(self.caret);
        if a == b { None } else { Some((a, b)) }
    }

    fn clear_selection(&mut self) {
        self.sel_anchor = None;
    }

    fn insert_text(&mut self, text: &str) {
        if let Some((a, b)) = self.selection_range() {
            self.compose.replace_range(a..b, text);
            self.caret = a + text.len();
        } else {
            let at = self.caret.min(self.compose.len());
            self.compose.insert_str(at, text);
            self.caret = at + text.len();
        }
        self.clear_selection();
        self.blink_visible = true;
    }

    fn backspace(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.compose.replace_range(a..b, "");
            self.caret = a;
            self.clear_selection();
            self.blink_visible = true;
            return;
        }
        if self.caret == 0 {
            return;
        }
        let prev = prev_boundary(&self.compose, self.caret);
        self.compose.replace_range(prev..self.caret, "");
        self.caret = prev;
        self.blink_visible = true;
    }

    fn delete_forward(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.compose.replace_range(a..b, "");
            self.caret = a;
            self.clear_selection();
            self.blink_visible = true;
            return;
        }
        if self.caret >= self.compose.len() {
            return;
        }
        let next = next_boundary(&self.compose, self.caret);
        self.compose.replace_range(self.caret..next, "");
        self.blink_visible = true;
    }

    fn move_caret(&mut self, motion: CaretMove, extend: bool) {
        if extend && self.sel_anchor.is_none() {
            self.sel_anchor = Some(self.caret);
        } else if !extend {
            self.clear_selection();
        }
        match motion {
            CaretMove::Left => self.caret = prev_boundary(&self.compose, self.caret),
            CaretMove::Right => self.caret = next_boundary(&self.compose, self.caret),
            CaretMove::LineStart => {
                let head = self.compose[..self.caret].rfind('\n').map_or(0, |i| i + 1);
                self.caret = head;
            }
            CaretMove::LineEnd => {
                let tail = self.compose[self.caret..]
                    .find('\n')
                    .map_or(self.compose.len(), |i| self.caret + i);
                self.caret = tail;
            }
        }
        self.blink_visible = true;
    }

    fn cycle_ui_focus(&mut self, reverse: bool) {
        let order = if self.confirm {
            [
                UiFocus::ConfirmCancel,
                UiFocus::ConfirmCopy,
                UiFocus::ConfirmNext,
                UiFocus::ConfirmBrowser,
            ]
            .as_slice()
        } else if self.section == XSection::Compose {
            [
                UiFocus::Compose,
                UiFocus::Clear,
                UiFocus::Attach,
                UiFocus::Suggest,
                UiFocus::Publish,
                UiFocus::Art,
            ]
            .as_slice()
        } else {
            [UiFocus::Art].as_slice()
        };
        let idx = order.iter().position(|f| *f == self.ui_focus).unwrap_or(0);
        let next = if reverse {
            if idx == 0 { order.len() - 1 } else { idx - 1 }
        } else {
            (idx + 1) % order.len()
        };
        self.ui_focus = order[next];
        self.compose_focused = self.ui_focus == UiFocus::Compose && !self.confirm;
        self.blink_visible = true;
    }
}

#[derive(Clone, Copy)]
enum CaretMove {
    Left,
    Right,
    LineStart,
    LineEnd,
}

fn prev_boundary(text: &str, offset: usize) -> usize {
    if offset == 0 {
        return 0;
    }
    let offset = offset.min(text.len());
    text[..offset]
        .char_indices()
        .next_back()
        .map_or(0, |(i, _)| i)
}

fn next_boundary(text: &str, offset: usize) -> usize {
    if offset >= text.len() {
        return text.len();
    }
    text[offset..]
        .chars()
        .next()
        .map_or(text.len(), |c| offset + c.len_utf8())
}

fn live_status(status: &termy_x::StatusLine) -> String {
    if status.provider == termy_x::ProviderId::Mock {
        format!(
            "{} \u{b7} No live X session answered \u{2014} open Chrome, make sure x.com is logged in as you, then press \u{201c}Connect my X\u{201d} above.",
            status.render()
        )
    } else {
        status.render()
    }
}

fn fetch_section(section: XSection, query: String, lookup: String, live: bool) -> FetchPayload {
    if live {
        let service = XPanel::service_free();
        match section {
            XSection::Timeline => {
                if let Ok(served) = service.timeline(10) {
                    return FetchPayload::Timeline {
                        posts: served.value,
                        status: live_status(&served.status),
                    };
                }
            }
            XSection::Search => {
                if let Ok(served) = service.search(&query, false, 15) {
                    return FetchPayload::Search {
                        posts: served.value,
                        status: live_status(&served.status),
                    };
                }
            }
            XSection::Trends => {
                if let Ok(served) = service.trends(Some("worldwide"), 8) {
                    return FetchPayload::Trends {
                        trends: served.value,
                        status: live_status(&served.status),
                    };
                }
            }
            XSection::Lookup => {
                if let Ok(served) = service.lookup(&lookup) {
                    return FetchPayload::Lookup {
                        profile: Some(served.value),
                        status: live_status(&served.status),
                    };
                }
            }
            XSection::Research => {
                let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
                let runner = termy_x::io::SystemRunner;
                let hits = service.research("gpui terminal", &runner, &config);
                return FetchPayload::Research {
                    hits,
                    status: "research Â· no X API credits".into(),
                };
            }
            XSection::Compose => {}
        }
    }

    match section {
        XSection::Timeline => FetchPayload::Timeline {
            posts: MockProvider
                .timeline(10)
                .map(|served| served.value)
                .unwrap_or_default(),
            status: "mock Â· fixture data Â· no X API credits".into(),
        },
        XSection::Search => FetchPayload::Search {
            posts: MockProvider
                .search(&query, false, 15)
                .map(|served| served.value)
                .unwrap_or_default(),
            status: format!("mock Â· search â€œ{query}â€ Â· no X API credits"),
        },
        XSection::Trends => FetchPayload::Trends {
            trends: MockProvider
                .trends(Some("worldwide"), 8)
                .map(|served| served.value)
                .unwrap_or_default(),
            status: "mock Â· trends Â· no X API credits".into(),
        },
        XSection::Lookup => FetchPayload::Lookup {
            profile: MockProvider
                .lookup(&lookup)
                .ok()
                .map(|served| served.value),
            status: format!(
                "mock Â· lookup @{} Â· no X API credits",
                lookup.trim_start_matches('@')
            ),
        },
        XSection::Research => FetchPayload::Research {
            hits: vec![ResearchHit {
                source: "fixture".into(),
                title: "GPUI terminals people actually use".into(),
                url: "https://example.com/gpui".into(),
                excerpt: "Fixture research for drafts. Install OpenCLI, yt-dlp, or curl to pull Reddit, YouTube, and the web.".into(),
                session: None,
            }],
            status: "research Â· fixture Â· no X API credits".into(),
        },
        XSection::Compose => FetchPayload::Timeline {
            posts: Vec::new(),
            status: "compose".into(),
        },
    }
}

impl Render for XPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending_focus {
            self.focus.focus(window, cx);
            self.pending_focus = false;
        }
        let char_limit = termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
            .char_limit_or_fallback();
        let count = weighted_len_limited(&self.compose, char_limit);
        let parts = split_thread_limited(&self.compose, char_limit);
        let columns = self.column_count(window);
        let art_opacity = self.bird_opacity();
        let dialog_width = if self.fill_window { 600.0 } else { 432.0 };

        let mut root = div()
            .id("termy-x-panel")
            .key_context("TermyX")
            .track_focus(&self.focus)
            .relative()
            .h_full()
            .overflow_hidden()
            .bg(rgb(0x070b14))
            .text_color(rgb(0xe7eefc))
            .font_family("sans-serif")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                }),
            )
            .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)));
        if self.fill_window {
            root = root.size_full();
        } else {
            root = root.w(px(480.0)).flex_none();
        }

        root.child(self.backdrop(art_opacity))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .p(px(16.0))
                    .gap(px(10.0))
                    .child(self.header(cx))
                    .child(self.tabs(cx))
                    .child(self.connection_bar(cx))
                    .child(self.status_line())
                    .child(if columns == 1 {
                        div()
                            .id("x-scroll")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .children(self.body(count.weighted, count.limit, &parts, cx))
                            .into_any_element()
                    } else {
                        self.dashboard(columns, count.weighted, count.limit, &parts, cx)
                    }),
            )
            .children(self.confirm.then(|| {
                self.confirm_dialog(count.weighted, count.limit, &parts, dialog_width, cx)
            }))
    }
}

impl XPanel {
    fn column_count(&self, window: &Window) -> usize {
        if self.splash || !self.fill_window {
            return 1;
        }
        let width: f32 = window.viewport_size().width.into();
        if width >= 1480.0 {
            3
        } else if width >= 1100.0 {
            2
        } else {
            1
        }
    }

    /// Full strength on the empty splash. Lists use the configured opacity.
    /// Art off returns 0 so the raster is never requested.
    fn bird_opacity(&self) -> f32 {
        if !self.background_art {
            return 0.0;
        }
        if self.splash || self.looks_empty() {
            1.0
        } else {
            self.background_opacity.clamp(0.0, 1.0)
        }
    }

    fn looks_empty(&self) -> bool {
        self.posts.is_empty()
            && self.search_posts.is_empty()
            && self.trends.is_empty()
            && self.research.is_empty()
            && self.profile.is_none()
            && self.variants.is_empty()
            && self.section != XSection::Compose
            && !self.confirm
    }

    fn backdrop(&mut self, opacity: f32) -> gpui_kit::AnyElement {
        let mut layer = div().absolute().top_0().left_0().size_full();
        if self.background_art && opacity > 0.0 {
            if self.art.is_none() {
                self.art = art::cached_image();
            }
            if let Some(image) = self.art.clone() {
                let source: ImageSource = image.into();
                layer = layer.child(
                    div().size_full().opacity(opacity).child(
                        img(source)
                            .id("termy-x-bird")
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    ),
                );
            }
        } else {
            layer = layer.child(div().size_full().bg(rgb(0x070b14)));
        }
        layer.into_any_element()
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div().flex().flex_col().child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x7af6ff))
                        .child("Termy X"),
                ),
            )
            .child({
                let art_focus = self.ui_focus == UiFocus::Art;
                div()
                    .id("x-art-toggle")
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(999.0))
                    .border_1()
                    .border_color(if art_focus {
                        rgb(0xff4fa3)
                    } else if self.background_art {
                        rgb(0x7af6ff)
                    } else {
                        tint(0x7af6ff, 40)
                    })
                    .bg(if art_focus {
                        rgb(0x2a0f3a)
                    } else {
                        tint(0x102033, 180)
                    })
                    .text_size(px(12.0))
                    .text_color(if art_focus {
                        rgb(0xffd0ea)
                    } else {
                        rgb(0xe7eefc)
                    })
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgb(0x1a3050))
                            .border_color(rgb(0x7af6ff))
                            .text_color(rgb(0x7af6ff))
                    })
                    .child(if self.background_art {
                        "Art on"
                    } else {
                        "Art off"
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.ui_focus = UiFocus::Art;
                            this.toggle_art();
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
            })
    }

    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.section;
        div().flex().gap(px(6.0)).flex_wrap().children(
            [
                XSection::Timeline,
                XSection::Search,
                XSection::Trends,
                XSection::Lookup,
                XSection::Compose,
                XSection::Research,
            ]
            .into_iter()
            .map(move |section| {
                let active = section == current;
                div()
                    .id(SharedString::from(format!("x-tab-{}", section.label())))
                    .px(px(10.0))
                    .py(px(5.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .border_1()
                    .border_color(if active {
                        rgb(0x7af6ff)
                    } else {
                        tint(0x7af6ff, 25)
                    })
                    .bg(if active {
                        rgb(0x122a55)
                    } else {
                        tint(0x101826, 170)
                    })
                    .text_color(if active { rgb(0x7af6ff) } else { rgb(0xb7c3dc) })
                    .hover(|style| {
                        style
                            .bg(rgb(0x1a3050))
                            .border_color(rgb(0xff4fa3))
                            .text_color(rgb(0xffd0ea))
                    })
                    .child(section.label())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.open_section(section, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
            }),
        )
    }

    fn status_line(&self) -> impl IntoElement {
        let label = if self.loading {
            format!("â³ {}", self.status)
        } else {
            self.status.clone()
        };
        div()
            .text_size(px(11.0))
            .text_color(rgb(0x93a4c3))
            .child(label)
    }

    fn dashboard(
        &self,
        columns: usize,
        weighted: usize,
        limit: usize,
        parts: &[String],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let mut row = div()
            .flex()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(12.0))
            .overflow_hidden();
        row = row.child(self.compose_column(weighted, limit, parts, cx));
        row = row.child(self.results_column(columns == 2));
        if columns >= 3 {
            row = row.child(self.trends_column());
        }
        row.into_any_element()
    }

    fn column_shell(
        &self,
        title: &str,
        body: impl IntoIterator<Item = gpui_kit::AnyElement>,
    ) -> gpui_kit::AnyElement {
        div()
            .id(SharedString::from(format!("x-col-{title}")))
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x7af6ff))
                    .child(title.to_string()),
            )
            .children(body)
            .into_any_element()
    }

    fn compose_column(
        &self,
        weighted: usize,
        limit: usize,
        parts: &[String],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        self.column_shell(
            "Compose",
            vec![self.compose_block(weighted, limit, parts, cx)],
        )
    }

    fn results_column(&self, include_side: bool) -> gpui_kit::AnyElement {
        let mut rows = Vec::new();
        rows.push(hint(format!("Search Â· {}", self.query)));
        let search = if self.search_posts.is_empty() {
            &self.posts
        } else {
            &self.search_posts
        };
        for post in search.iter().take(3) {
            rows.push(post_card(post));
        }
        rows.push(hint("Timeline".into()));
        for post in &self.posts {
            rows.push(post_card(post));
        }
        if let Some(root) = self.posts.first() {
            let replies = self.posts.iter().skip(1).take(2).count();
            rows.push(card(
                format!("Thread Â· @{}", root.author_handle),
                format!(
                    "{}\n{replies} repl{}",
                    root.text,
                    if replies == 1 { "y" } else { "ies" }
                ),
            ));
        }
        if include_side {
            rows.extend(self.side_rows());
        }
        self.column_shell("Results", rows)
    }

    fn trends_column(&self) -> gpui_kit::AnyElement {
        self.column_shell("Trends", self.side_rows())
    }

    fn side_rows(&self) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        for trend in &self.trends {
            rows.push(card(
                trend.name.clone(),
                format!(
                    "{} Â· volume {}",
                    trend.place.as_deref().unwrap_or("worldwide"),
                    trend.volume.unwrap_or(0)
                ),
            ));
        }
        if let Some(post) = self.posts.iter().max_by_key(|post| post.engagement()) {
            rows.push(card(
                format!("Popular Â· @{} Â· {} likes", post.author_handle, post.likes),
                post.text.clone(),
            ));
        }
        for hit in &self.research {
            rows.push(card(
                format!("[{}] {}", hit.source, hit.title),
                format!("{}\n{}", hit.url, hit.excerpt),
            ));
        }
        rows
    }

    fn body(
        &self,
        weighted: usize,
        limit: usize,
        parts: &[String],
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        if let Some(notice) = &self.notice {
            rows.push(hint(notice.clone()));
        }
        if self.live
            && matches!(self.connection, ConnState::NotConnected { .. })
            && self.section != XSection::Compose
        {
            rows.push(card(
                "Not connected yet".into(),
                "Press \u{201c}Connect my X\u{201d} above and follow the 3 steps. Your timeline, search, and trends fill in here as your account \u{2014} nothing is shown from anyone else's account.".into(),
            ));
            return rows;
        }
        match self.section {
            XSection::Timeline => {
                if self.posts.is_empty() {
                    if self.loading {
                        rows.push(empty_state(
                            "Loading timeline...",
                            "Using your free OpenCLI session. Nothing burns X API credits.",
                        ));
                    } else {
                        rows.push(empty_state(
                            "Timeline is quiet",
                            "OpenCLI / twitter-cli / fixtures fill this lane. Press Timeline to retry.",
                        ));
                    }
                } else if self.loading {
                    rows.push(hint("Refreshing timeline...".into()));
                }
                for post in &self.posts {
                    rows.push(self.tweet_card(post, cx));
                }
            }
            XSection::Search => {
                rows.push(self.search_bar(cx));
                if self.searched_query.is_empty() {
                    rows.push(empty_state(
                        "Search X for real",
                        "Type a query above and press Enter (or Search). It runs through your own X session, silently in the background \u{b7} no Chrome window will pop.",
                    ));
                } else if self.loading {
                    rows.push(hint(format!(
                        "Searching for \u{201c}{}\u{201d}\u{2026}",
                        self.searched_query
                    )));
                } else if self.search_posts.is_empty() {
                    rows.push(empty_state(
                        "No results",
                        format!(
                            "Nothing matched \u{201c}{}\u{201d}. Try a different word, a #tag, or from:someone.",
                            self.searched_query
                        ),
                    ));
                } else {
                    rows.push(hint(format!(
                        "Results for \u{201c}{}\u{201d} \u{b7} {} shown \u{b7} click a tweet to reply in place",
                        self.searched_query,
                        self.search_posts.len()
                    )));
                    for post in &self.search_posts {
                        rows.push(self.tweet_card(post, cx));
                    }
                }
            }
            XSection::Trends => {
                if self.trends.is_empty() {
                    if self.loading {
                        rows.push(empty_state(
                            "Scanning trends...",
                            "Free session path · no X API credits. Hang tight.",
                        ));
                    } else {
                        rows.push(empty_state(
                            "No trends yet",
                            "Hit Trends again, or check OpenCLI with x doctor. Mock fixtures fill in when offline.",
                        ));
                    }
                } else if self.loading {
                    rows.push(hint("Refreshing trends...".into()));
                }
                for trend in &self.trends {
                    rows.push(card(
                        trend.name.clone(),
                        format!(
                            "{} · volume {}",
                            trend.place.as_deref().unwrap_or("worldwide"),
                            trend.volume.unwrap_or(0)
                        ),
                    ));
                }
                if !self.posts.is_empty() {
                    rows.push(hint("Tweets \u{b7} click one to reply in place".into()));
                }
                for post in &self.posts {
                    rows.push(self.tweet_card(post, cx));
                }
            }
            XSection::Lookup => {
                if let Some(profile) = &self.profile {
                    rows.push(card(
                        format!("@{} · {}", profile.handle, profile.name),
                        format!(
                            "{}\n{} followers · {} following",
                            profile.bio, profile.followers, profile.following
                        ),
                    ));
                    for post in &profile.posts {
                        rows.push(self.tweet_card(post, cx));
                    }
                } else if self.loading {
                    rows.push(empty_state(
                        "Looking up...",
                        "Free session profile fetch. No X API credits.",
                    ));
                } else {
                    rows.push(empty_state(
                        "Lookup a handle",
                        "Enter @someone and hit Lookup. Verified / Premium shows in doctor post length.",
                    ));
                }
            }
            XSection::Research => {
                if self.research.is_empty() {
                    if self.loading {
                        rows.push(empty_state(
                            "Researching...",
                            "RSS + free sources. No X API credits.",
                        ));
                    } else {
                        rows.push(empty_state(
                            "Research is empty",
                            "Kick Research to pull free sources for your draft.",
                        ));
                    }
                } else if self.loading {
                    rows.push(hint("Refreshing research...".into()));
                }
                for hit in &self.research {
                    rows.push(card(
                        format!("[{}] {}", hit.source, hit.title),
                        format!("{}\n{}", hit.url, hit.excerpt),
                    ));
                }
            }
            XSection::Compose => {
                rows.push(self.compose_block(weighted, limit, parts, cx));
            }
        }
        rows
    }

    // First-launch splash is deferred; keep the view ready until it is wired in.
    #[allow(dead_code)]
    fn splash_copy(&self) -> gpui_kit::AnyElement {
        div()
            .mt(px(96.0))
            .px(px(24.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .text_size(px(28.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(rgb(0xe9fdff))
                    .child("Termy X - free session. Zero credits."),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(rgb(0x7af6ff))
                    .child("Timeline / Search / Trends / Compose / Research"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(0xc5d2ea))
                    .child("Your confirm before anything posts. Premium long-form up to 25,000."),
            )
            .into_any_element()
    }

    /// One tweet in Timeline / Search / Trends. Clicking it expands an
    /// inline reply box right underneath; a sent reply marks it Replied.
    fn tweet_card(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let selected = self.reply_post_id.as_deref() == Some(post.id.as_str());
        let replied = self.replied_ids.iter().any(|id| id == &post.id);
        let post_for_click = post.clone();
        let mut el = div()
            .id(SharedString::from(format!("x-tweet-{}", post.id)))
            .p(px(12.0))
            .rounded(px(12.0))
            .bg(if selected {
                tint(0x1a1030, 240)
            } else {
                tint(0x0c1422, 225)
            })
            .border_1()
            .border_color(if selected {
                rgb(0xff4fa3)
            } else {
                tint(0x7af6ff, 40)
            })
            .cursor_pointer()
            .hover(|style| style.bg(tint(0x122038, 245)).border_color(rgb(0x7af6ff)))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(0xd7fbff))
                    .child(format!(
                        "@{} \u{b7} {} likes{} \u{b7} click to reply",
                        post.author_handle,
                        post.likes,
                        if replied {
                            " \u{b7} \u{2713} Replied"
                        } else {
                            ""
                        }
                    )),
            )
            .child(
                div()
                    .mt(px(4.0))
                    .text_size(px(13.0))
                    .text_color(rgb(0xe7eefc))
                    .child(post.text.clone()),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    if this.reply_post_id.as_deref() == Some(post_for_click.id.as_str()) {
                        // Second click on the open tweet just refocuses it.
                        this.input_target = InputTarget::Reply;
                        this.pending_focus = true;
                    } else {
                        this.open_reply(&post_for_click);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        if selected {
            el = el.child(self.reply_box(post, cx));
        }
        el.into_any_element()
    }

    /// The inline reply editor shown under the selected tweet.
    fn reply_box(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let char_limit = termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
            .char_limit_or_fallback();
        let count = weighted_len_limited(&self.reply_text, char_limit);
        let focused = self.input_target == InputTarget::Reply;
        let mut box_el = div()
            .mt(px(8.0))
            .p(px(10.0))
            .rounded(px(10.0))
            .bg(tint(0x0f1c33, 245))
            .border_1()
            .border_color(if focused { rgb(0x7af6ff) } else { tint(0x7af6ff, 70) })
            .flex()
            .flex_col()
            .gap(px(6.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    // Clicking anywhere in the reply box (label, padding,
                    // count line) focuses the editor, not just the small
                    // text field itself. Buttons/editors inside stop
                    // propagation first, so their own handlers still win.
                    this.input_target = InputTarget::Reply;
                    this.pending_focus = true;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x7af6ff))
                    .child(format!("Reply to @{}", post.author_handle)),
            )
            .child(
                div()
                    .id("x-reply-editor")
                    .min_h(px(44.0))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(tint(0x0c1422, 235))
                    .cursor_pointer()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(if self.reply_text.is_empty() {
                                rgb(0x93a4c3)
                            } else {
                                rgb(0xf8fbff)
                            })
                            .child(if self.reply_text.is_empty() {
                                "Write your reply\u{2026} (Ctrl+Enter sends)".to_string()
                            } else if focused && self.blink_visible {
                                format!("{}\u{258f}", self.reply_text)
                            } else {
                                self.reply_text.clone()
                            }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.input_target = InputTarget::Reply;
                            this.pending_focus = true;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if count.weighted > count.limit {
                        rgb(0xff5d8f)
                    } else {
                        rgb(0x9ad7ff)
                    })
                    .child(format!(
                        "{} / {} characters \u{b7} Ctrl+Enter or the Reply button sends \u{b7} replies attach 1 image",
                        count.weighted, count.limit
                    )),
            );
        if !self.reply_media.is_empty() {
            for (index, path) in self.reply_media.iter().enumerate() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string();
                let thumb_source: ImageSource = path.clone().into();
                box_el = box_el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            img(thumb_source)
                                .w(px(40.0))
                                .h(px(40.0))
                                .object_fit(ObjectFit::Cover)
                                .rounded_md(),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x7af6ff))
                                .child(name),
                        )
                        .child(x_button!(
                            format!("x-reply-media-remove-{index}"),
                            "\u{2715} Remove",
                            cx.listener(move |this, _, _, cx| {
                                this.remove_reply_media(index, cx);
                                cx.stop_propagation();
                            })
                        )),
                );
            }
        }
        box_el = box_el.child(
            div()
                .flex()
                .gap(px(8.0))
                .flex_wrap()
                .child(x_button!(
                    "x-reply-attach",
                    "Attach image",
                    cx.listener(|this, _, _, cx| {
                        this.pick_target = PickTarget::Reply;
                        this.attach_images(cx);
                        cx.stop_propagation();
                    })
                ))
                .child(x_button!(
                    "x-reply-send",
                    if self.reply_posting {
                        "Replying\u{2026}"
                    } else {
                        "Reply"
                    },
                    cx.listener(|this, _, _, cx| {
                        this.submit_reply(cx);
                        cx.stop_propagation();
                    })
                ))
                .child(x_button!(
                    "x-reply-cancel",
                    "Cancel",
                    cx.listener(|this, _, _, cx| {
                        this.close_reply();
                        cx.stop_propagation();
                        cx.notify();
                    })
                )),
        );
        box_el.into_any_element()
    }

    /// Search tab header: the query field + Run button. Focused when the
    /// tab opens; Enter runs the same path as the button.
    fn search_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let focused = self.input_target == InputTarget::Search && self.section == XSection::Search;
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x7af6ff))
                    .child("Search X \u{b7} silent background \u{b7} up to 15 results (OpenCLI has no next-page button here)"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .items_center()
                    .child(
                        div()
                            .id("x-search-input")
                            .flex_1()
                            .p(px(10.0))
                            .rounded(px(8.0))
                            .bg(if focused {
                                tint(0x0f1c33, 245)
                            } else {
                                tint(0x0c1422, 235)
                            })
                            .border_1()
                            .border_color(if focused {
                                rgb(0x7af6ff)
                            } else {
                                tint(0x7af6ff, 70)
                            })
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .text_color(if self.search_input.is_empty() {
                                        rgb(0x93a4c3)
                                    } else {
                                        rgb(0xf8fbff)
                                    })
                                    .child(if self.search_input.is_empty() {
                                        "Type a query \u{2014} a word, #tag, or from:someone".to_string()
                                    } else if focused && self.blink_visible {
                                        format!("{}\u{258f}", self.search_input)
                                    } else {
                                        self.search_input.clone()
                                    }),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    this.input_target = InputTarget::Search;
                                    this.pending_focus = true;
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(x_button!(
                        "x-search-run",
                        if self.loading { "Searching\u{2026}" } else { "Search" },
                        cx.listener(|this, _, _, cx| {
                            this.run_search(cx);
                            cx.stop_propagation();
                        })
                    )),
            )
            .into_any_element()
    }

    fn attach_images(&mut self, cx: &mut Context<Self>) {
        // CRITICAL: never run the native file dialog synchronously inside a
        // GPUI mouse/key handler. On Windows the modal dialog pumps window
        // messages while GPUI's RefCell is still borrowed, which panicked
        // with "RefCell already borrowed" and killed the whole app
        // (crash.log 2026-10-18). The dialog runs on a background thread;
        // the GPUI update that consumes its result happens afterwards,
        // outside any handler borrow.
        if self.picking {
            self.notice = Some("Image picker is already open.".into());
            cx.notify();
            return;
        }
        let attached = match self.pick_target {
            PickTarget::Compose => self.media.len(),
            PickTarget::Reply => self.reply_media.len(),
        };
        if attached >= 4 {
            self.notice = Some("Max 4 images — remove one first.".into());
            cx.notify();
            return;
        }
        self.picking = true;
        self.notice = Some("Pick images… (JPG/PNG/GIF/WEBP, max 4)".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let paths = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_title("Attach images to Termy X")
                        .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp"])
                        .pick_files()
                        .unwrap_or_default()
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.picking = false;
                panel.add_picked_images(paths);
                cx.notify();
            });
        })
        .detach();
    }

    fn add_picked_images(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            // Cancelled picker: say so instead of claiming an attach happened.
            self.notice = Some("No image picked.".into());
            return;
        }
        let target = match self.pick_target {
            PickTarget::Compose => &mut self.media,
            PickTarget::Reply => &mut self.reply_media,
        };
        let mut added = 0usize;
        let mut rejected: Vec<String> = Vec::new();
        for path in paths {
            if target.len() >= 4 {
                rejected.push("max 4 images".into());
                break;
            }
            if target.contains(&path) {
                rejected.push(format!(
                    "{} (already attached)",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or("image")
                ));
                continue;
            }
            match termy_x::publish::validate_media_path(&path) {
                Ok(()) => {
                    target.push(path);
                    added += 1;
                }
                Err(error) => rejected.push(error),
            }
        }
        let total = target.len();
        self.notice = Some(if rejected.is_empty() {
            format!(
                "Attached {added} image(s) \u{2014} {total} attached. They show below; tap \u{2715} to remove one."
            )
        } else {
            format!(
                "Attached {added} image(s). Skipped: {}",
                rejected.join("; ")
            )
        });
    }

    fn remove_media(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.media.len() {
            let removed = self.media.remove(index);
            self.notice = Some(format!(
                "Removed {}. {} image(s) left.",
                removed
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image"),
                self.media.len()
            ));
        }
        cx.notify();
    }

    fn remove_reply_media(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.reply_media.len() {
            self.reply_media.remove(index);
            self.notice = Some(format!("{} reply image(s) left.", self.reply_media.len()));
        }
        cx.notify();
    }

    fn clear_media(&mut self, cx: &mut Context<Self>) {
        self.media.clear();
        self.notice = Some("Cleared attachments.".into());
        cx.notify();
    }

    fn compose_block(
        &self,
        weighted: usize,
        limit: usize,
        parts: &[String],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let counter = if limit >= 25_000 {
            format!("{weighted} / {limit} Premium")
        } else {
            format!("{weighted} / {limit}")
        };
        let suggest_label = if self.suggesting {
            "Suggesting…"
        } else {
            "Suggest variants"
        };
        let clear_active = self.ui_focus == UiFocus::Clear;
        let attach_active = self.ui_focus == UiFocus::Attach;
        let suggest_active = self.ui_focus == UiFocus::Suggest;
        let publish_active = self.ui_focus == UiFocus::Publish;
        let editor_focused =
            self.compose_focused && self.ui_focus == UiFocus::Compose && !self.confirm;
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            // Attachments strip
            .child({
                let mut row = div().flex().flex_col().gap(px(6.0));
                row = row.child(
                    div()
                        .flex()
                        .gap(px(8.0))
                        .flex_wrap()
                        .child(x_button!("x-attach", "Attach image", attach_active, cx.listener(|this, _, _, cx| {
                            this.pick_target = PickTarget::Compose;
                            this.attach_images(cx);
                        })))
                        .child(x_button!("x-media-clear", "Clear images", false, cx.listener(|this, _, _, cx| {
                            this.clear_media(cx);
                        }))),
                );
                if self.media.is_empty() {
                    row = row.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x93a4c3))
                            .child("No images attached. JPG/PNG/GIF/WEBP. Max 4."),
                    );
                } else {
                    for (index, path) in self.media.iter().enumerate() {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("image")
                            .to_string();
                        let label = format!("[{}] {}", index + 1, name);
                        let thumb_source: ImageSource = path.clone().into();
                        row = row.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(8.0))
                                .py(px(4.0))
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(0x2f7bff))
                                .bg(tint(0x12203a, 0xcc))
                                .child(
                                    img(thumb_source)
                                        .w(px(48.0))
                                        .h(px(48.0))
                                        .object_fit(ObjectFit::Cover)
                                        .rounded_md(),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(0x7af6ff))
                                        .child(label),
                                )
                                .child(x_button!(
                                    format!("x-media-remove-{index}"),
                                    "✕ Remove",
                                    cx.listener(move |this, _, _, cx| {
                                        this.remove_media(index, cx);
                                    })
                                )),
                        );
                    }
                }
                row
            })
            // Sticky actions FIRST so Publish is never below the fold.
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .flex_wrap()
                    .child(x_button!("x-clear", "Clear", clear_active, cx.listener(|this, _, _, cx| {
                        this.compose.clear();
                        this.caret = 0;
                        this.sel_anchor = None;
                        this.variants.clear();
                        this.notice = Some("Draft cleared.".into());
                        this.focus_compose_editor();
                        this.pending_focus = true;
                        cx.stop_propagation();
                        cx.notify();
                    })))
                    .child(x_button!("x-suggest", suggest_label, suggest_active, cx.listener(|this, _, _, cx| {
                        this.suggest_async(cx);
                        this.focus_compose_editor();
                        this.pending_focus = true;
                        cx.stop_propagation();
                        cx.notify();
                    })))
                    .child(x_button!("x-publish", "Publish", publish_active, cx.listener(|this, _, _, cx| {
                        this.ui_focus = UiFocus::ConfirmNext;
                        this.arm_confirm();
                        cx.stop_propagation();
                        cx.notify();
                    }))),
            )
            .child(
                div()
                    .id("x-compose-editor")
                    .p(px(12.0))
                    .min_h(px(132.0))
                    .rounded(px(12.0))
                    .bg(if editor_focused {
                        tint(0x0f1c33, 245)
                    } else {
                        tint(0x0c1422, 235)
                    })
                    .border_1()
                    .border_color(if editor_focused {
                        rgb(0x7af6ff)
                    } else {
                        tint(0x7af6ff, 70)
                    })
                    .cursor_pointer()
                    .hover(|style| style.border_color(rgb(0xff4fa3)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(if editor_focused {
                                        rgb(0x7af6ff)
                                    } else {
                                        rgb(0x9ad7ff)
                                    })
                                    .child(if editor_focused {
                                        "Compose · focused"
                                    } else {
                                        "Compose · click to focus"
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(0xf0a8d0))
                                    .child("Tab cycles · Esc cancels confirm"),
                            ),
                    )
                    .child(self.render_compose_editor())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, window, cx| {
                            this.focus.focus(window, cx);
                            this.focus_compose_editor();
                            // Empty draft: caret at left (0). Non-empty: jump to end.
                            if this.compose.is_empty() {
                                this.caret = 0;
                            } else {
                                this.caret = this.compose.len();
                            }
                            this.sel_anchor = None;
                            this.pending_focus = false;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(if weighted > limit { rgb(0xff5d8f) } else { rgb(0x9ad7ff) })
                    .child(counter),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xb7c3dc))
                    .child(format!(
                        "{} thread part(s). URLs count as 23. Confirm shows exact text before any post.",
                        parts.len().max(1)
                    )),
            )
            .children(parts.iter().enumerate().filter(|(_, part)| !part.is_empty()).map(|(index, part)| {
                card(format!("Part {}", index + 1), part.clone())
            }))
            .children(self.drafts.iter().map(|draft| {
                card(
                    format!("Draft · {}", draft.tone.as_deref().unwrap_or("note")),
                    draft.text.clone(),
                )
            }))
            .children(self.variants.iter().enumerate().map(|(index, draft)| {
                let label = format!("Variant {} · {}", index + 1, draft.source);
                div()
                    .id(SharedString::from(format!("x-variant-{index}")))
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(tint(0x10192b, 220))
                    .border_1()
                    .border_color(tint(0xff4fa3, 55))
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(rgb(0x1a1030))
                            .border_color(rgb(0xff4fa3))
                    })
                    .child(div().text_size(px(11.0)).text_color(rgb(0xf0a8d0)).child(label))
                    .child(div().mt(px(4.0)).text_size(px(13.0)).child(draft.text.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.accept_variant(index);
                            this.focus_compose_editor();
                            this.pending_focus = true;
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
            }))
            .into_any_element()
    }

    fn render_compose_editor(&self) -> gpui_kit::AnyElement {
        let focused = self.compose_focused && self.ui_focus == UiFocus::Compose && !self.confirm;
        let show_caret = focused && self.blink_visible;
        let caret = self.caret.min(self.compose.len());
        let sel = self.selection_range();

        if self.compose.is_empty() {
            // Caret at LEFT (index 0). Hint only when empty AND unfocused —
            // once the caret blinks, the placeholder must disappear.
            return div()
                .mt(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .children(show_caret.then(|| self.caret_bar()))
                .children((!focused).then(|| {
                    div()
                        .text_size(px(14.0))
                        .text_color(rgb(0x93a4c3))
                        .child(
                            "Click here - caret blinks, hint vanishes. Ctrl+V pastes. Premium counts to 25,000.",
                        )
                }))
                .into_any_element();
        }

        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
        let text = self.compose.as_str();
        let mut offset = 0usize;
        let parts: Vec<&str> = text.split('\n').collect();
        for (idx, line) in parts.iter().enumerate() {
            let line_start = offset;
            let line_end = line_start + line.len();
            let caret_on_line = caret >= line_start && caret <= line_end;
            let mut row = div().flex().flex_row().items_center().min_h(px(20.0));

            // Build character runs: [start, end, selected]
            let mut marks = vec![0usize, line.len()];
            if caret_on_line {
                marks.push((caret - line_start).min(line.len()));
            }
            if let Some((sa, sb)) = sel {
                marks.push(sa.saturating_sub(line_start).min(line.len()));
                marks.push(sb.saturating_sub(line_start).min(line.len()));
            }
            marks.sort_unstable();
            marks.dedup();

            let mut caret_drawn = false;
            for window in marks.windows(2) {
                let (s, e) = (window[0], window[1]);
                if show_caret && caret_on_line && !caret_drawn && (caret - line_start) == s {
                    row = row.child(self.caret_bar());
                    caret_drawn = true;
                }
                if s < e {
                    let abs_s = line_start + s;
                    let abs_e = line_start + e;
                    let selected = sel.is_some_and(|(sa, sb)| abs_s >= sa && abs_e <= sb);
                    let chunk = &line[s..e];
                    let mut el = div().text_size(px(14.0));
                    if selected {
                        el = el
                            .px(px(1.0))
                            .rounded(px(3.0))
                            .bg(tint(0xff4fa3, 130))
                            .text_color(rgb(0xffe8f5));
                    } else {
                        el = el.text_color(rgb(0xf8fbff));
                    }
                    row = row.child(el.child(chunk.to_string()));
                }
                if show_caret && caret_on_line && !caret_drawn && (caret - line_start) == e {
                    row = row.child(self.caret_bar());
                    caret_drawn = true;
                }
            }
            if show_caret && caret_on_line && !caret_drawn {
                row = row.child(self.caret_bar());
            }
            if line.is_empty() && !(show_caret && caret_on_line) {
                row = row.child(
                    div()
                        .text_size(px(14.0))
                        .text_color(rgb(0xf8fbff))
                        .child(" "),
                );
            }

            rows.push(row.into_any_element());
            offset = line_end + if idx + 1 < parts.len() { 1 } else { 0 };
        }

        div()
            .mt(px(8.0))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .children(rows)
            .into_any_element()
    }

    fn caret_bar(&self) -> gpui_kit::AnyElement {
        div()
            .w(px(2.0))
            .h(px(16.0))
            .rounded(px(1.0))
            .bg(rgb(0x7af6ff))
            .flex_none()
            .into_any_element()
    }

    fn confirm_dialog(
        &self,
        weighted: usize,
        limit: usize,
        parts: &[String],
        dialog_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let current = if parts.is_empty() {
            0
        } else {
            self.intent_index.min(parts.len() - 1)
        };
        let url = self.intent_urls.get(current).cloned().unwrap_or_default();

        let media_lines: Vec<String> = self
            .media
            .iter()
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string()
            })
            .collect();
        let media_block = if media_lines.is_empty() {
            div().into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0x7af6ff))
                        .child(format!("Attached media ({}):", media_lines.len()))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0xffd0ea))
                                .child("Post now sends these in-app via OpenCLI --images (no browser). If you choose Open in browser instead, attach them there — a web link can't upload files."),
                        ),
                )
                .children(media_lines.into_iter().map(|name| {
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xc5d2ea))
                        .child(format!("- {name}"))
                        .into_any_element()
                }))
                .into_any_element()
        };

        let part_rows = parts.iter().enumerate().map(|(index, part)| {
            let active = index == current;
            let part_count = weighted_len_limited(part, limit).weighted;
            div()
                .p(px(10.0))
                .rounded(px(8.0))
                .bg(rgb(0x070b14))
                .border_1()
                .border_color(if active {
                    rgb(0xff4fa3)
                } else {
                    tint(0x7af6ff, 40)
                })
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(if active { rgb(0xffd0ea) } else { rgb(0x93a4c3) })
                        .child(format!(
                            "Part {} of {} Â· {part_count} weighted",
                            index + 1,
                            parts.len()
                        )),
                )
                .child(
                    div()
                        .mt(px(4.0))
                        .text_size(px(14.0))
                        .text_color(rgb(0xf8fbff))
                        .child(part.clone()),
                )
        });
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(tint(0x03060c, 170))
            .child(
                div()
                    .id("x-confirm")
                    .w(px(dialog_width))
                    .max_w(px(640.0))
                    .max_h(px(720.0))
                    .p(px(16.0))
                    .rounded(px(14.0))
                    .bg(rgb(0x101624))
                    .border_1()
                    .border_color(rgb(0xff4fa3))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(rgb(0xffd0ea))
                            .child("Confirm - nothing posts until you say yes"),
                    )
                    .child(media_block)
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x93a4c3))
                            .child("Nothing is posted automatically. Post now posts in-app (no browser). Open in browser is optional and only happens if you click it."),
                    )
                    .children(part_rows)
                    .child(div().text_size(px(13.0)).child(format!(
                        "{weighted} weighted characters Â· {} part(s)",
                        parts.len()
                    )))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x9ad7ff))
                            .child(url),
                    )
                    .child({
                        let cancel_active = self.ui_focus == UiFocus::ConfirmCancel;
                        let copy_active = self.ui_focus == UiFocus::ConfirmCopy;
                        let next_active = self.ui_focus == UiFocus::ConfirmNext;
                        let browser_active = self.ui_focus == UiFocus::ConfirmBrowser;
                        div()
                            .flex()
                            .gap(px(8.0))
                            .flex_wrap()
                            .child(x_button!(
                                "x-cancel",
                                "Cancel",
                                cancel_active,
                                cx.listener(|this, _, _, cx| {
                                    this.confirm = false;
                                    this.focus_compose_editor();
                                    this.pending_focus = true;
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                            .child(x_button!(
                                "x-copy",
                                "Copy",
                                copy_active,
                                cx.listener(|this, _, _, cx| {
                                    this.ui_focus = UiFocus::ConfirmCopy;
                                    this.copy_current_part(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                            .child(x_button!(
                                "x-next-post",
                                if self.posting { "Posting…" } else { "Post now" },
                                next_active,
                                cx.listener(|this, _, _, cx| {
                                    this.ui_focus = UiFocus::ConfirmNext;
                                    // In-app post. Never opens a browser.
                                    this.post_now_async(cx);
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                            .child(x_button!(
                                "x-open-browser",
                                "Open in browser ↗",
                                browser_active,
                                cx.listener(|this, _, _, cx| {
                                    this.ui_focus = UiFocus::ConfirmBrowser;
                                    // The ONLY panel action that may open Chrome.
                                    this.open_current_intent();
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                    }),
            )
    }

    fn copy_current_part(&mut self, cx: &mut Context<Self>) {
        let char_limit = termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
            .char_limit_or_fallback();
        let parts = split_thread_limited(&self.compose, char_limit);
        let text = if parts.is_empty() {
            self.compose.clone()
        } else {
            let index = self.intent_index.min(parts.len() - 1);
            parts[index].clone()
        };
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
        self.notice = Some("Copied this part.".into());
    }

    /// Post IN-APP via the active real publisher (OpenCLI `--window background`,
    /// official API, etc.). Never opens Chrome/x.com. The web-intent publisher is
    /// deliberately NOT in this chain — browser posting is the explicit button only.
    fn post_now_async(&mut self, cx: &mut Context<Self>) {
        if self.posting {
            self.notice = Some("Already posting — wait for it to finish.".into());
            cx.notify();
            return;
        }
        if !self.live {
            self.notice =
                Some("Preview/mock never posts. Open the real Termy X panel to post.".into());
            cx.notify();
            return;
        }
        if let Err(error) = termy_x::publish::validate_media_paths(&self.media) {
            self.notice = Some(error);
            cx.notify();
            return;
        }
        let text = self.compose.trim().to_string();
        if text.is_empty() {
            self.notice = Some("Type something before Publish.".into());
            cx.notify();
            return;
        }
        let media = self.media.clone();
        self.posting = true;
        self.notice = Some(if media.is_empty() {
            "Posting in-app via your X session… no browser will open.".into()
        } else {
            format!(
                "Posting in-app with {} image(s) via your X session… no browser will open.",
                media.len()
            )
        });
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let service = XPanel::service_free();
                    let publishers = service.publishers;
                    let Some(publisher) =
                        termy_x::publish::first_available(publishers.as_slice())
                    else {
                        return Err(
                            "No in-app posting path found. Connect X first (x connect),                              or use the explicit 'Open in browser' button."
                                .to_string(),
                        );
                    };
                    // A publisher that cannot upload must say so, not fake it:
                    // OpenCLI supports --images; twitter-cli/official reject media.
                    let char_limit =
                        termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
                            .char_limit_or_fallback();
                    match termy_x::publish::confirm_and_publish_with_media(
                        &text,
                        None,
                        termy_x::publish::ConfirmDecision::Yes,
                        false,
                        publisher,
                        char_limit,
                        &media,
                    ) {
                        termy_x::publish::GateOutcome::Sent { receipt, .. } => Ok(receipt),
                        termy_x::publish::GateOutcome::Rejected { plan } => Err(plan
                            .session
                            .unwrap_or_else(|| "posting was rejected".to_string())),
                        termy_x::publish::GateOutcome::DryRun { .. } => {
                            Err("unexpected dry-run".to_string())
                        }
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.posting = false;
                match result {
                    Ok(receipt) => {
                        panel.confirm = false;
                        panel.media.clear();
                        panel.compose.clear();
                        panel.caret = 0;
                        panel.sel_anchor = None;
                        panel.notice = Some(format!(
                            "Posted in-app — no browser opened. {receipt}"
                        ));
                        panel.focus_compose_editor();
                        panel.pending_focus = true;
                    }
                    Err(error) => {
                        // Stay on confirm so the user can retry or explicitly
                        // choose 'Open in browser'. Nothing auto-opens.
                        panel.notice = Some(format!(
                            "Could not post in-app: {error} Nothing was posted; no browser opened."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// EXPLICIT browser open only — call this solely from the labelled
    /// 'Open in browser' button. Opens only the highlighted part.
    /// Preview and mock never open a browser. Debounced so repeated
    /// clicks/Enter cannot reopen Chrome in a burst.
    fn open_current_intent(&mut self) {
        if let Some(at) = self.last_browser_open
            && at.elapsed() < Duration::from_secs(5)
        {
            self.notice = Some("Browser already opened just now — not reopening it.".into());
            return;
        }
        self.last_browser_open = Some(Instant::now());
        let char_limit = termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
            .char_limit_or_fallback();
        let parts = split_thread_limited(&self.compose, char_limit);
        let parts = if parts.is_empty() {
            let trimmed = self.compose.trim().to_string();
            if trimmed.is_empty() {
                self.notice = Some("Nothing to post.".into());
                return;
            }
            vec![trimmed]
        } else {
            parts
        };
        let index = self.intent_index.min(parts.len() - 1);
        if self.intent_urls.len() != parts.len() {
            self.intent_urls = termy_x::publish::intent_urls(&parts, None);
        }
        let url = self.intent_urls.get(index).cloned().unwrap_or_default();
        if self.live {
            let open_url = url;
            std::thread::spawn(move || {
                let _ = termy_x::io::Opener::open(&termy_x::io::SystemOpener, &open_url);
            });
            self.notice = Some(if self.media.is_empty() {
                format!(
                    "Opened part {} of {} in the browser. You still click Post there.",
                    index + 1,
                    parts.len()
                )
            } else {
                format!(
                    "Opened part {} of {} in the browser. Attach your {} image(s) in the browser compose box, then click Post there.",
                    index + 1,
                    parts.len(),
                    self.media.len()
                )
            });
        } else {
            self.notice = Some(format!(
                "Held part {} of {}. Preview does not open a browser.",
                index + 1,
                parts.len()
            ));
        }
        if index + 1 < parts.len() {
            self.intent_index = index + 1;
        } else {
            // Stay on confirm so Jeremy can Cancel after the last part.
        }
    }
}

fn card(title: String, body: String) -> gpui_kit::AnyElement {
    div()
        .p(px(12.0))
        .rounded(px(12.0))
        .bg(tint(0x0c1422, 225))
        .border_1()
        .border_color(tint(0x7af6ff, 40))
        .hover(|style| style.bg(tint(0x122038, 245)).border_color(rgb(0x7af6ff)))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(0xd7fbff))
                .child(title),
        )
        .child(
            div()
                .mt(px(4.0))
                .text_size(px(13.0))
                .text_color(rgb(0xe7eefc))
                .child(body),
        )
        .into_any_element()
}

fn post_card(post: &Post) -> gpui_kit::AnyElement {
    card(
        format!("@{} Â· {} likes", post.author_handle, post.likes),
        post.text.clone(),
    )
}

fn hint(text: String) -> gpui_kit::AnyElement {
    div()
        .text_size(px(12.0))
        .text_color(rgb(0xf0a8d0))
        .child(text)
        .into_any_element()
}

fn empty_state(title: &str, detail: impl Into<String>) -> gpui_kit::AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .p(px(16.0))
        .rounded_xl()
        .border_1()
        .border_color(rgb(0x2f7bff))
        .bg(tint(0x0b1220, 0xcc))
        .child(
            div()
                .text_size(px(16.0))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(rgb(0x7af6ff))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(rgb(0xc5d2ea))
                .child(detail.into()),
        )
        .into_any_element()
}

fn tint(color: u32, alpha: u8) -> gpui_kit::Rgba {
    rgba((color << 8) | u32::from(alpha))
}

#[cfg(test)]
mod edit_keys_tests {
    use super::{InputTarget, XSection, edit_keys_owned};

    #[test]
    fn reply_and_search_own_paste_but_terminal_states_do_not() {
        // Inline reply open (any non-Search section): owns edit chords.
        assert!(edit_keys_owned(
            false,
            false,
            InputTarget::Reply,
            XSection::Timeline,
            true,
            false,
            false
        ));
        // Search field on the Search tab: owns edit chords.
        assert!(edit_keys_owned(
            false,
            false,
            InputTarget::Search,
            XSection::Search,
            false,
            false,
            false
        ));
        // Reply target with no reply open: must NOT steal terminal paste.
        assert!(!edit_keys_owned(
            false,
            false,
            InputTarget::Reply,
            XSection::Timeline,
            false,
            false,
            false
        ));
        // Search target off the Search tab: must NOT steal terminal paste.
        assert!(!edit_keys_owned(
            false,
            false,
            InputTarget::Search,
            XSection::Timeline,
            false,
            false,
            false
        ));
        // Confirm modal owns the keyboard itself; edit actions stay out.
        assert!(!edit_keys_owned(
            true,
            false,
            InputTarget::Reply,
            XSection::Timeline,
            true,
            false,
            false
        ));
        // Compose keeps its original, stricter gate.
        assert!(edit_keys_owned(
            false,
            false,
            InputTarget::Compose,
            XSection::Compose,
            false,
            true,
            true
        ));
        assert!(!edit_keys_owned(
            false,
            false,
            InputTarget::Compose,
            XSection::Compose,
            false,
            false,
            true
        ));
        // Bearer-key entry is a text field too.
        assert!(edit_keys_owned(
            false,
            true,
            InputTarget::Compose,
            XSection::Timeline,
            false,
            false,
            false
        ));
    }
}
