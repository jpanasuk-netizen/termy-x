//! Reply Guy tab: Treg finds fresh posts for a topic, Grok drafts reply
//! options, the user picks/edits, and Reply opens the normal Confirm
//! screen (OpenCLI post path). Nothing here posts, likes, or follows.

use std::collections::HashMap;

use super::*;
use termy_x::reply_guy::{self as rg, Secret, Sort};

pub struct RgState {
    pub key_for: Secret,
    pub grok_mask: Option<String>,
    pub treg_mask: Option<String>,
    pub cost: Option<String>,
    pub error: Option<String>,
    pub test: Option<String>,
    pub trends: Vec<String>,
    pub drafts: HashMap<String, Vec<String>>,
    pub drafting: Option<String>,
    pub reply_to: Option<String>,
    pub busy: bool,
    pub checked: bool,
}

impl RgState {
    /// Cheap: does not touch the keychain. Keys are read when the tab opens.
    pub fn new(_config: &XConfig) -> Self {
        Self {
            key_for: Secret::Treg,
            grok_mask: None,
            treg_mask: None,
            cost: None,
            error: None,
            test: None,
            trends: Vec::new(),
            drafts: HashMap::new(),
            drafting: None,
            reply_to: None,
            busy: false,
            checked: false,
        }
    }

    pub fn ready(&self) -> Result<(), String> {
        rg::ready(self.grok_mask.as_deref(), self.treg_mask.as_deref())
    }
}

fn to_sort(sort: termy_x::sort::ResultSort) -> Sort {
    match sort {
        termy_x::sort::ResultSort::Popular => Sort::Popular,
        termy_x::sort::ResultSort::Newest => Sort::Newest,
    }
}

/// Offline fixture for mock/preview mode: same shape as Treg's response.
const MOCK_SEARCH: &str = r#"{"output":{"data":{"items":[
{"id":"9001","authorUsername":"rustacean","authorName":"Rust Fan","text":"Shipping a terminal in Rust taught me more about text layout than any course.","likeCount":412,"retweetCount":38,"replyCount":51,"viewCount":22000,"createdUtc":1789811122},
{"id":"9002","authorUsername":"buildinpublic","authorName":"Indie Dev","text":"What is the one tool you would never give up for building in public?","likeCount":96,"retweetCount":7,"replyCount":140,"viewCount":8100,"createdUtc":1789812122}
]}},"costUsd":0.0}"#;

impl XPanel {
    pub(super) fn rg_refresh_keys(&mut self) {
        let config = self.live_config();
        self.desk.rg.grok_mask =
            rg::load_secret(&config, Secret::Grok).map(|value| rg::mask(&value));
        self.desk.rg.treg_mask =
            rg::load_secret(&config, Secret::Treg).map(|value| rg::mask(&value));
        self.desk.rg.checked = true;
    }

    pub(super) fn rg_save_key(&mut self, cx: &mut Context<Self>) {
        let value = std::mem::take(&mut self.desk.rg_key_input);
        let which = self.desk.rg.key_for;
        if value.trim().is_empty() {
            self.notice = Some(format!("Paste your {} first.", which.label()));
            cx.notify();
            return;
        }
        let config = self.live_config();
        self.notice = Some(match rg::store_secret(&config, which, &value) {
            Ok(()) => format!("Saved {}. Click Test to check it.", which.label()),
            Err(error) => format!("Could not save {}: {error}", which.label()),
        });
        self.input_target = InputTarget::RgTopic;
        self.rg_refresh_keys();
        cx.notify();
    }

    fn rg_remove_key(&mut self, which: Secret, cx: &mut Context<Self>) {
        let config = self.live_config();
        let _ = rg::clear_secret(&config, which);
        self.rg_refresh_keys();
        self.notice = Some(format!("Removed {}.", which.label()));
        cx.notify();
    }

