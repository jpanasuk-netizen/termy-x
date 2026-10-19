//! Termy X side panel. The geometric swift is an original SVG, drawn at
//! `background_opacity` behind lists and at full strength on the empty splash.

pub mod art;
mod media_fit;
mod scroll_math;
mod player;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::collections::HashMap;

use gpui_kit::{
    Animation, AnimationExt, Context, FocusHandle, Image, ImageSource, InteractiveElement,
    IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement,
    Render, RenderImage, ScrollHandle, ScrollWheelEvent, SharedString,
    ScrollDelta, StatefulInteractiveElement, Styled,
    StyledImage, Window, div, img, point, px, rgb, rgba,
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
use termy_x::model::{Draft, MediaItem, MediaKind, Post, Profile, ResearchHit, Tone, Trend};
use termy_x::providers::{MockProvider, TimelineFeed, XProvider};
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
    Quote,
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
        // Quote shares the reply's gate: caller passes reply||quote open.
        InputTarget::Quote => reply_open,
    }
}

/// The two simple end-caret text fields (Search query, inline reply).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextField {
    Search,
    Reply,
    Quote,
}

/// Which post a pending image-picker result belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickTarget {
    Compose,
    Reply,
    Quote,
}

#[derive(Clone, PartialEq)]
enum ConnState {
    Checking,
    Connected { handle: String, via: String },
    NotConnected { reason: String, next: String },
}

/// Remote tweet-media thumbnail state. Downloads/decode happen on the
/// background executor (prefetch at fetch-apply time); render only reads
/// this cache, so scrolling/typing never blocks on the network or decode.
enum MediaState {
    Loading,
    Ready(CachedImage),
    Failed,
}

#[derive(Clone)]
struct CachedImage {
    image: Arc<RenderImage>,
    width: u32,
    height: u32,
}
impl CachedImage {
    fn aspect(&self) -> Option<f32> {
        if self.width > 0 && self.height > 0 { Some(self.width as f32 / self.height as f32) } else { None }
    }
}

/// What the in-app viewer (lightbox / embedded video) is showing.
#[derive(Clone)]
struct Viewer {
    kind: MediaKind,
    url: String,
    poster: Option<String>,
    post_id: String,
}

pub struct XPanel {
    focus: FocusHandle,
    /// Char limit resolved once at startup. Loading x.toml from disk on
    /// every render (and again per reply box) was pure waste on the hot path.
    char_limit: usize,
    media_cache: HashMap<String, MediaState>,
    viewer: Option<Viewer>,
    video_player: Option<player::VideoPlayer>,
    player_pending: bool,
    player_paused: bool,
    player_muted: bool,
    player_error: Option<String>,
    #[cfg(target_os = "windows")]
    host_hwnd: isize,
    /// Last window viewport width (logical px), for seek-bar hit math.
    viewport_w: f32,
    /// Wall-clock label of the last manual ↻ refresh, shown in status.
    last_refresh: Option<String>,
    reply_opened_at: Option<Instant>,
    reply_key_logged: bool,
    last_render_log: Option<Instant>,
    section: XSection,
    posts: Vec<Post>,
    /// Following home feed (kept separate so each feed keeps its data).
    following_posts: Vec<Post>,
    timeline_feed: TimelineFeed,
    /// Shared scroll state for the tweet list (wheel, middle-click
    /// autoscroll and the draggable scrollbar all drive this one handle).
    list_scroll: ScrollHandle,
    autoscroll: Option<AutoScroll>,
    /// Bumped on every autoscroll engage/disengage so stale loops exit.
    autoscroll_gen: u64,
    /// Window handle + scale captured at engage for global cursor polls.
    autoscroll_hwnd: isize,
    autoscroll_scale: f32,
    /// Active scrollbar-thumb drag (start mouse Y, start offset, metrics).
    sb_drag: Option<SbDrag>,
    /// Saved list offset per feed, restored on feed switch.
    feed_offset: [f32; 2],
    /// How deep each feed is loaded (OpenCLI limit last requested).
    feed_depth: [usize; 2],
    /// One-shot (append, limit) override consumed by the next fetch.
    next_fetch: Option<(bool, usize)>,
    /// A Load-more (append) fetch is in flight: the button shows the
    /// breathing bird and cannot be re-clicked.
    append_inflight: bool,
    /// Per feed: the source returned no older tweets, so Load more is
    /// done (button reads "No more older tweets" and disables).
    feed_exhausted: [bool; 2],
    /// Manual refresh asked to land at the top when its content applies
    /// (pull-to-refresh semantics: refresh = newest first).
    pending_top: bool,
    /// Per-feed bottom cursor from the cursor-paginated timeline path.
    feed_cursor: [Option<String>; 2],
    /// Per-feed: the loaded list's depth is backed by `feed_cursor`, so
    /// Load more can continue strictly older instead of re-fetching.
    feed_cursor_ok: [bool; 2],
    /// Tweets this session liked / retweeted from the panel (payloads
    /// carry no favorited/retweeted flag, so state starts unknown and
    /// only reflects clicks made here; counts move locally with them).
    liked_ids: Vec<String>,
    retweeted_ids: Vec<String>,
    /// Post ids with a like/retweet/quote call in flight (buttons for
    /// these are inert until the call lands).
    engage_pending: Vec<String>,
    /// Inline quote composer state (quote = retweet with your own words).
    quote_post_id: Option<String>,
    quote_author: String,
    quote_text: String,
    quote_media: Vec<std::path::PathBuf>,
    quote_posting: bool,
    /// Poster URLs waiting for background decode (drained in batches).
    media_queue: std::collections::VecDeque<String>,
    media_draining: bool,
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
    /// Keyboard / Tab focus target for neon chrome.
    ui_focus: UiFocus,
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
/// Deep timeline: one OpenCLI call per feed returns this many unique
/// tweets (verified live; no pagination flag exists or is needed).
const TIMELINE_LIMIT: usize = 160;
/// Posts fetched up front per feed (Jeremy's depth ask: land at 420
/// real posts, then Load more pages older from there).
const TIMELINE_INITIAL: usize = 420;
/// Load more grows the OpenCLI limit by one batch (+160) at a time.
/// The real end is data-driven (a fetch that returns no new tweets);
/// this is only a high perf safety ceiling.
const TIMELINE_MAX: usize = 1600;

/// Next Load-more limit for a feed loaded to `current`, or None at cap.
fn next_timeline_limit(current: usize) -> Option<usize> {
    if current >= TIMELINE_MAX {
        None
    } else {
        Some((current + TIMELINE_LIMIT).min(TIMELINE_MAX))
    }
}

/// Scrollbar thumb drag state captured on mouse-down.
#[derive(Clone, Copy)]
struct SbDrag {
    start_y: f32,
    start_offset_y: f32,
    viewport_h: f32,
    max_y: f32,
}

/// Middle-click autoscroll state (list-relative coordinates): the press
/// anchor plus the latest in-window mouse Y. While engaged, a timer loop
/// polls the GLOBAL cursor position (Chrome keeps autoscrolling when the
/// pointer leaves the window) and scrolls at proportional velocity.
#[derive(Clone, Copy)]
struct AutoScroll {
    anchor_x: f32,
    anchor_y: f32,
    mouse_y: f32,
}

#[derive(Clone, Copy, Default)]
struct SectionTimes {
    timeline: Option<Instant>,
    timeline_following: Option<Instant>,
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
        feed: TimelineFeed,
        /// Load-more fetch: append only unseen ids behind the loaded list.
        append: bool,
        /// The OpenCLI limit this batch was requested with.
        requested: usize,
        /// Bottom cursor for the next older page (cursor path only).
        next_cursor: Option<String>,
        /// True when `next_cursor` came from the cursor-paginated path.
        cursor_ok: bool,
        /// Live fetch failed: keep the loaded list, surface the error,
        /// never substitute fixture posts behind the user's back.
        failed: bool,
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
        let char_limit = config.char_limit_or_fallback();
        let mut panel = Self {
            focus: cx.focus_handle(),
            char_limit,
            media_cache: HashMap::new(),
            viewer: None,
            video_player: None,
            player_pending: false,
            player_paused: false,
            player_muted: true,
            player_error: None,
            #[cfg(target_os = "windows")]
            host_hwnd: 0,
            viewport_w: 0.0,
            last_refresh: None,
            reply_opened_at: None,
            reply_key_logged: false,
            last_render_log: None,
            section: XSection::Timeline,
            posts: Vec::new(),
            following_posts: Vec::new(),
            timeline_feed: TimelineFeed::ForYou,
            list_scroll: ScrollHandle::new(),
            autoscroll: None,
            autoscroll_gen: 0,
            autoscroll_hwnd: 0,
            autoscroll_scale: 1.0,
            sb_drag: None,
            feed_offset: [0.0, 0.0],
            feed_depth: [TIMELINE_INITIAL, TIMELINE_INITIAL],
            next_fetch: None,
            append_inflight: false,
            feed_exhausted: [false, false],
            pending_top: false,
            feed_cursor: [None, None],
            feed_cursor_ok: [false, false],
            liked_ids: Vec::new(),
            retweeted_ids: Vec::new(),
            engage_pending: Vec::new(),
            quote_post_id: None,
            quote_author: String::new(),
            quote_text: String::new(),
            quote_media: Vec::new(),
            quote_posting: false,
            media_queue: std::collections::VecDeque::new(),
            media_draining: false,
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
            ui_focus: UiFocus::Compose,
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
        panel
    }

    pub fn preview(scene: &str, cx: &mut Context<Self>) -> Self {
        let mut panel = Self::new(cx, true);
        panel.background_art = true;
        panel.load_scene(scene, cx);
        panel
    }

