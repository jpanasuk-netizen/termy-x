//! Card media from OpenCLI rows: kinds, posters, quoted tweets, and a
//! width-bounded tile grid. Bytes live under the Termy XL cache.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{MediaItem, MediaKind};

pub const MEDIA_CACHE_CAP: u64 = 64 * 1024 * 1024;
const GRID_GAP: f32 = 4.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QuotedTweet {
    pub author: String,
    pub text: String,
    pub media: Vec<MediaItem>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn items_from_post(value: &Value) -> Vec<MediaItem> {
    crate::providers::media_items_of(value)
}

pub fn quoted_from_post(value: &Value) -> Option<QuotedTweet> {
    for key in [
        "quoted",
        "quoted_status",
        "quoted_tweet",
        "quoted_status_result",
    ] {
        let Some(node) = value.get(key).filter(|item| item.is_object()) else {
            continue;
        };
        let inner = node.get("tweet").unwrap_or(node);
        let media = items_from_post(inner);
        let text = text_of(inner);
        let author = author_of(inner);
        if media.is_empty() && text.is_empty() {
            continue;
        }
        return Some(QuotedTweet {
            author,
            text,
            media,
        });
    }
    None
}

fn text_of(value: &Value) -> String {
    for key in ["text", "full_text", "rawContent", "content"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    String::new()
}

fn author_of(value: &Value) -> String {
    if let Some(text) = value.get("author").and_then(Value::as_str) {
        return crate::model::strip_handle(text).to_string();
    }
    for key in ["username", "screen_name", "screenName"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return crate::model::strip_handle(text).to_string();
        }
    }
    if let Some(user) = value.get("user") {
        return author_of(user);
    }
    String::new()
}

pub fn items_from_urls(urls: &[String]) -> Vec<MediaItem> {
    urls.iter()
        .filter(|url| url.starts_with("http"))
        .map(|url| {
            let kind = crate::parse_borrowed::media_kind(url);
            let (width, height) = match crate::model::dims_from_media_url(url) {
                Some((w, h)) => (Some(w), Some(h)),
                None => (None, None),
            };
            MediaItem {
                kind,
                poster: (kind == MediaKind::Photo).then(|| url.clone()),
                url: url.clone(),
                width,
                height,
            }
        })
        .collect()
}

pub fn hydrate_media(post: &mut super::domain::SourcePost) {
    if post.media_items.is_empty() {
        post.media_items = items_from_urls(&post.media);
    }
}

pub fn preview_url(item: &MediaItem) -> Option<String> {
    match item.kind {
        MediaKind::Photo => Some(item.poster.clone().unwrap_or_else(|| item.url.clone())),
        MediaKind::Video | MediaKind::Gif => item.poster.clone().filter(|url| !url.is_empty()),
    }
}

pub fn play_badge(kind: MediaKind) -> Option<&'static str> {
    match kind {
        MediaKind::Photo => None,
        MediaKind::Video => Some("▶"),
        MediaKind::Gif => Some("GIF"),
    }
}

/// Tiles for one card. Every box stays inside `width`.
pub fn tile_grid(width: f32, count: usize) -> Vec<TileBox> {
    let width = if width.is_finite() {
        width.max(1.0)
    } else {
        1.0
    };
    let count = count.min(4);
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        let h = (width * 9.0 / 16.0).clamp(72.0, 220.0);
        return vec![TileBox {
            x: 0.0,
            y: 0.0,
            w: width,
            h,
        }];
    }
    let col_w = ((width - GRID_GAP) / 2.0).max(1.0);
    let row_h = (col_w * 0.75).clamp(64.0, 160.0);
    match count {
        2 => vec![
            TileBox {
                x: 0.0,
                y: 0.0,
                w: col_w,
                h: row_h,
            },
            TileBox {
                x: col_w + GRID_GAP,
                y: 0.0,
                w: col_w,
                h: row_h,
            },
        ],
        3 => vec![
            TileBox {
                x: 0.0,
                y: 0.0,
                w: col_w,
                h: row_h * 2.0 + GRID_GAP,
            },
            TileBox {
                x: col_w + GRID_GAP,
                y: 0.0,
                w: col_w,
                h: row_h,
            },
            TileBox {
                x: col_w + GRID_GAP,
                y: row_h + GRID_GAP,
                w: col_w,
                h: row_h,
            },
        ],
        _ => (0..count)
            .map(|index| {
                let col = (index % 2) as f32;
                let row = (index / 2) as f32;
                TileBox {
                    x: col * (col_w + GRID_GAP),
                    y: row * (row_h + GRID_GAP),
                    w: col_w,
                    h: row_h,
                }
            })
            .collect(),
    }
}

pub fn grid_inside(width: f32, tiles: &[TileBox]) -> bool {
    tiles.iter().all(|tile| {
        tile.w > 0.0
            && tile.h > 0.0
            && tile.x >= -0.5
            && tile.y >= -0.5
            && tile.x + tile.w <= width + 0.5
    })
}

pub fn cache_name(url: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}.bin")
}

