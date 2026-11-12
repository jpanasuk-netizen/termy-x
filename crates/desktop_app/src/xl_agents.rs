//! Agent launcher for the Termy XL compose card.
//!
//! Opening a pane and drafting never posts. Send stays on the compose card.

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use gpui_kit::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px, rgb,
};
use termy_x::io::Http;

use crate::trends_desk::TrendsDesk;
use termy_x::agents::Agent;
use termy_x::trends_desk::{
    AgentPane, HermesAuth, Launch, PromptCli, TakeProvider, choose_hermes_key, execute_draft,
    hermes_auth, hermes_models_line, hermes_models_request, known_hermes_exe, plan_agent_draft,
    redact_secret, reply_prompt, resolve_launch, seed_agent_block, variants_from_draft,
    write_hermes_settings,
};

pub(crate) struct AgentProc {
    stdin: Option<ChildStdin>,
    child: Child,
    output: Arc<Mutex<String>>,
}

impl Drop for AgentProc {
    fn drop(&mut self) {
        termy_x::trends_desk::reap_child(&mut self.child);
    }
}

impl AgentProc {
    fn spawn(launch: &Launch) -> Result<Self, String> {
        let mut command = Command::new(&launch.program);
        command.args(&launch.args);
        termy_x::trends_desk::apply_hidden_command(&mut command);
        let mut child = command.spawn().map_err(|error| {
            format!(
                "{} is missing: `{}` was not found ({error}).",
                launch.agent.label(),
                launch.program
            )
        })?;
        let output = Arc::new(Mutex::new(String::new()));
        if let Some(mut pipe) = child.stdout.take() {
            let slot = Arc::clone(&output);
            std::thread::spawn(move || {
                let mut buf = [0_u8; 1024];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut text) = slot.lock() {
                                text.push_str(&String::from_utf8_lossy(&buf[..n]));
                            }
                        }
                    }
                }
            });
        }
        if let Some(mut pipe) = child.stderr.take() {
            let slot = Arc::clone(&output);
            std::thread::spawn(move || {
                let mut buf = [0_u8; 1024];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut text) = slot.lock() {
                                text.push_str(&String::from_utf8_lossy(&buf[..n]));
                            }
                        }
                    }
                }
            });
        }
        Ok(Self {
            stdin: child.stdin.take(),
            child,
            output,
        })
    }

    fn write_line(&mut self, line: &str) {
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
    }

    fn take_new(&self, seen: usize) -> (usize, String) {
        let Ok(text) = self.output.lock() else {
            return (seen, String::new());
        };
        if text.len() <= seen {
            return (seen, String::new());
        }
        (text.len(), text[seen..].to_string())
    }
}

struct SystemCli {
    cwd: std::path::PathBuf,
}

impl PromptCli for SystemCli {
    fn run(&self, program: &str, args: &[String], stdin_text: &str) -> Result<String, String> {
        let mut command = Command::new(program);
        command.args(args).current_dir(&self.cwd);
        termy_x::trends_desk::apply_hidden_command(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("{program} is missing: {error}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(stdin_text.as_bytes());
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("{program} failed: {error}"))?;
        let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.is_empty() {
            text = String::from_utf8_lossy(&output.stderr).trim().to_string();
        }
        if text.is_empty() {
            Err(format!("{program} returned an empty draft"))
        } else {
            Ok(text)
        }
    }
}

impl TrendsDesk {
    pub(crate) fn seed_agent_file(&mut self) {
        if !self.writable {
            return;
        }
        let path = self.data_root.join("x.toml");
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        let next = seed_agent_block(&current);
        if next != current {
            let _ = std::fs::write(&path, next);
        }
    }

