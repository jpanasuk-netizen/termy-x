//! GPU-rendered Trends Desk window.
//!
//! The window is a normal GPUI surface, the same renderer Termy uses for the
//! rest of the desktop. OpenCLI still does the search on a background thread.
//! The GPU does not speed that up. `--fixture` draws labeled sample cards and
//! does not save them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{
    App, AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Window, WindowBounds, WindowKind, WindowOptions, div, point, px, rgb, size,
};
use termy_x::XConfig;
use termy_x::io::SystemRunner;
use termy_x::trends_desk::{
    ChangeTag, ColumnView, DeskSettings, DeskStore, GPU_NOTE, IMAGE_QUOTE_NOTE, InspectedPost, Nav,
    OpenCliDesk, OutcomeKind, Phase, RunOutcome, SessionState, SessionStatus, Surface, Topic,
    column_count_for_width, column_from_cache, column_from_outcome, column_loading, daily_due,
    filter_posts, fixture_column, prepare_draft, surface_after_nav, synthetic_fixture_posts,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct DeskOptions {
    pub fixture: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Focus,
    Filter,
    TopicName,
    TopicDesc,
    Draft,
    DailyAt,
}

struct ColumnUi {
    topic: Topic,
    view: ColumnView,
    cached: Option<termy_x::trends_desk::StoredReport>,
    tags: Vec<(String, ChangeTag)>,
}

pub struct TrendsDesk {
    focus_handle: FocusHandle,
    options: DeskOptions,
    config: XConfig,
    store: DeskStore,
    settings: DeskSettings,
    columns: Vec<ColumnUi>,
    surface: Surface,
    field: Option<Field>,
    focus_text: String,
    filter_text: String,
    topic_name: String,
    topic_desc: String,
    draft_text: String,
    draft_reply_to: Option<String>,
    draft_note: String,
    replies: Vec<InspectedPost>,
    reply_note: String,
    history: Vec<termy_x::trends_desk::StoredReport>,
    history_index: Option<usize>,
    session: SessionStatus,
    generation: u64,
    cancel: Arc<AtomicBool>,
    busy: bool,
    started: bool,
}

impl TrendsDesk {
    pub fn new(options: DeskOptions, cx: &mut Context<Self>) -> Self {
        let config = XConfig::load(&termy_x::config::default_config_dir());
        let store = DeskStore::new(&config.config_dir);
        let settings = store.load_settings();
        let history = store.all_reports();
        let columns = store
            .load_topics()
            .into_iter()
            .map(|topic| {
                let cached = store.latest(&topic.id);
                let view = if options.fixture {
                    fixture_column(&topic, fixture_posts_for(&topic))
                } else {
                    column_from_cache(cached.as_ref())
                };
                ColumnUi {
                    topic,
                    view,
                    cached,
                    tags: Vec::new(),
                }
            })
            .collect();
        Self {
            focus_handle: cx.focus_handle(),
            options,
            config,
            store,
            focus_text: settings.focus.clone(),
            settings,
            columns,
            surface: Surface::Columns,
            field: None,
            filter_text: String::new(),
            topic_name: String::new(),
            topic_desc: String::new(),
            draft_text: String::new(),
            draft_reply_to: None,
            draft_note: "Drafts stay on this machine until you publish from X compose.".into(),
            replies: Vec::new(),
            reply_note: "Inspect replies reads a thread. It does not post.".into(),
            history,
            history_index: None,
            session: SessionStatus {
                state: SessionState::ProbeFailed,
                label: "Checking OpenCLI…".into(),
                detail: GPU_NOTE.to_string(),
            },
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            busy: false,
            started: false,
        }
    }

    fn client(&self) -> OpenCliDesk {
        OpenCliDesk::new(&self.config, Arc::new(SystemRunner))
    }

    fn ensure_started(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        self.spawn_probe(cx);
        self.spawn_daily(cx);
    }

    fn spawn_probe(&mut self, cx: &mut Context<Self>) {
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async move {
                    OpenCliDesk::new(&config, Arc::new(SystemRunner)).probe(&AtomicBool::new(false))
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                desk.session = status;
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_daily(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                let _ = this.update(cx, |desk, cx| {
                    desk.maybe_daily(cx);
                });
            }
        })
        .detach();
    }

    fn maybe_daily(&mut self, cx: &mut Context<Self>) {
        if self.options.fixture
            || self.busy
            || !daily_due(termy_x::time::unix_now(), &self.settings)
        {
            return;
        }
        let (date, _, _) = termy_x::trends_desk::chicago_stamp(termy_x::time::unix_now());
        self.settings.daily_done = Some(date);
        let _ = self.store.save_settings(&self.settings);
        let ids: Vec<String> = self
            .columns
            .iter()
            .map(|column| column.topic.id.clone())
            .collect();
        self.start_run(&ids, cx);
    }

    fn start_run(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if self.options.fixture {
            self.draft_note = "Fixture mode does not call OpenCLI.".into();
            cx.notify();
            return;
        }
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.cancel.store(false, Ordering::Relaxed);
        self.busy = true;
        self.focus_text = self.settings.focus.clone();
        let focus = self.settings.focus.clone();
        let mut topics = Vec::new();
        for column in &mut self.columns {
            if !ids.iter().any(|id| id == &column.topic.id) {
                continue;
            }
            let query = termy_x::trends_desk::search_query(&column.topic.desc, &focus);
            column.view = column_loading(&query);
            topics.push(column.topic.clone());
        }
        cx.notify();
        let config = self.config.clone();
        let cancel = self.cancel.clone();
        let store_dir = self.config.config_dir.clone();
        cx.spawn(async move |this, cx| {
            let outcomes = cx
                .background_executor()
                .spawn(async move {
                    let desk = OpenCliDesk::new(&config, Arc::new(SystemRunner));
                    let mut outcomes = Vec::new();
                    for (index, topic) in topics.into_iter().enumerate() {
                        if cancel.load(Ordering::Relaxed) {
                            outcomes.push(termy_x::trends_desk::outcome_error(
                                &topic,
                                &focus,
                                "cancelled",
                            ));
                            continue;
                        }
                        if index > 0 {
                            paced_wait(&cancel);
                        }
                        outcomes.push(desk.run_topic(&topic, &focus, &cancel));
                    }
                    let store = DeskStore::new(&store_dir);
                    let now = termy_x::time::unix_now();
                    let mut saved = Vec::new();
                    for outcome in &outcomes {
                        if outcome.kind.writes_report() {
                            saved.push(store.write_fresh(outcome, now).ok());
                        } else {
                            saved.push(None);
                        }
                    }
                    (outcomes, saved)
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                if desk.generation != generation {
                    return;
                }
                desk.busy = false;
                desk.apply_outcomes(outcomes.0, outcomes.1);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_outcomes(
        &mut self,
        outcomes: Vec<RunOutcome>,
        saved: Vec<Option<termy_x::trends_desk::StoredReport>>,
    ) {
        for (outcome, written) in outcomes.into_iter().zip(saved) {
            let Some(column) = self
                .columns
                .iter_mut()
                .find(|column| column.topic.id == outcome.topic_id)
            else {
                continue;
            };
            let previous_urls: Vec<String> = column
                .cached
                .as_ref()
                .map(|report| report.posts.iter().map(|post| post.url.clone()).collect())
                .unwrap_or_default();
            column.view = column_from_outcome(&outcome, column.cached.as_ref());
            if outcome.kind == OutcomeKind::Fresh {
                let urls: Vec<String> = outcome.posts.iter().map(|post| post.url.clone()).collect();
                column.tags = termy_x::trends_desk::diff_urls(&previous_urls, &urls);
                if let Some(report) = written {
                    column.cached = Some(report.clone());
                    self.history.insert(0, report);
                }
            }
        }
    }

    fn cancel_run(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation = self.generation.saturating_add(1);
        self.busy = false;
        for column in &mut self.columns {
            if column.view.phase == Phase::Loading {
                column.view = column_from_cache(column.cached.as_ref());
                column.view.banner = format!("Cancelled. {}", column.view.banner);
            }
        }
        cx.notify();
    }

    fn inspect(&mut self, query: String, cx: &mut Context<Self>) {
        if self.options.fixture {
            self.reply_note = "Fixture mode does not read replies.".into();
            self.surface = Surface::Replies;
            cx.notify();
            return;
        }
        self.surface = Surface::Replies;
        self.reply_note = format!("Loading replies for “{query}”…");
        cx.notify();
        let config = self.config.clone();
        let cancel = self.cancel.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    OpenCliDesk::new(&config, Arc::new(SystemRunner))
                        .inspect_replies(&query, &cancel)
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                match result {
                    Ok(rows) => {
                        desk.reply_note = if rows.is_empty() {
                            "OpenCLI returned no reply rows.".into()
                        } else {
                            "Replies from OpenCLI. Nothing was posted.".into()
                        };
                        desk.replies = rows;
                    }
                    Err(error) => {
                        desk.reply_note = error;
                        desk.replies.clear();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn prepare(&mut self, title: &str, reply_to: Option<String>, cx: &mut Context<Self>) {
        if self.options.fixture {
            self.draft_text = termy_x::trends_desk::draft_text_from_title(title);
            self.draft_reply_to = reply_to;
            self.draft_note = "Fixture mode kept this draft in the window and did not write it. Nothing was posted.".into();
            self.surface = Surface::Draft;
            self.field = Some(Field::Draft);
            cx.notify();
            return;
        }
        let prep = prepare_draft(&self.config, title, reply_to.as_deref());
        self.draft_text = prep.text;
        self.draft_reply_to = prep.reply_to;
        self.draft_note = match (prep.saved_id, prep.save_error) {
            (Some(id), None) => format!(
                "Saved draft {id}. It is not posted. Publish it from X compose, or `termy-xcli publish {id} --dry-run`, and type yes only when you mean to send."
            ),
            (_, Some(error)) => format!("Draft kept in this window. Save failed: {error}"),
            (None, None) => "Draft kept in this window.".into(),
        };
        self.surface = Surface::Draft;
        self.field = Some(Field::Draft);
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.focus = self.focus_text.trim().to_string();
        if let Err(error) = self.store.save_settings(&self.settings) {
            self.draft_note = error;
        }
        cx.notify();
    }

    fn add_topic(&mut self, cx: &mut Context<Self>) {
        let Some(id) = termy_x::trends_desk::slug_id(&self.topic_name) else {
            self.draft_note = "Column name needs a letter or number.".into();
            cx.notify();
            return;
        };
        if self.columns.iter().any(|column| column.topic.id == id) {
            self.draft_note = format!("Column {id} already exists.");
            cx.notify();
            return;
        }
        let topic = Topic {
            id,
            name: self.topic_name.trim().to_string(),
            desc: self.topic_desc.trim().to_string(),
        };
        self.columns.push(ColumnUi {
            view: column_from_cache(None),
            topic: topic.clone(),
            cached: None,
            tags: Vec::new(),
        });
        let topics: Vec<Topic> = self
            .columns
            .iter()
            .map(|column| column.topic.clone())
            .collect();
        if let Err(error) = self.store.save_topics(&topics) {
            self.draft_note = error;
        } else {
            self.topic_name.clear();
            self.topic_desc.clear();
        }
        let _ = topic;
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if key == "escape" {
            self.field = None;
            cx.notify();
            return;
        }
        if key == "enter" && self.field == Some(Field::Focus) {
            self.save_settings(cx);
            self.field = None;
            return;
        }
        let Some(field) = self.field else {
            return;
        };
        let text = self.field_text(field);
        if mods.platform || mods.control {
            return;
        }
        if key == "backspace" {
            text.pop();
        } else if let Some(ch) = event.keystroke.key_char.as_deref()
            && !ch.is_empty()
            && ch.chars().all(|item| item != '\u{7f}')
        {
            if text.chars().count() < 240 {
                text.push_str(ch);
            }
        } else {
            return;
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn field_text(&mut self, field: Field) -> &mut String {
        match field {
            Field::Focus => &mut self.focus_text,
            Field::Filter => &mut self.filter_text,
            Field::TopicName => &mut self.topic_name,
            Field::TopicDesc => &mut self.topic_desc,
            Field::Draft => &mut self.draft_text,
            Field::DailyAt => &mut self.settings.daily_at,
        }
    }

    fn nav(&mut self, nav: Nav, cx: &mut Context<Self>) {
        self.surface = surface_after_nav(self.surface, nav);
        cx.notify();
    }
}

fn fixture_posts_for(topic: &Topic) -> Vec<termy_x::trends_desk::SourcePost> {
    let mut posts = synthetic_fixture_posts();
    for post in &mut posts {
        let rest = post
            .text
            .trim_start_matches("SYNTHETIC FIXTURE:")
            .trim_start_matches("SYNTHETIC FIXTURE")
            .trim();
        post.text = format!("SYNTHETIC FIXTURE · {} column · {rest}", topic.name);
    }
    posts
}

fn paced_wait(cancel: &AtomicBool) {
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_millis(1500) {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn open_in(cx: &mut App, options: DeskOptions) -> Result<(), String> {
    if let Some(existing) = cx
        .windows()
        .into_iter()
        .find_map(|handle| handle.downcast::<TrendsDesk>())
    {
        return existing
            .update(cx, |_desk, window, _cx| {
                window.activate_window();
            })
            .map_err(|error| error.to_string());
    }
    cx.open_window(desk_window_options(), move |window, cx| {
        window.set_window_title("Trends Desk");
        cx.new(|cx| TrendsDesk::new(options, cx))
    })
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub fn run_standalone(fixture: bool) {
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        if let Err(error) = open_in(cx, DeskOptions { fixture }) {
            eprintln!("Trends Desk did not open: {error}");
            eprintln!("The same desk can run without this window:");
            eprintln!("  termy-xcli trends-desk status");
            eprintln!("  termy-xcli trends-desk run --topic all");
        }
    });
}

fn desk_window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(24.0), px(24.0)),
            size: size(px(3440.0), px(1400.0)),
        })),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("Trends Desk".into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        #[cfg(target_os = "linux")]
        window_decorations: Some(gpui_kit::WindowDecorations::Server),
        kind: WindowKind::Normal,
        is_resizable: true,
        window_min_size: Some(size(px(960.0), px(640.0))),
        ..Default::default()
    }
}

impl Render for TrendsDesk {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_started(cx);
        let width: f32 = window.viewport_size().width.into();
        let per_row = column_count_for_width(width).min(self.columns.len().max(1));
        let surface = self.surface;
        div()
            .id("trends-desk")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x070b14))
            .text_color(rgb(0xe5e5ea))
            .child(self.header(per_row, cx))
            .child(self.nav_bar(cx))
            .child(
                div()
                    .id("trends-desk-body")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .p(px(16.0))
                    .child(match surface {
                        Surface::Columns => self.columns_view(width, per_row, cx),
                        Surface::History => self.history_view(),
                        Surface::Replies => self.replies_view(cx),
                        Surface::Draft => self.draft_view(cx),
                        Surface::Settings => self.settings_view(cx),
                    }),
            )
            .child(self.footer())
    }
}

impl TrendsDesk {
    fn header(&self, per_row: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .px(px(16.0))
            .py(px(12.0))
            .bg(rgb(0x10141c))
            .border_b_1()
            .border_color(rgb(0x2c2c2e))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(22.0))
                            .text_color(rgb(0x7af6ff))
                            .child("Trends Desk"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xc7c7cc))
                            .child(self.session.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x8e8e93))
                            .child(format!(
                                "{} columns across · focus {}",
                                per_row,
                                if self.focus_text.is_empty() {
                                    "none"
                                } else {
                                    self.focus_text.as_str()
                                }
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button(
                        "desk-run-all",
                        "Run all",
                        false,
                        cx.listener(|this, _, _, cx| {
                            let ids: Vec<String> = this
                                .columns
                                .iter()
                                .map(|column| column.topic.id.clone())
                                .collect();
                            this.start_run(&ids, cx);
                        }),
                    ))
                    .child(button(
                        "desk-cancel",
                        "Cancel",
                        false,
                        cx.listener(|this, _, _, cx| this.cancel_run(cx)),
                    )),
            )
            .into_any_element()
    }

    fn nav_bar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let current = self.surface;
        div()
            .flex()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(8.0))
            .child(nav_button(
                "desk-nav-columns",
                "Columns",
                current == Surface::Columns,
                cx,
                Nav::Columns,
            ))
            .child(nav_button(
                "desk-nav-history",
                "History",
                current == Surface::History,
                cx,
                Nav::History,
            ))
            .child(nav_button(
                "desk-nav-replies",
                "Replies",
                current == Surface::Replies,
                cx,
                Nav::Replies,
            ))
            .child(nav_button(
                "desk-nav-draft",
                "Draft",
                current == Surface::Draft,
                cx,
                Nav::Draft,
            ))
            .child(nav_button(
                "desk-nav-settings",
                "Settings",
                current == Surface::Settings,
                cx,
                Nav::Settings,
            ))
            .child(self.field_chip(
                "desk-filter",
                "Filter",
                &self.filter_text,
                Field::Filter,
                cx,
            ))
            .into_any_element()
    }

    fn columns_view(
        &self,
        width: f32,
        per_row: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let col_w = (width / per_row as f32).clamp(280.0, 1600.0);
        let mut rows: Vec<gpui_kit::AnyElement> = Vec::new();
        for chunk in self.columns.chunks(per_row) {
            let mut row = div().flex().flex_row().gap(px(12.0)).mb(px(12.0));
            for column in chunk {
                row = row.child(self.column_card(column, col_w, cx));
            }
            rows.push(row.into_any_element());
        }
        if self.session.state == SessionState::Missing {
            rows.insert(
                0,
                div()
                    .mb(px(12.0))
                    .p(px(12.0))
                    .rounded(px(12.0))
                    .bg(rgb(0x3a1d1d))
                    .text_size(px(13.0))
                    .child(self.session.detail.clone())
                    .into_any_element(),
            );
        }
        div().flex().flex_col().children(rows).into_any_element()
    }

    fn column_card(
        &self,
        column: &ColumnUi,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = column.topic.id.clone();
        let phase = phase_label(column.view.phase);
        let mut body = div()
            .w(px(width))
            .min_h(px(240.0))
            .p(px(12.0))
            .rounded(px(14.0))
            .bg(rgb(0x16161e))
            .border_1()
            .border_color(phase_color(column.view.phase))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(18.0)).child(column.topic.name.clone()))
                    .child(button(
                        SharedString::from(format!("desk-run-{id}")),
                        "Run",
                        false,
                        cx.listener(move |this, _, _, cx| {
                            this.start_run(std::slice::from_ref(&id), cx);
                        }),
                    )),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x8e8e93))
                    .child(column.topic.desc.clone()),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(phase_color(column.view.phase))
                    .child(format!("{phase} · {}", column.view.banner)),
            );
        let visible = filter_posts(&column.view.posts, &self.filter_text);
        if visible.is_empty() && column.view.phase != Phase::Loading {
            body = body.child(div().text_size(px(13.0)).child(
                if self.filter_text.trim().is_empty() {
                    "No cards in this column."
                } else {
                    "No cards match the filter."
                },
            ));
        }
        for post in visible {
            let tag =
                column
                    .tags
                    .iter()
                    .find(|(url, _)| url == &post.url)
                    .map(|(_, tag)| match tag {
                        ChangeTag::New => "NEW",
                        ChangeTag::Rising => "RISING",
                        ChangeTag::Fading => "FADING",
                    });
            let title = post.text.chars().take(110).collect::<String>();
            let url = post.url.clone();
            let reply_query = post.text.chars().take(120).collect::<String>();
            let created = post
                .created_at
                .clone()
                .unwrap_or_else(|| "time not provided".into());
            body = body.child(
                div()
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(rgb(0x10141c))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(div().text_size(px(14.0)).child(format!(
                        "{}{}",
                        tag.map(|tag| format!("{tag} · ")).unwrap_or_default(),
                        title
                    )))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xc7c7cc))
                            .child(format!(
                                "@{} · {} · {}",
                                post.author,
                                created,
                                post.engagement_label()
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x7af6ff))
                            .child(post.url.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(button(
                                SharedString::from(format!("desk-open-{}", post.id)),
                                "Open source",
                                false,
                                cx.listener(move |_, _, _, _| open_status(&url)),
                            ))
                            .child(button(
                                SharedString::from(format!("desk-replies-{}", post.id)),
                                "Inspect replies",
                                false,
                                cx.listener(move |this, _, _, cx| {
                                    this.inspect(reply_query.clone(), cx);
                                }),
                            ))
                            .child(button(
                                SharedString::from(format!("desk-draft-{}", post.id)),
                                "Prepare draft",
                                false,
                                {
                                    let title = title.clone();
                                    let reply = post.url.clone();
                                    cx.listener(move |this, _, _, cx| {
                                        this.prepare(&title, Some(reply.clone()), cx);
                                    })
                                },
                            )),
                    ),
            );
        }
        body.into_any_element()
    }

    fn history_view(&self) -> gpui_kit::AnyElement {
        let mut list = div().flex().flex_col().gap(px(8.0));
        if self.history.is_empty() {
            list = list.child("No saved reports yet. A failed run does not write one.");
        }
        for report in &self.history {
            list = list.child(
                div()
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(rgb(0x16161e))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(format!(
                        "{} · {} · cached file {}",
                        report.topic_id, report.query, report.file
                    ))
                    .child(report.markdown.clone()),
            );
        }
        list.into_any_element()
    }

    fn replies_view(&self, _cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut list = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.reply_note.clone());
        for post in &self.replies {
            let mut card = div()
                .p(px(10.0))
                .rounded(px(10.0))
                .bg(rgb(0x16161e))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(format!(
                    "{} · {}",
                    post.author,
                    post.created_at.clone().unwrap_or_default()
                ))
                .child(post.text.clone())
                .child(post.url.clone());
            for reply in &post.replies {
                card = card.child(format!(
                    "reply {} · {} · {}",
                    reply.author, reply.url, reply.text
                ));
            }
            list = list.child(card);
        }
        list.into_any_element()
    }

    fn draft_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.draft_note.clone())
            .child(self.field_chip(
                "desk-draft-text",
                "Draft",
                &self.draft_text,
                Field::Draft,
                cx,
            ))
            .child(
                div().text_size(px(12.0)).text_color(rgb(0x8e8e93)).child(
                    self.draft_reply_to
                        .clone()
                        .unwrap_or_else(|| "No reply target yet.".into()),
                ),
            )
            .child(
                div().text_size(px(12.0)).child(
                    "Sending stays in the existing X compose confirm. This desk does not post.",
                ),
            )
            .into_any_element()
    }

    fn settings_view(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(self.field_chip("desk-focus", "Focus", &self.focus_text, Field::Focus, cx))
            .child(self.field_chip(
                "desk-daily-at",
                "Daily at (America/Chicago)",
                &self.settings.daily_at,
                Field::DailyAt,
                cx,
            ))
            .child(self.field_chip(
                "desk-topic-name",
                "New column",
                &self.topic_name,
                Field::TopicName,
                cx,
            ))
            .child(self.field_chip(
                "desk-topic-desc",
                "Column description",
                &self.topic_desc,
                Field::TopicDesc,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(button(
                        "desk-save-settings",
                        "Save focus",
                        false,
                        cx.listener(|this, _, _, cx| this.save_settings(cx)),
                    ))
                    .child(button(
                        "desk-add-topic",
                        "Add column",
                        false,
                        cx.listener(|this, _, _, cx| this.add_topic(cx)),
                    ))
                    .child(button(
                        "desk-toggle-daily",
                        if self.settings.daily {
                            "Daily on"
                        } else {
                            "Daily off"
                        },
                        self.settings.daily,
                        cx.listener(|this, _, _, cx| {
                            this.settings.daily = !this.settings.daily;
                            let _ = this.store.save_settings(&this.settings);
                            cx.notify();
                        }),
                    )),
            )
            .child(self.session.detail.clone())
            .into_any_element()
    }

    fn field_chip(
        &self,
        id: &'static str,
        label: &str,
        value: &str,
        field: Field,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let active = self.field == Some(field);
        let shown = if value.is_empty() {
            format!("{label}: click to edit")
        } else {
            format!("{label}: {value}")
        };
        button(
            id,
            shown,
            active,
            cx.listener(move |this, _, window, cx| {
                this.field = Some(field);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }),
        )
    }

    fn footer(&self) -> gpui_kit::AnyElement {
        div()
            .px(px(16.0))
            .py(px(8.0))
            .text_size(px(11.0))
            .text_color(rgb(0x8e8e93))
            .bg(rgb(0x10141c))
            .child(format!("{GPU_NOTE} {IMAGE_QUOTE_NOTE}"))
            .into_any_element()
    }
}

