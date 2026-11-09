//! Compact, engine-owned terminal values. No renderer or PTY dependencies.

use std::sync::Arc;

/// Marks retained per cell. The longest standard emoji ZWJ suffix is about
/// 31 bytes; a larger cap only lets hostile output grow every cell (and makes
/// each append, which copies the suffix, quadratic in the cap).
pub(super) const MAX_COMBINING_BYTES: usize = 64;

/// A color occupies four bytes, including its default/indexed/RGB tag. Bytes
/// rather than a `u32` keep [`Style`] 1-aligned and free of padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color([u8; 4]);

impl Color {
    pub const DEFAULT: Self = Self([0; 4]);
    pub const fn indexed(index: u8) -> Self {
        Self([1, 0, 0, index])
    }
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([2, r, g, b])
    }
    pub const fn as_indexed(self) -> Option<u8> {
        if self.0[0] == 1 {
            Some(self.0[3])
        } else {
            None
        }
    }
    pub const fn as_rgb(self) -> Option<(u8, u8, u8)> {
        if self.0[0] == 2 {
            Some((self.0[1], self.0[2], self.0[3]))
        } else {
            None
        }
    }

    /// The tag and payload bytes, as stored in snapshots.
    pub(super) const fn to_bytes(self) -> [u8; 4] {
        self.0
    }

    /// Inverse of [`Self::to_bytes`]; `None` for an unknown tag or a
    /// non-canonical default/indexed payload.
    pub(super) const fn from_bytes(bytes: [u8; 4]) -> Option<Self> {
        match bytes {
            [0, 0, 0, 0] | [1, 0, 0, _] | [2, _, _, _] => Some(Self(bytes)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl UnderlineStyle {
    pub(super) const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::Single,
            2 => Self::Double,
            3 => Self::Curly,
            4 => Self::Dotted,
            5 => Self::Dashed,
            _ => return None,
        })
    }
}

/// Ten 1-aligned bytes, so a cell with its flags and character is 16 B.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub foreground: Color,
    pub background: Color,
    pub attributes: u8,
    pub underline: UnderlineStyle,
}

impl Style {
    pub const BOLD: u8 = 1;
    pub const DIM: u8 = 1 << 1;
    pub const ITALIC: u8 = 1 << 2;
    pub const INVERSE: u8 = 1 << 3;
    pub const HIDDEN: u8 = 1 << 4;
    pub const STRIKE: u8 = 1 << 5;
    pub const BLINK: u8 = 1 << 6;
    pub const PROTECTED: u8 = 1 << 7;

    pub fn set(&mut self, flags: u8, enabled: bool) {
        if enabled {
            self.attributes |= flags;
        } else {
            self.attributes &= !flags;
        }
    }

