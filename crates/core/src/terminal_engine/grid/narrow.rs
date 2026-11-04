//! Words of the common two-byte scripts (Latin, Greek, Cyrillic) eight
//! scalars at a time. Each group's sixteen bytes are checked and decoded as
//! eight 16-bit lanes and stored as eight cells, with the target's baseline
//! vectors (SSE2 on x86_64, NEON on aarch64).
use super::super::types::PlainCell;
use super::Cell;

/// Store the leading groups of eight two-byte scalars of `bytes` that
/// `narrow_two_byte` accepts into `cells`, from `plain`, as far as both
/// reach while the last cell stays clear (`put_cell` owns the margin), and
/// return how many groups that is. Cells are not checked for wide-glyph
/// repair. Out of line, so the run loop calling it keeps its registers.
#[inline(never)]
pub(super) fn store_octets(cells: &mut [Cell], bytes: &[u8], plain: PlainCell) -> usize {
    let groups = (cells.len().saturating_sub(1) / 8).min(bytes.len() / 16);
    store_groups(cells, bytes, groups, plain)
}

// Each 16-bit lane is a lead byte and the byte after it, little-endian: a
// lead of 110xxxxx and a continuation of 10xxxxxx, decoded to
// xxxxxyyyyyy, in the blocks `narrow_two_byte` accepts.
const ENCODING_MASK: u16 = 0xc0e0;
const ENCODING: u16 = 0x80c0;
const BLOCKS: [(u16, u16); 3] = [(0xc0, 0x2ff), (0x370, 0x482), (0x48a, 0x52f)];