pub fn media_cache_dir(root: &Path) -> PathBuf {
    root.join("cache")
}

pub fn cached_file(root: &Path, url: &str) -> PathBuf {
    media_cache_dir(root).join(cache_name(url))
}

pub fn read_cached(root: &Path, url: &str) -> Option<Vec<u8>> {
    let bytes = fs::read(cached_file(root, url)).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

pub fn write_cached(root: &Path, url: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    if bytes.is_empty() {
        return Err("empty image".into());
    }
    let dir = media_cache_dir(root);
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(cache_name(url));
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    let _ = prune_cache(&dir, MEDIA_CACHE_CAP);
    Ok(path)
}

/// Drop the oldest files until the directory is within `cap` bytes.
pub fn prune_cache(dir: &Path, cap: u64) -> Vec<PathBuf> {
    let mut files: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        files.push((path, modified, meta.len()));
    }
    files.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut total: u64 = files.iter().map(|(_, _, len)| *len).sum();
    let mut removed = Vec::new();
    for (path, _, len) in files {
        if total <= cap {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
            removed.push(path);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parser_keeps_photo_video_gif_and_quoted_media() {
        let row = json!({
            "id": "9",
            "author": "ada",
            "text": "two photos and a quote",
            "url": "https://x.com/ada/status/9",
            "media_urls": [
                "https://pbs.twimg.com/media/one.jpg",
                "https://pbs.twimg.com/media/two.jpg"
            ],
            "media_posters": ["", ""],
            "quoted": {
                "id": "8",
                "author": "grace",
                "text": "the clip",
                "media_urls": ["https://video.twimg.com/tweet_video/clip.mp4"],
                "media_posters": ["https://pbs.twimg.com/tweet_video_thumb/clip.jpg"]
            }
        });
        let items = items_from_post(&row);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, MediaKind::Photo);
        assert_eq!(items[1].url, "https://pbs.twimg.com/media/two.jpg");
        let quoted = quoted_from_post(&row).unwrap();
        assert_eq!(quoted.author, "grace");
        assert_eq!(quoted.media.len(), 1);
        assert_eq!(quoted.media[0].kind, MediaKind::Gif);
        assert_eq!(
            quoted.media[0].poster.as_deref(),
            Some("https://pbs.twimg.com/tweet_video_thumb/clip.jpg")
        );
        assert_eq!(
            preview_url(&quoted.media[0]).as_deref(),
            Some("https://pbs.twimg.com/tweet_video_thumb/clip.jpg")
        );
        assert_eq!(play_badge(MediaKind::Video), Some("▶"));
        assert_eq!(play_badge(MediaKind::Gif), Some("GIF"));
        assert!(play_badge(MediaKind::Photo).is_none());
        let post = super::super::domain::SourcePost::from_value(&row).unwrap();
        assert_eq!(post.media_items.len(), 2);
        assert_eq!(post.quoted.as_ref().unwrap().media[0].kind, MediaKind::Gif);
        let card = super::super::cards::card_from_post(&post);
        assert_eq!(card.media_items.len(), 2);
        assert!(card.quoted.is_some());
        let raw = r#"{"id":"1","author":"a","text":"pic","url":"https://x.com/a/status/1","source_url":"https://x.com/a/status/1","likes":1,"reposts":0,"views":0,"media":["https://pbs.twimg.com/media/ABC.jpg"]}"#;
        let mut stored: super::super::domain::SourcePost = serde_json::from_str(raw).unwrap();
        assert!(stored.media_items.is_empty());
        hydrate_media(&mut stored);
        assert_eq!(stored.media_items.len(), 1);
        assert_eq!(stored.media_items[0].kind, MediaKind::Photo);
        assert_eq!(
            stored.media_items[0].url,
            "https://pbs.twimg.com/media/ABC.jpg"
        );
    }

    #[test]
    fn grid_stays_inside_the_card_for_one_through_four() {
        for count in 1..=4 {
            for width in [240.0, 420.0, 687.0] {
                let tiles = tile_grid(width, count);
                assert_eq!(tiles.len(), count);
                assert!(grid_inside(width, &tiles), "{count} at {width}");
            }
        }
        assert!(tile_grid(320.0, 0).is_empty());
        assert!((tile_grid(320.0, 1)[0].w - 320.0).abs() < 0.5);
    }

    #[test]
    fn cache_cap_drops_the_oldest_file() {
        let dir = std::env::temp_dir().join(format!("xl-media-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let root = dir.join("data");
        write_cached(&root, "https://img.example/a.jpg", b"aaaa").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_cached(&root, "https://img.example/b.jpg", b"bbbbbbbb").unwrap();
        let removed = prune_cache(&media_cache_dir(&root), 8);
        assert!(
            removed
                .iter()
                .any(|path| path.ends_with(cache_name("https://img.example/a.jpg")))
        );
        assert!(read_cached(&root, "https://img.example/b.jpg").is_some());
        assert!(read_cached(&root, "https://img.example/a.jpg").is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
