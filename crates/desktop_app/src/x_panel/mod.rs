//! Termy X side panel. The geometric swift is an original SVG, dimmed behind
//! lists and shown at full strength on the empty splash.

use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, img, px, rgb, rgba, Context, FocusHandle, Image, ImageFormat, ImageSource,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, StyledImage, Window,
};

macro_rules! x_button {
    ($id:expr, $label:expr, $handler:expr) => {
        div()
            .id($id)
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(8.0))
            .bg(rgb(0x1636c9))
            .text_size(px(12.0))
            .text_color(rgb(0xf8fbff))
            .cursor_pointer()
            .child($label)
            .on_mouse_down(MouseButton::Left, $handler)
    };
}
use termy_x::drafts::new_id;
use termy_x::model::{Draft, Post, Profile, ResearchHit, Tone, Trend};
use termy_x::providers::{sample_posts, MockProvider, XProvider};
use termy_x::splash::BIRD_SVG;
use termy_x::text::{split_thread, weighted_len};
use termy_x::XConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XSection {
    Timeline,
    Search,
    Trends,
    Lookup,
    Compose,
    Research,
}

impl XSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::Timeline => "Timeline",
            Self::Search => "Search",
            Self::Trends => "Trends",
            Self::Lookup => "Lookup",
            Self::Compose => "Compose",
            Self::Research => "Research",
        }
    }

    pub fn from_command(name: &str) -> Self {
        match name {
            "search" => Self::Search,
            "trends" => Self::Trends,
            "lookup" => Self::Lookup,
            "compose" => Self::Compose,
            "research" => Self::Research,
            _ => Self::Timeline,
        }
    }
}

pub struct XPanel {
    focus: FocusHandle,
    section: XSection,
    posts: Vec<Post>,
    trends: Vec<Trend>,
    research: Vec<ResearchHit>,
    profile: Option<Profile>,
    query: String,
    lookup: String,
    compose: String,
    variants: Vec<Draft>,
    drafts: Vec<Draft>,
    status: String,
    confirm: bool,
    intent_urls: Vec<String>,
    intent_index: usize,
    background_art: bool,
    splash: bool,
    fill_window: bool,
    live: bool,
    art: Arc<Image>,
    notice: Option<String>,
}

impl XPanel {
    pub fn new(cx: &mut Context<Self>, fill_window: bool) -> Self {
        let config = if fill_window {
            XConfig::default()
        } else {
            termy_x::config::XConfig::load(&termy_x::config::default_config_dir())
        };
        let mut panel = Self {
            focus: cx.focus_handle(),
            section: XSection::Timeline,
            posts: Vec::new(),
            trends: Vec::new(),
            research: Vec::new(),
            profile: None,
            query: "gpui terminal".into(),
            lookup: "termy".into(),
            compose: "Termy X counts this URL as 23: https://termy.sh".into(),
            variants: Vec::new(),
            drafts: Vec::new(),
            status: "mock · fixture data · no X API credits".into(),
            confirm: false,
            intent_urls: Vec::new(),
            intent_index: 0,
            background_art: config.background_art,
            splash: false,
            fill_window,
            live: !fill_window,
            art: bird_image(),
            notice: None,
        };
        panel.load_timeline();
        panel
    }

    pub fn preview(scene: &str, cx: &mut Context<Self>) -> Self {
        let mut panel = Self::new(cx, true);
        panel.background_art = true;
        panel.load_scene(scene);
        panel
    }

    pub fn open_section(&mut self, section: XSection) {
        self.section = section;
        self.confirm = false;
        self.splash = false;
        match section {
            XSection::Timeline => self.load_timeline(),
            XSection::Search => self.load_search(),
            XSection::Trends => self.load_trends(),
            XSection::Lookup => self.load_lookup(),
            XSection::Research => self.load_research(),
            XSection::Compose => {
                if self.variants.is_empty() {
                    self.suggest();
                }
            }
        }
    }

    fn load_scene(&mut self, scene: &str) {
        match scene {
            "search" => self.open_section(XSection::Search),
            "compose" => self.open_section(XSection::Compose),
            "confirm" => {
                self.open_section(XSection::Compose);
                self.arm_confirm();
            }
            "splash" => {
                self.section = XSection::Timeline;
                self.posts.clear();
                self.splash = true;
                self.status = "splash · background art at full strength".into();
            }
            "trends" => self.open_section(XSection::Trends),
            _ => self.open_section(XSection::Timeline),
        }
    }

    fn service(&self) -> termy_x::providers::Service {
        let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
        termy_x::providers::Service::free(
            &config,
            Arc::new(termy_x::io::SystemRunner),
            Arc::new(termy_x::io::UreqHttp),
        )
    }