    /// \u{21bb} button: re-fetch the CURRENT tab on the existing silent
    /// background path. Single-flight (schedule_fetch queues/guards),
    /// never wipes a Compose draft, gives instant status feedback.
    fn refresh_current(&mut self, cx: &mut Context<Self>) {
        let section = self.section;
        if section == XSection::Compose {
            self.notice = Some("Compose draft kept \u{b7} refresh does nothing here.".into());
            cx.notify();
            return;
        }
        if self.fetching.iter().any(|f| *f) {
            self.status = "already refreshing \u{2014} one X session call at a time".into();
            cx.notify();
            return;
        }
        // Refresh means "newest again": depth resets to one batch.
        if section == XSection::Timeline {
            let idx = Self::feed_index(self.timeline_feed);
            self.feed_depth[idx] = TIMELINE_INITIAL;
            self.feed_exhausted[idx] = false;
            self.feed_cursor[idx] = None;
            self.feed_cursor_ok[idx] = false;
        }
        // Bypass the 30s freshness cache for this tab only.
        match section {
            XSection::Timeline => match self.timeline_feed {
                TimelineFeed::ForYou => self.fetched_at.timeline = None,
                TimelineFeed::Following => self.fetched_at.timeline_following = None,
            },
            XSection::Search => self.fetched_at.search = None,
            XSection::Trends => self.fetched_at.trends = None,
            XSection::Lookup => self.fetched_at.lookup = None,
            XSection::Research => self.fetched_at.research = None,
            XSection::Compose => {}
        }
        self.last_refresh = Some(local_hms());
        self.pending_top = true;
        self.loading = true;
        self.status = format!("refreshing {}\u{2026} (silent, background)", section.label());
        if section == XSection::Search && self.searched_query.is_empty() {
            self.loading = false;
            self.status = "search \u{b7} type a query and press Enter first \u{b7} then \u{21bb} re-runs it".into();
            cx.notify();
            return;
        }
        self.schedule_fetch(section, cx);
        cx.notify();
    }

