//! Automatic video posters. A provider still image is fetched as-is.
//! When that image is missing or cannot be decoded, one video frame is
//! derived without opening a window and without selecting an audio
//! stream. Work is bounded (byte cap, sample cap, timeout) and cached
//! by URL. A row that has moved on does not keep the previous poster.

use std::time::Duration;

use termy_x::model::{MediaItem, MediaKind, Post};

pub const PREVIEW_FETCH_BUDGET: Duration = Duration::from_secs(8);
pub const FRAME_PREFIX_BYTES: u64 = 512 * 1024;
/// Media Foundation reads at most this many samples, video stream only.
pub const FRAME_SAMPLE_CAP: u32 = 8;
/// The frame grabber does not select the audio stream.
pub const FRAME_CAPTURE_SELECTS_AUDIO: bool = false;
/// The frame grabber does not create a window.
pub const FRAME_CAPTURE_OPENS_WINDOW: bool = false;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewPlan {
    /// `url` is a provider still. `video` is the playable file used only
    /// if that still cannot be decoded.
    Still { url: String, video: Option<String> },
    /// No usable still. Derive one frame from this video URL.
    Frame { video: String },
    /// Nothing to load. The row must not spin.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheLookup {
    Missing,
    Loading,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewView {
    Image,
    Loading,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowSlot {
    pub generation: u64,
    pub key: String,
}

pub fn frame_cache_key(video: &str) -> String {
    format!("frame:{video}")
}

impl PreviewPlan {
    pub fn cache_key(&self) -> Option<String> {
        match self {
            PreviewPlan::Still { url, .. } => Some(url.clone()),
            PreviewPlan::Frame { video } => Some(frame_cache_key(video)),
            PreviewPlan::Unavailable => None,
        }
    }
}

pub fn preview_plan(media: &MediaItem) -> PreviewPlan {
    let still = media.poster.as_deref().and_then(provider_still);
    let video = match media.kind {
        MediaKind::Photo => None,
        MediaKind::Video | MediaKind::Gif => provider_video(&media.url),
    };
    if let Some(url) = still {
        return PreviewPlan::Still {
            url: url.to_string(),
            video: video.map(str::to_string),
        };
    }
    if let Some(video) = video {
        return PreviewPlan::Frame {
            video: video.to_string(),
        };
    }
    if media.kind == MediaKind::Photo
        && let Some(url) = provider_still(&media.url)
    {
        return PreviewPlan::Still {
            url: url.to_string(),
            video: None,
        };
    }
    PreviewPlan::Unavailable
}

/// Posters to start for this row, including the reposted original and
/// a quote's own media. Quote media stays on the quote.
pub fn preview_jobs(post: &Post) -> Vec<PreviewPlan> {
    let mut out = Vec::new();
    walk_jobs(post, &mut out, 0);
    out
}

fn walk_jobs(post: &Post, out: &mut Vec<PreviewPlan>, depth: u8) {
    if depth > 3 {
        return;
    }
    for media in &post.media {
        let plan = preview_plan(media);
        let key = plan.cache_key();
        if key.is_some() && out.iter().all(|have| have.cache_key() != key) {
            out.push(plan);
        }
    }
    if let Some(inner) = &post.retweeted {
        walk_jobs(inner, out, depth + 1);
    }
    if let Some(inner) = &post.quoted {
        walk_jobs(inner, out, depth + 1);
    }
}

/// The card's main picture: a repost shows the original, a quote shows
/// the quote tweet itself.
pub fn displayed_preview(post: &Post) -> Option<PreviewPlan> {
    post.engage_target().media.first().map(preview_plan)
}

pub fn preview_view(plan: &PreviewPlan, lookup: impl Fn(&str) -> CacheLookup) -> PreviewView {
    match plan {
        PreviewPlan::Unavailable => PreviewView::Unavailable,
        PreviewPlan::Still { url, video } => match lookup(url) {
            CacheLookup::Ready => PreviewView::Image,
            CacheLookup::Loading => PreviewView::Loading,
            CacheLookup::Failed => match video {
                Some(video) => frame_view(video, &lookup),
                None => PreviewView::Unavailable,
            },
            CacheLookup::Missing => PreviewView::Unavailable,
        },
        PreviewPlan::Frame { video } => frame_view(video, &lookup),
    }
}

fn frame_view(video: &str, lookup: &impl Fn(&str) -> CacheLookup) -> PreviewView {
    match lookup(&frame_cache_key(video)) {
        CacheLookup::Ready => PreviewView::Image,
        CacheLookup::Loading => PreviewView::Loading,
        CacheLookup::Failed | CacheLookup::Missing => PreviewView::Unavailable,
    }
}

pub fn ready_key(plan: &PreviewPlan, lookup: impl Fn(&str) -> CacheLookup) -> Option<String> {
    if preview_view(plan, &lookup) != PreviewView::Image {
        return None;
    }
    match plan {
        PreviewPlan::Still { url, video } => {
            if lookup(url) == CacheLookup::Ready {
                Some(url.clone())
            } else {
                video.as_ref().map(|video| frame_cache_key(video))
            }
        }
        PreviewPlan::Frame { video } => Some(frame_cache_key(video)),
        PreviewPlan::Unavailable => None,
    }
}

pub fn preview_caption(kind: MediaKind, view: PreviewView) -> &'static str {
    match (kind, view) {
        (MediaKind::Photo, PreviewView::Loading) => "Loading photo\u{2026}",
        (MediaKind::Photo, PreviewView::Unavailable) => "Photo (could not load preview)",
        (_, PreviewView::Loading) => "Loading video preview\u{2026}",
        (_, PreviewView::Unavailable) => "Video preview unavailable",
        (_, PreviewView::Image) => "",
    }
}

pub fn recycle_row(slot: &RowSlot, next_key: impl Into<String>) -> RowSlot {
    RowSlot {
        generation: slot.generation.saturating_add(1),
        key: next_key.into(),
    }
}

/// A finished download applies only to the row generation that started it.
pub fn accept_row_result(slot: &RowSlot, started_gen: u64, started_key: &str) -> bool {
    slot.generation == started_gen && slot.key == started_key
}

pub fn provider_still(url: &str) -> Option<&str> {
    let url = url.trim();
    if !is_http(url) || is_timed_media(url) {
        None
    } else {
        Some(url)
    }
}

fn provider_video(url: &str) -> Option<&str> {
    let url = url.trim();
    if is_http(url) && is_timed_media(url) {
        Some(url)
    } else {
        None
    }
}

fn is_http(url: &str) -> bool {
    !url.is_empty()
        && !url.chars().any(|c| c.is_control())
        && (url.starts_with("https://") || url.starts_with("http://"))
}

fn is_timed_media(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let ext = path.rsplit('.').next().unwrap_or("");
    const STILL: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "avif", "bmp"];
    if STILL.iter().any(|item| ext.eq_ignore_ascii_case(item)) {
        return false;
    }
    const TIMED: &[&str] = &[
        "mp4", "m4v", "webm", "mkv", "mov", "m3u8", "ts", "m4s", "mp3", "m4a", "aac", "wav", "ogg",
        "opus", "flac",
    ];
    TIMED.iter().any(|item| ext.eq_ignore_ascii_case(item))
        || path.to_ascii_lowercase().contains("video.twimg.com")
}