    fn load_timeline(&mut self) {
        if self.live {
            if let Ok(served) = self.service().timeline(10) {
                self.posts = served.value;
                self.status = served.status.render();
                return;
            }
        }
        self.posts = MockProvider.timeline(10).map(|served| served.value).unwrap_or_default();
        self.status = "mock · fixture data · no X API credits".into();
    }

    fn load_search(&mut self) {
        if self.live {
            if let Ok(served) = self.service().search(&self.query, false, 10) {
                self.posts = served.value;
                self.status = served.status.render();
                return;
            }
        }
        self.posts = MockProvider
            .search(&self.query, false, 10)
            .map(|served| served.value)
            .unwrap_or_default();
        self.status = format!("mock · search “{}” · no X API credits", self.query);
    }

    fn load_trends(&mut self) {
        if self.live {
            if let Ok(served) = self.service().trends(Some("worldwide"), 8) {
                self.status = served.status.render();
                self.trends = served.value;
                self.posts.clear();
                return;
            }
        }
        self.trends = MockProvider
            .trends(Some("worldwide"), 8)
            .map(|served| served.value)
            .unwrap_or_default();
        self.posts = sample_posts();
        self.status = "mock · trends · no X API credits".into();
    }

    fn load_lookup(&mut self) {
        if self.live {
            if let Ok(served) = self.service().lookup(&self.lookup) {
                self.status = served.status.render();
                self.profile = Some(served.value);
                return;
            }
        }
        self.profile = MockProvider.lookup(&self.lookup).ok().map(|served| served.value);
        self.status = format!("mock · lookup @{} · no X API credits", self.lookup.trim_start_matches('@'));
    }

    fn load_research(&mut self) {
        if self.live {
            let config = termy_x::config::XConfig::load(&termy_x::config::default_config_dir());
            let runner = termy_x::io::SystemRunner;
            self.research = self.service().research("gpui terminal", &runner, &config);
            self.status = "research · no X API credits".into();
            return;
        }
        self.research = vec![ResearchHit {
            source: "fixture".into(),
            title: "GPUI terminals people actually use".into(),
            url: "https://example.com/gpui".into(),
            excerpt: "Fixture research for drafts. Install OpenCLI, yt-dlp, or curl to pull Reddit, YouTube, and the web.".into(),
            session: None,
        }];
        self.status = "research · fixture · no X API credits".into();
    }

    fn suggest(&mut self) {
        self.variants = termy_x::ai::template_variants(&self.compose, Tone::Punchy, 3);
        self.notice = Some("Variants are drafts. Nothing was posted.".into());
    }

    fn arm_confirm(&mut self) {
        let parts = split_thread(&self.compose);
        self.intent_urls = termy_x::publish::intent_urls(&parts, None);
        self.intent_index = 0;
        self.confirm = true;
    }

    fn accept_variant(&mut self, index: usize) {
        if let Some(draft) = self.variants.get(index) {
            self.compose = draft.text.clone();
            self.drafts.push(Draft {
                id: new_id(),
                text: draft.text.clone(),
                tone: draft.tone.clone(),
                source: draft.source.clone(),
                created_at: draft.created_at.clone(),
                reply_to: None,
            });
        }
    }

    fn toggle_art(&mut self) {
        self.background_art = !self.background_art;
        let mut config = XConfig::default();
        config.background_art = self.background_art;
        if !self.fill_window {
            let _ = config.save_background_art();
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if self.section != XSection::Compose || self.confirm {
            return;
        }
        let key = event.keystroke.key.as_str();
        let plain = !event.keystroke.modifiers.modified();
        if !plain {
            return;
        }
        if key == "backspace" {
            self.compose.pop();
        } else if key == "space" {
            self.compose.push(' ');
        } else if key == "enter" {
            self.compose.push('\n');
        } else if key.chars().count() == 1 {
            self.compose.push_str(key);
        } else {
            return;
        }
        cx.notify();
    }
}

impl Render for XPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = weighted_len(&self.compose);
        let parts = split_thread(&self.compose);
        let dim = self.confirm
            || self.section != XSection::Timeline
            || (!self.splash && !self.posts.is_empty());
        let art_opacity_overlay = if self.splash { 0.0 } else { 0.80 };

        let mut root = div()
            .id("termy-x-panel")
            .key_context("TermyX")
            .track_focus(&self.focus)
            .relative()
            .h_full()
            .overflow_hidden()
            .bg(rgb(0x070b14))
            .text_color(rgb(0xe7eefc))
            .font_family("sans-serif")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                }),
            )
            .on_key_down(cx.listener(|this, event, _, cx| this.on_key(event, cx)));
        if self.fill_window {
            root = root.size_full();
        } else {
            root = root.w(px(480.0)).flex_none();
        }

        root.child(self.backdrop(dim, art_opacity_overlay))
            .child(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .p(px(16.0))
                    .gap(px(10.0))
                    .child(self.header(cx))
                    .child(self.tabs(cx))
                    .child(self.status_line())
                    .child(
                        div()
                            .id("x-scroll")
                            .flex_1()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .children(self.body(count.weighted, &parts, cx)),
                    ),
            )
            .children(self.confirm.then(|| self.confirm_dialog(count.weighted, &parts, window, cx)))
    }
}