    /// The style's ten bytes as two words, in memory order: equal styles
    /// have equal words, so hot loops compare two words instead of fields.
    #[inline(always)]
    pub(crate) fn bits(self) -> (u64, u16) {
        // SAFETY: `Style` is ten 1-byte fields (`UnderlineStyle` is
        // `repr(u8)`), so it has no padding and every byte is initialized.
        let [s0, s1, s2, s3, s4, s5, s6, s7, s8, s9] =
            unsafe { std::mem::transmute::<Self, [u8; 10]>(self) };
        (
            u64::from_ne_bytes([s0, s1, s2, s3, s4, s5, s6, s7]),
            u16::from_ne_bytes([s8, s9]),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hyperlink {
    pub id: String,
    pub uri: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellExtra {
    pub combining: String,
    pub hyperlink: Option<Arc<Hyperlink>>,
    /// SGR 58. Rare, so it lives here instead of on every cell's style.
    pub underline_color: Color,
}

impl CellExtra {
    fn is_empty(&self) -> bool {
        self.combining.is_empty()
            && self.hyperlink.is_none()
            && self.underline_color == Color::DEFAULT
    }
}

/// Plain cells allocate nothing. Uncommon combining text, OSC 8 metadata and
/// underline colors live in their row's metadata table (see [`Line`]), behind
/// a shared pointer so row copies do not copy strings.
/// `repr(C)` keeps every field in place with no padding, and the alignment
/// keeps each cell within one cache line, so the print loops store a cell
/// with one 16-byte write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, align(16))]
pub struct Cell {
    pub style: Style,
    pub flags: u16,
    pub character: char,
}

const _: () = assert!(size_of::<Style>() == 10 && size_of::<Cell>() == 16);
// The field offsets `PlainCell` assembles its words from.
const _: () = assert!(
    std::mem::offset_of!(Cell, style) == 0
        && std::mem::offset_of!(Cell, flags) == 10
        && std::mem::offset_of!(Cell, character) == 12
);

/// A cell as the two words it occupies, so the print and clear loops store
/// each cell with one 16-byte write instead of one per field (a 10-byte style
/// and 2-byte flags otherwise split into narrow stores, reloaded every
/// iteration because the output may alias them).
#[derive(Clone, Copy)]
pub(super) struct PlainCell {
    words: [u64; 2],
}

impl PlainCell {
    #[inline(always)]
    fn new(style: Style, flags: u16) -> Self {
        // `Cell` is `repr(C)`: the style's ten bytes, the flags and the
        // character, with no padding (asserted above). The words are
        // assembled in registers: a cell built on the stack and reread as
        // words would make each template wait for every store queued before it.
        let (head, [s8, s9]) = (style.bits().0, style.bits().1.to_ne_bytes());
        let [l0, l1] = flags.to_ne_bytes();
        Self {
            words: [head, u64::from_ne_bytes([s8, s9, l0, l1, 0, 0, 0, 0])],
        }
    }

    /// This template with `flags` instead of its own: the same style words,
    /// which a loop storing both templates then holds once.
    #[inline(always)]
    pub(super) fn with_flags(self, flags: u16) -> Self {
        let [s8, s9, ..] = self.words[1].to_ne_bytes();
        let [l0, l1] = flags.to_ne_bytes();
        Self {
            words: [
                self.words[0],
                u64::from_ne_bytes([s8, s9, l0, l1, 0, 0, 0, 0]),
            ],
        }
    }

    /// Overwrite `cell` with this template and `character`.
    #[inline(always)]
    pub(super) fn store(self, cell: &mut Cell, character: char) {
        // `character` is the last field: the high half of the last word.
        let shift = if cfg!(target_endian = "little") {
            32
        } else {
            0
        };
        let last = self.words[1] | (u64::from(u32::from(character)) << shift);
        // SAFETY: as in `new`, these words are the bytes of a valid `Cell`;
        // a cell is 16 bytes and 16-aligned, so both words are in bounds and
        // aligned.
        unsafe {
            std::ptr::from_mut(cell)
                .cast::<[u64; 2]>()
                .write([self.words[0], last]);
        }
    }

    /// The cell's two words, with no character: `store` adds one to the
    /// high half of the second (on little-endian targets).
    #[inline(always)]
    pub(super) fn words(self) -> [u64; 2] {
        self.words
    }

    /// `store` each byte of `text`, printable ASCII, into `cells`, as far as
    /// both reach. Sixteen bytes at a time with the target's baseline
    /// vectors (SSE2, NEON); the rest one at a time.
    #[inline]
    pub(super) fn store_ascii(self, cells: &mut [Cell], text: &[u8]) {
        let len = cells.len().min(text.len());
        let (cells, text) = (&mut cells[..len], &text[..len]);
        let done = self.store_ascii_blocks(cells, text);
        for (cell, &byte) in cells[done..].iter_mut().zip(&text[done..]) {
            self.store(cell, char::from(byte));
        }
    }

    /// The vector part of `store_ascii` for `cells` and `text` of one
    /// length, returning how many it stored. Each byte widens to a
    /// character word, which is interleaved with the template's words: a
    /// cell per register, stored whole.
    #[cfg(target_arch = "x86_64")]
    #[inline(always)]
    fn store_ascii_blocks(self, cells: &mut [Cell], text: &[u8]) -> usize {
        use std::arch::x86_64::{
            __m128i, _mm_loadu_si128, _mm_set1_epi32, _mm_set1_epi64x, _mm_setzero_si128,
            _mm_store_si128, _mm_unpackhi_epi8, _mm_unpackhi_epi16, _mm_unpackhi_epi32,
            _mm_unpackhi_epi64, _mm_unpacklo_epi8, _mm_unpacklo_epi16, _mm_unpacklo_epi32,
            _mm_unpacklo_epi64,
        };
        let blocks = text.len() / 16;
        // SAFETY: SSE2 is part of the x86_64 baseline. Block `n` reads bytes
        // `16n..16n + 16` of `text` and writes cells `16n..16n + 16` of
        // `cells`, which have its length, so both stay in bounds. Cells are
        // 16-aligned and 16 bytes. Each store is the two template words, the
        // low half of the second (style tail and flags) then a character in
        // its high half (x86 is little-endian): the bytes `store` writes.
        unsafe {
            let head = _mm_set1_epi64x(self.words[0] as i64);
            let tail = _mm_set1_epi32(self.words[1] as u32 as i32);
            let zero = _mm_setzero_si128();
            for block in 0..blocks {
                let bytes = _mm_loadu_si128(text.as_ptr().add(block * 16).cast::<__m128i>());
                let (low, high) = (
                    _mm_unpacklo_epi8(bytes, zero),
                    _mm_unpackhi_epi8(bytes, zero),
                );
                let quads = [
                    _mm_unpacklo_epi16(low, zero),
                    _mm_unpackhi_epi16(low, zero),
                    _mm_unpacklo_epi16(high, zero),
                    _mm_unpackhi_epi16(high, zero),
                ];
                let out = cells.as_mut_ptr().add(block * 16).cast::<__m128i>();
                for (quad, characters) in quads.into_iter().enumerate() {
                    // [tail, c0, tail, c1] and [tail, c2, tail, c3].
                    let first = _mm_unpacklo_epi32(tail, characters);
                    let second = _mm_unpackhi_epi32(tail, characters);
                    let out = out.add(quad * 4);
                    _mm_store_si128(out, _mm_unpacklo_epi64(head, first));
                    _mm_store_si128(out.add(1), _mm_unpackhi_epi64(head, first));
                    _mm_store_si128(out.add(2), _mm_unpacklo_epi64(head, second));
                    _mm_store_si128(out.add(3), _mm_unpackhi_epi64(head, second));
                }
            }
        }
        blocks * 16
    }

    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    #[inline(always)]
    fn store_ascii_blocks(self, cells: &mut [Cell], text: &[u8]) -> usize {
        use std::arch::aarch64::{
            vdupq_n_u32, vdupq_n_u64, vget_low_u8, vget_low_u16, vld1q_u8, vmovl_high_u8,
            vmovl_high_u16, vmovl_u8, vmovl_u16, vreinterpretq_u64_u32, vst1q_u64, vzip1q_u32,
            vzip1q_u64, vzip2q_u32, vzip2q_u64,
        };
        let blocks = text.len() / 16;
        // SAFETY: NEON is part of the aarch64 baseline. Bounds, alignment
        // and the stored bytes are as in the x86_64 version (this target is
        // little-endian too).
        unsafe {
            let head = vdupq_n_u64(self.words[0]);
            let tail = vdupq_n_u32(self.words[1] as u32);
            for block in 0..blocks {
                let bytes = vld1q_u8(text.as_ptr().add(block * 16));
                let (low, high) = (vmovl_u8(vget_low_u8(bytes)), vmovl_high_u8(bytes));
                let quads = [
                    vmovl_u16(vget_low_u16(low)),
                    vmovl_high_u16(low),
                    vmovl_u16(vget_low_u16(high)),
                    vmovl_high_u16(high),
                ];
                let out = cells.as_mut_ptr().add(block * 16).cast::<u64>();
                for (quad, characters) in quads.into_iter().enumerate() {
                    // [tail, c0, tail, c1] and [tail, c2, tail, c3].
                    let first = vreinterpretq_u64_u32(vzip1q_u32(tail, characters));
                    let second = vreinterpretq_u64_u32(vzip2q_u32(tail, characters));
                    let out = out.add(quad * 8);
                    vst1q_u64(out, vzip1q_u64(head, first));
                    vst1q_u64(out.add(2), vzip2q_u64(head, first));
                    vst1q_u64(out.add(4), vzip1q_u64(head, second));
                    vst1q_u64(out.add(6), vzip2q_u64(head, second));
                }
            }
        }
        blocks * 16
    }

    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_arch = "aarch64", target_endian = "little")
    )))]
    #[inline(always)]
    fn store_ascii_blocks(self, _cells: &mut [Cell], _text: &[u8]) -> usize {
        0
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            character: ' ',
            style: Style::default(),
            flags: 0,
        }
    }
}

