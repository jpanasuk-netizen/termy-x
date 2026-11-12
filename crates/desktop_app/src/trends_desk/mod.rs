//! GPU-rendered Trends Desk window. OpenCLI searches on a background thread.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{
    App, AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, WindowBounds, WindowKind,
    WindowOptions, div, point, px, rgb, size,
};
use termy_x::XConfig;
use termy_x::io::SystemRunner;
use termy_x::trends_desk::{
    COMPANION_HEIGHT, COMPANION_WIDTH, ChangeTag, ColumnView, DOCK_WIDTH, DeskFrame, DeskSettings,
    DeskStore, EditPopout, GPU_NOTE, IMAGE_QUOTE_NOTE, InspectedPost, Nav, OpenCliDesk,
    OutcomeKind, Phase, ReplyPhase, RunOutcome, SessionState, SessionStatus, Surface, TakeDraft,
    TakeProvider, Topic, VISIBLE_NAME, WINDOW_PAD, apply_send_result, card_columns, card_from_post,
    close_editor, column_count_for_width, column_from_cache, column_from_outcome,
    column_loading_keep, daily_due, draft_take, drafts_file, edit_reply, editor_send, filter_posts,
    fixture_column, frame_from_parts, isolated_opencli_config, layout_takes, move_selection,
    offline_take, open_editor, page_label, post_reply_opencli, prepare_draft_file, request_send,
    scroll_by, surface_after_nav, synthetic_fixture_posts, termy_xl_data_root, turn_page,
    within_termy_x_config, wrap_token,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct DeskOptions {
    pub fixture: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Focus,
    Filter,
    TopicName,
    TopicDesc,
    Draft,
    DailyAt,
    AiBase,
    AiModel,
    AiKey,
    HermesBase,
    HermesModel,
    HermesKey,
}

pub(crate) struct ColumnUi {
    pub(crate) topic: Topic,
    pub(crate) view: ColumnView,
    cached: Option<termy_x::trends_desk::StoredReport>,
    pub(crate) tags: Vec<(String, ChangeTag)>,
}

pub struct TrendsDesk {
    focus_handle: FocusHandle,
    pub(crate) search_focus: FocusHandle,
    pub(crate) options: DeskOptions,
    pub(crate) data_root: PathBuf,
    config: XConfig,
    pub(crate) store: DeskStore,
    pub(crate) writable: bool,
    pub(crate) settings: DeskSettings,
    pub(crate) columns: Vec<ColumnUi>,
    pub(crate) surface: Surface,
    pub(crate) field: Option<Field>,
    focus_text: String,
    pub(crate) filter_text: String,
    topic_name: String,
    topic_desc: String,
    ai_base_text: String,
    ai_model_text: String,
    ai_key_text: String,
    hermes_base_text: String,
    hermes_model_text: String,
    hermes_key_text: String,
    pub(crate) hermes_pill: String,
    pub(crate) agent_pane: Option<termy_x::trends_desk::AgentPane>,
    pub(crate) agent_input: bool,
    agent_proc: Option<xl_agents::AgentProc>,
    agent_seen: usize,
    pub(crate) model_notice: String,
    pub(crate) model_detail: String,
    pub(crate) action_note: String,
    pub(crate) free_halted: bool,
    draft_text: String,
    draft_reply_to: Option<String>,
    draft_note: String,
    replies: Vec<InspectedPost>,
    reply_note: String,
    history: Vec<termy_x::trends_desk::StoredReport>,
    history_index: Option<usize>,
    pub(crate) card_replies: HashMap<String, ReplyPhase>,
    pub(crate) takes: HashMap<String, termy_x::trends_desk::TakeDraft>,
    take_wave: u64,
    editing_card: Option<String>,
    pub(crate) selected: Option<String>,
    pub(crate) scroll_offsets: HashMap<String, f32>,
    pub(crate) scroll_handles: HashMap<String, gpui_kit::ScrollHandle>,
    pub(crate) popout: Option<EditPopout>,
    pub(crate) pop_caret: termy_x::draft_edit::DraftCaret,
    pub(crate) pack: termy_x::trends_desk::PackFile,
    pub(crate) palette: bool,
    pub(crate) palette_query: String,
    pub(crate) board: String,
    pub(crate) x_panel: Option<gpui_kit::Entity<crate::x_panel::XPanel>>,
    pub(crate) last_host: Option<termy_x::trends_desk::WindowRect>,
    pub(crate) last_host_key: Option<(termy_x::trends_desk::WindowRect, bool)>,
    pub(crate) last_follow: Option<Instant>,
    pub(crate) session: SessionStatus,
    generation: u64,
    cancel: Arc<AtomicBool>,
    pub(crate) busy: bool,
    started: bool,
    pub(crate) frame: DeskFrame,
    pub(crate) anim_target: Option<f32>,
    pub(crate) close_hooked: bool,
    fitted: bool,
    pub(crate) card_window: usize,
    pub(crate) media: xl_media::MediaDesk,
    pub(crate) take_epoch: u64,
    pub(crate) takes_halted: bool,
}

impl TrendsDesk {
    pub fn new(options: DeskOptions, cx: &mut Context<Self>) -> Self {
        let data_root = termy_xl_data_root();
        let (store, writable) = match DeskStore::open_isolated(&data_root) {
            Ok(store) => (store, true),
            Err(_) => (
                DeskStore::new(&std::env::temp_dir().join("termy-xl-blocked")),
                false,
            ),
        };
        let config = isolated_opencli_config(&data_root);
        let mut settings = if writable {
            store.load_settings()
        } else {
            DeskSettings::default()
        };
        if !config.take_provider.trim().is_empty() {
            settings.take_provider =
                termy_x::trends_desk::TakeProvider::parse(&config.take_provider)
                    .as_str()
                    .into();
        }
        let pack = if writable {
            termy_x::trends_desk::load_pack(&data_root)
        } else {
            termy_x::trends_desk::PackFile::default()
        };
        let frame = frame_from_parts(
            false,
            settings.frame_x as f32,
            settings.frame_y as f32,
            termy_x::trends_desk::LOCKED_OPEN_WIDTH,
            settings.frame_height as f32,
        );
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
        let ai_base_text = config.ai_base_url.clone();
        let ai_model_text = config.ai_model.clone();
        let ai_key_text = config.ai_api_key.clone();
        let hermes_base_text = if config.hermes_base_url.trim().is_empty() {
            termy_x::trends_desk::HERMES_BASE.to_string()
        } else {
            config.hermes_base_url.clone()
        };
        let hermes_model_text = if config.hermes_model.trim().is_empty() {
            termy_x::trends_desk::HERMES_MODEL.to_string()
        } else {
            config.hermes_model.clone()
        };
        let hermes_key_text = config.hermes_api_key.clone();
        Self {
            focus_handle: cx.focus_handle(),
            search_focus: cx.focus_handle(),
            options,
            data_root,
            config,
            store,
            writable,
            focus_text: settings.focus.clone(),
            settings,
            columns,
            surface: Surface::Columns,
            field: None,
            filter_text: String::new(),
            topic_name: String::new(),
            topic_desc: String::new(),
            ai_base_text,
            ai_model_text,
            ai_key_text,
            hermes_base_text,
            hermes_model_text,
            hermes_key_text,
            hermes_pill: "Hermes · checking".into(),
            agent_pane: None,
            agent_input: false,
            agent_proc: None,
            agent_seen: 0,
            model_notice: String::new(),
            model_detail: String::new(),
            action_note: String::new(),
            free_halted: false,
            draft_text: String::new(),
            draft_reply_to: None,
            draft_note: "Drafts stay in the Termy XL data folder. Copy to Termy X is a separate step and does not post.".into(),
            replies: Vec::new(),
            reply_note: "Inspect replies reads a thread. It does not post.".into(),
            history,
            history_index: None,
            card_replies: HashMap::new(),
            takes: HashMap::new(),
            take_wave: 0,
            editing_card: None,
            selected: None,
            scroll_offsets: HashMap::new(),
            scroll_handles: HashMap::new(),
            popout: None,
            pop_caret: termy_x::draft_edit::DraftCaret::collapsed(0),
            pack,
            palette: false,
            palette_query: String::new(),
            board: "subject".into(),
            x_panel: None,
            last_host: None,
            last_host_key: None,
            last_follow: None,
            session: SessionStatus {
                state: SessionState::ProbeFailed,
                label: "Checking OpenCLI…".into(),
                detail: GPU_NOTE.to_string(),
            },
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            busy: false,
            started: false,
            frame,
            anim_target: None,
            close_hooked: false,
            fitted: false,
            card_window: 12,
            media: xl_media::MediaDesk::default(),
            take_epoch: 0,
            takes_halted: false,
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
        self.fetch_empty_subjects(cx);
        self.spawn_probe(cx);
        self.spawn_daily(cx);
        self.spawn_refresh(cx);
        self.spawn_follow(cx);
        self.probe_hermes_models(cx);
        self.seed_agent_file();
    }

    fn fetch_empty_subjects(&mut self, cx: &mut Context<Self>) {
        if self.options.fixture {
            return;
        }
        let ids: Vec<String> = self
            .columns
            .iter()
            .filter(|column| column.view.posts.is_empty() && column.view.phase != Phase::Loading)
            .map(|column| column.topic.id.clone())
            .collect();
        if ids.is_empty() {
            return;
        }
        self.start_run(&ids, cx);
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
                let cached = desk
                    .columns
                    .iter()
                    .any(|column| !column.view.posts.is_empty() || column.cached.is_some());
                desk.session = status;
                if desk.session.state == SessionState::ProbeFailed {
                    termy_x::trends_desk::append_desk_log(
                        &desk.data_root,
                        &format!("opencli probe: {}", desk.session.detail),
                    );
                }
                if desk.session.state == SessionState::ProbeFailed && cached {
                    desk.session.label =
                        "Saved reports are loaded. OpenCLI whoami did not answer.".into();
                    desk.session.detail =
                        "Refresh runs a live search. The cards stay on the saved report.".into();
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_follow(&mut self, cx: &mut Context<Self>) {
        chrome::follow_loop(cx);
    }

    fn spawn_refresh(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                let _ = this.update(cx, |desk, cx| {
                    desk.maybe_refresh(cx);
                    cx.notify();
                });
            }
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

    pub(crate) fn start_run(&mut self, ids: &[String], cx: &mut Context<Self>) {
        if self.options.fixture {
            self.action_note = "Fixture mode does not call OpenCLI.".into();
            cx.notify();
            return;
        }
        self.action_note = "Loading…".into();
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
            let kept = if !column.view.posts.is_empty() {
                column.view.posts.clone()
            } else {
                column
                    .cached
                    .as_ref()
                    .map(|report| report.posts.clone())
                    .unwrap_or_default()
            };
            column.view = column_loading_keep(&query, kept);
            topics.push(column.topic.clone());
        }
        cx.notify();
        let config = self.config.clone();
        let cancel = self.cancel.clone();
        let data_root = self.data_root.clone();
        let writable = self.writable;
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
                    let store = if writable {
                        DeskStore::open_isolated(&data_root).ok()
                    } else {
                        None
                    };
                    let now = termy_x::time::unix_now();
                    let mut saved = Vec::new();
                    for outcome in &outcomes {
                        if outcome.kind.writes_report()
                            && let Some(store) = &store
                        {
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
        self.takes_halted = true;
        self.take_epoch = self.take_epoch.wrapping_add(1);
        self.take_wave = self.take_wave.saturating_add(1);
        termy_x::trends_desk::halt_takes();
        self.takes.retain(|_, take| take.source != "pending");
        self.busy = false;
        self.action_note = "Cancelled.".into();
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
        let path = drafts_file(&self.data_root);
        if !self.writable || within_termy_x_config(&path) {
            self.draft_text = termy_x::trends_desk::draft_text_from_title(title);
            self.draft_reply_to = reply_to;
            self.draft_note = "Draft kept in this window. It was not written into Termy X.".into();
            self.surface = Surface::Draft;
            self.field = Some(Field::Draft);
            cx.notify();
            return;
        }
        let prep = prepare_draft_file(&path, title, reply_to.as_deref());
        self.draft_text = prep.text;
        self.draft_reply_to = prep.reply_to;
        self.draft_note = match (prep.saved_id, prep.save_error) {
            (Some(id), None) => format!(
                "Saved draft {id} in the Termy XL data folder. It was not copied to Termy X and it was not posted."
            ),
            (_, Some(error)) => format!("Draft kept in this window. Save failed: {error}"),
            (None, None) => "Draft kept in this window.".into(),
        };
        self.surface = Surface::Draft;
        self.field = Some(Field::Draft);
        cx.notify();
    }

    pub(crate) fn page_ids(&self) -> Vec<String> {
        let index = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
            .unwrap_or(0);
        let Some(column) = self.columns.get(index) else {
            return Vec::new();
        };
        let mut ids = vec![format!("subject:{}", column.topic.id)];
        for post in filter_posts(&column.view.posts, &self.filter_text) {
            ids.push(format!("take:{}", post.id));
        }
        ids
    }

    fn popout_key(&mut self, key: &str, event: &KeyDownEvent, cx: &mut Context<Self>) {
        use termy_x::draft_edit::Motion;
        if self.agent_input {
            if key == "escape" {
                self.agent_input = false;
                cx.notify();
                return;
            }
            if key == "enter" {
                self.submit_agent_line();
                cx.notify();
                return;
            }
            if key == "backspace" {
                if let Some(pane) = self.agent_pane.as_mut() {
                    pane.backspace();
                }
                cx.notify();
                return;
            }
            if let Some(ch) = event.keystroke.key_char.as_deref()
                && !event.keystroke.modifiers.control
                && !event.keystroke.modifiers.platform
                && !ch.is_empty()
            {
                if let Some(pane) = self.agent_pane.as_mut() {
                    pane.push_str(ch);
                }
                cx.notify();
            }
            return;
        }
        if key == "escape" {
            self.close_popout();
            self.action_note = "Closed the draft. The text is still on the card.".into();
            cx.notify();
            return;
        }
        if termy_x::trends_desk::posts_on_ctrl_enter(
            key,
            event.keystroke.modifiers.control || event.keystroke.modifiers.platform,
        ) {
            self.action_note = "Posting…".into();
            self.send_from_popout(cx);
            return;
        }
        if key == "c" && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform) {
            if let Some(pop) = self.popout.clone() {
                self.copy_text(&pop.draft, cx);
                self.action_note = "Copied the reply.".into();
            }
            cx.notify();
            return;
        }
        let Some(mut pop) = self.popout.clone() else {
            return;
        };
        let mut caret = self.pop_caret;
        let lines = [(0, pop.draft.len())];
        if key == "backspace" {
            caret.backspace(&mut pop.draft);
        } else if key == "left" {
            caret.move_by(&pop.draft, &lines, Motion::Left, false);
        } else if key == "right" {
            caret.move_by(&pop.draft, &lines, Motion::Right, false);
        } else if let Some(ch) = event.keystroke.key_char.as_deref()
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.platform
            && !ch.is_empty()
        {
            caret.insert(&mut pop.draft, ch);
        }
        self.pop_caret = caret;
        self.popout = Some(pop);
        cx.notify();
    }

    pub(crate) fn send_from_popout(&mut self, cx: &mut Context<Self>) {
        let Some(pop) = self.popout.clone() else {
            return;
        };
        if self
            .card_replies
            .get(&pop.card_id)
            .is_some_and(termy_x::trends_desk::send_is_busy)
        {
            self.action_note = "Sending…".into();
            cx.notify();
            return;
        }
        match editor_send(&pop, true) {
            Err(error) => self.action_note = error,
            Ok(text) => {
                self.action_note = "Sending…".into();
                self.card_replies
                    .insert(pop.card_id.clone(), ReplyPhase::Editing { text });
                self.send_reply(pop.card_id, cx);
            }
        }
    }

    pub(crate) fn set_pop_tone(&mut self, tone: &str) {
        let Some(pop) = self.popout.as_mut() else {
            return;
        };
        let parsed = termy_x::trends_desk::TakeTone::parse(tone);
        let name = termy_x::trends_desk::panel_tone_name(parsed);
        pop.tone = name.to_string();
        self.pack.tone = name.to_ascii_lowercase();
        self.save_pack_file();
    }

    pub(crate) fn open_popout(&mut self, id: &str) {
        let text = self
            .card_replies
            .get(id)
            .map(|phase| phase.text().to_string())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| self.takes.get(id).map(|take| take.text.clone()))
            .unwrap_or_default();
        let provider = TakeProvider::parse(&self.settings.take_provider);
        self.selected = Some(termy_x::trends_desk::select_take(id));
        self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(text.len());
        self.card_replies
            .entry(id.to_string())
            .or_insert_with(|| ReplyPhase::Editing { text: text.clone() });
        let stored = self
            .takes
            .get(id)
            .map(|take| take.variants.clone())
            .unwrap_or_default();
        let mut editor = open_editor(id, &text, provider);
        if !stored.is_empty() {
            editor.variants = termy_x::trends_desk::distinct_variants(&text, &stored);
        }
        self.popout = Some(editor);
    }

    pub(crate) fn close_popout(&mut self) {
        let Some(pop) = self.popout.take() else {
            return;
        };
        let closed = close_editor(&pop);
        self.card_replies.insert(
            pop.card_id.clone(),
            edit_reply(&ReplyPhase::Idle, closed.draft),
        );
    }

    pub(crate) fn draft_take_for(&mut self, id: String, cx: &mut Context<Self>) {
        self.show_inline_editor(&id, termy_x::trends_desk::DRAFTING_PLACEHOLDER);
        cx.notify();
        self.spawn_one_take(id, true, cx);
    }

    fn spawn_one_take(&mut self, id: String, open_compose: bool, cx: &mut Context<Self>) {
        self.takes_halted = false;
        termy_x::trends_desk::resume_takes();
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
            .cloned()
        else {
            if open_compose {
                let message = termy_x::trends_desk::text_after_take(
                    termy_x::trends_desk::DRAFTING_PLACEHOLDER,
                    "",
                    "this card has no post",
                );
                self.show_inline_editor(&id, &message);
                cx.notify();
            }
            return;
        };
        self.takes.insert(
            id.clone(),
            TakeDraft {
                text: String::new(),
                source: "pending",
                note: String::new(),
                variants: Vec::new(),
            },
        );
        let provider = TakeProvider::parse(&self.settings.take_provider);
        let config = self.config.clone();
        let tone = termy_x::trends_desk::panel_tone_name(termy_x::trends_desk::TakeTone::parse(
            &self.pack.tone,
        ))
        .to_string();
        if open_compose {
            self.show_inline_editor(&id, termy_x::trends_desk::DRAFTING_PLACEHOLDER);
        }
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let http = termy_x::io::UreqHttp::default();
                    let drafted = termy_x::trends_desk::run_take_chain(
                        provider, &config, &post, &http, &tone,
                    );
                    drafted
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                desk.takes.insert(id.clone(), result.clone());
                if open_compose {
                    let current = if desk
                        .popout
                        .as_ref()
                        .is_some_and(|pop| pop.open && pop.card_id == id)
                    {
                        desk.popout
                            .as_ref()
                            .map(|pop| pop.draft.clone())
                            .unwrap_or_default()
                    } else {
                        desk.card_replies
                            .get(&id)
                            .map(|phase| phase.text().to_string())
                            .unwrap_or_default()
                    };
                    let next =
                        termy_x::trends_desk::text_after_take(&current, &result.text, &result.note);
                    if let Some(pop) = desk
                        .popout
                        .as_mut()
                        .filter(|pop| pop.open && pop.card_id == id)
                    {
                        pop.draft = next.clone();
                        if !result.variants.is_empty() {
                            pop.variants =
                                termy_x::trends_desk::distinct_variants(&next, &result.variants);
                        }
                        desk.pop_caret = termy_x::draft_edit::DraftCaret::at_end(next.len());
                    }
                    desk.card_replies
                        .insert(id.clone(), edit_reply(&ReplyPhase::Idle, next));
                    desk.draft_note = result.note.clone();
                    if !result.note.is_empty() {
                        desk.action_note = result.note.clone();
                    }
                    desk.selected = Some(format!("take:{id}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn send_reply(&mut self, id: String, cx: &mut Context<Self>) {
        let phase = self
            .card_replies
            .get(&id)
            .cloned()
            .unwrap_or(ReplyPhase::Idle);
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == id)
            .cloned()
        else {
            return;
        };
        match request_send(&phase, true) {
            Err(error) => {
                self.action_note = error.clone();
                let text = phase.text().to_string();
                if !text.is_empty() {
                    self.card_replies
                        .insert(id, ReplyPhase::Failed { text, error });
                }
                cx.notify();
            }
            Ok(pending) => {
                let text = pending.text().to_string();
                self.card_replies.insert(id.clone(), pending.clone());
                cx.notify();
                let bin = self.config.opencli_bin.clone();
                let url = post.url.clone();
                let receipt_url = url.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            let runner = termy_x::io::SystemRunner;
                            post_reply_opencli(&bin, &runner, &url, &text)
                        })
                        .await;
                    let _ = this.update(cx, |desk, cx| {
                        let pending = desk.card_replies.get(&id).cloned().unwrap_or(pending);
                        desk.pack.receipts.push(termy_x::trends_desk::ReplyMark {
                            card_id: id.clone(),
                            url: receipt_url.clone(),
                            at_unix: TrendsDesk::now_unix(),
                            status: if result.is_ok() {
                                "confirmed".into()
                            } else {
                                "failed".into()
                            },
                            text: pending.text().to_string(),
                        });
                        desk.save_pack_file();
                        desk.action_note = match &result {
                            Ok(_) => "Sent.".into(),
                            Err(error) => error.clone(),
                        };
                        desk.card_replies
                            .insert(id, apply_send_result(&pending, result));
                        cx.notify();
                    });
                })
                .detach();
            }
        }
    }

    fn save_model(&mut self, cx: &mut Context<Self>) {
        self.config.ai_base_url = self.ai_base_text.trim().to_string();
        self.config.ai_model = self.ai_model_text.trim().to_string();
        self.config.ai_api_key = self.ai_key_text.trim().to_string();
        self.config.ai_explicit = !self.config.ai_base_url.is_empty();
        self.free_halted = false;
        self.model_notice.clear();
        self.model_detail.clear();
        match self.config.save_ai(
            &self.config.ai_base_url,
            &self.config.ai_model,
            &self.config.ai_api_key,
        ) {
            Ok(()) => {
                self.draft_note =
                    "Saved the free model in Termy XL. The key stays in that file and is not logged."
                        .into();
            }
            Err(error) => self.draft_note = error,
        }
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

    fn shift_subject(&mut self, delta: i32, cx: &mut Context<Self>) {
        let Some(index) = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
        else {
            return;
        };
        let Some(next) = termy_x::trends_desk::shift_index(self.columns.len(), index, delta) else {
            return;
        };
        self.columns.swap(index, next);
        self.persist_topics();
        cx.notify();
    }

    fn remove_subject(&mut self, cx: &mut Context<Self>) {
        if self.columns.len() <= 1 {
            self.draft_note = "Keep at least one subject.".into();
            cx.notify();
            return;
        }
        let Some(index) = self
            .columns
            .iter()
            .position(|column| column.topic.id == self.settings.subject_id)
        else {
            return;
        };
        self.columns.remove(index);
        if let Some(column) = self.columns.get(index.min(self.columns.len() - 1)) {
            self.settings.subject_id = column.topic.id.clone();
        }
        self.persist_topics();
        let _ = self.store.save_settings(&self.settings);
        cx.notify();
    }

    fn persist_topics(&mut self) {
        let topics: Vec<Topic> = self
            .columns
            .iter()
            .map(|column| column.topic.clone())
            .collect();
        if let Err(error) = self.store.save_topics(&topics) {
            self.draft_note = error;
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if key == "escape" && self.close_media() {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if key == "x" && mods.shift && (mods.control || mods.platform) {
            self.toggle_frame(window, cx);
            cx.stop_propagation();
            return;
        }
        if let Some(id) = self.editing_card.clone() {
            let phase = self
                .card_replies
                .get(&id)
                .cloned()
                .unwrap_or(ReplyPhase::Idle);
            if key == "escape" {
                self.editing_card = None;
            } else if key == "backspace" {
                let mut text = phase.text().to_string();
                text.pop();
                self.card_replies.insert(id, edit_reply(&phase, text));
            } else if let Some(ch) = event.keystroke.key_char.as_deref()
                && !mods.platform
                && !mods.control
                && !ch.is_empty()
            {
                let mut text = phase.text().to_string();
                if text.chars().count() < 240 {
                    text.push_str(ch);
                }
                self.card_replies.insert(id, edit_reply(&phase, text));
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.popout.as_ref().is_some_and(|pop| pop.open) {
            self.popout_key(key, event, cx);
            cx.stop_propagation();
            return;
        }
        if self.palette && key != "escape" && key != "backspace" && !(mods.control && key == "k") {
            self.note_key(&event.keystroke);
        }
        if self.pack_key(key, mods.control || mods.platform, mods.shift, window, cx) {
            cx.stop_propagation();
            return;
        }
        if key == "escape" {
            let effect = termy_x::trends_desk::escape_effect(
                false,
                false,
                false,
                self.field.is_some(),
                self.selected.is_some(),
                self.frame.expanded,
            );
            if termy_x::trends_desk::escape_requests_quit(effect) {
                termy_x::trends_desk::note_exit_reason("escape requested quit");
            }
            match effect {
                termy_x::trends_desk::EscapeEffect::ClearField => self.field = None,
                termy_x::trends_desk::EscapeEffect::ClearSelection => self.selected = None,
                termy_x::trends_desk::EscapeEffect::CollapseDock => {
                    termy_x::trends_desk::append_desk_log(
                        &self.data_root,
                        "INFO exit: escape collapsed the dock; the window stayed open",
                    );
                    self.toggle_frame(window, cx);
                }
                _ => {}
            }
            cx.notify();
            return;
        }
        if key == "enter" && self.field == Some(Field::Filter) {
            self.run_live_search(cx);
            self.field = None;
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
            Field::AiBase => &mut self.ai_base_text,
            Field::AiModel => &mut self.ai_model_text,
            Field::AiKey => &mut self.ai_key_text,
            Field::HermesBase => &mut self.hermes_base_text,
            Field::HermesModel => &mut self.hermes_model_text,
            Field::HermesKey => &mut self.hermes_key_text,
        }
    }

    fn nav(&mut self, nav: Nav, cx: &mut Context<Self>) {
        if self.popout.as_ref().is_some_and(|pop| pop.open) {
            self.close_popout();
        }
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
    cx.open_window(
        crate::xl_host::desk_window_options(&crate::xl_host::snapped_frame(cx)),
        move |window, cx| {
            window.set_window_title(VISIBLE_NAME);
            cx.new(|cx| TrendsDesk::new(options, cx))
        },
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub fn run_standalone(fixture: bool) {
    set_standalone_app_id();
    let root = termy_xl_data_root();
    termy_x::trends_desk::install_panic_log(&root);
    termy_x::trends_desk::append_desk_log(&root, "INFO startup: opening Termy XL");
    let Some(_instance) = crate::xl_instance::claim(&root) else {
        termy_x::trends_desk::append_desk_log(
            &root,
            "INFO startup: already open, focusing the existing window",
        );
        crate::xl_host::focus_existing();
        eprintln!("{VISIBLE_NAME} is already open.");
        return;
    };
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        if let Err(error) = open_in(cx, DeskOptions { fixture }) {
            termy_x::trends_desk::append_desk_log(
                &termy_xl_data_root(),
                &format!("INFO startup: window did not open: {error}"),
            );
            eprintln!("{VISIBLE_NAME} did not open: {error}");
            eprintln!("The same desk can run without this window:");
            eprintln!("  termy-xcli trends-desk status");
            eprintln!("  termy-xcli trends-desk run --topic all");
        }
    });
    let reason = termy_x::trends_desk::exit_reason();
    let detail = if reason.is_empty() {
        "process ending; no quit() call and no window-close callback".to_string()
    } else {
        format!("process ending after {reason}")
    };
    termy_x::trends_desk::append_desk_log(&root, &termy_x::trends_desk::exit_log_line(&detail));
}

/// Taskbar identity for the standalone window only.
/// The Termy process keeps `lassejlv.Termy`, so the two shortcuts stay separate.
#[cfg(target_os = "windows")]
fn set_standalone_app_id() {
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    if let Err(error) =
        unsafe { SetCurrentProcessExplicitAppUserModelID(windows::core::w!("lassejlv.TermyXL")) }
    {
        log::warn!("Failed to set the Termy XL AppUserModelID: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
fn set_standalone_app_id() {}

impl Render for TrendsDesk {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_started(cx);
        self.ensure_close_hook(window, cx);
        self.focus_open_media(window, cx);
        self.ensure_scrolls();
        self.maybe_refresh(cx);
        self.follow_host(window, cx);
        self.pull_agent_output();
        if !self.fitted {
            self.fitted = crate::xl_host::fit_window(window);
        }
        let width: f32 = window.viewport_size().width.into();
        let mut root = div()
            .id("trends-desk")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key(event, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x070b14))
            .text_color(rgb(0xe5e5ea));
        if width <= DOCK_WIDTH + 8.0 {
            root = root.items_center().justify_center().child(self.xl_mark(cx));
        } else {
            let per_row = column_count_for_width(width).min(self.columns.len().max(1));
            let surface = self.surface;
            root = root
                .child(self.header(per_row, cx))
                .child(self.nav_bar(cx))
                .child({
                    let mut body = div()
                        .id("trends-desk-body")
                        .flex_1()
                        .min_h(px(0.0))
                        .p(px(16.0));
                    if surface != Surface::Columns {
                        body = body.overflow_y_scroll();
                    }
                    body = body.relative().child(match surface {
                        Surface::Columns => self.columns_view(width, per_row, cx),
                        Surface::History => self.history_view(),
                        Surface::Replies => self.replies_view(cx),
                        Surface::Draft => self.draft_view(cx),
                        Surface::Settings => self.settings_view(cx),
                    });
                    body
                })
                .child(self.footer());
        }
        if self.palette {
            root = root.child(self.palette_view(cx));
        }
        if let Some(overlay) = self.media_overlay(cx) {
            root = root.child(overlay);
        }
        if let Some(shield) = self.media_shield(cx) {
            root = root.child(shield);
        }
        if let Some(close) = self.media_close(cx) {
            root = root.child(close);
        }
        root
    }
}

mod chrome;
#[path = "../xl_agents.rs"]
mod xl_agents;
#[path = "../xl_media.rs"]
mod xl_media;
pub(crate) use chrome::{HintView, button, open_status, page_button};
