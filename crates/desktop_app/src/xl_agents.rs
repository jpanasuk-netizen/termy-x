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
    AgentPane, HermesAuth, Launch, PromptCli, execute_draft, hermes_auth, hermes_health_request,
    hermes_pill, known_hermes_exe, plan_agent_draft, reply_prompt, resolve_launch,
    seed_agent_block, variants_from_draft, write_hermes_settings,
};

pub(crate) struct AgentProc {
    stdin: Option<ChildStdin>,
    child: Child,
    output: Arc<Mutex<String>>,
}

impl Drop for AgentProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl AgentProc {
    fn spawn(launch: &Launch) -> Result<Self, String> {
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
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

struct SystemCli;

impl PromptCli for SystemCli {
    fn run(&self, program: &str, args: &[String], stdin_text: &str) -> Result<String, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
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

    pub(crate) fn spawn_hermes_health(&mut self, cx: &mut Context<Self>) {
        let base = self.hermes_base_text.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let http =
                        termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(3));
                    http.request(hermes_health_request(&base)).map(|_| ())
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                desk.hermes_pill = hermes_pill(result);
                cx.notify();
            });
        })
        .detach();
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
        self.spawn_hermes_health(cx);
        cx.notify();
    }

    pub(crate) fn open_agent(&mut self, agent: Agent, cx: &mut Context<Self>) {
        let configured = self.config.agent_command(agent);
        let exe_text = local_hermes_exe().unwrap_or_default();
        match resolve_launch(
            agent,
            configured,
            &command_exists,
            Some(exe_text.as_str()).filter(|value| !value.is_empty()),
        ) {
            Ok(launch) => {
                let mut pane = AgentPane::open(agent, &launch.display);
                match AgentProc::spawn(&launch) {
                    Ok(proc) => {
                        self.agent_proc = Some(proc);
                        pane.notice = format!(
                            "Agent pane · {}. Nothing is posted from here.",
                            agent.label()
                        );
                    }
                    Err(error) => {
                        self.agent_proc = None;
                        pane.notice = error;
                    }
                }
                self.agent_seen = 0;
                self.agent_pane = Some(pane);
                self.agent_input = true;
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
                        &SystemCli,
                        &termy_x::io::UreqHttp::with_timeout(std::time::Duration::from_secs(45)),
                    )
                })
                .await;
            let _ = this.update(cx, |desk, cx| {
                let (lines, notice) = variants_from_draft(&post.text, result);
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
                            .child(notice),
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
                            .child(if input.is_empty() {
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