impl Cell {
    pub const WIDE: u16 = 1;
    pub const WIDE_SPACER: u16 = 1 << 1;
    pub const LEADING_WIDE_SPACER: u16 = 1 << 2;

    /// A word-store template for cells in `style`; see [`PlainCell`].
    #[inline(always)]
    pub(super) fn plain(style: Style, flags: u16) -> PlainCell {
        PlainCell::new(style, flags)
    }

    /// Fill `cells` with a plain character in `style`.
    #[inline]
    pub(super) fn fill_plain(cells: &mut [Cell], character: char, style: Style) {
        let template = PlainCell::new(style, 0);
        for cell in cells {
            template.store(cell, character);
        }
    }

    /// Write printable ASCII into `cells`.
    #[inline]
    pub(super) fn write_plain_ascii(cells: &mut [Cell], text: &[u8], style: Style) {
        PlainCell::new(style, 0).store_ascii(cells, text);
    }

    /// The right half of a wide character. It keeps the style so decorations
    /// span both columns, but no text; see [`ExtraSlot::spacer_extra`] for
    /// its metadata.
    #[inline]
    pub(super) fn wide_spacer(&self) -> Self {
        Self {
            character: ' ',
            style: self.style,
            flags: Self::WIDE_SPACER,
        }
    }
}

