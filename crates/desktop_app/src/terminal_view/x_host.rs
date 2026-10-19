use super::*;
use termy::x_panel::{XPanel, XSection, edit_action_goes_to_panel};

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
            self.x_panel = Some(cx.new(|cx| XPanel::new(cx, false)));
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
                        // Compose, Search, or the open inline reply — never
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
