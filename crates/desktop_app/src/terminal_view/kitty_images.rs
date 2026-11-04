use gpui_kit::{App, ImageSource, RenderImage};
use std::{collections::HashMap, rc::Rc, sync::Arc};
use termy_core::KittyGraphicsRenderPlacement;

struct Texture {
    source: ImageSource,
    cx: gpui_kit::AsyncApp,
}

impl Drop for Texture {
    fn drop(&mut self) {
        let source = self.source.clone();
        // Cache invalidation and pane removal happen while App is borrowed.
        // Release atlas entries on the next foreground turn, after painting.
        self.cx
            .spawn(async move |cx| {
                cx.update(|cx| release_texture(source, cx));
            })
            .detach();
    }
}

fn release_texture(source: ImageSource, cx: &mut App) {
    match source {
        ImageSource::Render(image) => cx.drop_image(image, None),
        ImageSource::Image(image) => {
            // GPUI decodes PNGs into its app-wide asset cache, and each window
            // uploads the decoded frames to its sprite atlas. Neither is freed
            // when the `Image` drops, so release both explicitly. Reading the
            // asset needs a window, but the cache and `drop_image` cover all.
            if let Some(window) = cx.windows().first().copied() {
                let image = image.clone();
                let _ = window.update(cx, |_, window, cx| {
                    if let Some(decoded) = image.get_render_image(window, cx) {
                        cx.drop_image(decoded, Some(window));
                    }
                });
            }
            image.remove_asset(cx);
        }
        ImageSource::Resource(_) | ImageSource::Custom(_) => {}
    }
}

#[derive(Clone, Default)]
pub(super) struct KittyImageCache {
    textures: TextureCache<Texture>,
}

impl KittyImageCache {
    pub(super) fn clear(&mut self) {
        self.textures.clear();
    }

    pub(super) fn get(
        &mut self,
        placement: &KittyGraphicsRenderPlacement,
        cx: &App,
    ) -> ImageSource {
        let key = (placement.image_id, placement.image_generation);
        if let Some(texture) = self.textures.get(key) {
            return texture.source.clone();
        }
        let source = if let Some(rgba) = placement.image.rgba() {
            let bgra = padded_bgra(rgba, placement.image_width, placement.image_height);
            let buffer = image::RgbaImage::from_raw(
                placement.image_width + 2,
                placement.image_height + 2,
                bgra,
            )
            .expect("validated Kitty image dimensions");
            ImageSource::Render(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        } else {
            Arc::new(gpui_kit::Image::from_bytes(
                gpui_kit::ImageFormat::Png,
                placement.image.png().to_vec(),
            ))
            .into()
        };
        let bytes = if placement.image.rgba().is_some() {
            (placement.image_width as usize + 2) * (placement.image_height as usize + 2) * 4
        } else {
            placement.image.byte_len()
        };
        self.textures.insert(
            key,
            Texture {
                source: source.clone(),
                cx: cx.to_async(),
            },
            bytes,
            placement.animation_deadline.is_some(),
        );
        source
    }
}

/// Texture bookkeeping keyed by `(image id, generation)`, independent of GPUI.
struct TextureCache<T> {
    entries: HashMap<(u32, u64), CachedTexture<T>>,
    clock: u64,
    bytes: usize,
}

struct CachedTexture<T> {
    texture: Rc<T>,
    bytes: usize,
    used: u64,
}

impl<T> Default for TextureCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            bytes: 0,
        }
    }
}

impl<T> Clone for TextureCache<T> {
    fn clone(&self) -> Self {
        Self {
            entries: self
                .entries
                .iter()
                .map(|(key, entry)| {
                    (
                        *key,
                        CachedTexture {
                            texture: entry.texture.clone(),
                            bytes: entry.bytes,
                            used: entry.used,
                        },
                    )
                })
                .collect(),
            clock: self.clock,
            bytes: self.bytes,
        }
    }
}

impl<T> TextureCache<T> {
    const MAX_ENTRIES: usize = 256;
    const MAX_BYTES: usize = 128 * 1024 * 1024;

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    fn get(&mut self, key: (u32, u64)) -> Option<Rc<T>> {
        self.clock = self.clock.wrapping_add(1);
        let entry = self.entries.get_mut(&key)?;
        entry.used = self.clock;
        Some(entry.texture.clone())
    }

    /// `animating` is set while the image plays an animation: its frames share
    /// the id with one generation each and come around again, so they stay
    /// cached (bounded by the LRU limits). Otherwise a new generation is a
    /// retransmission or edit, and older ones can never be requested again,
    /// so they don't wait for LRU eviction.
    fn insert(&mut self, key: (u32, u64), texture: T, bytes: usize, animating: bool) {
        if !animating {
            let freed = &mut self.bytes;
            self.entries.retain(|(id, generation), entry| {
                let stale = *id == key.0 && *generation != key.1;
                if stale {
                    *freed = freed.saturating_sub(entry.bytes);
                }
                !stale
            });
        }
        if let Some(replaced) = self.entries.insert(
            key,
            CachedTexture {
                texture: Rc::new(texture),
                bytes,
                used: self.clock,
            },
        ) {
            self.bytes = self.bytes.saturating_sub(replaced.bytes);
        }
        self.bytes = self.bytes.saturating_add(bytes);
        while self.entries.len() > Self::MAX_ENTRIES
            || (self.entries.len() > 1 && self.bytes > Self::MAX_BYTES)
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
        }
    }
}