    /// Tab / command open. Must return in microseconds: swap local state only,
    /// never call OpenCLI / twitter-cli on the UI thread.
    pub fn open_section(&mut self, section: XSection, cx: &mut Context<Self>) {
        let started = Instant::now();
        self.close_viewer();
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

    /// Load more (Timeline only): silently fetch the next older batch by
    /// re-requesting a larger OpenCLI limit and appending only unseen
    /// tweet ids behind the loaded list. Single-flight: while anything
    /// is fetching, the click just notes it; calls never stack. The
    /// scroll offset is never touched, so the user stays put.
    fn load_more_timeline(&mut self, cx: &mut Context<Self>) {
        if self.section != XSection::Timeline || self.feed_posts().is_empty() {
            return;
        }
        if self.loading || self.fetching.iter().any(|f| *f) {
            self.status =
                "already loading \u{2014} one X session call at a time".into();
            cx.notify();
            return;
        }
        let feed = self.timeline_feed;
        let idx = Self::feed_index(feed);
        if self.feed_exhausted[idx] {
            self.status = format!(
                "no more older tweets \u{b7} {} shown",
                self.feed_posts().len()
            );
            cx.notify();
            return;
        }
        if self.feed_posts().len() >= TIMELINE_MAX {
            self.status = format!(
                "{} shown \u{b7} safety limit ({TIMELINE_MAX})",
                self.feed_posts().len()
            );
            cx.notify();
            return;
        }
        // Cursor-backed list: fetch just the next older batch from the
        // stored bottom cursor. Legacy-backed list (cursor path was
        // unavailable at load): establish the chain by walking from the
        // top past everything already loaded, once.
        let next = if self.feed_cursor_ok[idx] {
            TIMELINE_LIMIT
        } else {
            (self.feed_depth[idx] + TIMELINE_LIMIT).min(TIMELINE_MAX)
        };
        self.next_fetch = Some((true, next));
        self.append_inflight = true;
        self.loading = true;
        self.status = format!("loading older {} tweets\u{2026}", feed.label());
        self.schedule_fetch(XSection::Timeline, cx);
        cx.notify();
    }

    /// The Load more row rendered after the last timeline card.
    fn load_more_row(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let idx = Self::feed_index(self.timeline_feed);
        if self.feed_exhausted[idx] && !self.append_inflight {
            return hint(format!(
                "No more older tweets \u{b7} {} shown \u{b7} \u{21bb} refreshes to the newest {}",
                self.feed_posts().len(),
                TIMELINE_INITIAL
            ));
        }
        if self.feed_posts().len() >= TIMELINE_MAX {
            return hint(format!(
                "{} shown \u{b7} safety limit ({TIMELINE_MAX}) \u{b7} \u{21bb} refreshes to the newest {TIMELINE_INITIAL}",
                self.feed_posts().len()
            ));
        }
        if self.append_inflight {
            // In-flight: breathing bird + label, clicks guarded off by
            // load_more_timeline's single-flight check as well.
            return div()
                .id("x-load-more-loading")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(tint(0x7af6ff, 55))
                .bg(rgb(0x1636c9))
                .text_size(px(12.0))
                .text_color(rgb(0xf8fbff))
                .child(breathing_bird())
                .child("Loading older tweets\u{2026}")
                .into_any_element();
        }
        let label = format!(
            "Load more \u{b7} older tweets ({} shown)",
            self.feed_posts().len()
        );
        x_button!(
            "x-load-more",
            label,
            cx.listener(|this, _, _, cx| this.load_more_timeline(cx))
        )
        .into_any_element()
    }

    fn feed_posts(&self) -> &Vec<Post> {
        match self.timeline_feed {
            TimelineFeed::ForYou => &self.posts,
            TimelineFeed::Following => &self.following_posts,
        }
    }

    fn feed_index(feed: TimelineFeed) -> usize {
        match feed {
            TimelineFeed::ForYou => 0,
            TimelineFeed::Following => 1,
        }
    }

    /// Switch the Timeline between X's For you and Following home feeds.
    /// Silent background fetch on the existing single-flight path; each
    /// feed keeps its own posts and scroll offset.
    fn set_timeline_feed(&mut self, feed: TimelineFeed, cx: &mut Context<Self>) {
        if feed == self.timeline_feed {
            return;
        }
        let old = Self::feed_index(self.timeline_feed);
        self.feed_offset[old] = self.list_scroll.offset().y.into();
        self.timeline_feed = feed;
        self.autoscroll = None;
        if self.reply_post_id.is_some() {
            self.close_reply();
        }
        let saved = self.feed_offset[Self::feed_index(feed)];
        self.list_scroll
            .set_offset(point(px(0.0), px(saved)));
        if self.is_fresh(XSection::Timeline) {
            self.loading = false;
            self.status = format!("{} \u{b7} cached (<30s)", feed.label());
        } else {
            self.loading = true;
            self.status = format!("loading {} timeline...", feed.label());
            self.schedule_fetch(XSection::Timeline, cx);
        }
        cx.notify();
    }

    fn is_fresh(&self, section: XSection) -> bool {
        let at = match section {
            XSection::Timeline => match self.timeline_feed {
                TimelineFeed::ForYou => self.fetched_at.timeline,
                TimelineFeed::Following => self.fetched_at.timeline_following,
            },
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
            XSection::Timeline => !self.feed_posts().is_empty(),
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
        let feed = self.timeline_feed;
        let (append, limit) = self.next_fetch.take().unwrap_or((
            false,
            self.feed_depth[Self::feed_index(feed)],
        ));
        // Continuation fetches resume from the stored bottom cursor;
        // fresh loads (refresh / first open) always start at the top.
        let fetch_cursor = if section == XSection::Timeline && append {
            self.feed_cursor[Self::feed_index(feed)].clone()
        } else {
            None
        };

        cx.spawn(async move |this, cx| {
            let payload = cx
                .background_executor()
                .spawn(async move {
                    fetch_section(section, query, lookup, live, feed, append, limit, fetch_cursor)
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.fetching[slot] = false;
                if panel.inflight[slot] != seq {
                    return;
                }
                panel.loading = false;
                panel.apply_fetch(section, payload);
                panel.prefetch_media(cx);
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
            FetchPayload::Timeline {
                posts,
                status,
                feed,
                append,
                requested: _,
                next_cursor,
                cursor_ok,
                failed,
            } => {
                let idx = Self::feed_index(feed);
                self.append_inflight = false;
                if failed {
                    // Live Load-more failed (rate limit, dead session):
                    // keep every loaded tweet, keep the cursor, and say
                    // so. Never swap in fixture posts, never mark the
                    // feed exhausted for a transient error.
                    self.status = status;
                } else if append {
                    // Load more: existing order untouched, only unseen
                    // ids extend the list downward. Scroll is not moved.
                    let (total, added) = match feed {
                        TimelineFeed::ForYou => {
                            let before = self.posts.len();
                            self.posts = termy_x::providers::append_new_posts(
                                std::mem::take(&mut self.posts),
                                posts,
                            );
                            (self.posts.len(), self.posts.len() - before)
                        }
                        TimelineFeed::Following => {
                            let before = self.following_posts.len();
                            self.following_posts =
                                termy_x::providers::append_new_posts(
                                    std::mem::take(&mut self.following_posts),
                                    posts,
                                );
                            (
                                self.following_posts.len(),
                                self.following_posts.len() - before,
                            )
                        }
                    };
                    if cursor_ok {
                        self.feed_cursor[idx] = next_cursor.clone();
                        self.feed_cursor_ok[idx] = true;
                        if next_cursor.is_none() {
                            self.feed_exhausted[idx] = true;
                        }
                    } else if added == 0 {
                        self.feed_exhausted[idx] = true;
                    }
                    self.feed_depth[idx] = total;
                    self.status = if added == 0 {
                        format!("no older tweets came back \u{b7} {total} shown")
                    } else {
                        format!("+{added} older tweets \u{b7} {total} shown")
                    };
                    match feed {
                        TimelineFeed::ForYou => self.fetched_at.timeline = Some(now),
                        TimelineFeed::Following => {
                            self.fetched_at.timeline_following = Some(now)
                        }
                    }
                } else {
                    let landed = posts.len();
                    match feed {
                        TimelineFeed::ForYou => {
                            self.posts = posts;
                            self.fetched_at.timeline = Some(now);
                        }
                        TimelineFeed::Following => {
                            self.following_posts = posts;
                            self.fetched_at.timeline_following = Some(now);
                        }
                    }
                    self.feed_depth[idx] = landed;
                    if cursor_ok {
                        self.feed_cursor[idx] = next_cursor.clone();
                        self.feed_cursor_ok[idx] = true;
                        self.feed_exhausted[idx] = next_cursor.is_none();
                    }
                    self.status = status;
                }
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
        // Manual refresh lands on the newest post: jump the shared list
        // to the very top as the refreshed content applies. Refresh wins
        // over saved per-feed scroll positions; other fetch kinds
        // (switch, append) never set the flag, so they never jump.
        if self.pending_top {
            self.pending_top = false;
            if section == self.section {
                self.list_scroll.set_offset(point(px(0.0), px(0.0)));
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
                .child(
                    // Small "Switch" chip right beside the account line:
                    // same disconnect/switch flow, compact label + size.
                    div()
                        .id("x-disconnect")
                        .px(px(7.0))
                        .py(px(3.0))
                        .rounded(px(999.0))
                        .border_1()
                        .border_color(tint(0x7af6ff, 55))
                        .bg(tint(0x101826, 170))
                        .text_size(px(11.0))
                        .text_color(rgb(0xb7c3dc))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x1a3050)).border_color(rgb(0xff4fa3)).text_color(rgb(0xffd0ea)))
                        .child("Switch")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                this.disconnect_x(cx);
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
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
                    col = col.child(focus_glow(
                        "glow-bearer",
                        true,
                        false,
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .text_size(px(12.0))
                            .text_color(rgb(0x7af6ff))
                            .child(format!("Your key: {masked}"))
                            .child(caret_element("caret-bearer"))
                            .child(format!(
                                " ({} characters) \u{2014} Ctrl+V pastes, Enter saves, Esc cancels",
                                self.bearer_input.chars().count()
                            ))
                            .into_any_element()),
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
            if self.autoscroll.take().is_some() {
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if self.viewer.is_some() {
                self.close_viewer();
                self.notice = Some("Viewer closed \u{b7} nothing was posted.".into());
                cx.stop_propagation();
                cx.notify();
                return;
            }
            if self.quote_post_id.is_some() {
                self.close_quote();
                cx.stop_propagation();
                cx.notify();
                return;
            }
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

        if self.viewer.as_ref().is_some_and(|v| v.kind != MediaKind::Photo) {
            match key {
                "space" => { if let Some(pl) = &mut self.video_player { pl.toggle_pause(); self.player_paused = !self.player_paused; } }
                "left" => { if let Some(pl) = &mut self.video_player { pl.seek_relative(-10); } }
                "right" => { if let Some(pl) = &mut self.video_player { pl.seek_relative(10); } }
                "m" => { self.player_muted = !self.player_muted; if let Some(pl) = &mut self.video_player { pl.set_muted(self.player_muted); } }
                _ => {}
            }
            if matches!(key, "space" | "left" | "right" | "m") {
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }

        if self.input_target == InputTarget::Search && self.section == XSection::Search {
            self.on_text_input_key(event, TextField::Search, cx);
            return;
        }
        if self.input_target == InputTarget::Reply && self.reply_post_id.is_some() {
            self.on_text_input_key(event, TextField::Reply, cx);
            return;
        }
        if self.input_target == InputTarget::Quote && self.quote_post_id.is_some() {
            self.on_text_input_key(event, TextField::Quote, cx);
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
            self.reply_post_id.is_some() || self.quote_post_id.is_some(),
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
                InputTarget::Quote => self.quote_text.push_str(clean),
            }
        }
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
        self.mark_edit_chord();
    }

    fn mark_edit_chord(&mut self) {
        self.last_edit_chord_at = Some(Instant::now());
    }

    fn chord_already_applied(&self) -> bool {
        self.last_edit_chord_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(80))
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
            } else if field == TextField::Quote {
                self.submit_quote(cx);
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
                TextField::Quote => self.quote_text.push('\n'),
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
            TextField::Quote => &mut self.quote_text,
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
        if field == TextField::Reply && !self.reply_key_logged {
            self.reply_key_logged = true;
            if let Some(opened) = self.reply_opened_at {
                perf_log(&format!(
                    "reply first-key after {}ms",
                    opened.elapsed().as_millis()
                ));
            }
        }
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

    fn open_reply_parts(&mut self, post_id: &str, author_handle: &str) {
        self.autoscroll = None;
        self.close_viewer();
        self.quote_post_id = None;
        if self.reply_post_id.as_deref() != Some(post_id) {
            self.reply_text.clear();
            self.reply_media.clear();
        }
        self.reply_post_id = Some(post_id.to_string());
        self.reply_author = author_handle.to_string();
        self.input_target = InputTarget::Reply;
        self.pending_focus = true;
        self.reply_opened_at = Some(Instant::now());
        self.reply_key_logged = false;
        perf_log(&format!("open_reply id={post_id} (local only, no fetch)"));
        self.status = format!(
            "reply to @{} \u{b7} type below, then Post (or Ctrl+Enter)",
            author_handle
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

    /// Optimistically flip a like/retweet, then run the real OpenCLI
    /// engage in the background. Failure reverts the flip and says why;
    /// counts move locally with the flip, never via a re-fetch, so the
    /// scroll position cannot jump.
    fn toggle_engage(
        &mut self,
        action: termy_x::publish::EngageAction,
        id: &str,
        url: &str,
        author: &str,
        cx: &mut Context<Self>,
    ) {
        use termy_x::publish::EngageAction;
        let id = id.to_string();
        if self.engage_pending.iter().any(|p| p == &id) {
            return;
        }
        if !self.live {
            self.notice = Some("Preview/mock never likes or retweets.".into());
            cx.notify();
            return;
        }
        let url = url.to_string();
        let author = author.to_string();
        let is_like = matches!(action, EngageAction::Like | EngageAction::Unlike);
        let now_active = match action {
            EngageAction::Like | EngageAction::Retweet => true,
            _ => false,
        };
        self.set_engage_state(&id, is_like, now_active);
        self.engage_pending.push(id.clone());
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
                            "No in-app posting path found. Connect X first, then try again."
                                .to_string(),
                        );
                    };
                    publisher
                        .engage(action, &url, None, &[])
                        .map_err(|e| e)
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.engage_pending.retain(|p| p != &id);
                match result {
                    Ok(_) => {
                        panel.status = format!(
                            "{} @{}",
                            match action {
                                termy_x::publish::EngageAction::Like => "liked",
                                termy_x::publish::EngageAction::Unlike => "unliked",
                                termy_x::publish::EngageAction::Retweet => "retweeted",
                                termy_x::publish::EngageAction::Unretweet => "retweet removed from",
                                termy_x::publish::EngageAction::Quote => "quoted",
                            },
                            author
                        );
                    }
                    Err(error) => {
                        // Revert the optimistic flip.
                        panel.set_engage_state(&id, is_like, !now_active);
                        panel.notice = Some(format!(
                            "That did not go through: {error} Nothing changed on X."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Flip local liked/retweeted state + the visible count for one post
    /// in every loaded list (no re-fetch, no scroll movement).
    fn set_engage_state(&mut self, id: &str, is_like: bool, active: bool) {
        let ids = if is_like {
            &mut self.liked_ids
        } else {
            &mut self.retweeted_ids
        };
        let present = ids.iter().any(|p| p == id);
        if active && !present {
            ids.push(id.to_string());
        } else if !active && present {
            ids.retain(|p| p != id);
        } else {
            return;
        }
        let delta: i64 = if active { 1 } else { -1 };
        for list in [
            &mut self.posts,
            &mut self.following_posts,
            &mut self.search_posts,
        ] {
            for post in list.iter_mut() {
                if post.id == id {
                    if is_like {
                        post.likes = (post.likes as i64 + delta).max(0) as u64;
                    } else {
                        post.reposts = (post.reposts as i64 + delta).max(0) as u64;
                    }
                }
            }
        }
    }

    fn open_quote(&mut self, post_id: &str, author_handle: &str) {
        self.autoscroll = None;
        self.close_viewer();
        self.reply_post_id = None;
        if self.quote_post_id.as_deref() != Some(post_id) {
            self.quote_text.clear();
            self.quote_media.clear();
        }
        self.quote_post_id = Some(post_id.to_string());
        self.quote_author = author_handle.to_string();
        self.input_target = InputTarget::Quote;
        self.pending_focus = true;
        self.status = format!(
            "quote @{} \u{b7} your words on their tweet, then Post",
            author_handle
        );
    }

    fn close_quote(&mut self) {
        self.quote_post_id = None;
        self.quote_author.clear();
        self.quote_text.clear();
        self.quote_media.clear();
        self.input_target = if self.section == XSection::Search {
            InputTarget::Search
        } else {
            InputTarget::Compose
        };
        self.status = "quote closed \u{b7} nothing was posted".into();
    }

    /// Post the quote (retweet with your own words) via OpenCLI
    /// `twitter quote <url> <text>` in the silent background session.
    fn submit_quote(&mut self, cx: &mut Context<Self>) {
        if self.quote_posting {
            self.notice = Some("Already sending that quote \u{2014} wait for it.".into());
            cx.notify();
            return;
        }
        let Some(target_id) = self.quote_post_id.clone() else {
            return;
        };
        let text = self.quote_text.trim().to_string();
        if text.is_empty() {
            self.notice = Some("Write your quote first \u{2014} a quote is your own words on their tweet.".into());
            cx.notify();
            return;
        }
        if !self.live {
            self.notice = Some("Preview/mock never posts a quote.".into());
            cx.notify();
            return;
        }
        // Target URL from the loaded post (any list), like reply uses ids.
        let target_post = self
            .posts
            .iter()
            .chain(self.following_posts.iter())
            .chain(self.search_posts.iter())
            .find(|post| post.id == target_id);
        let Some(target_post) = target_post else {
            self.notice = Some("That tweet is no longer loaded \u{2014} nothing was posted.".into());
            cx.notify();
            return;
        };
        let url = if target_post.url.is_empty() {
            format!(
                "https://x.com/{}/status/{}",
                target_post.author_handle, target_post.id
            )
        } else {
            target_post.url.clone()
        };
        if let Err(error) = termy_x::publish::validate_media_paths(&self.quote_media) {
            self.notice = Some(error);
            cx.notify();
            return;
        }
        let media = self.quote_media.clone();
        let author = self.quote_author.clone();
        self.quote_posting = true;
        self.notice = Some(format!(
            "Quoting @{} in-app, silently in the background\u{2026} no browser will open.",
            author
        ));
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
                            "No in-app posting path found. Connect X first, then try again."
                                .to_string(),
                        );
                    };
                    publisher.engage(
                        termy_x::publish::EngageAction::Quote,
                        &url,
                        Some(&text),
                        &media,
                    )
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.quote_posting = false;
                match result {
                    Ok(_) => {
                        if !panel.retweeted_ids.contains(&target_id) {
                            panel.retweeted_ids.push(target_id.clone());
                        }
                        panel.quote_post_id = None;
                        panel.quote_text.clear();
                        panel.quote_media.clear();
                        panel.input_target = if panel.section == XSection::Search {
                            InputTarget::Search
                        } else {
                            InputTarget::Compose
                        };
                        panel.notice = Some(format!(
                            "Quoted @{} \u{b7} posted in-app, no browser opened.",
                            author
                        ));
                        panel.status = format!("quoted @{}", author);
                    }
                    Err(error) => {
                        panel.notice = Some(format!(
                            "Could not quote: {error} Nothing was posted; no browser opened."
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
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
    }

    fn backspace(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.compose.replace_range(a..b, "");
            self.caret = a;
            self.clear_selection();
            return;
        }
        if self.caret == 0 {
            return;
        }
        let prev = prev_boundary(&self.compose, self.caret);
        self.compose.replace_range(prev..self.caret, "");
        self.caret = prev;
    }

    fn delete_forward(&mut self) {
        if let Some((a, b)) = self.selection_range() {
            self.compose.replace_range(a..b, "");
            self.caret = a;
            self.clear_selection();
            return;
        }
        if self.caret >= self.compose.len() {
            return;
        }
        let next = next_boundary(&self.compose, self.caret);
        self.compose.replace_range(self.caret..next, "");
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

fn fetch_section(
    section: XSection,
    query: String,
    lookup: String,
    live: bool,
    feed: TimelineFeed,
    append: bool,
    limit: usize,
    cursor: Option<String>,
) -> FetchPayload {
    if live {
        let service = XPanel::service_free();
        match section {
            XSection::Timeline => {
                // Cursor-paginated path first: refresh walks from the
                // top, Load more continues strictly older from the
                // stored bottom cursor \u{2014} never a bigger re-fetch.
                match service.timeline_page(feed, cursor.as_deref(), limit) {
                    Ok(served) => {
                        return FetchPayload::Timeline {
                            posts: served.value.posts,
                            status: live_status(&served.status),
                            feed,
                            append,
                            requested: limit,
                            next_cursor: served.value.next_cursor,
                            cursor_ok: true,
                            failed: false,
                        };
                    }
                    Err(error) => {
                        if append {
                            // Honest failure: no fixture substitution,
                            // no fake "no more tweets".
                            return FetchPayload::Timeline {
                                posts: Vec::new(),
                                status: format!(
                                    "could not load older tweets \u{2014} {error}"
                                ),
                                feed,
                                append,
                                requested: limit,
                                next_cursor: None,
                                cursor_ok: false,
                                failed: true,
                            };
                        }
                        // Fresh load: legacy one-shot fetch still works
                        // when the cursor session cannot start.
                        if let Ok(served) = service.timeline_feed(feed, limit) {
                            return FetchPayload::Timeline {
                                posts: served.value,
                                status: live_status(&served.status),
                                feed,
                                append,
                                requested: limit,
                                next_cursor: None,
                                cursor_ok: false,
                                failed: false,
                            };
                        }
                    }
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
                .timeline_feed(feed, limit)
                .map(|served| served.value)
                .unwrap_or_default(),
            feed,
            status: "mock Â· fixture data Â· no X API credits".into(),
            append,
            requested: limit,
            next_cursor: None,
            cursor_ok: false,
            failed: false,
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
            feed,
            next_cursor: None,
            cursor_ok: false,
            failed: false,
            append: false,
            requested: TIMELINE_LIMIT,
        },
    }
}

impl Drop for XPanel {
    fn drop(&mut self) {
        // Never leave an embedded player process/HWND behind Termy.
        self.stop_video_player();
    }
}

impl Render for XPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_video_player(window, cx);
        if self.pending_focus {
            self.focus.focus(window, cx);
            self.pending_focus = false;
        }
        let render_started = Instant::now();
        let char_limit = self.char_limit;
        // Off the Compose tab the thread math is never displayed; skip it
        // so Timeline/Search keystrokes and blink frames stay cheap.
        let (count, parts) = if self.section == XSection::Compose || self.confirm || self.fill_window {
            let count = weighted_len_limited(&self.compose, char_limit);
            let parts = split_thread_limited(&self.compose, char_limit);
            (count, parts)
        } else {
            (
                weighted_len_limited("", char_limit),
                Vec::new(),
            )
        };
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

        let root = root.child(self.backdrop(art_opacity))
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
                    .children((columns == 1 && self.section == XSection::Timeline).then(|| {
                        self.feed_switch(cx)
                    }))
                    .child(if columns == 1 {
                        div()
                            .relative()
                            .flex_1()
                            .min_h(px(0.0))
                            .child(
                                div()
                                    .id("x-scroll")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.list_scroll)
                                    .flex()
                                    .flex_col()
                                    .gap(px(8.0))
                                    .children(self.body(count.weighted, count.limit, &parts, cx))
                                    .on_scroll_wheel(cx.listener(
                                        |this, event: &ScrollWheelEvent, window, cx| {
                                            // GPUI applies the raw wheel delta
                                            // natively in this same dispatch;
                                            // adjust the offset so the net is
                                            // tamed_wheel_delta (Chrome-like
                                            // reading speed, well under one
                                            // tweet per notch). Additive, so
                                            // listener order does not matter.
                                            let native: f32 = event
                                                .delta
                                                .pixel_delta(window.line_height())
                                                .y
                                                .into();
                                            // Exactly Chrome: line wheels
                                            // scroll ~100px per notch;
                                            // precise (hi-res/touchpad)
                                            // deltas pass through 1:1, and
                                            // a per-event cap stops driver
                                            // multipliers throwing pages.
                                            let desired = match &event.delta {
                                                ScrollDelta::Lines(p) => {
                                                    scroll_math::chrome_line_delta(p.y)
                                                }
                                                ScrollDelta::Pixels(p) => {
                                                    f32::from(p.y).clamp(-110.0, 110.0)
                                                }
                                            };
                                            let correction = desired - native;
                                            if correction.abs() > 0.5 {
                                                let cur = this.list_scroll.offset();
                                                let max_y: f32 =
                                                    this.list_scroll.max_offset().y.into();
                                                let new_y = (f32::from(cur.y) + correction)
                                                    .clamp(-max_y.max(0.0), 0.0);
                                                this.list_scroll
                                                    .set_offset(point(cur.x, px(new_y)));
                                            }
                                            cx.notify();
                                        },
                                    ))
                                    .on_mouse_down(
                                        MouseButton::Middle,
                                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                            this.toggle_autoscroll(event, window, cx);
                                            cx.stop_propagation();
                                            cx.notify();
                                        }),
                                    ),
                            )
                            .children(self.scrollbar_overlay(cx))
                            .children(self.autoscroll.map(|anchor| {
                                div()
                                    .absolute()
                                    .left(px(anchor.anchor_x - 14.0))
                                    .top(px(anchor.anchor_y - 14.0))
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .rounded_full()
                                    .border_1()
                                    .border_color(rgb(0x7af6ff))
                                    .bg(rgba(0x0f1c33ee))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(15.0))
                                    .text_color(rgb(0x7af6ff))
                                    .child("\u{2195}")
                            }))
                            .on_mouse_move(cx.listener(
                                |this, event: &MouseMoveEvent, _, cx| {
                                    if this.scrollbar_drag_move(event)
                                        || this.autoscroll_move(event)
                                    {
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    if this.sb_drag.take().is_some() {
                                        cx.notify();
                                    }
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, _| {
                                    // Any plain click disengages autoscroll;
                                    // the click itself still works normally.
                                    this.autoscroll = None;
                                }),
                            )
                            .into_any_element()
                    } else {
                        self.dashboard(columns, count.weighted, count.limit, &parts, cx)
                    }),
            )
            .children(self.confirm.then(|| {
                self.confirm_dialog(count.weighted, count.limit, &parts, dialog_width, cx)
            }))
            .children(self.viewer_overlay(cx));
        let elapsed = render_started.elapsed();
        if elapsed.as_millis() > 12
            && self.last_render_log.is_none_or(|t| t.elapsed() > Duration::from_secs(1))
        {
            self.last_render_log = Some(Instant::now());
            perf_log(&format!("render slow {}ms section={:?}", elapsed.as_millis(), self.section));
        }
        root
    }
}

/// Download + decode one remote image on a background thread (5 MB cap).
/// Local copy of the release-notes loader: x_panel lives in the lib crate,
/// which cannot reach the binary crate's `ui` module.
fn fetch_image_rgba(url: &str) -> Result<image::RgbaImage, String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("only http(s) images".into());
    }
    use std::io::Read;
    let response = ureq::get(url)
        .set("User-Agent", "TermyX/1.0")
        .call()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(5_000_001)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 5_000_000 {
        return Err("image too large".into());
    }
    image::load_from_memory(&bytes)
        .map(|d| d.to_rgba8())
        .map_err(|e| e.to_string())
}

/// Global cursor Y in list-relative logical coordinates, read straight
/// from the OS so autoscroll keeps tracking (Chrome-style) even when the
/// pointer is outside the Termy window. `None` off Windows or on failure,
/// in which case callers fall back to the last in-window mouse position.
#[cfg(target_os = "windows")]
fn global_cursor_list_y(hwnd: isize, scale: f32, list_top: f32) -> Option<f32> {
    #[repr(C)]
    struct Pt {
        x: i32,
        y: i32,
    }
    unsafe extern "system" {
        fn GetCursorPos(pt: *mut Pt) -> i32;
        fn ScreenToClient(hwnd: isize, pt: *mut Pt) -> i32;
    }
    if hwnd == 0 {
        return None;
    }
    let mut pt = Pt { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut pt) == 0 || ScreenToClient(hwnd, &mut pt) == 0 {
            return None;
        }
    }
    Some(pt.y as f32 / scale.max(0.5) - list_top)
}

#[cfg(not(target_os = "windows"))]
fn global_cursor_list_y(_hwnd: isize, _scale: f32, _list_top: f32) -> Option<f32> {
    None
}

/// Local wall-clock HH:MM:SS for the refresh status label.
fn local_hms() -> String {
    #[cfg(target_os = "windows")]
    {
        #[repr(C)]
        struct SystemTime { year: u16, month: u16, dow: u16, day: u16, hour: u16, minute: u16, second: u16, ms: u16 }
        unsafe extern "system" { fn GetLocalTime(t: *mut SystemTime); }
        let mut st = SystemTime { year: 0, month: 0, dow: 0, day: 0, hour: 0, minute: 0, second: 0, ms: 0 };
        unsafe { GetLocalTime(&mut st) };
        return format!("{:02}:{:02}:{:02}", st.hour, st.minute, st.second);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        format!("{:02}:{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60, secs % 60)
    }
}

/// Slow "breath" cycle for loading/caret pulses: eased 0 -> 1 -> 0 over
/// one period, so elements gently solidify/grow then faint/shrink instead
/// of flashing on/off. GPUI drives the frames (max ~9 fps); no entity
/// notify loop, so a breathing caret can never starve typing.
fn breath_animation(cycle_ms: u64) -> Animation {
    Animation::new(Duration::from_millis(cycle_ms))
        .repeat()
        .with_max_fps(9.0)
        .with_easing(|t| 0.5 - 0.5 * (t * std::f32::consts::TAU).cos())
}

/// Breathing text caret shared by every panel editor. Always rendered
/// (fixed cell, zero layout shift); only its opacity breathes.
fn caret_element(id: &'static str) -> gpui_kit::AnyElement {
    div()
        .flex_none()
        .child("|")
        .with_animation(id, breath_animation(2_300), |el, v| {
            el.opacity(0.12 + 0.88 * v)
        })
        .into_any_element()
}

/// Red<->blue glow colour at breath phase `v` (0 = warm red, 1 = blue).
fn glow_rgba(v: f32) -> gpui_kit::Rgba {
    let mix = |a: f32, b: f32| (a + (b - a) * v).round() as u32;
    let hex = (mix(255.0, 45.0) << 16) | (mix(45.0, 110.0) << 8) | mix(80.0, 255.0);
    rgba((hex << 8) | 130)
}

/// Shared focused-editor glow: a thin ring around the field whose
/// background breathes red<->blue while the field owns the keyboard.
/// One implementation for every writing surface (Search, Reply, Quote,
/// Compose, bearer-key entry). The ring is always mounted (transparent
/// when unfocused) so focus changes never shift layout; only a tiny
/// element animates, at the caret's 9fps breath cap.
fn focus_glow(
    id: &'static str,
    focused: bool,
    grow: bool,
    child: gpui_kit::AnyElement,
) -> gpui_kit::AnyElement {
    let mut ring = div().p(px(2.0)).rounded(px(11.0)).child(child);
    if grow {
        ring = ring.flex_1();
    }
    if focused {
        ring.with_animation(id, breath_animation(2_800), |el, v| {
            el.bg(glow_rgba(v))
        })
        .into_any_element()
    } else {
        ring.into_any_element()
    }
}

/// Small breathing bird shown while any tab loads (shared status line):
/// scales up + solidifies on the inhale, shrinks + fades on the exhale.
/// Exists only while `loading`, so the animation stops itself.
fn breathing_bird() -> gpui_kit::AnyElement {
    let Some(image) = art::cached_image() else {
        return div().w(px(30.0)).h(px(18.0)).into_any_element();
    };
    let source: ImageSource = image.into();
    // Red glow layer: the pre-rasterized warm-red variant of the same
    // bird, crossfaded in only at the inhale peak (opacity driven by
    // the same breath cycle). Cheap: opacity/scale on the tiny glyph,
    // both rasters cached once per process, nothing decoded per frame.
    let red_layer = art::cached_red_image().map(|red| {
        let red_source: ImageSource = red.into();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(img(red_source).size_full().object_fit(ObjectFit::Contain))
            .with_animation("x-loading-bird-red", breath_animation(2_800), |el, v| {
                el.opacity(v * v * 0.95)
            })
    });
    div()
        .w(px(30.0))
        .h(px(18.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .relative()
                .child(img(source).size_full().object_fit(ObjectFit::Contain))
                .children(red_layer)
                .with_animation("x-loading-bird", breath_animation(2_800), |el, v| {
                    let w = 19.0 + 5.0 * v;
                    el.w(px(w)).h(px(w * 0.625)).opacity(0.30 + 0.70 * v)
                }),
        )
        .into_any_element()
}

/// Append one line to %TEMP%/termy-x-perf.txt (slow renders + reply timing).
fn perf_log(line: &str) {
    use std::io::Write;
    let path = std::env::temp_dir().join("termy-x-perf.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
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
        let refresh_btn = {
            let refreshing = self.loading;
        div()
            .id("x-refresh")
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(8.0))
            .text_size(px(13.0))
            .cursor_pointer()
            .border_1()
            .border_color(if refreshing { rgb(0xff4fa3) } else { tint(0x7af6ff, 45) })
            .bg(if refreshing { rgb(0x2a0f3a) } else { tint(0x101826, 170) })
            .text_color(if refreshing { rgb(0xffd0ea) } else { rgb(0x7af6ff) })
            .hover(|style| style.bg(rgb(0x1a3050)).border_color(rgb(0x7af6ff)))
            .child(if refreshing { "\u{21bb}\u{2026}" } else { "\u{21bb}" })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.refresh_current(cx);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
        };
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
        .child(refresh_btn)
    }

    fn status_line(&self) -> impl IntoElement {
        // Shared loading label for every tab: while `loading`, the little
        // bird breathes next to the status text; it unmounts (and stops)
        // the moment loading finishes.
        let label = if self.loading {
            self.status.clone()
        } else {
            match &self.last_refresh { Some(at) => format!("{} \u{b7} updated {at}", self.status), None => self.status.clone() }
        };
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_size(px(11.0))
            .text_color(rgb(0x93a4c3));
        if self.loading {
            row = row.child(breathing_bird());
        }
        row.child(label)
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
        row = row.child(self.results_column(columns == 2, cx));
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

    fn results_column(&self, include_side: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut rows = Vec::new();
        rows.push(hint(format!("Search Â· {}", self.query)));
        let search = if self.search_posts.is_empty() {
            &self.posts
        } else {
            &self.search_posts
        };
        for post in search.iter().take(3) {
            rows.push(self.tweet_card(post, cx));
        }
        rows.push(hint("Timeline".into()));
        for post in &self.posts {
            rows.push(self.tweet_card(post, cx));
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
                if !self.feed_posts().is_empty() {
                    rows.push(hint(format!(
                        "{} \u{b7} {} shown",
                        self.timeline_feed.label(),
                        self.feed_posts().len()
                    )));
                }
                if self.feed_posts().is_empty() {
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
                for post in self.feed_posts() {
                    rows.push(self.tweet_card(post, cx));
                }
                if !self.feed_posts().is_empty() {
                    rows.push(self.load_more_row(cx));
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

    /// Start background downloads for every poster/photo in the current
    /// lists. Called once per fetch-apply, never from render or typing.
    /// Queue poster decodes instead of firing them all at once: with a
    /// 160-post timeline an all-at-once burst would flood the network and
    /// the render loop. The drain loop starts a few at a time on the
    /// background executor; scrolling and typing never wait on it.
    fn prefetch_media(&mut self, cx: &mut Context<Self>) {
        let mut urls: Vec<String> = Vec::new();
        let collect = |posts: &[Post], urls: &mut Vec<String>| {
            for post in posts {
                for media in &post.media {
                    if let Some(poster) = &media.poster
                        && !poster.is_empty()
                    {
                        urls.push(poster.clone());
                    }
                }
            }
        };
        collect(&self.posts, &mut urls);
        collect(&self.following_posts, &mut urls);
        collect(&self.search_posts, &mut urls);
        if let Some(profile) = &self.profile {
            collect(&profile.posts, &mut urls);
        }
        urls.sort();
        urls.dedup();
        for url in urls {
            if !self.media_cache.contains_key(&url) && !self.media_queue.contains(&url) {
                self.media_queue.push_back(url);
            }
        }
        self.drain_media_queue(cx);
    }

    fn drain_media_queue(&mut self, cx: &mut Context<Self>) {
        if self.media_draining {
            return;
        }
        self.media_draining = true;
        cx.spawn(async move |this, cx| {
            loop {
                let started = this
                    .update(cx, |panel, cx| {
                        let mut started = 0;
                        while started < 6 {
                            let Some(url) = panel.media_queue.pop_front() else {
                                break;
                            };
                            if panel.media_cache.contains_key(&url) {
                                continue;
                            }
                            panel.spawn_media_fetch(url, cx);
                            started += 1;
                        }
                        if started == 0 {
                            panel.media_draining = false;
                        }
                        started
                    })
                    .unwrap_or(0);
                if started == 0 {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
            }
        })
        .detach();
    }

    fn spawn_media_fetch(&mut self, url: String, cx: &mut Context<Self>) {
        if self.media_cache.contains_key(&url) {
            return;
        }
        self.media_cache.insert(url.clone(), MediaState::Loading);
        let fetch_url = url.clone();
        let bg = cx.background_executor().spawn(async move {
            fetch_image_rgba(&fetch_url)
        });
        cx.spawn(async move |this, cx| {
            let result = bg.await;
            let _ = this.update(cx, |panel, cx| {
                let state = match result {
                    Ok(buffer) => {
                        let (width, height) = (buffer.width(), buffer.height());
                        MediaState::Ready(CachedImage {
                            image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
                            width,
                            height,
                        })
                    }
                    Err(_) => MediaState::Failed,
                };
                panel.media_cache.insert(url.clone(), state);
                cx.notify();
            });
        })
        .detach();
    }

    fn media_aspect(&self, media: &MediaItem) -> Option<f32> {
        if let Some(poster) = &media.poster
            && let Some(MediaState::Ready(cached)) = self.media_cache.get(poster) {
                return cached.aspect();
            }
        if let Some(MediaState::Ready(cached)) = self.media_cache.get(&media.url) {
            return cached.aspect();
        }
        media.aspect()
    }

    fn open_viewer(&mut self, media: &MediaItem, post_id: &str, cx: &mut Context<Self>) {
        self.autoscroll = None;
        self.stop_video_player();
        self.viewer = Some(Viewer {
            kind: media.kind,
            url: media.url.clone(),
            poster: media.poster.clone(),
            post_id: post_id.to_string(),
        });
        self.player_error = None;
        self.player_paused = false;
        self.player_muted = true;
        if media.kind == MediaKind::Photo {
            let url = media.url.clone();
            self.spawn_media_fetch(url, cx);
            self.notice = Some("Photo opened in Termy — Esc or ✕ to close.".into());
        } else {
            self.player_pending = true;
            self.notice = Some("Playing video inside Termy (muted) — Esc or ✕ to stop.".into());
        }
        self.pending_focus = true;
    }

    fn stop_video_player(&mut self) {
        self.player_pending = false;
        self.player_paused = false;
        if let Some(player) = self.video_player.take() {
            player.stop();
        }
    }

    /// Every viewer exit path funnels here (Esc, \u{2715}, tab switch,
    /// mpv EOF/death via the sync loop): player killed, viewer cleared,
    /// and the "Playing video \u{2014} Esc or \u{2715} to stop" notice is
    /// cleared with it so the status can never stick after playback ends.
    fn close_viewer(&mut self) {
        self.stop_video_player();
        self.viewer = None;
        self.player_error = None;
        if matches!(
            self.notice.as_deref(),
            Some(n) if n.starts_with("Playing video") || n.starts_with("Photo opened")
        ) {
            self.notice = None;
        }
        self.pending_focus = true;
    }
    /// Reserved media area for a tweet, sized from the media's real
    /// aspect ratio (decoded poster first, then provider/URL dims, then
    /// 16:9) and contain-fitted: the WHOLE photo/poster is always shown,
    /// letterboxed on the dark card, never cropped or squashed. Height is
    /// reserved before decode, so cards do not jump. Clicking media (only)
    /// opens the in-app viewer and never expands a reply or a browser.
    fn media_box(&self, post: &Post, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let media = post.media.first()?;
        let kind = media.kind;
        let poster = media.poster.clone().unwrap_or_default();
        let viewer_media = media.clone();
        let post_id = post.id.clone();
        let aspect = self.media_aspect(media);
        // Panel content width: 480 panel - 32 panel padding - 24 card padding.
        let box_h = media_fit::media_box_height(424.0, aspect);
        // Shrink-to-fit: the preview element itself is only as big as the
        // whole frame inside the capped media area (portrait media renders
        // small and centred-by-width, never a full-width tall box).
        let (fit_w, fit_h) = media_fit::contain_size(424.0, box_h, aspect);
        let extra = post.media.len().saturating_sub(1);
        let badge = match kind {
            MediaKind::Video => "\u{25b6} VIDEO \u{b7} tap to play in Termy",
            MediaKind::Gif => "\u{25b6} GIF \u{b7} tap to play in Termy",
            MediaKind::Photo => "\u{1f50d} tap to view",
        };
        let mut area = div()
            .id(SharedString::from(format!("x-media-{}", post.id)))
            .relative()
            .w(px(fit_w))
            .h(px(box_h))
            .flex_none()
            .mt(px(8.0))
            .rounded(px(10.0))
            .overflow_hidden()
            .bg(tint(0x0f1c33, 245))
            .border_1()
            .border_color(tint(0x7af6ff, 55))
            .cursor_pointer();
        match self.media_cache.get(&poster) {
            Some(MediaState::Ready(cached)) => {
                let source: ImageSource = ImageSource::Render(cached.image.clone());
                area = area.child(img(source).w(px(fit_w)).h(px(fit_h)).object_fit(ObjectFit::Contain));
            }
            Some(MediaState::Failed) => {
                area = area.child(
                    div().p(px(10.0)).text_size(px(12.0)).text_color(rgb(0x93a4c3))
                        .child(if kind == MediaKind::Photo { "Photo (could not load preview)" } else { "Video (could not load preview)" }),
                );
            }
            _ => {
                area = area.child(
                    div().p(px(10.0)).text_size(px(12.0)).text_color(rgb(0x93a4c3))
                        .child(if kind == MediaKind::Photo { "Loading photo\u{2026}" } else { "Loading video preview\u{2026}" }),
                );
            }
        }
        {
            let label = if extra > 0 { format!("{badge}  +{extra} more") } else { badge.to_string() };
            area = area.child(
                div()
                    .absolute()
                    .bottom(px(8.0))
                    .left(px(8.0))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(999.0))
                    .bg(tint(0x070b14, 220))
                    .text_size(px(11.0))
                    .text_color(rgb(0xe9fdff))
                    .child(label),
            );
        }
        area = area.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                if !this.live {
                    this.notice = Some("Preview does not open media.".into());
                } else {
                    this.open_viewer(&viewer_media, &post_id, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }),
        );
        Some(area.into_any_element())
    }


    /// Start/position the embedded mpv surface over the viewer's video
    /// area. Called from render only when a video viewer is open; the
    /// rect is derived from the window viewport (panel is the rightmost
    /// 480px, viewer header 56px + controls 72px), so the native surface
    /// tracks Termy moves/resizes and never changes Termy's geometry.
    fn sync_video_player(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(viewer) = self.viewer.clone() else { return };
        if viewer.kind == MediaKind::Photo { return; }
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = HasWindowHandle::window_handle(window)
                && let RawWindowHandle::Win32(win32) = handle.as_raw() {
                    self.host_hwnd = win32.hwnd.get() as isize;
                }
        }
        let viewport = window.viewport_size();
        let vw: f32 = viewport.width.into();
        self.viewport_w = vw;
        let vh: f32 = viewport.height.into();
        let panel_w = if self.fill_window { vw } else { 480.0_f32.min(vw) };
        let rect: player::Rect = (vw - panel_w, 56.0, panel_w, (vh - 160.0).max(80.0));
        let scale = window.scale_factor();
        if let Some(player) = &mut self.video_player {
            player.reposition(scale, rect);
        } else if self.player_pending {
            self.player_pending = false;
            if !self.live {
                self.player_error = Some("Preview does not play video.".into());
                return;
            }
            #[cfg(target_os = "windows")]
            match player::VideoPlayer::start(self.host_hwnd, scale, rect, &viewer.url) {
                Ok(player) => {
                    self.video_player = Some(player);
                    self.player_error = None;
                    // Keep the seek bar/time readout live: re-render ~2x/sec
                    // while the video viewer is open (cheap, panel-only).
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(Duration::from_millis(600)).await;
                            let alive = this.update(cx, |panel, cx| {
                                let open = panel.viewer.as_ref().is_some_and(|v| v.kind != MediaKind::Photo) && panel.video_player.is_some();
                                if !open { return false; }
                                // mpv died / its window was closed, or
                                // playback reached EOF: end the viewer
                                // cleanly instead of leaving a stuck
                                // "Playing video" status over a dead frame.
                                let (pos, dur) = panel.video_player.as_ref().map_or((0.0, 0.0), |pl| pl.progress());
                                let player_gone = panel.video_player.as_mut().is_some_and(|pl| !pl.is_alive());
                                let at_eof = dur > 0.0 && pos >= dur - 0.25;
                                if player_gone || at_eof {
                                    panel.close_viewer();
                                    panel.notice = Some("Video ended \u{b7} back to the timeline.".into());
                                    cx.notify();
                                    return false;
                                }
                                cx.notify();
                                true
                            }).unwrap_or(false);
                            if !alive { break; }
                        }
                    }).detach();
                }
                Err(e) => { self.player_error = Some(format!("In-app player unavailable: {e}")); }
            }
            #[cfg(not(target_os = "windows"))]
            { self.player_error = Some("In-app video needs the Windows mpv build.".into()); }
        }
    }

    /// Seek-bar click/drag at window-x `x`: fraction from the bar's known
    /// geometry (panel is the rightmost 480px, bar inset 12px each side).
    fn seek_bar_click(&mut self, x: f32, cx: &mut Context<Self>) {
        let panel_w = if self.fill_window { self.viewport_w } else { 480.0_f32.min(self.viewport_w) };
        let bar_x = self.viewport_w - panel_w + 12.0;
        let bar_w = (panel_w - 24.0).max(1.0);
        let (_, dur) = self.video_player.as_ref().map_or((0.0, 0.0), |pl| pl.progress());
        if dur > 0.0 {
            let target = f64::from(media_fit::seek_fraction(x, bar_x, bar_w)) * dur;
            if let Some(pl) = &mut self.video_player { pl.seek_absolute(target); }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn viewer_overlay(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let viewer = self.viewer.as_ref()?;
        let is_video = viewer.kind != MediaKind::Photo;
        let title = match viewer.kind {
            MediaKind::Photo => "Photo \u{b7} in Termy",
            MediaKind::Video => "Video \u{b7} playing in Termy (muted)",
            MediaKind::Gif => "GIF \u{b7} playing in Termy (muted)",
        };
        // Middle: photo image (contain, whole) or the native video host area.
        let middle: gpui_kit::AnyElement = if is_video {
            let mut area = div().relative().w_full().h_full().bg(rgb(0x000000));
            if let Some(error) = &self.player_error {
                area = area.child(
                    div().absolute().top(px(12.0)).left(px(12.0)).right(px(12.0))
                        .p(px(10.0)).rounded(px(8.0)).bg(tint(0x2a0f1a, 235))
                        .text_size(px(12.0)).text_color(rgb(0xffd0ea))
                        .child(format!("{error} The poster is shown instead \u{2014} no browser was opened.")),
                );
            } else if self.video_player.is_none() {
                area = area.child(div().p(px(12.0)).text_size(px(12.0)).text_color(rgb(0x93a4c3)).child("Starting in-app player\u{2026}"));
            }
            area.into_any_element()
        } else {
            let key = if self.media_cache.contains_key(&viewer.url) { viewer.url.clone() } else { viewer.poster.clone().unwrap_or_else(|| viewer.url.clone()) };
            let mut area = div().relative().w_full().h_full().flex().items_center().justify_center()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_viewer();
                    cx.stop_propagation();
                    cx.notify();
                }));
            match self.media_cache.get(&key) {
                Some(MediaState::Ready(cached)) => {
                    let source: ImageSource = ImageSource::Render(cached.image.clone());
                    area = area.child(img(source).w_full().h_full().object_fit(ObjectFit::Contain));
                }
                Some(MediaState::Failed) => {
                    area = area.child(div().text_size(px(13.0)).text_color(rgb(0x93a4c3)).child("Photo could not load."));
                }
                _ => {
                    area = area.child(div().text_size(px(13.0)).text_color(rgb(0x93a4c3)).child("Loading photo\u{2026}"));
                }
            }
            area.into_any_element()
        };
        let mut controls = div().h(px(104.0)).flex_none().flex().flex_col().justify_center().gap(px(6.0)).px(px(12.0));
        if is_video {
            let (pos, dur) = self.video_player.as_ref().map_or((0.0, 0.0), |pl| pl.progress());
            let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let panel_w = if self.fill_window { self.viewport_w } else { 480.0_f32.min(self.viewport_w) };
            let bar_w = (panel_w - 24.0).max(1.0);
            controls = controls.child(
                div()
                    .id("x-viewer-seek")
                    .relative()
                    .w_full()
                    .h(px(22.0))
                    .flex_none()
                    .rounded(px(999.0))
                    .bg(tint(0x1a2a4a, 255))
                    .cursor_pointer()
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h_full()
                            .w(px(bar_w * frac))
                            .rounded(px(999.0))
                            .bg(rgb(0x7af6ff))
                            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                let x: f32 = event.position.x.into();
                                this.seek_bar_click(x, cx);
                            })),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(3.0))
                            .right(px(10.0))
                            .text_size(px(11.0))
                            .text_color(rgb(0xe9fdff))
                            .child(format!("{} / {}", media_fit::format_time(pos), media_fit::format_time(dur))),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                        let x: f32 = event.position.x.into();
                        this.seek_bar_click(x, cx);
                    }))
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            let x: f32 = event.position.x.into();
                            this.seek_bar_click(x, cx);
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, event: &MouseUpEvent, _, cx| {
                        // Drag release lands the seek even if move events
                        // were coalesced away.
                        let x: f32 = event.position.x.into();
                        this.seek_bar_click(x, cx);
                    })),
            );
            controls = controls.child(
                div().flex().items_center().gap(px(8.0))
                .child(x_button!("x-viewer-pause", if self.player_paused { "\u{25b6} Play" } else { "\u{23f8} Pause" }, cx.listener(|this, _, _, cx| {
                    if let Some(pl) = &mut this.video_player { pl.toggle_pause(); this.player_paused = !this.player_paused; }
                    cx.stop_propagation(); cx.notify();
                })))
                .child(x_button!("x-viewer-back", "-10s", cx.listener(|this, _, _, cx| {
                    if let Some(pl) = &mut this.video_player { pl.seek_relative(-10); }
                    cx.stop_propagation(); cx.notify();
                })))
                .child(x_button!("x-viewer-fwd", "+10s", cx.listener(|this, _, _, cx| {
                    if let Some(pl) = &mut this.video_player { pl.seek_relative(10); }
                    cx.stop_propagation(); cx.notify();
                })))
                .child(x_button!("x-viewer-mute", if self.player_muted { "Unmute" } else { "Mute" }, cx.listener(|this, _, _, cx| {
                    this.player_muted = !this.player_muted;
                    if let Some(pl) = &mut this.video_player { pl.set_muted(this.player_muted); }
                    cx.stop_propagation(); cx.notify();
                })))
                .child(div().text_size(px(11.0)).text_color(rgb(0x93a4c3)).child("Drag the bar to scrub \u{b7} Space pause \u{b7} Esc close")),
            );
        } else {
            controls = controls.child(div().text_size(px(11.0)).text_color(rgb(0x93a4c3)).child("Click image or backdrop, Esc, or \u{2715} to close \u{b7} whole photo, never cropped"));
        }
        let el = div()
            .id("x-viewer")
            .absolute().top_0().left_0().size_full()
            .flex().flex_col()
            .bg(tint(0x02040a, 242))
            .child(
                div().h(px(56.0)).flex_none().flex().items_center().justify_between().px(px(12.0))
                    .child(div().text_size(px(13.0)).text_color(rgb(0xe9fdff)).child(title))
                    .child(x_button!("x-viewer-close", "\u{2715} Close", cx.listener(|this, _, _, cx| {
                        this.close_viewer();
                        cx.stop_propagation(); cx.notify();
                    }))),
            )
            .child(div().flex_1().min_h(px(0.0)).child(middle))
            .child(controls)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, cx| {
                // Backdrop clicks are handled by the photo area / Close; keep
                // clicks in chrome from falling through to tweets underneath.
                let _ = this;
                cx.stop_propagation();
            }));
        Some(el.into_any_element())
    }

