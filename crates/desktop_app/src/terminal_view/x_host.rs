use super::*;
use termy::x_panel::{XAskEvent, XPanel, XSection, edit_action_goes_to_panel};
use termy_x::agents::{self, Agent, PaneInfo, PanePick};

impl TerminalView {
    pub(super) fn toggle_x_panel(&mut self, cx: &mut Context<Self>) {
        self.ensure_x_panel(cx);
        self.x_panel_open = !self.x_panel_open;
        cx.notify();
    }

    pub(super) fn open_x_section(&mut self, section: XSection, cx: &mut Context<Self>) {
        self.ensure_x_panel(cx);
        self.x_panel_open = true;
        if let Some(panel) = &self.x_panel {
            panel.update(cx, |panel, cx| {
                panel.open_section(section, cx);
                cx.notify();
            });
        }
        cx.notify();
    }

    /// Horizontal space the docked X panel takes from the terminal
    /// content area while open (zero when closed). The terminal pane
    /// layout subtracts this so the CLI grid ends at the panel edge
    /// instead of extending underneath it.
    pub(super) fn x_panel_reserved_width(&self) -> f32 {
        if self.x_panel_open && self.x_panel.is_some() {
            termy::x_panel::PANEL_DOCKED_WIDTH
        } else {
            0.0
        }
    }

    fn ensure_x_panel(&mut self, cx: &mut Context<Self>) {
        if self.x_panel.is_none() {
            let panel = cx.new(|cx| XPanel::new(cx, false));
            cx.subscribe(&panel, |view, panel, event: &XAskEvent, cx| {
                view.on_x_ask(panel, event, cx);
            })
            .detach();
            self.x_panel = Some(panel);
        }
    }

    /// A panel text field (Compose, Search, inline reply, bearer key)
    /// has focus: route Paste/Copy/SelectAll into the X panel, never the shell.
    /// Returns true when the action was handled (or intentionally swallowed).
    pub(super) fn x_panel_handle_edit_action(
        &mut self,
        action: CommandAction,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.x_panel_open {
            return false;
        }
        let Some(panel) = self.x_panel.clone() else {
            return false;
        };
        // Focus decides (Jeremy's rule): with the terminal focused the
        // shell owns Ctrl+C (interrupt) even while the panel is open;
        // only a focused panel's editors may intercept edit chords.
        let (focused, owns) = {
            let panel = panel.read(cx);
            (panel.is_focused(window), panel.owns_edit_keys())
        };
        if !edit_action_goes_to_panel(self.x_panel_open, focused, owns) {
            return false;
        }
        match action {
            CommandAction::Paste => {
                let clip = cx.read_from_clipboard().and_then(|item| item.text());
                if let Some(text) = clip {
                    panel.update(cx, |panel, cx| {
                        // Idempotent with KeyDown path: insert once here if the
                        // panel did not already receive the chord. Routes to
                        // Compose, Search, or the open inline reply ΓÇö never
                        // the shell behind the panel.
                        panel.paste_active_text(&text);
                        cx.notify();
                    });
                }
                true
            }
            CommandAction::Copy => {
                panel.update(cx, |panel, cx| {
                    if let Some(selected) = panel.active_copy_text() {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(selected));
                    }
                });
                true
            }
            CommandAction::SelectAll => {
                panel.update(cx, |panel, cx| {
                    panel.select_all_active();
                    cx.notify();
                });
                true
            }
            _ => false,
        }
    }
}

/// Ask here host state: which pane got the last prompt, and panes Termy
/// started for an agent.
#[derive(Default)]
pub(super) struct XAskHost {
    ask_pane: Option<String>,
    launched: std::collections::HashMap<String, Agent>,
}

const SHELLS: &[&str] = &[
    "pwsh",
    "powershell",
    "cmd",
    "bash",
    "zsh",
    "sh",
    "fish",
    "nu",
    "wsl",
    "conhost",
];

fn is_shell(process: Option<&str>) -> bool {
    process.is_none_or(|process| {
        let base = process
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(process)
            .to_ascii_lowercase();
        let stem = base.strip_suffix(".exe").unwrap_or(&base).to_string();
        SHELLS.contains(&stem.as_str())
    })
}