// GPUI linearly samples atlas tiles at their pixel edges. A one-pixel repeated
// border keeps that sampling inside the image, including when a 1x1 image is
// enlarged over many terminal cells. The renderer clips this border away.
fn padded_bgra(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let stride = (width + 2) * 4;
    let mut output = vec![0; stride * (height + 2)];
    for y in 0..height {
        let row = &mut output[(y + 1) * stride..(y + 2) * stride];
        let source = &rgba[y * width * 4..(y + 1) * width * 4];
        for (source, destination) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(row[4..4 + width * 4].as_chunks_mut::<4>().0)
        {
            destination.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }
        row.copy_within(4..8, 0);
        row.copy_within(width * 4..width * 4 + 4, (width + 1) * 4);
    }
    output.copy_within(stride..2 * stride, 0);
    output.copy_within(
        height * stride..(height + 1) * stride,
        (height + 1) * stride,
    );
    output
}

#[cfg(test)]
mod tests {
    use super::{Texture, TextureCache, padded_bgra};
    use gpui_kit::{
        Context, Image, ImageFormat, ImageSource, IntoElement, Render, Styled as _, TestAppContext,
        Window, img,
    };
    use std::sync::Arc;

    struct PngView(Arc<Image>);

    impl Render for PngView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            img(ImageSource::Image(self.0.clone())).size_full()
        }
    }

    #[gpui_kit::test]
    fn kitty_png_texture_leaves_gpui_asset_cache_when_dropped(cx: &mut TestAppContext) {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("encode test PNG");
        let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
        let window = cx.add_window(|_, _| PngView(image.clone()));
        cx.run_until_parked();
        // Paint again now that the decode finished, uploading it to the atlas.
        window
            .update(cx, |_, window, _| window.refresh())
            .expect("window is open");
        cx.run_until_parked();
        assert!(cx.update(|cx| image.is_asset_cached(cx)));

        drop(Texture {
            source: ImageSource::Image(image.clone()),
            cx: cx.to_async(),
        });
        cx.run_until_parked();
        assert!(!cx.update(|cx| image.is_asset_cached(cx)));
    }

    #[test]
    fn kitty_animation_frames_keep_their_textures_while_playing() {
        let mut cache = TextureCache::<u64>::default();
        // Three frames of one playing animation, then a second loop.
        for generation in [1, 2, 3] {
            assert!(cache.get((7, generation)).is_none());
            cache.insert((7, generation), generation, 10, true);
        }
        for generation in [1, 2, 3] {
            assert_eq!(cache.get((7, generation)).as_deref(), Some(&generation));
        }
        assert_eq!(cache.bytes, 30);
    }

    #[test]
    fn kitty_retransmission_drops_older_generations_of_the_id() {
        let mut cache = TextureCache::<u64>::default();
        cache.insert((7, 1), 1, 10, true);
        cache.insert((7, 2), 2, 10, true);
        cache.insert((8, 1), 1, 10, false);
        cache.insert((7, 5), 5, 10, false);
        assert!(cache.get((7, 1)).is_none());
        assert!(cache.get((7, 2)).is_none());
        assert!(cache.get((7, 5)).is_some());
        assert!(cache.get((8, 1)).is_some(), "other ids are untouched");
        assert_eq!(cache.bytes, 20);
    }

    #[test]
    fn kitty_texture_cache_evicts_least_recently_used_over_budget() {
        let mut cache = TextureCache::<u64>::default();
        let limit = TextureCache::<u64>::MAX_ENTRIES as u32;
        for generation in 0..=limit {
            cache.insert((1, u64::from(generation)), 0, 1, true);
            if generation == 0 {
                continue;
            }
            // Keep the first frame warm.
            assert!(cache.get((1, 0)).is_some());
        }
        assert_eq!(cache.entries.len(), TextureCache::<u64>::MAX_ENTRIES);
        assert_eq!(cache.bytes, TextureCache::<u64>::MAX_ENTRIES);
        assert!(cache.get((1, 0)).is_some());
        assert!(cache.get((1, 1)).is_none());
    }

    #[test]
    fn kitty_one_pixel_texture_has_no_transparent_sampling_border() {
        let texture = padded_bgra(&[240, 160, 50, 255], 1, 1);
        assert_eq!(texture.len(), 3 * 3 * 4);
        assert!(
            texture
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel == &[50, 160, 240, 255])
        );
    }

    #[test]
    fn kitty_texture_border_repeats_each_edge_and_preserves_alpha() {
        let texture = padded_bgra(&[1, 2, 3, 40, 5, 6, 7, 80], 2, 1);
        let row = [3, 2, 1, 40, 3, 2, 1, 40, 7, 6, 5, 80, 7, 6, 5, 80];
        assert_eq!(texture, row.repeat(3));
    }
}