/// A cell's metadata where it is held apart from the cell: the pen's, or a
/// row table entry taken out to edit.
pub(super) trait ExtraSlot {
    fn combining(&self) -> &str;
    fn hyperlink(&self) -> Option<&Arc<Hyperlink>>;
    fn underline_color(&self) -> Color;
    fn set_underline_color(&mut self, color: Color);
    fn push_combining(&mut self, character: char);
    /// Metadata for this wide cell's spacer: only its underline color.
    /// `cache` keeps the last color-only extra built for a cell that also
    /// has text or a link, so a run of such glyphs shares one allocation.
    fn spacer_extra(&self, cache: &mut Option<Arc<CellExtra>>) -> Option<Arc<CellExtra>>;
}

impl ExtraSlot for Option<Arc<CellExtra>> {
    fn combining(&self) -> &str {
        self.as_ref().map_or("", |extra| extra.combining.as_str())
    }
    fn hyperlink(&self) -> Option<&Arc<Hyperlink>> {
        self.as_ref().and_then(|extra| extra.hyperlink.as_ref())
    }
    fn underline_color(&self) -> Color {
        self.as_ref()
            .map_or(Color::DEFAULT, |extra| extra.underline_color)
    }
    fn set_underline_color(&mut self, color: Color) {
        if self.underline_color() == color {
            return;
        }
        let extra = self.get_or_insert_with(|| Arc::new(CellExtra::default()));
        Arc::make_mut(extra).underline_color = color;
        if extra.is_empty() {
            *self = None;
        }
    }
    fn push_combining(&mut self, character: char) {
        // Bound a hostile stream of combining marks attached to a single cell.
        if self.combining().len().saturating_add(character.len_utf8()) > MAX_COMBINING_BYTES {
            return;
        }
        let extra = self.get_or_insert_with(|| Arc::new(CellExtra::default()));
        // Exact capacity, like every other path that builds combining text,
        // so history's metadata charge (which counts its length) is real.
        let combining = &mut Arc::make_mut(extra).combining;
        combining.reserve_exact(character.len_utf8());
        combining.push(character);
    }
    #[inline]
    fn spacer_extra(&self, cache: &mut Option<Arc<CellExtra>>) -> Option<Arc<CellExtra>> {
        self.as_ref()
            .and_then(|extra| spacer_extra_of(extra, cache))
    }
}