fn nav_button(
    id: &'static str,
    label: &'static str,
    active: bool,
    cx: &mut Context<TrendsDesk>,
    nav: Nav,
) -> gpui_kit::AnyElement {
    button(
        id,
        label,
        active,
        cx.listener(move |this, _, _, cx| this.nav(nav, cx)),
    )
}

fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    active: bool,
    handler: impl Fn(&gpui_kit::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(id.into())
        .px(px(10.0))
        .py(px(6.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if active { rgb(0x0a84ff) } else { rgb(0x3a3a3c) })
        .bg(if active { rgb(0x1a2a44) } else { rgb(0x2c2c2e) })
        .text_size(px(12.0))
        .cursor_pointer()
        .child(label.into())
        .on_mouse_down(MouseButton::Left, handler)
        .into_any_element()
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "Idle",
        Phase::Loading => "Loading",
        Phase::Fresh => "Fresh",
        Phase::Cached => "Cached",
        Phase::Empty => "Empty",
        Phase::Error => "Error",
        Phase::Fixture => "Fixture",
    }
}

fn phase_color(phase: Phase) -> Rgba {
    match phase {
        Phase::Fresh => rgb(0x30d158),
        Phase::Cached => rgb(0xffd60a),
        Phase::Loading => rgb(0x7af6ff),
        Phase::Empty => rgb(0xff9f0a),
        Phase::Error => rgb(0xff453a),
        Phase::Fixture => rgb(0xbf5af2),
        Phase::Idle => rgb(0x8e8e93),
    }
}

fn open_status(url: &str) {
    if url.starts_with("https://x.com/") && url.contains("/status/") {
        let _ = webbrowser::open(url);
    }
}