    /// One tweet in Timeline / Search / Trends. Clicking it expands an
    /// inline reply box right underneath; a sent reply marks it Replied.
    /// Thin draggable scrollbar overlaid on the list's right edge.
    /// Geometry comes from the shared scroll handle, so wheel scrolling,
    /// middle-click autoscroll and this thumb never disagree. Track
    /// clicks page up/down. Painted absolutely: zero layout width cost.
    fn scrollbar_overlay(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let bounds = self.list_scroll.bounds();
        let viewport_h: f32 = bounds.size.height.into();
        let max_y: f32 = self.list_scroll.max_offset().y.into();
        if viewport_h <= 1.0 || max_y <= 1.0 {
            return None;
        }
        let offset_y: f32 = self.list_scroll.offset().y.into();
        let (thumb_y, thumb_h) = scroll_math::thumb_geometry(viewport_h, max_y, offset_y);
        let dragging = self.sb_drag.is_some();
        let top_edge: f32 = bounds.origin.y.into();
        Some(
            div()
                .absolute()
                .top_0()
                .right_0()
                .w(px(12.0))
                .h_full()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        // Track click: page toward the click.
                        let rel_y: f32 = event.position.y.into();
                        let rel_y = rel_y - top_edge;
                        let page = viewport_h * 0.9;
                        let cur: f32 = this.list_scroll.offset().y.into();
                        let new = if rel_y < thumb_y {
                            (cur + page).min(0.0)
                        } else {
                            (cur - page).max(-max_y)
                        };
                        let cur_point = this.list_scroll.offset();
                        this.list_scroll.set_offset(point(cur_point.x, px(new)));
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .right(px(2.0))
                        .top(px(thumb_y))
                        .w(px(8.0))
                        .h(px(thumb_h))
                        .rounded(px(4.0))
                        .bg(if dragging {
                            rgb(0x7af6ff)
                        } else {
                            rgba(0x3b82c4cc)
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                let start_y: f32 = event.position.y.into();
                                this.sb_drag = Some(SbDrag {
                                    start_y,
                                    start_offset_y: offset_y,
                                    viewport_h,
                                    max_y,
                                });
                                this.autoscroll = None;
                                cx.stop_propagation();
                                cx.notify();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Mouse moved while dragging the scrollbar thumb.
    fn scrollbar_drag_move(&mut self, event: &MouseMoveEvent) -> bool {
        let Some(drag) = self.sb_drag else {
            return false;
        };
        // Hold-to-drag, Chrome-style: the drag exists only while the
        // left button is physically held. If the release happened
        // outside the panel (so no mouse-up was delivered here), the
        // first plain move ends the drag instead of latching it on.
        if event.pressed_button != Some(MouseButton::Left) {
            self.sb_drag = None;
            return false;
        }
        let mouse_y: f32 = event.position.y.into();
        let (start_thumb_y, _) =
            scroll_math::thumb_geometry(drag.viewport_h, drag.max_y, drag.start_offset_y);
        let new_offset = scroll_math::offset_for_thumb_y(
            drag.viewport_h,
            drag.max_y,
            start_thumb_y + (mouse_y - drag.start_y),
        );
        let current = self.list_scroll.offset();
        if (f32::from(current.y) - new_offset).abs() < 0.5 {
            return false;
        }
        self.list_scroll.set_offset(point(current.x, px(new_offset)));
        true
    }

    /// "For you" / "Following" toggle at the top of the Timeline tab.
    /// Both are real X home feeds via OpenCLI `--type`.
    fn feed_switch(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let feed = self.timeline_feed;
        div()
            .flex()
            .gap(px(6.0))
            .child(x_button!(
                "x-feed-for-you",
                "For you",
                feed == TimelineFeed::ForYou,
                cx.listener(|this, _, _, cx| {
                    this.set_timeline_feed(TimelineFeed::ForYou, cx);
                    cx.stop_propagation();
                })
            ))
            .child(x_button!(
                "x-feed-following",
                "Following",
                feed == TimelineFeed::Following,
                cx.listener(|this, _, _, cx| {
                    this.set_timeline_feed(TimelineFeed::Following, cx);
                    cx.stop_propagation();
                })
            ))
            .into_any_element()
    }

    /// Middle-click: engage autoscroll at this anchor, or disengage if
    /// already active. Never triggers reply/media (those are Left-only).
    /// Chrome semantics: engaging starts a loop that keeps scrolling
    /// while the cursor is held away from the anchor — no further mouse
    /// movement needed — tracking the GLOBAL cursor so leaving the
    /// window neither pauses nor disengages; only a click or Esc exits.
    fn toggle_autoscroll(
        &mut self,
        event: &MouseDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.autoscroll_gen += 1;
        if self.autoscroll.take().is_some() {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = HasWindowHandle::window_handle(window)
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                self.autoscroll_hwnd = win32.hwnd.get() as isize;
            }
            self.autoscroll_scale = window.scale_factor();
        }
        let bounds = self.list_scroll.bounds();
        let anchor_x: f32 = event.position.x.into();
        let anchor_y: f32 = event.position.y.into();
        let top: f32 = bounds.origin.y.into();
        let left: f32 = bounds.origin.x.into();
        self.autoscroll = Some(AutoScroll {
            anchor_x: anchor_x - left,
            anchor_y: anchor_y - top,
            mouse_y: anchor_y - top,
        });
        let agen = self.autoscroll_gen;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                let keep = this
                    .update(cx, |panel, cx| {
                        if panel.autoscroll_gen != agen {
                            return false;
                        }
                        let Some(anchor) = panel.autoscroll else {
                            return false;
                        };
                        let bounds = panel.list_scroll.bounds();
                        let top: f32 = bounds.origin.y.into();
                        let mouse_y = global_cursor_list_y(
                            panel.autoscroll_hwnd,
                            panel.autoscroll_scale,
                            top,
                        )
                        .unwrap_or(anchor.mouse_y);
                        let velocity = scroll_math::autoscroll_velocity(anchor.anchor_y, mouse_y);
                        if velocity.abs() > 0.0 {
                            let cur = panel.list_scroll.offset();
                            let max_y: f32 = panel.list_scroll.max_offset().y.into();
                            let new_y = (f32::from(cur.y) + velocity * 0.03)
                                .clamp(-max_y.max(0.0), 0.0);
                            if (f32::from(cur.y) - new_y).abs() > 0.01 {
                                panel.list_scroll.set_offset(point(cur.x, px(new_y)));
                                cx.notify();
                            }
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

    /// In-window mouse moved while autoscrolling: record the position as
    /// the fallback for platforms without global cursor reads. The
    /// engage loop does the actual scrolling; the anchor never moves.
    fn autoscroll_move(&mut self, event: &MouseMoveEvent) -> bool {
        let Some(anchor) = &mut self.autoscroll else {
            return false;
        };
        let bounds = self.list_scroll.bounds();
        let mouse_y: f32 = event.position.y.into();
        let top: f32 = bounds.origin.y.into();
        anchor.mouse_y = mouse_y - top;
        false
    }

    fn tweet_card(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let selected = self.reply_post_id.as_deref() == Some(post.id.as_str());
        let replied = self.replied_ids.iter().any(|id| id == &post.id);
        // Click handlers capture only the two small strings they need:
        // cloning the whole Post (text + media) for every card on every
        // render was measurable with a 160-post list.
        let click_id = post.id.clone();
        let click_author = post.author_handle.clone();
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
            .children(self.media_box(post, cx))
            .child(self.tweet_actions(post, cx))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    if this.reply_post_id.as_deref() == Some(click_id.as_str()) {
                        // Second click on the open tweet just refocuses it.
                        this.input_target = InputTarget::Reply;
                        this.pending_focus = true;
                    } else {
                        this.open_reply_parts(&click_id, &click_author);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        if selected {
            el = el.child(self.reply_box(post, cx));
        }
        if self.quote_post_id.as_deref() == Some(post.id.as_str()) {
            el = el.child(self.quote_box(post, cx));
        }
        el.into_any_element()
    }

    /// Like / Retweet / Quote row on every tweet card. Small, distinct
    /// click targets that stop propagation, so hitting one never opens
    /// the reply editor (and vice versa). Counts shown are the post's
    /// own payload counts, moved locally by optimistic flips.
    fn tweet_actions(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let id = post.id.clone();
        let url = if post.url.is_empty() {
            format!("https://x.com/{}/status/{}", post.author_handle, post.id)
        } else {
            post.url.clone()
        };
        let author = post.author_handle.clone();
        let liked = self.liked_ids.iter().any(|p| p == &post.id);
        let retweeted = self.retweeted_ids.iter().any(|p| p == &post.id);
        let pending = self.engage_pending.iter().any(|p| p == &post.id);
        let (like_id, rt_id) = (id.clone(), id.clone());
        let (like_url, rt_url) = (url.clone(), url.clone());
        let (like_author, rt_author) = (author.clone(), author.clone());
        let quote_id = post.id.clone();
        let quote_author = post.author_handle.clone();
        div()
            .mt(px(8.0))
            .flex()
            .gap(px(8.0))
            .flex_wrap()
            .child(x_button!(
                format!("x-like-{}", post.id),
                if pending {
                    "\u{2026}"
                } else if liked {
                    "Liked"
                } else {
                    "Like"
                },
                liked,
                cx.listener(move |this, _, _, cx| {
                    let action = if this.liked_ids.iter().any(|p| p == &like_id) {
                        termy_x::publish::EngageAction::Unlike
                    } else {
                        termy_x::publish::EngageAction::Like
                    };
                    this.toggle_engage(action, &like_id, &like_url, &like_author, cx);
                    cx.stop_propagation();
                })
            ))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x93a4c3))
                    .child(format!("{} likes \u{b7} {} retweets", post.likes, post.reposts)),
            )
            .child(x_button!(
                format!("x-rt-{}", post.id),
                if pending {
                    "\u{2026}"
                } else if retweeted {
                    "Retweeted"
                } else {
                    "Retweet"
                },
                retweeted,
                cx.listener(move |this, _, _, cx| {
                    let action = if this.retweeted_ids.iter().any(|p| p == &rt_id) {
                        termy_x::publish::EngageAction::Unretweet
                    } else {
                        termy_x::publish::EngageAction::Retweet
                    };
                    this.toggle_engage(action, &rt_id, &rt_url, &rt_author, cx);
                    cx.stop_propagation();
                })
            ))
            .child(x_button!(
                format!("x-quote-{}", post.id),
                "Quote",
                cx.listener(move |this, _, _, cx| {
                    this.open_quote(&quote_id, &quote_author);
                    cx.stop_propagation();
                    cx.notify();
                })
            ))
            .into_any_element()
    }

    /// Inline quote composer under a card: retweet with your own words.
    /// Same shape as the reply box (fixed height, breathing caret from
    /// the first focused frame, Post button, Esc/Cancel, 1 image).
    fn quote_box(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let char_limit = self.char_limit;
        let count = weighted_len_limited(&self.quote_text, char_limit);
        let focused = self.input_target == InputTarget::Quote;
        let editor = div()
            .id("x-quote-editor")
            .h(px(60.0))
            .flex_none()
            .overflow_hidden()
            .p(px(8.0))
            .rounded(px(8.0))
            .bg(tint(0x0c1422, 235))
            .border_1()
            .border_color(tint(0x7af6ff, 70))
            .cursor_pointer()
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(if self.quote_text.is_empty() {
                        rgb(0x93a4c3)
                    } else {
                        rgb(0xf8fbff)
                    })
                    .child(if self.quote_text.is_empty() {
                        if focused {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .child(caret_element("caret-quote"))
                                .child("Add your words\u{2026} (Ctrl+Enter posts the quote)")
                        } else {
                            div().child("Add your words\u{2026} (Ctrl+Enter posts the quote)")
                        }
                    } else if focused {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(self.quote_text.clone())
                            .child(caret_element("caret-quote"))
                    } else {
                        div().child(self.quote_text.clone())
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.input_target = InputTarget::Quote;
                    this.pending_focus = true;
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
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
                    this.input_target = InputTarget::Quote;
                    this.pending_focus = true;
                    // Must stop here: without this the click bubbles to
                    // the card's own handler, which clears the quote and
                    // opens a reply instead (clicking the quote box's
                    // padding/header used to turn it into a reply).
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x7af6ff))
                    .child(format!("Quote @{} \u{b7} retweet with your own words", post.author_handle)),
            )
            .child(focus_glow("glow-quote", focused, false, editor.into_any_element()))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if count.weighted > count.limit {
                        rgb(0xff5d8f)
                    } else {
                        rgb(0x9ad7ff)
                    })
                    .child(format!(
                        "{} / {} characters \u{b7} Ctrl+Enter or Post sends \u{b7} quotes attach 1 image",
                        count.weighted, count.limit
                    )),
            );
        if !self.quote_media.is_empty() {
            for (index, path) in self.quote_media.iter().enumerate() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("image")
                    .to_string();
                box_el = box_el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x7af6ff))
                                .child(name),
                        )
                        .child(x_button!(
                            format!("x-quote-media-remove-{index}"),
                            "\u{2715} Remove",
                            cx.listener(move |this, _, _, cx| {
                                if index < this.quote_media.len() {
                                    this.quote_media.remove(index);
                                }
                                cx.stop_propagation();
                                cx.notify();
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
                    "x-quote-attach",
                    "Attach image",
                    cx.listener(|this, _, _, cx| {
                        this.pick_target = PickTarget::Quote;
                        this.attach_images(cx);
                        cx.stop_propagation();
                    })
                ))
                .child(x_button!(
                    "x-quote-send",
                    if self.quote_posting {
                        "Posting\u{2026}"
                    } else {
                        "Post"
                    },
                    cx.listener(|this, _, _, cx| {
                        this.submit_quote(cx);
                        cx.stop_propagation();
                    })
                ))
                .child(x_button!(
                    "x-quote-cancel",
                    "Cancel",
                    cx.listener(|this, _, _, cx| {
                        this.close_quote();
                        cx.stop_propagation();
                        cx.notify();
                    })
                )),
        );
        box_el.into_any_element()
    }

    /// The inline reply editor shown under the selected tweet.
    fn reply_box(&self, post: &Post, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let char_limit = self.char_limit;
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
                    // Same bubble guard as the quote box: clicks inside
                    // the reply box must never re-trigger the card.
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x7af6ff))
                    .child(format!("Reply to @{}", post.author_handle)),
            )
            .child(focus_glow(
                "glow-reply",
                focused,
                false,
                div()
                    .id("x-reply-editor")
                    // Fixed height: the box never grows/shrinks as you type
                    // or as the caret blinks, so cards below never jump.
                    .h(px(60.0))
                    .flex_none()
                    .overflow_hidden()
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
                                if focused {
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .child(caret_element("caret-reply"))
                                        .child("Write your reply\u{2026} (Ctrl+Enter sends)")
                                } else {
                                    div().child("Write your reply\u{2026} (Ctrl+Enter sends)")
                                }
                            } else if focused {
                                // Breathing caret in a fixed cell: always
                                // present, only its opacity pulses, so the
                                // text width never changes between frames.
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .child(self.reply_text.clone())
                                    .child(caret_element("caret-reply"))
                            } else {
                                div().child(self.reply_text.clone())
                            }),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.input_target = InputTarget::Reply;
                            this.pending_focus = true;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                    .into_any_element(),
            ))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(if count.weighted > count.limit {
                        rgb(0xff5d8f)
                    } else {
                        rgb(0x9ad7ff)
                    })
                    .child(format!(
                        "{} / {} characters \u{b7} Ctrl+Enter or the Post button sends \u{b7} replies attach 1 image",
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
                                .object_fit(ObjectFit::Contain)
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
                        "Posting\u{2026}"
                    } else {
                        "Post"
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
                    .child(focus_glow(
                        "glow-search",
                        focused,
                        true,
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
                                        if focused {
                                            div()
                                                .flex()
                                                .flex_row()
                                                .items_center()
                                                .child(caret_element("caret-search"))
                                                .child("Type a query \u{2014} a word, #tag, or from:someone")
                                        } else {
                                            div().child("Type a query \u{2014} a word, #tag, or from:someone")
                                        }
                                    } else if focused {
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .child(self.search_input.clone())
                                            .child(caret_element("caret-search"))
                                    } else {
                                        div().child(self.search_input.clone())
                                    }),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                                    this.input_target = InputTarget::Search;
                                    this.pending_focus = true;
                                    cx.stop_propagation();
                                    cx.notify();
                                }))
                            .into_any_element(),
                    ))
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
            PickTarget::Quote => self.quote_media.len(),
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
            PickTarget::Quote => &mut self.quote_media,
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
                                        .object_fit(ObjectFit::Contain)
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
            .child(focus_glow(
                "glow-compose",
                editor_focused,
                false,
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
                        }))
                    .into_any_element(),
            ))
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
        let show_caret = focused;
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
            .with_animation("caret-compose", breath_animation(2_300), |el, v| {
                el.opacity(0.12 + 0.88 * v)
            })
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
    use super::next_timeline_limit;

    #[test]
    fn load_more_limit_grows_by_batch_to_cap() {
        assert_eq!(next_timeline_limit(160), Some(320));
        assert_eq!(next_timeline_limit(320), Some(480));
        assert_eq!(next_timeline_limit(1440), Some(1600));
        assert_eq!(next_timeline_limit(1600), None);
        assert_eq!(next_timeline_limit(9999), None);
    }

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
