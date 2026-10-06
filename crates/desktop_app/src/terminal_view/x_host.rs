use super::*;
use termy::x_panel::{XPanel, XSection};

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

    fn ensure_x_panel(&mut self, cx: &mut Context<Self>) {
        if self.x_panel.is_none() {
            self.x_panel = Some(cx.new(|cx| XPanel::new(cx, false)));
        }
    }

    /// Compose has focus: route Paste/Copy/SelectAll into the X panel, never the shell.
    /// Returns true when the action was handled (or intentionally swallowed).
    pub(super) fn x_panel_handle_edit_action(
        &mut self,
        action: CommandAction,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.x_panel_open {
            return false;
        }
        let Some(panel) = self.x_panel.clone() else {
            return false;
        };
        if !panel.read(cx).owns_compose_keys() {
            return false;
        }
        match action {
            CommandAction::Paste => {
                let clip = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text().map(|s| s.to_string()));
                if let Some(text) = clip {
                    panel.update(cx, |panel, cx| {
                        // Idempotent with KeyDown path: insert once here if the
                        // panel did not already receive the chord.
                        panel.focus_compose_editor_public();
                        panel.insert_compose_text(&text);
                        cx.notify();
                    });
                }
                true
            }
            CommandAction::Copy => {
                panel.update(cx, |panel, cx| {
                    if let Some(selected) = panel.compose_selection_text() {
                        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(selected));
                    }
                });
                true
            }
            CommandAction::SelectAll => {
                panel.update(cx, |panel, cx| {
                    panel.select_all_compose();
                    cx.notify();
                });
                true
            }
            _ => false,
        }
    }
}
