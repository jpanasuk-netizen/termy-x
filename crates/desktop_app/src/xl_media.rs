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

pub(crate) struct MediaDesk {
    states: HashMap<String, Load>,
    inflight: RefCell<HashSet<String>>,
    viewer: Option<String>,
    player: Option<crate::x_panel::player::VideoPlayer>,
    pub(crate) notice: String,
}

impl Default for MediaDesk {
    fn default() -> Self {
        Self {
            states: HashMap::new(),
            inflight: RefCell::new(HashSet::new()),
            viewer: None,
            player: None,
            notice: String::new(),
        }
    }
}

impl MediaDesk {
    fn close(&mut self) -> bool {
        let open = self.viewer.is_some() || self.player.is_some();
        self.viewer = None;
        if let Some(player) = self.player.take() {
            player.stop();
        }
        open
    }
}

impl TrendsDesk {
    pub(crate) fn close_media(&mut self) -> bool {
        self.media.close()
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
            MediaDispatch::Vlc => self.start_video(&item.url, window),
            MediaDispatch::Unplayable => {
                self.media.notice = crate::x_panel::player::NO_PLAYABLE_MEDIA.into();
            }
        }
        cx.notify();
    }

    fn start_video(&mut self, url: &str, window: &Window) {
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
        let rect = (
            24.0,
            72.0,
            (width - 48.0).max(200.0),
            (height - 140.0).max(160.0),
        );
        match crate::x_panel::player::VideoPlayer::start(hwnd, window.scale_factor(), rect, url, 80)
        {
            Ok(player) => {
                self.media.player = Some(player);
                self.media.notice = crate::x_panel::player::VLC_OPEN_NOTICE.into();
            }
            Err(error) => self.media.notice = error,
        }
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
            return win32.hwnd.get() as isize;
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