/// A JPEG embedded in a video prefix (cover art or a still muxed in).
/// Returns None when the markers are missing or the span is too small
/// to be a picture.
pub fn embedded_jpeg(bytes: &[u8]) -> Option<&[u8]> {
    let start = bytes.windows(3).position(|w| w == [0xff, 0xd8, 0xff])?;
    let rest = &bytes[start + 3..];
    let end = rest.windows(2).position(|w| w == [0xff, 0xd9])?;
    let stop = start + 3 + end + 2;
    let jpeg = &bytes[start..stop];
    (jpeg.len() >= 32).then_some(jpeg)
}

pub(crate) fn fetch_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    if provider_still(url).is_none() && provider_video(url).is_none() {
        return Err("unsupported preview url".into());
    }
    let response = image_agent()
        .get(url)
        .set("User-Agent", "TermyX/1.0")
        .call()
        .map_err(|_| "preview download failed".to_string())?;
    let mut bytes = Vec::new();
    use std::io::Read;
    response
        .into_reader()
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "preview download failed".to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err("preview too large".into());
    }
    Ok(bytes)
}

fn image_agent() -> &'static ureq::Agent {
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(PREVIEW_FETCH_BUDGET)
            .build()
    })
}

pub(crate) fn decode_still(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    if let Ok(image) = image::load_from_memory(bytes) {
        return Ok(image.to_rgba8());
    }
    let jpeg = embedded_jpeg(bytes).ok_or_else(|| "no still frame".to_string())?;
    image::load_from_memory(jpeg)
        .map(|image| image.to_rgba8())
        .map_err(|_| "still frame could not be decoded".to_string())
}

