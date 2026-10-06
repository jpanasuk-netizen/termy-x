//! Cached raster of the Termy X bird.
//!
//! The SVG is parsed once and rasterized once at [`MAX_ART_WIDTH`]. The GPUI
//! image is stored in a process-wide [`OnceLock`] and reused every frame.
//! Resizes do not rasterize again. `background_art = false` must not call
//! [`load`] at all.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::{Image, ImageFormat};

use termy_x::splash::BIRD_SVG;

/// Cap the raster at a normal desktop width. Wider windows scale this bitmap.
pub const MAX_ART_WIDTH: u32 = 1600;
/// View box of `assets/x/termy-x-bird.svg`. Height follows this ratio.
const VIEWBOX_W: f32 = 1600.0;
const VIEWBOX_H: f32 = 1000.0;

/// Where a bird image came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtSource {
    Memory,
    Rendered,
}

#[derive(Clone)]
pub struct ArtLoad {
    pub image: Arc<Image>,
    pub width: u32,
    pub height: u32,
    pub source: ArtSource,
    pub elapsed: Duration,
}

/// Every window width shares one raster. Scaling is the renderer's job.
pub fn bucket_for(_target_px: f32) -> u32 {
    MAX_ART_WIDTH
}

pub fn height_for(width: u32) -> u32 {
    ((width as f32) * VIEWBOX_H / VIEWBOX_W).round().max(1.0) as u32
}

fn parsed_tree() -> Result<&'static resvg::usvg::Tree, String> {
    static TREE: OnceLock<Result<resvg::usvg::Tree, String>> = OnceLock::new();
    match TREE.get_or_init(|| {
        let options = resvg::usvg::Options::default();
        resvg::usvg::Tree::from_str(BIRD_SVG, &options).map_err(|error| error.to_string())
    }) {
        Ok(tree) => Ok(tree),
        Err(error) => Err(error.clone()),
    }
}

/// Rasterise the already-parsed SVG. Callers outside the cache should use [`load`].
pub fn rasterize_png(width: u32) -> anyhow::Result<Vec<u8>> {
    let tree = parsed_tree().map_err(|error| anyhow::anyhow!(error))?;
    let height = height_for(width);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow::anyhow!("bad bird size {width}x{height}"))?;
    let size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap.encode_png()?)
}

fn write_timing(line: &str) {
    let path = std::env::temp_dir().join("termy-x-art-timing.txt");
    let _ = std::fs::write(path, format!("{line}\n"));
}

fn render_once() -> Option<ArtLoad> {
    let started = Instant::now();
    let width = bucket_for(MAX_ART_WIDTH as f32);
    match rasterize_png(width) {
        Ok(bytes) => {
            let height = height_for(width);
            let elapsed = started.elapsed();
            let ms = elapsed.as_millis();
            let line = format!("termy-x art: rasterized {width}x{height} in {ms} ms");
            eprintln!("{line}");
            write_timing(&line);
            Some(ArtLoad {
                image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
                width,
                height,
                source: ArtSource::Rendered,
                elapsed,
            })
        }
        Err(error) => {
            let line = format!("termy-x art: raster failed: {error}");
            eprintln!("{line}");
            write_timing(&line);
            None
        }
    }
}

/// Parse, rasterize, and decode once per process. Later calls return the same image.
pub fn load() -> Option<ArtLoad> {
    static CACHE: OnceLock<Option<ArtLoad>> = OnceLock::new();
    static HITS: AtomicU64 = AtomicU64::new(0);
    let Some(rendered) = CACHE.get_or_init(render_once) else {
        return None;
    };
    let hit = HITS.fetch_add(1, Ordering::Relaxed);
    Some(ArtLoad {
        image: Arc::clone(&rendered.image),
        width: rendered.width,
        height: rendered.height,
        source: if hit == 0 {
            ArtSource::Rendered
        } else {
            ArtSource::Memory
        },
        elapsed: rendered.elapsed,
    })
}

pub fn cached_image() -> Option<Arc<Image>> {
    load().map(|art| art.image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_bucket_keeps_the_desktop_ratio() {
        assert_eq!(bucket_for(300.0), MAX_ART_WIDTH);
        assert_eq!(bucket_for(1280.0), MAX_ART_WIDTH);
        assert_eq!(bucket_for(1920.0), MAX_ART_WIDTH);
        assert_eq!(bucket_for(6880.0), MAX_ART_WIDTH);
        assert_eq!(height_for(1600), 1000);
        const { assert!(MAX_ART_WIDTH <= 1600) };
    }

    #[test]
    fn raster_is_a_png_at_the_capped_size() {
        let bytes = rasterize_png(MAX_ART_WIDTH).expect("raster");
        assert!(bytes.starts_with(b"\x89PNG"));
        let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
        assert_eq!((width, height), (1600, 1000));
    }

    #[test]
    fn render_once_then_reuse_and_record_timing() {
        let first = load().expect("art");
        assert_eq!(first.source, ArtSource::Rendered);
        assert_eq!((first.width, first.height), (1600, 1000));
        let second = load().expect("cached");
        assert_eq!(second.source, ArtSource::Memory);
        assert!(Arc::ptr_eq(&first.image, &second.image));
        let timing = std::fs::read_to_string(std::env::temp_dir().join("termy-x-art-timing.txt"))
            .expect("timing file");
        assert!(timing.contains("termy-x art: rasterized 1600x1000 in "));
        assert!(timing.contains(" ms"));
    }
}
