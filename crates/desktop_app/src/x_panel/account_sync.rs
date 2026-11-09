//! Optional account sync on the Help tab.
//!
//! Email and password stay in this view. The password is cleared as soon
//! as a button runs, and it is never written to disk. Network work is
//! spawned so the panel stays live.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    None,
    Email,
    Password,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    SignIn,
    SignUp,
    Recover,
    SignOut,
}

pub struct UiState {
    email: String,
    password: String,
    email_caret: usize,
    password_caret: usize,
    focus: Focus,
    status: String,
    busy: bool,
}

impl UiState {
    pub fn from_config(config: &termy_x::config::XConfig) -> Self {
        Self {
            email: config.supabase_email.clone(),
            password: String::new(),
            email_caret: config.supabase_email.len(),
            password_caret: 0,
            focus: Focus::None,
            status: termy_x::supabase_sync::settings_summary(config)
                .lines()
                .next()
                .unwrap_or("Supabase sync: off")
                .trim_start_matches("Supabase sync: ")
                .to_string(),
            busy: false,
        }
    }
}

impl XPanel {
    pub(super) fn launch_account_sync(&mut self, cx: &mut Context<Self>) {
        self.spawn_sync_job(Job::SignIn, true, cx);
    }

    pub(super) fn account_sync_section(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let email_on = self.account_sync.focus == Focus::Email;
        let password_on = self.account_sync.focus == Focus::Password;
        div()
            .p(px(10.0))
            .rounded(px(10.0))
            .bg(rgb(0x1c1c1e))
            .border_1()
            .border_color(rgb(0x3a3a3c))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf5f5f7))
                    .child("Account sync (optional)"),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x9ad7ff))
                    .child(self.account_sync.status.clone()),
            )
            .child(self.sync_field(
                "sync-email",
                "Email",
                &self.account_sync.email,
                self.account_sync.email_caret,
                email_on,
                false,
                cx,
            ))
            .child(self.sync_field(
                "sync-password",
                "Password",
                &self.account_sync.password,
                self.account_sync.password_caret,
                password_on,
                true,
                cx,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .flex_wrap()
                    .child(x_button!(
                        "sync-sign-in",
                        "Sign in",
                        cx.listener(|this, _, _, cx| {
                            this.spawn_sync_job(Job::SignIn, false, cx);
                        })
                    ))
                    .child(x_button!(
                        "sync-sign-up",
                        "Create account",
                        cx.listener(|this, _, _, cx| {
                            this.spawn_sync_job(Job::SignUp, false, cx);
                        })
                    ))
                    .child(x_button!(
                        "sync-recover",
                        "Forgot password",
                        cx.listener(|this, _, _, cx| {
                            this.spawn_sync_job(Job::Recover, false, cx);
                        })
                    ))
                    .child(x_button!(
                        "sync-sign-out",
                        "Sign out",
                        cx.listener(|this, _, _, cx| {
                            this.spawn_sync_job(Job::SignOut, false, cx);
                        })
                    )),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0xaeaeb2))
                    .child("Uses the hosted sync project unless x.toml sets another URL. The password is not saved. This PC keeps its own likes and follows if sync is off."),
            )
            .into_any_element()
    }

    pub(super) fn account_sync_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.section != XSection::Help || self.account_sync.focus == Focus::None {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;
        if key == "escape" {
            self.account_sync.focus = Focus::None;
        } else if key == "tab" {
            self.account_sync.focus = if self.account_sync.focus == Focus::Email {
                Focus::Password
            } else {
                Focus::Email
            };
        } else if key == "enter" {
            self.spawn_sync_job(Job::SignIn, false, cx);
        } else if key == "backspace" {
            self.sync_backspace();
        } else if !(mods.control || mods.alt || mods.platform || mods.function)
            && let Some(ch) = event.keystroke.key_char.as_deref()
            && !ch.is_empty()
            && ch != "\n"
            && ch != "\t"
        {
            self.sync_insert(ch);
        } else {
            return false;
        }
        cx.stop_propagation();
        cx.notify();
        true
    }

    fn sync_field(
        &self,
        id: &'static str,
        label: &str,
        text: &str,
        caret: usize,
        focused: bool,
        mask: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let shown = if mask {
            "•".repeat(text.chars().count())
        } else {
            text.to_string()
        };
        let shown = with_caret(&shown, caret, focused);
        let focus = if mask { Focus::Password } else { Focus::Email };
        div()
            .id(id)
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if focused { rgb(0x0a84ff) } else { rgb(0x3a3a3c) })
            .bg(rgb(0x2c2c2e))
            .cursor_text()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.account_sync.focus = focus;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_size(px(11.0))
                    .text_color(rgb(0xaeaeb2))
                    .child(label.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(rgb(0xf5f5f7))
                    .child(if shown.is_empty() {
                        if mask { "Password".to_string() } else { "you@example.com".to_string() }
                    } else {
                        shown
                    }),
            )
            .into_any_element()
    }

    fn sync_insert(&mut self, text: &str) {
        let (buf, caret) = self.sync_buf();
        let at = floor_char(buf, *caret);
        buf.insert_str(at, text);
        *caret = at + text.len();
    }

    fn sync_backspace(&mut self) {
        let (buf, caret) = self.sync_buf();
        let at = floor_char(buf, *caret);
        if at == 0 {
            return;
        }
        let mut prev = at - 1;
        while prev > 0 && !buf.is_char_boundary(prev) {
            prev -= 1;
        }
        buf.replace_range(prev..at, "");
        *caret = prev;
    }

    fn sync_buf(&mut self) -> (&mut String, &mut usize) {
        if self.account_sync.focus == Focus::Password {
            (
                &mut self.account_sync.password,
                &mut self.account_sync.password_caret,
            )
        } else {
            (
                &mut self.account_sync.email,
                &mut self.account_sync.email_caret,
            )
        }
    }

    fn spawn_sync_job(&mut self, job: Job, launch: bool, cx: &mut Context<Self>) {
        if self.account_sync.busy {
            return;
        }
        if launch && self.fill_window {
            return;
        }
        let email = self.account_sync.email.trim().to_string();
        let password = if launch {
            String::new()
        } else {
            std::mem::take(&mut self.account_sync.password)
        };
        self.account_sync.password_caret = 0;
        if !launch && matches!(job, Job::SignIn | Job::SignUp) && (email.is_empty() || password.is_empty()) {
            self.account_sync.status = "Enter an email and a password. Nothing was sent.".into();
            cx.notify();
            return;
        }
        if !launch && job == Job::Recover && email.is_empty() {
            self.account_sync.status = "Enter the email for that account. Nothing was sent.".into();
            cx.notify();
            return;
        }
        self.account_sync.busy = true;
        if !launch {
            self.account_sync.status = "syncing".into();
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let done = cx
                .background_executor()
                .spawn(async move { run_job(job, launch, email, password) })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.account_sync.busy = false;
                panel.account_sync.status = done.line;
                if let Some(outcome) = done.synced {
                    let mut changed = false;
                    for row in &outcome.toggles {
                        changed |= panel.action_db.merge_cloud_toggle(row);
                    }
                    for row in &outcome.receipts {
                        changed |= panel.action_db.merge_cloud_receipt(row);
                    }
                    if changed {
                        panel.hydrate_actions();
                        panel.persist_actions();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

struct Done {
    line: String,
    synced: Option<termy_x::supabase_sync::SyncOutcome>,
}

fn run_job(job: Job, launch: bool, email: String, password: String) -> Done {
    let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
    let http = termy_x::io::UreqHttp::default();
    if launch {
        let db = termy_x::action_state::ActionDb::load(&config.action_state_path()).unwrap_or_default();
        let outcome = termy_x::supabase_auth::refresh_and_sync(&config, &http, &db, true);
        return Done {
            line: outcome.status.label(),
            synced: Some(outcome),
        };
    }
    let result = match job {
        Job::SignIn => termy_x::supabase_auth::sign_in_store(&config, &http, &email, &password),
        Job::SignUp => termy_x::supabase_auth::sign_up(&config, &http, &email, &password),
        Job::Recover => termy_x::supabase_auth::recover(&config, &http, &email),
        Job::SignOut => Ok(termy_x::supabase_auth::sign_out(&config, &http)),
    };
    drop(password);
    match result {
        Ok(termy_x::supabase_auth::AuthOutcome::SignedIn) => {
            let db = termy_x::action_state::ActionDb::load(&config.action_state_path()).unwrap_or_default();
            let outcome = termy_x::supabase_sync::sync(&config, &http, &db, true);
            Done {
                line: outcome.status.label(),
                synced: Some(outcome),
            }
        }
        Ok(outcome) => Done {
            line: termy_x::supabase_auth::outcome_text(&outcome),
            synced: None,
        },
        Err(error) => Done {
            line: format!("error: {error}"),
            synced: None,
        },
    }
}

fn with_caret(text: &str, caret: usize, focused: bool) -> String {
    if !focused {
        return text.to_string();
    }
    let at = floor_char(text, caret.min(text.len()));
    let mut out = String::new();
    out.push_str(&text[..at]);
    out.push('|');
    out.push_str(&text[at..]);
    out
}

fn floor_char(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
