use std::sync::{Arc, Mutex, Weak};

/// Immutable image pixels shared by protocol storage, snapshots and renderers.
/// PNG encoding is deferred until an export or clipboard consumer requests it.
#[derive(Debug)]
pub struct GraphicsImage {
    pub width: u32,
    pub height: u32,
    // `Arc<Vec>` adopts the decoded buffer; `Arc<[u8]>` would copy it.
    rgba: Option<Arc<Vec<u8>>>,
    png: Option<Arc<Vec<u8>>>,
    png_export: Mutex<Weak<Vec<u8>>>,
    encoded_source_bytes: usize,
    #[cfg(test)]
    png_encodings: std::sync::atomic::AtomicUsize,
}

impl PartialEq for GraphicsImage {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.rgba == other.rgba
            && (self.rgba.is_some() || self.png == other.png)
    }
}
impl Eq for GraphicsImage {}

/// See [`GraphicsImage::snapshot_parts`].
pub(crate) type SnapshotParts<'a> = (Option<&'a [u8]>, Option<(&'a [u8], usize)>);

impl GraphicsImage {
    pub fn from_rgba(width: u32, height: u32, mut rgba: Vec<u8>) -> Self {
        assert_eq!(u64::from(width) * u64::from(height) * 4, rgba.len() as u64);
        // `byte_len` charges the pixels only. Chunk assembly and inflation
        // can leave up to as much again in spare capacity.
        rgba.shrink_to_fit();
        Self {
            width,
            height,
            rgba: Some(Arc::new(rgba)),
            png: None,
            png_export: Mutex::new(Weak::new()),
            encoded_source_bytes: 0,
            #[cfg(test)]
            png_encodings: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn from_png(width: u32, height: u32, png: Vec<u8>) -> Self {
        let encoded_source_bytes = png.capacity();
        Self {
            width,
            height,
            rgba: None,
            encoded_source_bytes,
            png: Some(Arc::new(png)),
            png_export: Mutex::new(Weak::new()),
            #[cfg(test)]
            png_encodings: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Decoded pixels, else the PNG source and the bytes it was charged.
    pub(crate) fn snapshot_parts(&self) -> SnapshotParts<'_> {
        (
            self.rgba(),
            self.png
                .as_deref()
                .map(|png| (png.as_slice(), self.encoded_source_bytes)),
        )
    }

    /// The image [`Self::snapshot_parts`] described.
    pub(crate) fn from_png_charged(
        width: u32,
        height: u32,
        png: Vec<u8>,
        encoded_source_bytes: usize,
    ) -> Self {
        Self {
            encoded_source_bytes,
            ..Self::from_png(width, height, png)
        }
    }

    pub fn rgba(&self) -> Option<&[u8]> {
        self.rgba.as_deref().map(Vec::as_slice)
    }

    /// Writable pixels, for editing an image nobody else holds yet.
    pub(crate) fn rgba_mut(&mut self) -> Option<&mut [u8]> {
        // A cached export would describe the old pixels.
        *self
            .png_export
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Weak::new();
        self.rgba
            .as_mut()
            .map(|rgba| Arc::make_mut(rgba).as_mut_slice())
    }

    #[cfg(test)]
    pub(crate) fn rgba_capacity(&self) -> usize {
        self.rgba.as_ref().map_or(0, |rgba| rgba.capacity())
    }

    /// Decoded size is charged even for compressed PNGs, bounding texture memory.
    /// PNG exports of RGBA pixels belong to their callers and are not retained
    /// by this image after the last export owner releases them.
    pub fn byte_len(&self) -> usize {
        (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4)
            .saturating_add(self.encoded_source_bytes)
    }

    /// Return an owned PNG export, sharing encoding work with overlapping exports.
    /// Retain this handle while exporting several placements of the same image.
    pub fn png(&self) -> Arc<Vec<u8>> {
        if let Some(png) = &self.png {
            return png.clone();
        }
        let mut cached = self
            .png_export
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(png) = cached.upgrade() {
            return png;
        }
        let png = Arc::new(super::encode_png(
            self.width,
            self.height,
            4,
            self.rgba().unwrap_or_default(),
        ));
        // Weak<Vec<u8>> retains only the Vec header after the last strong
        // owner drops; Weak<[u8]> would retain the inline pixel allocation.
        *cached = Arc::downgrade(&png);
        #[cfg(test)]
        self.png_encodings
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        png
    }

    #[cfg(test)]
    pub(crate) fn png_encoding_count(&self) -> usize {
        self.png_encodings
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