    pub(crate) fn probe_hermes_models(&mut self, cx: &mut Context<Self>) {
        let base = self.hermes_base_text.clone();
        let saved = self.hermes_key_text.clone();
        cx.spawn(async move |this, cx| {
            let line = cx
                .background_executor()
                .spawn(async move {
                    let env = hermes_env_text();
                    let key = choose_hermes_key(&saved, &env).unwrap_or_default();
                    let http =
                        termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(3));
                    let response = http
                        .request(hermes_models_request(&base, &key))
                        .map_err(|error| redact_secret(&error, &key));
                    match &response {
                        Ok(body) => hermes_models_line(&base, Ok(body)),
                        Err(error) => hermes_models_line(&base, Err(error)),
                    }
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                desk.hermes_pill = line;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn remember_provider(&mut self, name: &str) {
        let name = termy_x::trends_desk::canonical_provider(name);
        self.settings.take_provider = name.clone();
        self.config.take_provider = name.clone();
        if let Some(pop) = self.popout.as_mut() {
            pop.provider = termy_x::trends_desk::provider_kind(&name);
        }
        if self.writable {
            let _ = self.store.save_settings(&self.settings);
            let _ = self.config.save_take_provider(&name);
        }
    }

    pub(crate) fn regenerate_open_variants(&mut self, cx: &mut Context<Self>) {
        self.takes_halted = false;
        termy_x::trends_desk::resume_takes();
        let Some(pop) = self.popout.clone() else {
            return;
        };
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == pop.card_id)
            .cloned()
        else {
            return;
        };
        let saved = self.settings.take_provider.clone();
        let mut config = self.config.clone();
        let provider = termy_x::trends_desk::provider_kind(&saved);
        if provider == TakeProvider::Hermes {
            config.hermes_model = termy_x::trends_desk::HERMES_MODEL.to_string();
            config.hermes_api_key =
                termy_x::trends_desk::hermes_key_resolved(&config.hermes_api_key);
        }
        if provider == TakeProvider::Vyce
            && let Some(model) = termy_x::trends_desk::provider_model(&saved)
        {
            config.ai_model = model;
            config.ai_explicit = true;
        }
        let tone = pop.tone.clone();
        let previous = pop.variants.clone();
        let id = pop.card_id.clone();
        let previous_text = pop.draft.clone();
        if let Some(open) = self.popout.as_mut() {
            open.draft = termy_x::trends_desk::DRAFTING_PLACEHOLDER.into();
        }
        self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(
            termy_x::trends_desk::DRAFTING_PLACEHOLDER.len(),
        );
        self.model_notice = format!("Drafting with {}…", provider.as_str());
        let draft_tone = tone.clone();
        cx.spawn(async move |this, cx| {
            let (note_tx, note_rx) = std::sync::mpsc::channel();
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _guard = termy_x::trends_desk::bind_retry_notice(Some(note_tx));
                let limit = if provider == TakeProvider::Hermes || provider == TakeProvider::Grok {
                    termy_x::trends_desk::HERMES_TAKE_TIMEOUT
                } else {
                    termy_x::trends_desk::TAKE_TIMEOUT
                };
                let http = termy_x::io::UreqHttp::with_timeout(limit);
                let draft = termy_x::trends_desk::run_take_chain(
                    provider,
                    &config,
                    &post,
                    &http,
                    &draft_tone,
                );
                let _ = done_tx.send(draft);
                drop(_guard);
            });
            let draft = loop {
                match done_rx.try_recv() {
                    Ok(draft) => break draft,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        break termy_x::trends_desk::TakeDraft {
                            text: String::new(),
                            source: "offline",
                            note: "take stopped".into(),
                            variants: Vec::new(),
                        };
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        while let Ok(msg) = note_rx.try_recv() {
                            let _ = this.update(cx, |desk, cx| {
                                desk.action_note = msg;
                                cx.notify();
                            });
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(150))
                            .await;
                    }
                }
            };
            let _ = previous;
            let _ = this.update(cx, |desk, cx| {
                let next = if !draft.text.trim().is_empty() {
                    draft.text.clone()
                } else if previous_text != termy_x::trends_desk::DRAFTING_PLACEHOLDER
                    && !previous_text.trim().is_empty()
                {
                    previous_text.clone()
                } else {
                    String::new()
                };
                if let Some(open) = desk.popout.as_mut().filter(|item| item.card_id == id) {
                    open.variants = draft.variants.clone();
                    open.draft = next.clone();
                    desk.pop_caret = termy_x::draft_edit::DraftCaret::at_end(next.len());
                    desk.reply_edit = Some(termy_x::draft_edit::ReplyEditor::new(&next));
                    desk.card_replies.insert(
                        id.clone(),
                        termy_x::trends_desk::ReplyPhase::Editing { text: next },
                    );
                }
                desk.action_note = draft.note.clone();
                desk.model_notice = desk.action_note.clone();
                termy_x::trends_desk::append_desk_log(
                    &desk.data_root,
                    &termy_x::trends_desk::take_log_line(
                        draft.source,
                        if draft.text.trim().is_empty() {
                            draft.note.as_str()
                        } else {
                            "ok"
                        },
                    ),
                );
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn focus_root_after_click(
        &mut self,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            handle.focus(window, cx);
        });
    }

    pub(crate) fn run_live_search(&mut self, cx: &mut Context<Self>) {
        let term = self.filter_text.trim().to_string();
        if term.is_empty() {
            return;
        }
        self.model_notice = format!("Search: {term}");
        self.columns.retain(|column| column.topic.id != "search");
        let topic = termy_x::trends_desk::Topic {
            id: "search".into(),
            name: format!("Search: {term}"),
            desc: term.clone(),
        };
        self.columns.insert(
            0,
            crate::trends_desk::ColumnUi {
                topic: topic.clone(),
                view: termy_x::trends_desk::column_loading(&term),
                cached: None,
                tags: Vec::new(),
            },
        );
        self.settings.subject_id = "search".into();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let runner = std::sync::Arc::new(termy_x::io::SystemRunner);
                    let desk = termy_x::trends_desk::OpenCliDesk::new(&config, runner)
                        .with_hermes_dir(None);
                    desk.run_topic(&topic, "", &std::sync::atomic::AtomicBool::new(false))
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                if let Some(column) = desk
                    .columns
                    .iter_mut()
                    .find(|column| column.topic.id == "search")
                {
                    column.view = termy_x::trends_desk::column_from_outcome(&outcome, None);
                }
                desk.model_notice = format!("Search: {term} · {}", outcome.message);
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn set_take_provider(&mut self, name: &str, cx: &mut Context<Self>) {
        self.remember_provider(name);
        self.free_halted = false;
        self.takes_halted = false;
        termy_x::trends_desk::resume_takes();
        let provider = TakeProvider::parse(name);
        self.action_note = format!("Next take uses {}.", provider.as_str());
        if provider == TakeProvider::Hermes {
            self.probe_hermes_models(cx);
        }
        cx.notify();
    }

    pub(crate) fn save_hermes(&mut self, cx: &mut Context<Self>) {
        let path = self.data_root.join("x.toml");
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        let next = write_hermes_settings(
            &current,
            self.hermes_base_text.trim(),
            self.hermes_model_text.trim(),
            self.hermes_key_text.trim(),
        );
        self.config.hermes_base_url = self.hermes_base_text.trim().to_string();
        self.config.hermes_model = self.hermes_model_text.trim().to_string();
        self.config.hermes_api_key = self.hermes_key_text.trim().to_string();
        if self.writable {
            if let Err(error) = std::fs::write(&path, next) {
                self.draft_note = format!("Hermes settings stayed in this window: {error}");
                cx.notify();
                return;
            }
        }
        self.draft_note = "Saved Hermes settings in Termy XL. The key is not logged.".into();
        self.probe_hermes_models(cx);
        cx.notify();
    }

    pub(crate) fn open_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let work = termy_x::trends_desk::agent_work_dir(&self.data_root);
        let _ = std::fs::create_dir_all(&work);
        let configured = self.config.agent_command(agent);
        let exe_text = local_hermes_exe().unwrap_or_default();
        match resolve_launch(
            agent,
            configured,
            &command_exists,
            Some(exe_text.as_str()).filter(|value| !value.is_empty()),
        ) {
            Ok(launch) => {
                let shown = termy_x::trends_desk::interactive_command(&launch.program).join(" ");
                let mut pane = AgentPane::open(agent, &shown);
                match spawn_visible(&launch, &work) {
                    Ok(note) => {
                        self.agent_proc = None;
                        pane.notice = if agent == Agent::Muse {
                            termy_x::trends_desk::muse_trust_line(&work)
                        } else {
                            note
                        };
                    }
                    Err(error) => {
                        self.agent_proc = None;
                        pane.notice = error;
                    }
                }
                self.agent_seen = 0;
                self.agent_pane = Some(pane);
                self.agent_input = false;
                self.watch_agent(cx);
            }
            Err(error) => {
                self.agent_proc = None;
                self.agent_seen = 0;
                self.agent_pane = Some(AgentPane::missing(agent, &error));
                self.agent_input = false;
            }
        }
    }

    fn watch_agent(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(400))
                    .await;
                let alive = this.update(cx, |desk, cx| {
                    let before = desk
                        .agent_pane
                        .as_ref()
                        .map(|pane| pane.scrollback.len())
                        .unwrap_or(0);
                    desk.pull_agent_output();
                    let after = desk
                        .agent_pane
                        .as_ref()
                        .map(|pane| pane.scrollback.len())
                        .unwrap_or(0);
                    if after != before {
                        cx.notify();
                    }
                    desk.agent_proc.is_some()
                });
                if !matches!(alive, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn submit_agent_line(&mut self) {
        let Some(pane) = self.agent_pane.as_mut() else {
            return;
        };
        let line = pane.submit();
        if line.is_empty() {
            return;
        }
        if let Some(proc) = self.agent_proc.as_mut() {
            proc.write_line(&line);
        }
    }

    pub(crate) fn pull_agent_output(&mut self) {
        let Some(proc) = self.agent_proc.as_ref() else {
            return;
        };
        let (next, extra) = proc.take_new(self.agent_seen);
        if extra.is_empty() {
            return;
        }
        self.agent_seen = next;
        if let Some(pane) = self.agent_pane.as_mut() {
            pane.append_output(&extra);
        }
    }

    pub(crate) fn use_agent_draft(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.agent_pane.as_ref().map(|pane| pane.use_as_draft()) else {
            return;
        };
        if text.trim().is_empty() {
            self.draft_note = "The agent pane has no draft text yet.".into();
            cx.notify();
            return;
        }
        if let Some(pop) = self.popout.as_mut() {
            pop.draft = text.clone();
            self.pop_caret = termy_x::draft_edit::DraftCaret::at_end(text.len());
            self.card_replies.insert(
                pop.card_id.clone(),
                termy_x::trends_desk::edit_reply(&termy_x::trends_desk::ReplyPhase::Idle, text),
            );
        }
        self.draft_note =
            "Copied the agent pane into the draft. Send is still a separate click.".into();
        cx.notify();
    }

    pub(crate) fn draft_with_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let Some(pop) = self.popout.clone() else {
            return;
        };
        let Some(post) = self
            .columns
            .iter()
            .flat_map(|column| column.view.posts.iter())
            .find(|post| post.id == pop.card_id)
            .cloned()
        else {
            self.draft_note = "That card has no post to draft from.".into();
            cx.notify();
            return;
        };
        let prompt = reply_prompt(&post.author, &post.text, &pop.tone);
        let configured = self.config.agent_command(agent).map(str::to_string);
        let exe_text = local_hermes_exe().unwrap_or_default();
        let saved_key = self.hermes_key_text.clone();
        let base = self.hermes_base_text.clone();
        let model = self.hermes_model_text.clone();
        let card = pop.card_id.clone();
        let work = termy_x::trends_desk::agent_work_dir(&self.data_root);
        let _ = std::fs::create_dir_all(&work);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let launch = resolve_launch(
                        agent,
                        configured.as_deref(),
                        &command_exists,
                        Some(exe_text.as_str()).filter(|value| !value.is_empty()),
                    );
                    let env_text = hermes_env_text();
                    let auth: HermesAuth = hermes_auth(&base, &model, &saved_key, &env_text);
                    let route = plan_agent_draft(
                        agent,
                        launch.as_ref().map_err(String::as_str),
                        Some(&auth),
                        &prompt,
                    );
                    execute_draft(
                        route,
                        &SystemCli { cwd: work.clone() },
                        &termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(45)),
                    )
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                let (mut lines, mut notice) = variants_from_draft(&post.text, result);
                if agent == termy_x::agents::Agent::Agy
                    && let Some(error) =
                        termy_x::trends_desk::agy_credit_error(&notice).or_else(|| {
                            lines
                                .iter()
                                .find_map(|line| termy_x::trends_desk::agy_credit_error(line))
                        })
                {
                    lines.clear();
                    notice = error;
                }
                if agent == termy_x::agents::Agent::Muse
                    && termy_x::trends_desk::muse_needs_trust(&notice)
                {
                    notice = termy_x::trends_desk::muse_trust_line(
                        &termy_x::trends_desk::agent_work_dir(&desk.data_root),
                    );
                }
                if let Some(pop) = desk.popout.as_mut().filter(|pop| pop.card_id == card) {
                    pop.variants = lines;
                    if let Some(first) = pop.variants.first().cloned() {
                        if pop.draft.trim().is_empty() {
                            pop.draft = first.clone();
                            desk.pop_caret = termy_x::draft_edit::DraftCaret::at_end(first.len());
                        }
                    }
                }
                if notice.is_empty() {
                    desk.draft_note = format!(
                        "Drafted 3 replies with {}. Nothing was posted.",
                        agent.label()
                    );
                } else {
                    desk.draft_note = notice.clone();
                }
                if let Some(pane) = desk.agent_pane.as_mut() {
                    if !notice.is_empty() {
                        pane.notice = notice;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn agent_controls(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut column = div().flex().flex_col().gap(px(6.0));
        let mut open_row = div().flex().gap(px(6.0)).flex_wrap();
        let mut draft_row = div().flex().gap(px(6.0)).flex_wrap();
        for agent in Agent::ALL {
            let open_id = SharedString::from(format!("desk-agent-open-{}", agent.tag()));
            let draft_id = SharedString::from(format!("desk-agent-draft-{}", agent.tag()));
            open_row = open_row.child(agent_button(
                open_id,
                format!("Open {}", agent.label()),
                agent,
                cx,
                |desk, agent, cx| desk.open_agent(agent, cx),
            ));
            draft_row = draft_row.child(agent_button(
                draft_id,
                format!("Draft with {}", agent.label()),
                agent,
                cx,
                |desk, agent, cx| desk.draft_with_agent(agent, cx),
            ));
        }
        column = column.child(open_row).child(draft_row);
        if let Some(pane) = &self.agent_pane {
            let notice = pane.notice.clone();
            let scroll = pane.scrollback.clone();
            let input = pane.input.clone();
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .p(px(8.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x10141c))
                    .border_1()
                    .border_color(rgb(0xff4fa3))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0xff4fa3))
                            .child(format!("{} pane · {}", pane.agent, pane.command)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0xffd60a))
                            .child(notice.clone()),
                    )
                    .child(
                        div()
                            .id("desk-agent-scroll")
                            .h(px(96.0))
                            .overflow_y_scroll()
                            .text_size(px(12.0))
                            .whitespace_normal()
                            .child(scroll),
                    )
                    .child(
                        div()
                            .id("desk-agent-input")
                            .p(px(6.0))
                            .rounded(px(6.0))
                            .bg(rgb(0x0c1422))
                            .text_size(px(12.0))
                            .cursor_pointer()
                            .child(if notice.contains("terminal window") {
                                "Type in the terminal window that opened.".to_string()
                            } else if input.is_empty() {
                                "Click to type in the agent pane".to_string()
                            } else {
                                format!("{input}|")
                            })
                            .on_mouse_down(
                                gpui_kit::MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.agent_input = true;
                                    cx.stop_propagation();
                                    cx.notify();
                                }),
                            ),
                    )
                    .child(agent_button(
                        SharedString::from("desk-agent-use"),
                        "Use as draft".to_string(),
                        Agent::Grok,
                        cx,
                        |desk, _, cx| desk.use_agent_draft(cx),
                    )),
            );
        }
        column.into_any_element()
    }
}

