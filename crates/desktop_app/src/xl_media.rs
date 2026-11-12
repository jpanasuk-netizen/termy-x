//! Card pictures for Termy XL. Downloads and decode stay off the UI thread.
//! Photos use the same in-app viewer decision as Termy X. Video and GIF
//! playback goes through the same VLC engine.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::{
    Context, ImageSource, InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement,
    RenderImage, StatefulInteractiveElement, Styled, StyledImage, Window, div, img, px, rgb,
};

use crate::trends_desk::TrendsDesk;
use termy_x::model::{MediaItem, MediaKind};
use termy_x::trends_desk::{play_badge, preview_url, read_cached, tile_grid, write_cached};

#[derive(Clone)]
enum Load {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

struct PendingPlay {
    url: String,
    hwnd: isize,
    scale: f32,
    rect: (f32, f32, f32, f32),
    tries: u8,
}

pub(crate) struct MediaDesk {
    states: HashMap<String, Load>,
    inflight: RefCell<HashSet<String>>,
    viewer: Option<String>,
    player: Option<crate::x_panel::player::VideoPlayer>,
    pending: Option<PendingPlay>,
    needs_focus: bool,
    pub(crate) notice: String,
}

impl Default for MediaDesk {
    fn default() -> Self {
        Self {
            states: HashMap::new(),
            inflight: RefCell::new(HashSet::new()),
            viewer: None,
            player: None,
            pending: None,
            needs_focus: false,
            notice: String::new(),
        }
    }
}

impl MediaDesk {
    fn close(&mut self) -> bool {
        let open = self.viewer.is_some() || self.player.is_some() || self.pending.is_some();
        self.viewer = None;
        self.pending = None;
        self.notice.clear();
        if let Some(player) = self.player.take() {
            player.stop();
        }
        open
    }

    fn player_open(&self) -> bool {
        self.player.is_some() || self.viewer.is_some() || self.pending.is_some()
    }
}

impl TrendsDesk {
    pub(crate) fn close_media(&mut self) -> bool {
        self.media.close()
    }

    pub(crate) fn media_blocks_card_click(&self) -> bool {
        !termy_x::trends_desk::card_click_drafts(self.media.player_open())
    }

    pub(crate) fn focus_open_media(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.media.needs_focus {
            return;
        }
        self.media.needs_focus = false;
        self.focus_root_after_click(window, cx);
    }

    pub(crate) fn media_block(
        &self,
        items: &[MediaItem],
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let shown: Vec<MediaItem> = items.iter().take(4).cloned().collect();
        for item in &shown {
            if let Some(url) = preview_url(item) {
                self.ensure_image(url, cx);
            }
        }
        let tiles = tile_grid(width, shown.len());
        let height = tiles
            .iter()
            .map(|tile| tile.y + tile.h)
            .fold(0.0_f32, f32::max);
        let mut grid = div().relative().w(px(width)).h(px(height.max(1.0)));
        for (item, tile) in shown.iter().zip(tiles) {
            grid = grid.child(self.media_tile(item, tile, cx));
        }
        grid.into_any_element()
    }