#[cold]
fn spacer_extra_of(
    extra: &Arc<CellExtra>,
    cache: &mut Option<Arc<CellExtra>>,
) -> Option<Arc<CellExtra>> {
    if extra.underline_color == Color::DEFAULT {
        None
    } else if extra.combining.is_empty() && extra.hyperlink.is_none() {
        Some(Arc::clone(extra))
    } else {
        // Cells copy-on-write shared extras, so the cached one never changes.
        if cache
            .as_ref()
            .is_none_or(|cached| cached.underline_color != extra.underline_color)
        {
            *cache = Some(Arc::new(CellExtra {
                underline_color: extra.underline_color,
                ..CellExtra::default()
            }));
        }
        cache.clone()
    }
}

/// The rendition printed cells take: a style, and the metadata (an OSC 8
/// link, an SGR 58 color) they share.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Pen {
    pub(super) style: Style,
    pub(super) extra: Option<Arc<CellExtra>>,
}

impl Pen {
    pub(super) fn hyperlink(&self) -> Option<&Arc<Hyperlink>> {
        self.extra.hyperlink()
    }
    pub(super) fn underline_color(&self) -> Color {
        self.extra.underline_color()
    }
    pub(super) fn set_underline_color(&mut self, color: Color) {
        self.extra.set_underline_color(color);
    }
}

/// A row's cells, with the metadata of those that have some: combining
/// text, an OSC 8 link, an underline color. Indexes like the cell slice it
/// derefs to; column arguments count from its first cell.
#[derive(Clone, Copy)]
pub struct Line<'a> {
    cells: &'a [Cell],
    // Sorted by column, counted from the row's first cell, `start` columns
    // before this line's first cell.
    extras: &'a [(usize, Arc<CellExtra>)],
    start: usize,
}

impl<'a> Line<'a> {
    pub(super) fn new(cells: &'a [Cell], extras: &'a [(usize, Arc<CellExtra>)]) -> Self {
        Self {
            cells,
            extras,
            start: 0,
        }
    }

    pub fn cells(&self) -> &'a [Cell] {
        self.cells
    }

    /// The metadata of the cell at `col`, if it has any.
    #[inline]
    pub fn extra(&self, col: usize) -> Option<&'a CellExtra> {
        if self.extras.is_empty() {
            return None;
        }
        let col = self.start + col;
        self.extras
            .binary_search_by_key(&col, |(col, _)| *col)
            .ok()
            .map(|index| &*self.extras[index].1)
    }

    pub fn combining(&self, col: usize) -> &'a str {
        self.extra(col).map_or("", |extra| extra.combining.as_str())
    }

    pub fn hyperlink(&self, col: usize) -> Option<&'a Arc<Hyperlink>> {
        self.extra(col).and_then(|extra| extra.hyperlink.as_ref())
    }

    pub fn underline_color(&self, col: usize) -> Color {
        self.extra(col)
            .map_or(Color::DEFAULT, |extra| extra.underline_color)
    }

    /// The cells of `range`, with their metadata.
    pub fn slice(&self, range: std::ops::Range<usize>) -> Self {
        let cells = &self.cells[range.clone()];
        let (first, last) = (self.start + range.start, self.start + range.end);
        let from = self.extras.partition_point(|(col, _)| *col < first);
        let to = self.extras.partition_point(|(col, _)| *col < last);
        Self {
            cells,
            extras: &self.extras[from..to],
            start: first,
        }
    }

    /// Each cell that has metadata: its column and the metadata.
    pub fn extras(&self) -> impl Iterator<Item = (usize, &'a CellExtra)> + 'a {
        let start = self.start;
        self.extras
            .iter()
            .map(move |(col, extra)| (col - start, &**extra))
    }
}

#[cfg(test)]
impl Line<'_> {
    /// An owned copy of the cells and their metadata, to compare later.
    pub(crate) fn snapshot(&self) -> (Vec<Cell>, Vec<(usize, CellExtra)>) {
        let extras = self.extras().map(|(col, extra)| (col, extra.clone()));
        (self.cells.to_vec(), extras.collect())
    }
}