/// Plain text of the last `max_lines` lines of a pane (scrollback included).
fn pane_text(terminal: &Terminal, max_lines: i32) -> String {
    let Some((first, last)) = terminal.line_bounds() else {
        return String::new();
    };
    let start = (last - max_lines + 1).max(first);
    let mut rows: Vec<(i32, String)> = Vec::new();
    terminal.for_each_line_cell_range(start, last, |_range, line, _col, cell| {
        if rows.last().is_none_or(|(row, _)| *row != line) {
            rows.push((line, String::new()));
        }
        let Some((_, text)) = rows.last_mut() else {
            return;
        };
        if cell.is_wide_spacer() || cell.is_trailing_wide_spacer() {
            return;
        }
        let character = cell.character();
        text.push(
            if character == '\0' || character.is_control() || cell.is_hidden() {
                ' '
            } else {
                character
            },
        );
        cell.append_combining_to(text);
    });
    rows.into_iter()
        .map(|(_, text)| text.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Bracketed-paste frame (when the pane asked for it); ESC is stripped from
/// the payload so a prompt can never end the paste early.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let body: String = text
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| *c != '\u{1b}')
        .map(|c| if c == '\n' { '\r' } else { c })
        .collect();
    if bracketed {
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(body.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        body.into_bytes()
    }
}

impl TerminalView {
    fn x_pane_infos(&self) -> Vec<PaneInfo> {
        let Some(tab) = self.active_tab_ref() else {
            return Vec::new();
        };
        let active = tab.active_pane_id().map(str::to_string);
        let title = format!(
            "{} {}",
            tab.title,
            tab.current_command.as_deref().unwrap_or_default()
        );
        tab.panes
            .iter()
            .map(|pane| {
                let terminal = pane.terminal();
                let focused = active.as_deref() == Some(pane.id.as_str());
                PaneInfo {
                    id: pane.id.clone(),
                    process: terminal
                        .child_pid()
                        .and_then(termy_core::foreground_process_name_of_pid),
                    focused,
                    launched_for: self.x_ask.launched.get(&pane.id).copied(),
                    // The tab title describes the focused pane only.
                    title: if focused {
                        title.clone()
                    } else {
                        String::new()
                    },
                    tail: pane_text(terminal, 8),
                }
            })
            .collect()
    }

    fn x_send_prompt(
        &self,
        pane_id: String,
        prompt: String,
        delay: std::time::Duration,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let _ = this.update(cx, |view, _| {
                let bracketed = view
                    .pane_terminal_by_id(&pane_id)
                    .is_some_and(|terminal| terminal.bracketed_paste_mode());
                view.send_input_to_pane(&pane_id, &paste_bytes(&prompt, bracketed));
            });
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let _ = this.update(cx, |view, _| {
                view.send_input_to_pane(&pane_id, b"\r");
            });
        })
        .detach();
    }

    fn on_x_ask(
        &mut self,
        panel: gpui_kit::Entity<XPanel>,
        event: &XAskEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            XAskEvent::Probe => {
                let running = agents::detect_running(&self.x_pane_infos());
                panel.update(cx, |panel, cx| panel.rg_set_running(running, cx));
            }
            XAskEvent::Ask {
                agent,
                command,
                prompt,
            } => {
                let agent = *agent;
                let label = agent.label();
                let panes = self.x_pane_infos();
                let startup = std::time::Duration::from_secs(4);
                let message = match agents::pick_pane(agent, &panes) {
                    PanePick::Agent(id) => {
                        self.x_send_prompt(id.clone(), prompt.clone(), Default::default(), cx);
                        self.x_ask.ask_pane = Some(id);
                        format!("Sent to {label} in its pane.")
                    }
                    PanePick::Focused(id) => {
                        let process = panes
                            .iter()
                            .find(|pane| pane.id == id)
                            .and_then(|pane| pane.process.clone());
                        if is_shell(process.as_deref()) {
                            // Never paste a prompt into a bare shell: start
                            // the agent there first.
                            let Some(command) = command.clone() else {
                                panel.update(cx, |panel, cx| {
                                    panel.rg_ask_notice(agent.install_hint().into(), cx);
                                });
                                return;
                            };
                            self.send_input_to_pane(&id, format!("{command}\r").as_bytes());
                            self.x_ask.launched.insert(id.clone(), agent);
                            self.x_send_prompt(id.clone(), prompt.clone(), startup, cx);
                            self.x_ask.ask_pane = Some(id);
                            format!(
                                "Started {label} in the focused pane; prompt follows in a few seconds."
                            )
                        } else {
                            self.x_send_prompt(id.clone(), prompt.clone(), Default::default(), cx);
                            self.x_ask.ask_pane = Some(id);
                            format!("Sent to {label} in the focused pane.")
                        }
                    }
                    PanePick::NewPane => {
                        let Some(command) = command.clone() else {
                            panel.update(cx, |panel, cx| {
                                panel.rg_ask_notice(agent.install_hint().into(), cx);
                            });
                            return;
                        };
                        let launch = TerminalLaunch::ShellCommand(command);
                        if !self.split_active_pane_vertical_with_launch(None, Some(&launch), cx) {
                            panel.update(cx, |panel, cx| {
                                panel.rg_ask_notice(
                                    format!("Could not open a pane for {label}."),
                                    cx,
                                );
                            });
                            return;
                        }
                        let Some(id) = self.active_pane_id().map(str::to_string) else {
                            return;
                        };
                        self.x_ask.launched.insert(id.clone(), agent);
                        self.x_send_prompt(id.clone(), prompt.clone(), startup, cx);
                        self.x_ask.ask_pane = Some(id);
                        format!("Started {label} in a new pane; prompt follows in a few seconds.")
                    }
                };
                panel.update(cx, |panel, cx| panel.rg_ask_notice(message, cx));
                cx.notify();
            }
            XAskEvent::Grab { marker, quiet } => {
                let terminal = self
                    .x_ask
                    .ask_pane
                    .as_deref()
                    .and_then(|id| self.pane_terminal_by_id(id))
                    .or_else(|| self.active_terminal());
                let text = terminal
                    .map(|terminal| pane_text(terminal, 3000))
                    .unwrap_or_default();
                let (marker, quiet) = (marker.clone(), *quiet);
                panel.update(cx, |panel, cx| panel.rg_grabbed(&text, &marker, quiet, cx));
            }
        }
    }
}

#[cfg(test)]
mod ask_here_tests {
    use super::{is_shell, paste_bytes};

    #[test]
    fn shells_are_never_pasted_into() {
        assert!(is_shell(Some("pwsh.exe")));
        assert!(is_shell(Some(r"C:\Windows\System32\cmd.exe")));
        assert!(is_shell(None));
        assert!(!is_shell(Some("grok.exe")));
        assert!(!is_shell(Some("vim.exe")));
    }

    #[test]
    fn paste_is_framed_and_escape_free() {
        let bytes = paste_bytes("a\r\nb\x1b[201~c", true);
        assert_eq!(bytes, b"\x1b[200~a\rb[201~c\x1b[201~".to_vec());
        assert_eq!(paste_bytes("a\nb", false), b"a\rb".to_vec());
    }
}
