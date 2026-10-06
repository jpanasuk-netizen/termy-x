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
}