impl std::ops::Deref for Line<'_> {
    type Target = [Cell];

    fn deref(&self) -> &[Cell] {
        self.cells
    }
}

impl PartialEq for Line<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cells == other.cells && self.extras().eq(other.extras())
    }
}

impl Eq for Line<'_> {}

// Plain cells carry no metadata: a line equals them only when it has none.
impl PartialEq<[Cell]> for Line<'_> {
    fn eq(&self, other: &[Cell]) -> bool {
        self.extras.is_empty() && self.cells == other
    }
}

impl PartialEq<&[Cell]> for Line<'_> {
    fn eq(&self, other: &&[Cell]) -> bool {
        *self == **other
    }
}

impl PartialEq<Vec<Cell>> for Line<'_> {
    fn eq(&self, other: &Vec<Cell>) -> bool {
        *self == **other
    }
}

impl PartialEq<&Vec<Cell>> for Line<'_> {
    fn eq(&self, other: &&Vec<Cell>) -> bool {
        *self == ***other
    }
}

impl<'a> IntoIterator for Line<'a> {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl<'a> IntoIterator for &Line<'a> {
    type Item = &'a Cell;
    type IntoIter = std::slice::Iter<'a, Cell>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl std::fmt::Debug for Line<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Line")
            .field("cells", &self.cells)
            .field("extras", &self.extras().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub visible: bool,
    pub shape: CursorShape,
    pub blinking: bool,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            visible: true,
            shape: CursorShape::Block,
            blinking: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirtySpan {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

/// Column ranges are half open, matching Rust slice indexing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    Full,
    Partial(Vec<DirtySpan>),
}

/// A viewport row rotation to replay before applying the accompanying damage.
/// `bottom` is exclusive; positive `lines` move rows up, negative values down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportScroll {
    pub top: usize,
    pub bottom: usize,
    pub lines: i32,
}

/// Ordered text-grid operations consumed by the graphics placement bridge.
/// Scroll regions and column spans both use exclusive upper bounds. Positive
/// line counts move cells up; negative counts move them down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridEffect {
    Scroll {
        alternate: bool,
        top: usize,
        bottom: usize,
        lines: i64,
        retains_history: bool,
        history_before: usize,
        history_after: usize,
    },
    Clear {
        alternate: bool,
        history_size: usize,
    },
    ClearHistory {
        removed: usize,
    },
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: usize,
    pub rows: usize,
}

impl Default for Size {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

impl Size {
    pub const MAX_COLS: usize = 4096;
    pub const MAX_ROWS: usize = 4096;
    pub const MAX_CELLS: usize = 1_048_576;
    pub(super) fn clamped(self) -> Self {
        let cols = self.cols.clamp(1, Self::MAX_COLS);
        let rows = self
            .rows
            .clamp(1, Self::MAX_ROWS)
            .min(Self::MAX_CELLS / cols);
        Self { cols, rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The vector stores must write exactly what storing each byte does,
    // for every length around the block size and at unaligned text.
    #[test]
    fn ascii_blocks_store_what_single_cells_do() {
        let style = Style {
            foreground: Color::rgb(1, 2, 3),
            background: Color::indexed(200),
            attributes: Style::BOLD | Style::ITALIC,
            underline: UnderlineStyle::Curly,
        };
        let text: Vec<u8> = (0..80).map(|index| b' ' + (index * 7 % 95) as u8).collect();
        for flags in [0, Cell::WIDE] {
            let template = PlainCell::new(style, flags);
            for start in 0..3 {
                for len in 0..text.len() - start {
                    let text = &text[start..start + len];
                    let mut expected = vec![Cell::default(); len + 2];
                    for (cell, &byte) in expected.iter_mut().zip(text) {
                        template.store(cell, char::from(byte));
                    }
                    let mut cells = vec![Cell::default(); len + 2];
                    template.store_ascii(&mut cells, text);
                    assert_eq!(cells, expected, "start {start}, length {len}");
                    // Shorter cells bound the store too.
                    let mut short = vec![Cell::default(); len / 2];
                    template.store_ascii(&mut short, text);
                    assert_eq!(short, expected[..len / 2], "start {start}, length {len}");
                }
            }
        }
    }
}