/// Whether group `group` (in bounds) of `bytes` may be eight letters: its
/// eighth pair starts with a lead byte. The caller checked the first; past
/// it, most words end, and this spares the vector checks that would say so.
#[inline(always)]
fn leads_eighth(bytes: &[u8], group: usize) -> bool {
    group == 0 || bytes[group * 16 + 14] & 0xe0 == 0xc0
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn store_groups(cells: &mut [Cell], bytes: &[u8], groups: usize, plain: PlainCell) -> usize {
    use std::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_cmpeq_epi16, _mm_cmpgt_epi16, _mm_cmplt_epi16, _mm_loadu_si128,
        _mm_movemask_epi8, _mm_or_si128, _mm_set1_epi16, _mm_set1_epi32, _mm_set1_epi64x,
        _mm_setzero_si128, _mm_slli_epi16, _mm_srli_epi16, _mm_store_si128, _mm_unpackhi_epi16,
        _mm_unpackhi_epi32, _mm_unpackhi_epi64, _mm_unpacklo_epi16, _mm_unpacklo_epi32,
        _mm_unpacklo_epi64,
    };
    let [head, tail] = plain.words();
    // SAFETY: SSE2 is part of the x86_64 baseline. Group `n` loads bytes
    // `16n..16n + 16` of `bytes` and writes cells `8n..8n + 8`, both in
    // bounds by `groups`. Cells are 16-aligned and 16 bytes. Each store is
    // the template's two words with a character in the high half of the
    // second (x86 is little-endian): the bytes `PlainCell::store` writes.
    unsafe {
        let head = _mm_set1_epi64x(head as i64);
        let tail = _mm_set1_epi32(tail as u32 as i32);
        let zero = _mm_setzero_si128();
        let splat = |value: u16| _mm_set1_epi16(value as i16);
        for group in 0..groups {
            if !leads_eighth(bytes, group) {
                return group;
            }
            let lanes = _mm_loadu_si128(bytes.as_ptr().add(group * 16).cast::<__m128i>());
            let encoded =
                _mm_cmpeq_epi16(_mm_and_si128(lanes, splat(ENCODING_MASK)), splat(ENCODING));
            let value = _mm_or_si128(
                _mm_slli_epi16(_mm_and_si128(lanes, splat(0x1f)), 6),
                _mm_and_si128(_mm_srli_epi16(lanes, 8), splat(0x3f)),
            );
            // Values are below 0x800, so signed compares hold.
            let [first, second, third] = BLOCKS.map(|(low, high)| {
                _mm_and_si128(
                    _mm_cmpgt_epi16(value, splat(low - 1)),
                    _mm_cmplt_epi16(value, splat(high + 1)),
                )
            });
            let accepted = _mm_and_si128(encoded, _mm_or_si128(_mm_or_si128(first, second), third));
            if _mm_movemask_epi8(accepted) != 0xffff {
                return group;
            }
            let out = cells.as_mut_ptr().add(group * 8).cast::<__m128i>();
            let quads = [
                _mm_unpacklo_epi16(value, zero),
                _mm_unpackhi_epi16(value, zero),
            ];
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
    groups
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
#[inline(always)]
fn store_groups(cells: &mut [Cell], bytes: &[u8], groups: usize, plain: PlainCell) -> usize {
    use std::arch::aarch64::{
        vandq_u16, vceqq_u16, vcgeq_u16, vcleq_u16, vdupq_n_u16, vdupq_n_u32, vdupq_n_u64,
        vget_low_u16, vld1q_u8, vminvq_u16, vmovl_high_u16, vmovl_u16, vorrq_u16,
        vreinterpretq_u16_u8, vreinterpretq_u64_u32, vshlq_n_u16, vshrq_n_u16, vst1q_u64,
        vzip1q_u32, vzip1q_u64, vzip2q_u32, vzip2q_u64,
    };
    let [head, tail] = plain.words();
    // SAFETY: NEON is part of the aarch64 baseline. Bounds, alignment and
    // the stored bytes are as in the x86_64 version (this target is
    // little-endian too).
    unsafe {
        let head = vdupq_n_u64(head);
        let tail = vdupq_n_u32(tail as u32);
        for group in 0..groups {
            if !leads_eighth(bytes, group) {
                return group;
            }
            let lanes = vreinterpretq_u16_u8(vld1q_u8(bytes.as_ptr().add(group * 16)));
            let encoded = vceqq_u16(
                vandq_u16(lanes, vdupq_n_u16(ENCODING_MASK)),
                vdupq_n_u16(ENCODING),
            );
            let value = vorrq_u16(
                vshlq_n_u16::<6>(vandq_u16(lanes, vdupq_n_u16(0x1f))),
                vandq_u16(vshrq_n_u16::<8>(lanes), vdupq_n_u16(0x3f)),
            );
            let [first, second, third] = BLOCKS.map(|(low, high)| {
                vandq_u16(
                    vcgeq_u16(value, vdupq_n_u16(low)),
                    vcleq_u16(value, vdupq_n_u16(high)),
                )
            });
            let accepted = vandq_u16(encoded, vorrq_u16(vorrq_u16(first, second), third));
            if vminvq_u16(accepted) != u16::MAX {
                return group;
            }
            let out = cells.as_mut_ptr().add(group * 8).cast::<u64>();
            let quads = [vmovl_u16(vget_low_u16(value)), vmovl_high_u16(value)];
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
    groups
}

#[cfg(not(any(
    target_arch = "x86_64",
    all(target_arch = "aarch64", target_endian = "little")
)))]
#[inline(always)]
fn store_groups(_cells: &mut [Cell], _bytes: &[u8], _groups: usize, _plain: PlainCell) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use crate::terminal_engine::{Engine, Options, Size};

    // Words fed whole go through the eight-at-a-time stores where they are
    // long enough; fed a byte at a time, each scalar is printed on its own.
    // Both leave the same screen, wherever the words start, end or meet
    // wide glyphs and the margin.
    #[test]
    fn octets_store_what_single_scalars_do() {
        let mut seed = 0x6c07_8965_u32;
        let mut next = |bound: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % bound
        };
        // Latin, Greek and Cyrillic letters, a mark the stores turn away,
        // and letters just outside the blocks.
        let letters = [
            'é', 'ž', 'ɐ', 'λ', 'Ω', 'ж', 'Я', 'ӿ', 'ԯ', '\u{301}', '\u{483}', 'Ա',
        ];
        for _ in 0..400 {
            let mut text = String::new();
            for _ in 0..next(24) {
                match next(8) {
                    0 => text.push(' '),
                    1 => text.push('界'),
                    2 => text.push_str(&format!("\x1b[{};{}H", 1 + next(4), 1 + next(31))),
                    3 => text.push(letters[next(letters.len() as u32) as usize]),
                    _ => {
                        let script = [0x430, 0x3b1, 0x100][next(3) as usize];
                        for _ in 0..1 + next(20) {
                            text.push(char::from_u32(script + next(24)).unwrap());
                        }
                    }
                }
            }
            let engine = || Engine::new(Size { cols: 31, rows: 4 }, Options::default());
            let mut whole = engine();
            whole.feed(text.as_bytes());
            let mut bytewise = engine();
            for byte in text.as_bytes() {
                bytewise.feed(&[*byte]);
            }
            assert_eq!(whole.cursor(), bytewise.cursor(), "{text:?}");
            for row in 0..4 {
                assert_eq!(
                    whole.viewport_row(row).unwrap().snapshot(),
                    bytewise.viewport_row(row).unwrap().snapshot(),
                    "row {row} of {text:?}"
                );
            }
        }
    }
}
