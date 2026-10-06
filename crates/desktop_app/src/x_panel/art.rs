//! Cached raster of the Termy X bird.
//!
//! GPUI can draw the SVG directly, but it rasterises at 2x the intrinsic size
//! on every new image id, which cost ~6 s in debug builds (and ~30 s when the
//! intrinsic size was 3440x1440). Instead we rasterise once with resvg at a
//! capped width bucket (max [`MAX_ART_WIDTH`]), write a PNG to the user cache
//! dir keyed by size and SVG hash, and keep the encoded PNG in memory. GPUI's
//! asset system then decodes that PNG once per app (it caches by byte hash).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::{Image, ImageFormat};
use sha2::{Digest, Sha256};
use termy_x::splash::BIRD_SVG;

/// Never rasterise wider than this. 1720 is half of a 3440 ultrawide; the art
/// is soft and dimmed, so upscaling it to full width is visually lossless.
pub const MAX_ART_WIDTH: u32 = 1720;
/// Width buckets. Resizing a window only re-uses one of these sizes.
pub const ART_BUCKETS: [u32; 4] = [430, 860, 1290, 1720];
const VIEWBOX_W: f32 = 3440.0;
const VIEWBOX_H: f32 = 1440.0;

/// Where a bird image came from, for logging and the benchmark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtSource {
    Memory,
    Disk,
    Rendered,
}

#[derive(Clone, Debug)]
pub struct ArtLoad {
    pub image: Arc<Image>,
    pub width: u32,
    pub height: u32,
    pub source: ArtSource,
    pub elapsed: Duration,
}

fn memory() -> &'static Mutex<HashMap<u32, Arc<Image>>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Arc<Image>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pick a raster width for a target pixel width (window width times scale).
pub fn bucket_for(target_px: f32) -> u32 {
    let target = target_px.max(1.0).ceil() as u32;
    ART_BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= target)
        .unwrap_or(MAX_ART_WIDTH)
        .min(MAX_ART_WIDTH)
}

pub fn height_for(width: u32) -> u32 {
    ((width as f32) * VIEWBOX_H / VIEWBOX_W).round() as u32
}

pub fn svg_hash() -> String {
    let digest = Sha256::digest(BIRD_SVG.as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("TERMY_X_ART_CACHE") {
        return PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("termy-x")
}

pub fn cache_path(width: u32) -> PathBuf {
    cache_dir().join(format!(
        "bird-{width}x{}-{}.png",
        height_for(width),
        svg_hash()
    ))
}

/// Rasterise the SVG to PNG bytes at `width` (height follows the 21:9 viewBox).
pub fn rasterize_png(width: u32) -> anyhow::Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let height = height_for(width);
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(BIRD_SVG, &options)?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| anyhow::anyhow!("bad bird size {width}x{height}"))?;
    let size = tree.size();
    let transform =
        tiny_skia::Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(pixmap.encode_png()?)
}

/// Load the bird at a bucketed width: memory, then disk, then render + store.
pub fn load(width: u32) -> ArtLoad {
    let width = bucket_for(width as f32);
    let height = height_for(width);
    let started = Instant::now();
    if let Some(image) = memory().lock().ok().and_then(|cache| cache.get(&width).cloned()) {
        return ArtLoad { image, width, height, source: ArtSource::Memory, elapsed: started.elapsed() };
    }
    let path = cache_path(width);
    let (bytes, source) = match std::fs::read(&path) {
        Ok(bytes) if bytes.starts_with(b"\x89PNG") => (bytes, ArtSource::Disk),
        _ => match rasterize_png(width) {
            Ok(bytes) => {
                store(&path, &bytes);
                (bytes, ArtSource::Rendered)
            }
            Err(error) => {
                log::warn!("termy-x: bird raster failed ({error}); falling back to SVG");
                let image = Arc::new(Image::from_bytes(ImageFormat::Svg, BIRD_SVG.as_bytes().to_vec()));
                return ArtLoad { image, width, height, source: ArtSource::Rendered, elapsed: started.elapsed() };
            }
        },
    };
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    if let Ok(mut cache) = memory().lock() {
        cache.insert(width, image.clone());
    }
    let elapsed = started.elapsed();
    log::info!("termy-x: bird {width}x{height} from {source:?} in {elapsed:?}");
    ArtLoad { image, width, height, source, elapsed }
}

fn store(path: &std::path::Path, bytes: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = path.with_extension(format!("png.{}.tmp", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

#[cfg(test)]
pub(crate) fn clear_memory() {
    if let Ok(mut cache) = memory().lock() {
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_are_capped_and_keep_the_ultrawide_ratio() {
        assert_eq!(bucket_for(300.0), 430);
        assert_eq!(bucket_for(861.0), 1290);
        assert_eq!(bucket_for(6880.0), MAX_ART_WIDTH);
        assert_eq!(height_for(1720), 720);
        assert!(cache_path(1720).to_string_lossy().contains("bird-1720x720-"));
    }

    #[test]
    fn render_then_disk_then_memory() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module are the only readers of this variable.
        unsafe { std::env::set_var("TERMY_X_ART_CACHE", dir.path()) };
        clear_memory();
        let first = load(430);
        assert_eq!(first.source, ArtSource::Rendered);
        assert_eq!((first.width, first.height), (430, 180));
        assert!(cache_path(430).exists());
        clear_memory();
        let second = load(430);
        assert_eq!(second.source, ArtSource::Disk);
        assert_eq!(second.image.id(), first.image.id());
        let third = load(400);
        assert_eq!(third.source, ArtSource::Memory);
        assert!(Arc::ptr_eq(&third.image, &second.image));
    }
}