    fn media_tile(
        &self,
        item: &MediaItem,
        tile: termy_x::trends_desk::TileBox,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let preview = preview_url(item);
        let state = preview.as_ref().and_then(|url| self.media.states.get(url));
        let open = item.clone();
        let mut tile_el = div()
            .absolute()
            .left(px(tile.x))
            .top(px(tile.y))
            .w(px(tile.w))
            .h(px(tile.h))
            .rounded(px(8.0))
            .overflow_hidden()
            .bg(rgb(0x1c1c1e))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    this.open_card_media(open.clone(), window, cx);
                    cx.stop_propagation();
                }),
            );
        match state {
            Some(Load::Ready(image)) => {
                tile_el = tile_el.child(
                    img(ImageSource::Render(image.clone()))
                        .w(px(tile.w))
                        .h(px(tile.h))
                        .object_fit(ObjectFit::Contain),
                );
            }
            Some(Load::Failed) | None => {
                tile_el = tile_el.child(
                    div()
                        .w(px(tile.w))
                        .h(px(tile.h.clamp(48.0, 72.0)))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8e8e93))
                        .child("image"),
                );
            }
            Some(Load::Loading) => {
                tile_el = tile_el.child(
                    div()
                        .w(px(tile.w))
                        .h(px(tile.h))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.0))
                        .text_color(rgb(0x8e8e93))
                        .child("…"),
                );
            }
        }
        if let Some(badge) = play_badge(item.kind) {
            tile_el = tile_el.child(
                div()
                    .absolute()
                    .left(px(6.0))
                    .bottom(px(6.0))
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(8.0))
                    .bg(rgb(0x070b14))
                    .text_size(px(12.0))
                    .text_color(rgb(0xe9fdff))
                    .child(badge),
            );
        }
        tile_el.into_any_element()
    }

    fn ensure_image(&self, url: String, cx: &mut Context<Self>) {
        if self.media.states.contains_key(&url)
            || !self.media.inflight.borrow_mut().insert(url.clone())
        {
            return;
        }
        let root = self.data_root.clone();
        let fetch_url = url.clone();
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { load_bytes(&root, &fetch_url) })
                .await;
            let _ = this.update(cx, |desk, cx| {
                let state = match decoded {
                    Ok(bytes) => match crate::x_panel::preview::decode_still(&bytes) {
                        Ok(buffer) => {
                            Load::Ready(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
                        }
                        Err(_) => Load::Failed,
                    },
                    Err(_) => Load::Failed,
                };
                desk.media.inflight.borrow_mut().remove(&url);
                desk.media.states.insert(url, state);
                cx.notify();
            });
        })
        .detach();
    }

    fn open_card_media(&mut self, item: MediaItem, window: &mut Window, cx: &mut Context<Self>) {
        use crate::x_panel::player::{MediaDispatch, dispatch_media};
        self.media.notice.clear();
        match dispatch_media(item.kind, &item.url) {
            MediaDispatch::StillImage => {
                let url = preview_url(&item).unwrap_or(item.url);
                self.ensure_image(url.clone(), cx);
                self.media.viewer = Some(url);
            }
            MediaDispatch::Vlc => self.start_video(&item.url, window, cx),
            MediaDispatch::Unplayable => {
                self.media.notice = crate::x_panel::player::NO_PLAYABLE_MEDIA.into();
            }
        }
        cx.notify();
    }

    fn start_video(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(player) = self.media.player.take() {
            player.stop();
        }
        let hwnd = window_hwnd(window);
        if hwnd == 0 {
            self.media.notice = "Video playback needs the Termy window.".into();
            return;
        }
        let size = window.viewport_size();
        let width: f32 = size.width.into();
        let height: f32 = size.height.into();
        let (x, y) = termy_x::trends_desk::player_origin();
        self.media.pending = Some(PendingPlay {
            url: url.to_string(),
            hwnd,
            scale: window.scale_factor(),
            rect: (x, y, (width - 48.0).max(200.0), (height - 140.0).max(160.0)),
            tries: 0,
        });
        self.media.notice.clear();
        self.open_pending_video(cx);
        self.focus_handle.focus(window, cx);
        self.focus_root_after_click(window, cx);
    }

    /// Esc hides the host and releases libVLC on a worker thread. A second
    /// play waits until that release finishes, then opens a new host.
    fn open_pending_video(&mut self, cx: &mut Context<Self>) {
        if self.media.pending.is_none() {
            return;
        }
        if !crate::x_panel::player::engine_ready_for_replay() {
            let tries = self
                .media
                .pending
                .as_ref()
                .map(|pending| pending.tries)
                .unwrap_or(0);
            if let Some(pending) = self.media.pending.as_mut() {
                pending.tries = pending.tries.saturating_add(1);
            }
            if tries == 0 {
                termy_x::trends_desk::append_desk_log(
                    &self.data_root,
                    "INFO media: waiting for the previous video host to release",
                );
            }
            if tries >= 80 {
                self.media.pending = None;
                self.media.notice =
                    "Video did not reopen. The previous player is still closing.".into();
                termy_x::trends_desk::append_desk_log(
                    &self.data_root,
                    "INFO media: video did not reopen; the previous host was still closing",
                );
                cx.notify();
                return;
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let _ = this.update(cx, |desk, cx| desk.open_pending_video(cx));
            })
            .detach();
            return;
        }
        let Some(pending) = self.media.pending.take() else {
            return;
        };
        crate::x_panel::player::reap_video_hosts();
        match crate::x_panel::player::VideoPlayer::start(
            pending.hwnd,
            pending.scale,
            pending.rect,
            &pending.url,
            80,
        ) {
            Ok(player) => {
                self.media.player = Some(player);
                self.media.notice = crate::x_panel::player::VLC_OPEN_NOTICE.into();
                self.media.needs_focus = true;
                termy_x::trends_desk::append_desk_log(
                    &self.data_root,
                    "INFO media: video host shown",
                );
            }
            Err(error) => {
                self.media.notice = error.clone();
                termy_x::trends_desk::append_desk_log(
                    &self.data_root,
                    &format!("INFO media: video did not open: {error}"),
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn media_shield(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if self.media.player.is_none() && self.media.pending.is_none() {
            return None;
        }
        let (x, y) = termy_x::trends_desk::player_origin();
        Some(
            div()
                .id("desk-player-shield")
                .absolute()
                .left(px(x))
                .top(px(y))
                .right(px(24.0))
                .bottom(px(68.0))
                .bg(rgb(0x070b14))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|_this, _, _, cx| {
                        cx.stop_propagation();
                    }),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn media_close(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let playing = self.media.player.is_some()
            || self.media.viewer.is_some()
            || self.media.pending.is_some();
        if !playing && self.media.notice.is_empty() {
            return None;
        }
        let note: String = if playing {
            String::new()
        } else {
            self.media.notice.chars().take(80).collect()
        };
        let hit = termy_x::trends_desk::CLOSE_HIT;
        let mut row = div()
            .absolute()
            .top(px(8.0))
            .right(px(8.0))
            .flex()
            .items_center()
            .gap(px(8.0));
        if !note.is_empty() {
            row = row.child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0xe5e5ea))
                    .child(note),
            );
        }
        Some(
            row.child(
                div()
                    .id("desk-media-close")
                    .w(px(hit))
                    .h(px(hit))
                    .min_w(px(hit))
                    .rounded(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgb(0xff4fa3))
                    .text_size(px(28.0))
                    .text_color(rgb(0x070b14))
                    .cursor_pointer()
                    .child(termy_x::trends_desk::CLOSE_ICON)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            this.close_media();
                            this.focus_handle.focus(window, cx);
                            cx.stop_propagation();
                            cx.notify();
                        }),
                    ),
            )
            .into_any_element(),
        )
    }

    pub(crate) fn media_overlay(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let url = self.media.viewer.clone()?;
        let state = self.media.states.get(&url).cloned();
        let mut sheet = div()
            .id("desk-photo-viewer")
            .absolute()
            .inset_0()
            .bg(rgb(0x070b14))
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.close_media();
                    cx.stop_propagation();
                    cx.notify();
                }),
            );
        match state {
            Some(Load::Ready(image)) => {
                sheet = sheet.child(
                    img(ImageSource::Render(image))
                        .max_w_full()
                        .max_h_full()
                        .object_fit(ObjectFit::Contain),
                );
            }
            _ => {
                sheet = sheet.child(
                    div()
                        .p(px(12.0))
                        .rounded(px(8.0))
                        .bg(rgb(0x1c1c1e))
                        .text_size(px(12.0))
                        .text_color(rgb(0x8e8e93))
                        .child("image"),
                );
            }
        }
        Some(sheet.into_any_element())
    }
}

fn load_bytes(root: &std::path::Path, url: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = read_cached(root, url) {
        return Ok(bytes);
    }
    let bytes = crate::x_panel::preview::fetch_bounded(url, 5_000_000)?;
    let _ = write_cached(root, url, &bytes);
    Ok(bytes)
}

fn window_hwnd(window: &Window) -> isize {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(win32) = handle.as_raw()
        {
            return win32.hwnd.get();
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = window;
    0
}

#[allow(dead_code)]
fn _kind_label(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Photo => "photo",
        MediaKind::Video => "video",
        MediaKind::Gif => "gif",
    }
}