/// One silent frame. Provider bytes win when they are already a picture.
/// Otherwise Windows reads a single video sample with audio deselected
/// and no window. Other platforms stop here.
pub(crate) fn derive_frame(url: &str) -> Result<image::RgbaImage, String> {
    if let Ok(bytes) = fetch_bounded(url, FRAME_PREFIX_BYTES)
        && let Ok(image) = decode_still(&bytes)
    {
        return Ok(image);
    }
    #[cfg(target_os = "windows")]
    {
        return capture_first_frame_muted(url);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = url;
        Err("no still frame in the video".into())
    }
}

#[cfg(target_os = "windows")]
fn capture_first_frame_muted(url: &str) -> Result<image::RgbaImage, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = url.to_string();
    std::thread::Builder::new()
        .name("termy-preview".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                read_one_video_sample(&owned)
            }));
            let _ = tx.send(result);
        })
        .map_err(|_| "preview thread did not start".to_string())?;
    match rx.recv_timeout(PREVIEW_FETCH_BUDGET) {
        Ok(Ok(Ok(image))) => Ok(image),
        Ok(Ok(Err(err))) => Err(err),
        Ok(Err(_)) => Err("preview capture failed".into()),
        Err(_) => Err("preview capture timed out".into()),
    }
}

#[cfg(target_os = "windows")]
unsafe fn read_one_video_sample(url: &str) -> Result<image::RgbaImage, String> {
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows::Win32::Media::MediaFoundation::{
        IMFAttributes, IMFMediaBuffer, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE,
        MF_MT_SUBTYPE, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
        MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM,
        MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateMediaType, MFCreateSourceReaderFromURL,
        MFMediaType_Video, MFSTARTUP_NOSOCKET, MFStartup, MFVideoFormat_RGB32,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    static STARTED: Once = Once::new();
    static OK: AtomicBool = AtomicBool::new(false);
    STARTED.call_once(|| {
        // Full startup (flag 0) so an https URL can be opened. NOSOCKET
        // would refuse the network. The value is named so the call stays
        // obvious next to the constant we must not pass.
        let flags = 0u32;
        let _ = MFSTARTUP_NOSOCKET;
        OK.store(
            unsafe { MFStartup(MF_VERSION, flags).is_ok() },
            Ordering::Relaxed,
        );
    });
    if !OK.load(Ordering::Relaxed) {
        return Err("media foundation did not start".into());
    }
    let mut wide: Vec<u16> = url.encode_utf16().collect();
    wide.push(0);
    let reader = unsafe {
        MFCreateSourceReaderFromURL(windows::core::PCWSTR(wide.as_ptr()), None::<IMFAttributes>)
    }
    .map_err(|_| "preview source did not open".to_string())?;
    let all = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
    let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    unsafe { reader.SetStreamSelection(all, false) }
        .map_err(|_| "preview stream could not be selected".to_string())?;
    unsafe { reader.SetStreamSelection(video, true) }
        .map_err(|_| "preview stream could not be selected".to_string())?;
    if FRAME_CAPTURE_SELECTS_AUDIO {
        let audio = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        unsafe { reader.SetStreamSelection(audio, true) }
            .map_err(|_| "preview audio could not be selected".to_string())?;
    }
    if FRAME_CAPTURE_OPENS_WINDOW {
        return Err("preview must not open a window".into());
    }
    let output = unsafe { MFCreateMediaType() }.map_err(|_| "preview type failed".to_string())?;
    unsafe {
        output
            .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|_| "preview type failed".to_string())?;
        output
            .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
            .map_err(|_| "preview type failed".to_string())?;
        reader
            .SetCurrentMediaType(video, None, &output)
            .map_err(|_| "preview type failed".to_string())?;
    }
    let mut sample = None;
    for _ in 0..FRAME_SAMPLE_CAP {
        let mut flags = 0u32;
        sample = None;
        unsafe {
            reader
                .ReadSample(video, 0, None, Some(&mut flags), None, Some(&mut sample))
                .map_err(|_| "preview sample failed".to_string())?;
        }
        let flag = flags as i32;
        if flag & MF_SOURCE_READERF_ERROR.0 != 0 {
            return Err("preview sample failed".into());
        }
        if sample.is_some() {
            break;
        }
        if flag & MF_SOURCE_READERF_ENDOFSTREAM.0 != 0 {
            break;
        }
    }
    let sample = sample.ok_or_else(|| "preview sample was empty".to_string())?;
    let media_type = unsafe { reader.GetCurrentMediaType(video) }
        .map_err(|_| "preview size missing".to_string())?;
    let packed = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
        .map_err(|_| "preview size missing".to_string())?;
    let width = (packed >> 32) as u32;
    let height = (packed & 0xffff_ffff) as u32;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("preview size is unusable".into());
    }
    let stride = unsafe { media_type.GetUINT64(&MF_MT_DEFAULT_STRIDE) }
        .ok()
        .map(|value| value as u32)
        .filter(|stride| *stride == width.saturating_mul(4))
        .unwrap_or(width.saturating_mul(4));
    let buffer: IMFMediaBuffer = unsafe { sample.ConvertToContiguousBuffer() }
        .map_err(|_| "preview buffer missing".to_string())?;
    let mut ptr = std::ptr::null_mut();
    let mut current = 0u32;
    unsafe { buffer.Lock(&mut ptr, None, Some(&mut current)) }
        .map_err(|_| "preview buffer missing".to_string())?;
    let copy = if ptr.is_null() || current == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(ptr, current as usize) }.to_vec()
    };
    unsafe { buffer.Unlock() }.map_err(|_| "preview buffer missing".to_string())?;
    let row = stride as usize;
    let need = row.saturating_mul(height as usize);
    if copy.len() < need || row < width as usize * 4 {
        return Err("preview buffer is short".into());
    }
    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = y * row + x * 4;
            rgba.extend_from_slice(&[copy[i + 2], copy[i + 1], copy[i], copy[i + 3]]);
        }
    }
    image::RgbaImage::from_raw(width, height, rgba)
        .ok_or_else(|| "preview image failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn video(url: &str, poster: Option<&str>) -> MediaItem {
        MediaItem {
            kind: MediaKind::Video,
            url: url.to_string(),
            poster: poster.map(str::to_string),
            width: None,
            height: None,
        }
    }

    fn photo(url: &str) -> MediaItem {
        MediaItem {
            kind: MediaKind::Photo,
            url: url.to_string(),
            poster: None,
            width: None,
            height: None,
        }
    }

    fn lookup(cache: &HashMap<String, CacheLookup>) -> impl Fn(&str) -> CacheLookup + '_ {
        move |key| cache.get(key).copied().unwrap_or(CacheLookup::Missing)
    }

    #[test]
    fn provider_still_is_used_and_a_video_url_is_not_a_poster() {
        let poster = "https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg";
        let mp4 = "https://video.twimg.com/amplify_video/9/vid/avc1/320x568/a.mp4?tag=29";
        let plan = preview_plan(&video(mp4, Some(poster)));
        assert_eq!(
            plan,
            PreviewPlan::Still {
                url: poster.to_string(),
                video: Some(mp4.to_string()),
            }
        );
        assert_eq!(
            preview_plan(&video(mp4, Some(mp4))),
            PreviewPlan::Frame {
                video: mp4.to_string(),
            }
        );
        assert_eq!(
            preview_plan(&video(mp4, None)),
            PreviewPlan::Frame {
                video: mp4.to_string(),
            }
        );
        assert!(matches!(
            preview_plan(&photo(poster)),
            PreviewPlan::Still { video: None, .. }
        ));
        assert_eq!(
            preview_plan(&video("https://x.com/a/status/1", None)),
            PreviewPlan::Unavailable
        );
        assert!(!FRAME_CAPTURE_SELECTS_AUDIO);
        assert!(!FRAME_CAPTURE_OPENS_WINDOW);
        assert_eq!(FRAME_SAMPLE_CAP, 8);
    }

    #[test]
    fn missing_or_broken_poster_ends_instead_of_spinning() {
        let poster = "https://pbs.twimg.com/amplify_video_thumb/9/img/p.jpg";
        let mp4 = "https://video.twimg.com/ext_tw_video/1/pu/vid/a.mp4";
        let still = preview_plan(&video(mp4, Some(poster)));
        let frame = preview_plan(&video(mp4, None));
        let mut cache = HashMap::new();
        assert_eq!(
            preview_view(&still, lookup(&cache)),
            PreviewView::Unavailable
        );
        assert_eq!(
            preview_caption(MediaKind::Video, PreviewView::Unavailable),
            "Video preview unavailable"
        );
        assert_ne!(
            preview_caption(MediaKind::Video, PreviewView::Unavailable),
            preview_caption(MediaKind::Video, PreviewView::Loading)
        );
        cache.insert(poster.to_string(), CacheLookup::Loading);
        assert_eq!(preview_view(&still, lookup(&cache)), PreviewView::Loading);
        cache.insert(poster.to_string(), CacheLookup::Failed);
        assert_eq!(
            preview_view(&still, lookup(&cache)),
            PreviewView::Unavailable
        );
        cache.insert(frame_cache_key(mp4), CacheLookup::Loading);
        assert_eq!(preview_view(&still, lookup(&cache)), PreviewView::Loading);
        cache.insert(frame_cache_key(mp4), CacheLookup::Failed);
        assert_eq!(
            preview_view(&still, lookup(&cache)),
            PreviewView::Unavailable
        );
        cache.insert(frame_cache_key(mp4), CacheLookup::Ready);
        assert_eq!(preview_view(&still, lookup(&cache)), PreviewView::Image);
        assert_eq!(
            ready_key(&still, lookup(&cache)).as_deref(),
            Some(frame_cache_key(mp4).as_str())
        );
        assert_eq!(
            preview_view(&frame, lookup(&HashMap::new())),
            PreviewView::Unavailable
        );
        cache.clear();
        cache.insert(frame_cache_key(mp4), CacheLookup::Loading);
        assert_eq!(preview_view(&frame, lookup(&cache)), PreviewView::Loading);
    }

    #[test]
    fn repost_uses_the_original_poster_and_a_quote_stays_separate() {
        let original_poster = "https://pbs.twimg.com/amplify_video_thumb/9/img/orig.jpg";
        let original_mp4 = "https://video.twimg.com/amplify_video/9/vid/avc1/320x568/a.mp4";
        let mut original = Post::default();
        original.id = "40".into();
        original.author_handle = "orig".into();
        original.media = vec![video(original_mp4, Some(original_poster))];
        let mut repost = Post::default();
        repost.id = "50".into();
        repost.author_handle = "reposter".into();
        repost.text = "RT @orig: hello".into();
        repost.retweeted = Some(Box::new(original.clone()));
        let shown = displayed_preview(&repost).unwrap();
        assert_eq!(shown.cache_key().as_deref(), Some(original_poster));
        assert!(
            preview_jobs(&repost)
                .iter()
                .any(|plan| plan.cache_key().as_deref() == Some(original_poster))
        );

        let quote_poster = "https://pbs.twimg.com/media/quote.jpg";
        let mut quoted = Post::default();
        quoted.id = "70".into();
        quoted.author_handle = "quoted".into();
        quoted.media = vec![video(original_mp4, Some(quote_poster))];
        let mut quote = Post::default();
        quote.id = "80".into();
        quote.author_handle = "quoter".into();
        quote.text = "my words".into();
        quote.quoted = Some(Box::new(quoted));
        quote.media = vec![photo("https://pbs.twimg.com/media/mine.jpg")];
        let shown = displayed_preview(&quote).unwrap();
        assert_eq!(
            shown.cache_key().as_deref(),
            Some("https://pbs.twimg.com/media/mine.jpg")
        );
        let jobs = preview_jobs(&quote);
        assert!(
            jobs.iter()
                .any(|plan| plan.cache_key().as_deref() == Some(quote_poster))
        );
        assert!(jobs.iter().any(
            |plan| plan.cache_key().as_deref() == Some("https://pbs.twimg.com/media/mine.jpg")
        ));
        assert_ne!(shown.cache_key().as_deref(), Some(quote_poster));
    }

    #[test]
    fn a_recycled_row_drops_the_previous_posters_result() {
        let first = "https://pbs.twimg.com/media/a.jpg";
        let second = "https://pbs.twimg.com/media/b.jpg";
        let slot = RowSlot {
            generation: 1,
            key: first.to_string(),
        };
        let recycled = recycle_row(&slot, second);
        assert!(!accept_row_result(&recycled, slot.generation, first));
        assert!(accept_row_result(&recycled, recycled.generation, second));
        let mut cache = HashMap::new();
        cache.insert(first.to_string(), CacheLookup::Ready);
        let current = preview_plan(&photo(second));
        assert_ne!(preview_view(&current, lookup(&cache)), PreviewView::Image);
        assert_eq!(
            preview_view(&current, lookup(&cache)),
            PreviewView::Unavailable
        );
    }

    #[test]
    fn embedded_jpeg_is_taken_from_a_video_prefix_and_a_broken_one_is_not() {
        let jpeg = {
            let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Jpeg,
                )
                .unwrap();
            bytes
        };
        assert!(jpeg.len() >= 32);
        let mut prefix = b"ftypisomjunk".to_vec();
        prefix.extend_from_slice(&jpeg);
        prefix.extend_from_slice(b"mdat");
        let found = embedded_jpeg(&prefix).unwrap();
        assert!(decode_still(found).is_ok());
        assert!(embedded_jpeg(b"no jpeg here at all..............").is_none());
        let mut broken = jpeg.clone();
        broken.pop();
        broken.pop();
        assert!(embedded_jpeg(&broken).is_none());
    }
}