fn spawn_visible(launch: &Launch, cwd: &std::path::Path) -> Result<String, String> {
    let program = launch.program.clone();
    let args = launch.args.clone();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut attempts: Vec<std::process::Command> = Vec::new();
        let mut wt = Command::new("wt.exe");
        wt.args(["-w", "0", "-d"])
            .arg(cwd)
            .arg(&program)
            .args(&args);
        attempts.push(wt);
        let mut conhost = Command::new("conhost.exe");
        conhost.arg(&program).args(&args).current_dir(cwd);
        attempts.push(conhost);
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/k", &program]).args(&args).current_dir(cwd);
        cmd.creation_flags(0x0000_0010);
        attempts.push(cmd);
        let mut last = format!("{program} is missing");
        for mut command in attempts {
            match command.spawn() {
                Ok(_) => {
                    return Ok(format!(
                        "Opened {program} in a terminal window. cwd {}",
                        cwd.display()
                    ));
                }
                Err(error) => last = error.to_string(),
            }
        }
        return Err(last);
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new(&program);
        command.args(&args).current_dir(cwd);
        command
            .spawn()
            .map_err(|error| format!("{program} is missing: {error}"))?;
        Ok(format!("Started {program}. cwd {}", cwd.display()))
    }
}

fn command_exists(name: &str) -> bool {
    use termy_x::io::CommandRunner;
    CommandRunner::exists(&termy_x::io::SystemRunner, name)
}

fn local_hermes_exe() -> Option<String> {
    let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    known_hermes_exe(local.as_deref()).map(|path| path.display().to_string())
}

fn hermes_env_text() -> String {
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return String::new();
    };
    let path = std::path::PathBuf::from(root).join("hermes").join(".env");
    std::fs::read_to_string(path).unwrap_or_default()
}

fn agent_button(
    id: SharedString,
    label: String,
    agent: Agent,
    cx: &mut Context<TrendsDesk>,
    handler: impl Fn(&mut TrendsDesk, Agent, &mut Context<TrendsDesk>) + 'static,
) -> gpui_kit::AnyElement {
    div()
        .id(id)
        .px(px(8.0))
        .py(px(4.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(0xff4fa3))
        .bg(rgb(0x2a0f1a))
        .text_size(px(11.0))
        .text_color(rgb(0xffd6ea))
        .cursor_pointer()
        .child(label)
        .on_mouse_down(
            gpui_kit::MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                handler(this, agent, cx);
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .into_any_element()
}