    fn rg_test(&mut self, which: Secret, cx: &mut Context<Self>) {
        let config = self.live_config();
        let Some(secret) = rg::load_secret(&config, which) else {
            self.desk.rg.test = Some(format!("No {} saved yet.", which.label()));
            cx.notify();
            return;
        };
        self.desk.rg.test = Some(format!("Testing {}\u{2026}", which.label()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let http = termy_x::io::UreqHttp::default();
                    match which {
                        Secret::Grok => rg::test_grok(&http, &secret),
                        Secret::Treg => rg::test_treg(&http, &secret),
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.desk.rg.test = Some(match result {
                    Ok(text) => text,
                    Err(error) => format!(
                        "{} failed: {}",
                        which.label(),
                        termy_x::publish::humanize_error(&error)
                    ),
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn rg_fetch(&mut self, cx: &mut Context<Self>) {
        if let Err(reason) = self.desk.rg.ready() {
            self.notice = Some(reason);
            cx.notify();
            return;
        }
        let topic = self.desk.rg_topic.trim().to_string();
        if topic.is_empty() {
            self.notice = Some("Type a topic or niche first.".into());
            cx.notify();
            return;
        }
        if self.desk.rg.busy {
            return;
        }
        self.desk.rg.busy = true;
        self.desk.rg.error = None;
        self.status = format!("reply guy \u{b7} fetching {topic}\u{2026}");
        cx.notify();
        let live = self.live;
        let sort = to_sort(self.list_sort);
        let config = self.live_config();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if live {
                        let token = rg::load_secret(&config, Secret::Treg)
                            .ok_or_else(|| "Treg key missing".to_string())?;
                        let http = termy_x::io::UreqHttp::default();
                        rg::fetch_posts(&http, &token, &topic, sort, 20)
                    } else {
                        rg::parse_search(MOCK_SEARCH)
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.desk.rg.busy = false;
                match result {
                    Ok(fetched) => {
                        panel.desk.rg.cost =
                            Some(rg::cost_label(fetched.cost_usd, rg::TREG_SEARCH_COST_USD));
                        panel.status = format!(
                            "reply guy \u{b7} {} posts \u{b7} {}",
                            fetched.posts.len(),
                            panel.desk.rg.cost.clone().unwrap_or_default()
                        );
                        panel.desk.rg_posts = fetched.posts;
                    }
                    Err(error) => {
                        panel.desk.rg.error = Some(termy_x::publish::humanize_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rg_load_trends(&mut self, cx: &mut Context<Self>) {
        if let Err(reason) = self.desk.rg.ready() {
            self.notice = Some(reason);
            cx.notify();
            return;
        }
        if self.desk.rg.busy {
            return;
        }
        self.desk.rg.busy = true;
        cx.notify();
        let live = self.live;
        let config = self.live_config();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if live {
                        let token = rg::load_secret(&config, Secret::Treg)
                            .ok_or_else(|| "Treg key missing".to_string())?;
                        let http = termy_x::io::UreqHttp::default();
                        rg::fetch_trends(&http, &token, "")
                    } else {
                        Ok((
                            vec!["Rust".to_string(), "Build in public".to_string()],
                            None,
                        ))
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.desk.rg.busy = false;
                match result {
                    Ok((names, cost)) => {
                        panel.desk.rg.trends = names;
                        panel.desk.rg.cost = Some(rg::cost_label(cost, rg::TREG_TRENDS_COST_USD));
                    }
                    Err(error) => {
                        panel.desk.rg.error = Some(termy_x::publish::humanize_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rg_draft(&mut self, post: &Post, cx: &mut Context<Self>) {
        if self.desk.rg.drafting.is_some() {
            self.notice = Some("Already drafting \u{2014} wait for it.".into());
            cx.notify();
            return;
        }
        let post = post.engage_target().clone();
        let id = post.id.clone();
        self.desk.rg.drafting = Some(id.clone());
        self.notice = Some("Grok is drafting \u{2014} nothing will be posted.".into());
        cx.notify();
        let live = self.live;
        let niche = self.desk.rg_topic.clone();
        let config = self.live_config();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if live {
                        let key = rg::load_secret(&config, Secret::Grok)
                            .ok_or_else(|| "Grok key missing".to_string())?;
                        let model = if config.ai_base_url.contains("api.x.ai") {
                            config.ai_model.clone()
                        } else {
                            String::new()
                        };
                        let http = termy_x::io::UreqHttp::default();
                        rg::draft_replies(&http, &key, &model, &post, &niche, 3)
                    } else {
                        Ok(termy_x::ai::template_variants(&post.text, Tone::Reply, 2)
                            .into_iter()
                            .map(|draft| draft.text)
                            .collect())
                    }
                })
                .await;
            let _ = this.update(cx, |panel, cx| {
                panel.desk.rg.drafting = None;
                match result {
                    Ok(options) => {
                        panel.desk.rg.drafts.insert(id, options);
                        panel.notice = Some(
                            "Drafts ready. Use one, edit it, then Reply opens Confirm.".into(),
                        );
                    }
                    Err(error) => {
                        panel.notice = Some(format!(
                            "Grok draft failed: {}",
                            termy_x::publish::humanize_error(&error)
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rg_key_row(&self, which: Secret, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mask = match which {
            Secret::Grok => self.desk.rg.grok_mask.clone(),
            Secret::Treg => self.desk.rg.treg_mask.clone(),
        };
        let tag = match which {
            Secret::Grok => "grok",
            Secret::Treg => "treg",
        };
        let sign_in = match which {
            Secret::Grok => "Sign in to Grok",
            Secret::Treg => "Sign in to Treg",
        };
        div()
            .flex()
            .gap(px(6.0))
            .flex_wrap()
            .child(hint(format!(
                "{} \u{b7} {}",
                which.label(),
                mask.unwrap_or_else(|| "not set".into())
            )))
            .child(x_button!(
                SharedString::from(format!("x-rg-signin-{tag}")),
                sign_in,
                cx.listener(move |this, _, _, cx| {
                    let _ =
                        termy_x::io::Opener::open(&termy_x::io::SystemOpener, which.sign_in_url());
                    this.desk.rg.key_for = which;
                    this.input_target = InputTarget::RgKey;
                    this.notice = Some(
                        "Create a key on the page that opened, then paste it here and press Enter."
                            .to_string(),
                    );
                    cx.stop_propagation();
                    cx.notify();
                })
            ))
            .child(x_button!(
                SharedString::from(format!("x-rg-paste-{tag}")),
                "Paste key",
                self.input_target == InputTarget::RgKey && self.desk.rg.key_for == which,
                cx.listener(move |this, _, _, cx| {
                    this.desk.rg.key_for = which;
                    this.input_target = InputTarget::RgKey;
                    this.pending_focus = true;
                    cx.stop_propagation();
                    cx.notify();
                })
            ))
            .child(x_button!(
                SharedString::from(format!("x-rg-test-{tag}")),
                "Test connection",
                cx.listener(move |this, _, _, cx| {
                    this.rg_test(which, cx);
                    cx.stop_propagation();
                })
            ))
            .child(x_button!(
                SharedString::from(format!("x-rg-remove-{tag}")),
                "Remove",
                cx.listener(move |this, _, _, cx| {
                    this.rg_remove_key(which, cx);
                    cx.stop_propagation();
                })
            ))
            .into_any_element()
    }

    pub(super) fn reply_guy_rows(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        rows.push(hint(
            "Reply Guy: Treg finds fresh posts for your niche, Grok drafts replies, you pick and edit. Reply opens Confirm. Nothing auto-posts.".into(),
        ));
        rows.push(self.rg_key_row(Secret::Treg, cx));
        rows.push(self.rg_key_row(Secret::Grok, cx));
        if self.input_target == InputTarget::RgKey {
            rows.push(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(self.single_line_field(TextField::RgKey, cx))
                    .child(x_button!(
                        "x-rg-key-save",
                        "Save",
                        cx.listener(|this, _, _, cx| {
                            this.rg_save_key(cx);
                            cx.stop_propagation();
                        })
                    ))
                    .into_any_element(),
            );
            rows.push(hint(format!(
                "Saving your {} \u{b7} kept in the OS keychain, never shown or logged.",
                self.desk.rg.key_for.label()
            )));
        }
        rows.push(hint(
            "Grok: xAI has no sign-in for third-party apps to use, so Sign in to Grok opens console.x.ai to create an API key. Treg: use your own key from treg.to (or `treg login`).".into(),
        ));
        if let Some(test) = &self.desk.rg.test {
            rows.push(hint(test.clone()));
        }
        if let Err(reason) = self.desk.rg.ready() {
            rows.push(empty_state("Reply Guy is off", reason));
            return rows;
        }
        rows.push(
            div()
                .flex()
                .gap(px(8.0))
                .flex_wrap()
                .child(self.single_line_field(TextField::RgTopic, cx))
                .child(x_button!(
                    "x-rg-fetch",
                    if self.desk.rg.busy {
                        "Fetching\u{2026}"
                    } else {
                        "Fetch posts"
                    },
                    cx.listener(|this, _, _, cx| {
                        this.rg_fetch(cx);
                        cx.stop_propagation();
                    })
                ))
                .child(x_button!(
                    "x-rg-popular",
                    "Popular",
                    self.list_sort == termy_x::sort::ResultSort::Popular,
                    cx.listener(|this, _, _, cx| {
                        this.list_sort = termy_x::sort::ResultSort::Popular;
                        cx.stop_propagation();
                        cx.notify();
                    })
                ))
                .child(x_button!(
                    "x-rg-newest",
                    "Newest",
                    self.list_sort == termy_x::sort::ResultSort::Newest,
                    cx.listener(|this, _, _, cx| {
                        this.list_sort = termy_x::sort::ResultSort::Newest;
                        cx.stop_propagation();
                        cx.notify();
                    })
                ))
                .child(x_button!(
                    "x-rg-trends",
                    "Trends",
                    cx.listener(|this, _, _, cx| {
                        this.rg_load_trends(cx);
                        cx.stop_propagation();
                    })
                ))
                .into_any_element(),
        );
        rows.push(hint(self.desk.rg.cost.clone().unwrap_or_else(|| {
            format!(
                "Treg est. ~${:.5} per fetch (your Treg balance)",
                rg::TREG_SEARCH_COST_USD
            )
        })));
        if !self.desk.rg.trends.is_empty() {
            let mut chips = div().flex().gap(px(6.0)).flex_wrap();
            for (index, name) in self.desk.rg.trends.iter().take(12).enumerate() {
                let topic = name.clone();
                chips = chips.child(x_button!(
                    SharedString::from(format!("x-rg-trend-{index}")),
                    name.clone(),
                    cx.listener(move |this, _, _, cx| {
                        this.desk.rg_topic = topic.clone();
                        this.rg_fetch(cx);
                        cx.stop_propagation();
                    })
                ));
            }
            rows.push(chips.into_any_element());
        }
        if let Some(error) = &self.desk.rg.error {
            rows.push(hint(error.clone()));
        }
        let mut posts = self.desk.rg_posts.clone();
        posts.sort_by(|left, right| termy_x::sort::cmp_posts(left, right, self.list_sort));
        if posts.is_empty() && !self.desk.rg.busy {
            rows.push(empty_state(
                "No posts yet",
                "Type a niche (e.g. rust, indie hackers) and press Enter, or pick a trend.",
            ));
        }
        for post in posts {
            rows.push(self.tweet_card(&post, cx));
            let post_draft = post.clone();
            let drafting = self.desk.rg.drafting.as_deref() == Some(post.id.as_str());
            rows.push(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(x_button!(
                        SharedString::from(format!("x-rg-draft-{}", post.id)),
                        if drafting {
                            "Drafting\u{2026}"
                        } else {
                            "Grok drafts"
                        },
                        cx.listener(move |this, _, _, cx| {
                            this.rg_draft(&post_draft, cx);
                            cx.stop_propagation();
                        })
                    ))
                    .into_any_element(),
            );
            let target = post.engage_target().id.clone();
            if let Some(options) = self.desk.rg.drafts.get(&post.engage_target().id) {
                for (index, option) in options.iter().enumerate() {
                    rows.push(card(format!("Option {}", index + 1), option.clone()));
                    let text_use = option.clone();
                    let text_reply = option.clone();
                    let to_use = target.clone();
                    let to_reply = target.clone();
                    rows.push(
                        div()
                            .flex()
                            .gap(px(6.0))
                            .child(x_button!(
                                SharedString::from(format!("x-rg-use-{target}-{index}")),
                                "Edit in Compose",
                                cx.listener(move |this, _, _, cx| {
                                    this.compose = text_use.clone();
                                    this.caret = this.compose.len();
                                    this.desk.rg.reply_to = Some(to_use.clone());
                                    this.notice = Some(
                                        "Edit it, then come back and click Reply \u{2014} Confirm shows before anything posts."
                                            .into(),
                                    );
                                    this.open_section(XSection::Compose, cx);
                                    cx.stop_propagation();
                                })
                            ))
                            .child(x_button!(
                                SharedString::from(format!("x-rg-reply-{target}-{index}")),
                                "Reply\u{2026}",
                                cx.listener(move |this, _, _, cx| {
                                    // Use the edited Compose text when it was edited for this post.
                                    let text = if this.desk.rg.reply_to.as_deref()
                                        == Some(to_reply.as_str())
                                        && !this.compose.trim().is_empty()
                                    {
                                        this.compose.clone()
                                    } else {
                                        text_reply.clone()
                                    };
                                    this.arm_confirm_reply(&text, &to_reply, None);
                                    cx.stop_propagation();
                                    cx.notify();
                                })
                            ))
                            .into_any_element(),
                    );
                }
            }
        }
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_fixture_parses() {
        let fetched = rg::parse_search(MOCK_SEARCH).unwrap();
        assert_eq!(fetched.posts.len(), 2);
    }

    #[test]
    fn sort_follows_panel_toggle() {
        assert_eq!(to_sort(termy_x::sort::ResultSort::Popular), Sort::Popular);
        assert_eq!(to_sort(termy_x::sort::ResultSort::Newest), Sort::Newest);
    }

    #[test]
    fn off_until_both_keys() {
        let mut state = RgState::new(&XConfig::default());
        assert!(state.ready().is_err());
        state.treg_mask = Some("x".into());
        assert!(state.ready().is_err());
        state.grok_mask = Some("y".into());
        assert!(state.ready().is_ok());
    }

    #[test]
    fn reply_guy_tab_is_listed_and_routes_keys() {
        assert!(XSection::all().contains(&XSection::ReplyGuy));
        assert_eq!(XSection::from_command("replyguy"), XSection::ReplyGuy);
        assert!(edit_keys_owned(
            false,
            false,
            InputTarget::RgKey,
            XSection::ReplyGuy,
            false,
            false,
            false
        ));
    }
}