impl XPanel {
    fn backdrop(&self, dim: bool, overlay: f32) -> impl IntoElement {
        let source: ImageSource = self.art.clone().into();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(if self.background_art {
                img(source)
                    .id("termy-x-bird")
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            } else {
                div().size_full().bg(rgb(0x070b14)).into_any_element()
            })
            .when(self.background_art && dim, |layer| {
                layer.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(tint(0x070b14, (overlay * 255.0) as u8)),
                )
            })
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div().flex().flex_col().child(
                    div()
                        .text_size(px(18.0))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(rgb(0x7af6ff))
                        .child("Termy X"),
                ),
            )
            .child(
                div()
                    .id("x-art-toggle")
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(999.0))
                    .bg(tint(0x102033, 180))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .child(if self.background_art {
                        "Art on"
                    } else {
                        "Art off"
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            this.toggle_art();
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
    }

    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.section;
        div().flex().gap(px(6.0)).flex_wrap().children(
            [
                XSection::Timeline,
                XSection::Search,
                XSection::Trends,
                XSection::Lookup,
                XSection::Compose,
                XSection::Research,
            ]
            .into_iter()
            .map(move |section| {
                let active = section == current;
                div()
                    .id(SharedString::from(format!("x-tab-{}", section.label())))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(8.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .bg(if active {
                        tint(0x1d4ed8, 210)
                    } else {
                        tint(0x101826, 170)
                    })
                    .text_color(if active { rgb(0xf8fbff) } else { rgb(0xb7c3dc) })
                    .child(section.label())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.open_section(section);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    )
            }),
        )
    }

    fn status_line(&self) -> impl IntoElement {
        div()
            .text_size(px(11.0))
            .text_color(rgb(0x93a4c3))
            .child(self.status.clone())
    }

    fn body(
        &self,
        weighted: usize,
        parts: &[String],
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        if let Some(notice) = &self.notice {
            rows.push(hint(notice.clone()));
        }
        match self.section {
            XSection::Timeline | XSection::Search => {
                if self.posts.is_empty() {
                    rows.push(self.splash_copy());
                }
                for post in &self.posts {
                    rows.push(post_card(post));
                }
            }
            XSection::Trends => {
                for trend in &self.trends {
                    rows.push(card(
                        trend.name.clone(),
                        format!(
                            "{} · volume {}",
                            trend.place.as_deref().unwrap_or("worldwide"),
                            trend.volume.unwrap_or(0)
                        ),
                    ));
                }
                for post in &self.posts {
                    rows.push(post_card(post));
                }
            }
            XSection::Lookup => {
                if let Some(profile) = &self.profile {
                    rows.push(card(
                        format!("@{} · {}", profile.handle, profile.name),
                        format!(
                            "{}\n{} followers · {} following",
                            profile.bio, profile.followers, profile.following
                        ),
                    ));
                    for post in &profile.posts {
                        rows.push(post_card(post));
                    }
                }
            }
            XSection::Research => {
                for hit in &self.research {
                    rows.push(card(
                        format!("[{}] {}", hit.source, hit.title),
                        format!("{}\n{}", hit.url, hit.excerpt),
                    ));
                }
            }
            XSection::Compose => {
                rows.push(self.compose_block(weighted, parts, cx));
            }
        }
        rows
    }

    fn splash_copy(&self) -> gpui_kit::AnyElement {
        div()
            .mt(px(80.0))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .text_size(px(28.0))
                    .text_color(rgb(0xe9fdff))
                    .child("Look it up from the terminal."),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(rgb(0xc5d2ea))
                    .child("No X API credits. Your session, or the browser intent."),
            )
            .into_any_element()
    }

    fn compose_block(&self, weighted: usize, parts: &[String], cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let counter = format!("{weighted} / 280");
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(card(
                "Live preview".into(),
                self.compose.clone(),
            ))
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(if weighted > 280 { rgb(0xff5d8f) } else { rgb(0x9ad7ff) })
                    .child(counter),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xb7c3dc))
                    .child(format!("{} thread part(s). URLs count as 23.", parts.len())),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(x_button!("x-suggest", "Suggest variants", cx.listener(|this, _, _, cx| {
                        this.suggest();
                        cx.notify();
                    })))
                    .child(x_button!("x-publish", "Publish", cx.listener(|this, _, _, cx| {
                        this.arm_confirm();
                        cx.notify();
                    }))),
            )
            .children(self.variants.iter().enumerate().map(|(index, draft)| {
                let label = format!("Variant {} · {}", index + 1, draft.source);
                div()
                    .id(SharedString::from(format!("x-variant-{index}")))
                    .p(px(10.0))
                    .rounded(px(10.0))
                    .bg(tint(0x10192b, 220))
                    .cursor_pointer()
                    .child(div().text_size(px(11.0)).text_color(rgb(0xf0a8d0)).child(label))
                    .child(div().mt(px(4.0)).text_size(px(13.0)).child(draft.text.clone()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.accept_variant(index);
                            cx.notify();
                        }),
                    )
            }))
            .into_any_element()
    }

    fn confirm_dialog(
        &self,
        weighted: usize,
        parts: &[String],
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let url = self
            .intent_urls
            .get(self.intent_index)
            .cloned()
            .unwrap_or_default();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(tint(0x03060c, 170))
            .child(
                div()
                    .id("x-confirm")
                    .w(px(460.0))
                    .max_w(px(640.0))
                    .p(px(16.0))
                    .rounded(px(14.0))
                    .bg(rgb(0x101624))
                    .border_1()
                    .border_color(rgb(0xff4fa3))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .text_color(rgb(0xffd0ea))
                            .child("Publish this exact text?"),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(0x93a4c3))
                            .child("mock · no X API credits · you still confirm"),
                    )
                    .child(
                        div()
                            .p(px(10.0))
                            .rounded(px(8.0))
                            .bg(rgb(0x070b14))
                            .text_size(px(14.0))
                            .child(self.compose.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .child(format!("{weighted} weighted characters · {} part(s)", parts.len())),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(0x9ad7ff))
                            .child(url),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.0))
                            .child(x_button!("x-cancel", "Cancel", cx.listener(|this, _, _, cx| {
                                this.confirm = false;
                                cx.notify();
                            })))
                            .child(x_button!(
                                "x-confirm-publish",
                                "Publish",
                                cx.listener(|this, _, _, cx| {
                                    this.publish_confirmed();
                                    cx.notify();
                                })
                            ))
                            .child(x_button!("x-next-post", "Next post", cx.listener(|this, _, window, cx| {
                                if this.intent_urls.is_empty() {
                                    return;
                                }
                                let next = (this.intent_index + 1) % this.intent_urls.len();
                                this.intent_index = next;
                                if let Some(url) = this.intent_urls.get(next) {
                                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url.clone()));
                                }
                                let _ = window;
                                cx.notify();
                            }))),
                    ),
            )
    }

    fn publish_confirmed(&mut self) {
        self.confirm = false;
        if !self.live {
            self.notice = Some(
                "Dry run in this preview. The exact text was not posted.".into(),
            );
            return;
        }
        let mut service = self.service();
        service
            .publishers
            .push(Box::new(termy_x::publish::IntentPublisher {
                open_browser: true,
                opener: Arc::new(termy_x::io::SystemOpener),
            }));
        let Some(publisher) = termy_x::publish::first_available(&service.publishers) else {
            self.notice = Some("No publisher is available.".into());
            return;
        };
        let outcome = termy_x::publish::confirm_and_publish(
            &self.compose,
            None,
            termy_x::ConfirmDecision::Yes,
            false,
            publisher,
        );
        self.notice = Some(match outcome {
            termy_x::GateOutcome::Sent { plan, receipt } => {
                format!("{} · {receipt}", plan.provider.label())
            }
            termy_x::GateOutcome::DryRun { plan } => {
                format!("dry run · {} weighted", plan.count.weighted)
            }
            termy_x::GateOutcome::Rejected { plan } => format!(
                "not posted · {}",
                plan.session.unwrap_or_else(|| "cancelled".into())
            ),
        });
    }
}

fn bird_image() -> Arc<Image> {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        BIRD_SVG.as_bytes().to_vec(),
    ))
}

fn card(title: String, body: String) -> gpui_kit::AnyElement {
    div()
        .p(px(12.0))
        .rounded(px(12.0))
        .bg(tint(0x0c1422, 225))
        .border_1()
        .border_color(tint(0x7af6ff, 40))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(0xd7fbff))
                .child(title),
        )
        .child(
            div()
                .mt(px(4.0))
                .text_size(px(13.0))
                .text_color(rgb(0xe7eefc))
                .child(body),
        )
        .into_any_element()
}

fn post_card(post: &Post) -> gpui_kit::AnyElement {
    card(
        format!("@{} · {} likes", post.author_handle, post.likes),
        post.text.clone(),
    )
}

fn hint(text: String) -> gpui_kit::AnyElement {
    div()
        .text_size(px(12.0))
        .text_color(rgb(0xf0a8d0))
        .child(text)
        .into_any_element()
}

fn tint(color: u32, alpha: u8) -> gpui_kit::Rgba {
    rgba((color << 8) | u32::from(alpha))
}
