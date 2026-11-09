//! Runs of wide CJK text four scalars at a time. Each group's twelve bytes
//! are shuffled into four words, checked and decoded together, and stored
//! as four glyph and spacer pairs, with byte shuffles the target has (NEON
//! on aarch64, SSSE3 on x86_64 when the processor has it).
use super::super::types::PlainCell;
use super::Cell;

/// Store the leading groups of four three-byte scalars of `bytes` that
/// `wide_cjk` accepts into `pairs`, a glyph from `wide` and a spacer from
/// `spacer` each, as far as both reach, and return how many scalars that
/// is. Cells are not checked for wide-glyph repair. Stops early where
/// fewer than sixteen bytes remain, leaving the tail to the caller.
#[inline]
pub(super) fn store_quads(
    pairs: &mut [[Cell; 2]],
    bytes: &[u8],
    wide: PlainCell,
    spacer: PlainCell,
) -> usize {
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("ssse3") {
        // SAFETY: the processor has SSSE3.
        return unsafe { store_quads_ssse3(pairs, bytes, wide, spacer) };
    }
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    return store_quads_neon(pairs, bytes, wide, spacer);
    #[allow(unreachable_code)]
    {
        let _ = (pairs, bytes, wide, spacer);
        0
    }
}

/// How many groups of four fit in `pairs` and may be loaded, sixteen bytes
/// each, from `bytes`.
#[inline(always)]
fn groups(pairs: usize, bytes: usize) -> usize {
    (pairs / 4).min(bytes.saturating_sub(4) / 12)
}

// The bytes of four scalars as four little-endian words (a fourth byte
// of zero), and the checks and decode `wide_cjk_word` makes of one.
const SHUFFLE: [u8; 16] = [0, 1, 2, 0xff, 3, 4, 5, 0xff, 6, 7, 8, 0xff, 9, 10, 11, 0xff];

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "ssse3")]
unsafe fn store_quads_ssse3(
    pairs: &mut [[Cell; 2]],
    bytes: &[u8],
    wide: PlainCell,
    spacer: PlainCell,
) -> usize {
    use std::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_cmpeq_epi32, _mm_cmpgt_epi32, _mm_cmplt_epi32, _mm_loadu_si128,
        _mm_movemask_epi8, _mm_or_si128, _mm_set_epi64x, _mm_set1_epi32, _mm_set1_epi64x,
        _mm_shuffle_epi8, _mm_slli_epi32, _mm_srli_epi32, _mm_store_si128, _mm_unpackhi_epi32,
        _mm_unpackhi_epi64, _mm_unpacklo_epi32, _mm_unpacklo_epi64,
    };
    let groups = groups(pairs.len(), bytes.len());
    let [glyph_head, glyph_tail] = wide.words();
    let [spacer_head, spacer_tail] = spacer.words();
    // SAFETY: SSE2 and SSSE3 are available. Group `n` loads bytes
    // `12n..12n + 16` of `bytes` and writes pairs `4n..4n + 4`, both in
    // bounds by `groups`. Cells are 16-aligned and 16 bytes. A glyph's
    // words are the template's with the decoded character in the high half
    // of the second (x86 is little-endian), a spacer's the spacer
    // template's with ' ': the bytes `PlainCell::store` writes.
    unsafe {
        let shuffle = _mm_loadu_si128(SHUFFLE.as_ptr().cast::<__m128i>());
        let head = _mm_set1_epi64x(glyph_head as i64);
        let tail = _mm_set1_epi32(glyph_tail as u32 as i32);
        let blank = _mm_set_epi64x(
            (spacer_tail | u64::from(u32::from(' ')) << 32) as i64,
            spacer_head as i64,
        );
        let splat = |value: i32| _mm_set1_epi32(value);
        // Values are below 0x10000, so signed compares hold.
        let within = |value, low: i32, high: i32| {
            _mm_and_si128(
                _mm_cmpgt_epi32(value, splat(low - 1)),
                _mm_cmplt_epi32(value, splat(high + 1)),
            )
        };
        for group in 0..groups {
            let raw = _mm_loadu_si128(bytes.as_ptr().add(group * 12).cast::<__m128i>());
            let words = _mm_shuffle_epi8(raw, shuffle);
            let encoded =
                _mm_cmpeq_epi32(_mm_and_si128(words, splat(0x00c0_c0f0)), splat(0x0080_80e0));
            let value = _mm_or_si128(
                _mm_or_si128(
                    _mm_slli_epi32(_mm_and_si128(words, splat(0x0f)), 12),
                    _mm_srli_epi32(_mm_and_si128(words, splat(0x3f00)), 2),
                ),
                _mm_and_si128(_mm_srli_epi32(words, 16), splat(0x3f)),
            );
            let blocks = _mm_or_si128(
                _mm_or_si128(within(value, 0x4e00, 0x9fff), within(value, 0x3400, 0x4dbf)),
                within(value, 0xac00, 0xd7a3),
            );
            if _mm_movemask_epi8(_mm_and_si128(encoded, blocks)) != 0xffff {
                return group * 4;
            }
            // [tail, c0, tail, c1] and [tail, c2, tail, c3].
            let first = _mm_unpacklo_epi32(tail, value);
            let second = _mm_unpackhi_epi32(tail, value);
            let out = pairs.as_mut_ptr().add(group * 4).cast::<__m128i>();
            _mm_store_si128(out, _mm_unpacklo_epi64(head, first));
            _mm_store_si128(out.add(1), blank);
            _mm_store_si128(out.add(2), _mm_unpackhi_epi64(head, first));
            _mm_store_si128(out.add(3), blank);
            _mm_store_si128(out.add(4), _mm_unpacklo_epi64(head, second));
            _mm_store_si128(out.add(5), blank);
            _mm_store_si128(out.add(6), _mm_unpackhi_epi64(head, second));
            _mm_store_si128(out.add(7), blank);
        }
    }
    groups * 4
}

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
#[inline(always)]
fn store_quads_neon(
    pairs: &mut [[Cell; 2]],
    bytes: &[u8],
    wide: PlainCell,
    spacer: PlainCell,
) -> usize {
    use std::arch::aarch64::{
        vandq_u32, vceqq_u32, vcgeq_u32, vcleq_u32, vdupq_n_u32, vdupq_n_u64, vld1q_u8, vld1q_u64,
        vminvq_u32, vorrq_u32, vqtbl1q_u8, vreinterpretq_u32_u8, vreinterpretq_u64_u32,
        vshlq_n_u32, vshrq_n_u32, vst1q_u64, vzip1q_u32, vzip1q_u64, vzip2q_u32, vzip2q_u64,
    };
    let groups = groups(pairs.len(), bytes.len());
    let [glyph_head, glyph_tail] = wide.words();
    let [spacer_head, spacer_tail] = spacer.words();
    let blank_words = [spacer_head, spacer_tail | u64::from(u32::from(' ')) << 32];
    // SAFETY: NEON is part of the aarch64 baseline. Bounds, alignment and
    // the stored bytes are as in the x86_64 version (this target is
    // little-endian too).
    unsafe {
        let shuffle = vld1q_u8(SHUFFLE.as_ptr());
        let head = vdupq_n_u64(glyph_head);
        let tail = vdupq_n_u32(glyph_tail as u32);
        let blank = vld1q_u64(blank_words.as_ptr());
        let within = |value, low: u32, high: u32| {
            vandq_u32(
                vcgeq_u32(value, vdupq_n_u32(low)),
                vcleq_u32(value, vdupq_n_u32(high)),
            )
        };
        for group in 0..groups {
            let raw = vld1q_u8(bytes.as_ptr().add(group * 12));
            let words = vreinterpretq_u32_u8(vqtbl1q_u8(raw, shuffle));
            let encoded = vceqq_u32(
                vandq_u32(words, vdupq_n_u32(0x00c0_c0f0)),
                vdupq_n_u32(0x0080_80e0),
            );
            let value = vorrq_u32(
                vorrq_u32(
                    vshlq_n_u32::<12>(vandq_u32(words, vdupq_n_u32(0x0f))),
                    vshrq_n_u32::<2>(vandq_u32(words, vdupq_n_u32(0x3f00))),
                ),
                vandq_u32(vshrq_n_u32::<16>(words), vdupq_n_u32(0x3f)),
            );
            let blocks = vorrq_u32(
                vorrq_u32(within(value, 0x4e00, 0x9fff), within(value, 0x3400, 0x4dbf)),
                within(value, 0xac00, 0xd7a3),
            );
            if vminvq_u32(vandq_u32(encoded, blocks)) != u32::MAX {
                return group * 4;
            }
            // [tail, c0, tail, c1] and [tail, c2, tail, c3].
            let first = vreinterpretq_u64_u32(vzip1q_u32(tail, value));
            let second = vreinterpretq_u64_u32(vzip2q_u32(tail, value));
            let out = pairs.as_mut_ptr().add(group * 4).cast::<u64>();
            vst1q_u64(out, vzip1q_u64(head, first));
            vst1q_u64(out.add(2), blank);
            vst1q_u64(out.add(4), vzip2q_u64(head, first));
            vst1q_u64(out.add(6), blank);
            vst1q_u64(out.add(8), vzip1q_u64(head, second));
            vst1q_u64(out.add(10), blank);
            vst1q_u64(out.add(12), vzip2q_u64(head, second));
            vst1q_u64(out.add(14), blank);
        }
    }
    groups * 4
}

#[cfg(test)]
mod tests {
    use crate::terminal_engine::{Engine, Options, Size};

    // Text fed whole goes through the run loops (the four-scalar stores
    // among them); fed a byte at a time, each scalar is printed on its own.
    // Both leave the same screen.
    #[test]
    fn quads_store_what_single_scalars_do() {
        let mut seed = 0x2545_f491_u32;
        let mut next = |bound: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed % bound
        };
        for _ in 0..300 {
            let mut text = Vec::new();
            for _ in 0..next(90) {
                let scalar = match next(14) {
                    0 => char::from(b'a' + next(26) as u8),
                    // Two-byte scalars, the narrow scripts' and the marks
                    // among them.
                    12 | 13 => char::from_u32(0xc0 + next(0x470)).unwrap(),
                    // Just outside and at the ends of the blocks.
                    1 => ['\u{4dbf}', '\u{4dc0}', '\u{4dff}', '\u{9fff}', '\u{a000}']
                        [next(5) as usize],
                    2 => ['\u{33ff}', '\u{3400}', '\u{abff}', '\u{d7a3}', '\u{d7a4}']
                        [next(5) as usize],
                    // Wide, but not in the blocks the stores take.
                    3 => char::from_u32(0x3041 + next(0x50)).unwrap(),
                    4 => char::from_u32(0xac00 + next(0x2ba4)).unwrap(),
                    5 => {
                        // A lead byte and continuations that end early.
                        text.extend_from_slice(&[0xe4, 0xb8][..1 + next(2) as usize]);
                        continue;
                    }
                    // A jump back over text written earlier, so later text
                    // overwrites halves of its wide glyphs; and spaces.
                    6 => {
                        let jump = format!("\x1b[{};{}H", 1 + next(6), 1 + next(37));
                        text.extend_from_slice(jump.as_bytes());
                        continue;
                    }
                    7 => ' ',
                    // ASCII words.
                    8 => {
                        for _ in 0..2 + next(5) {
                            text.push(b'a' + next(26) as u8);
                        }
                        continue;
                    }
                    _ => char::from_u32(0x4e00 + next(0x5200)).unwrap(),
                };
                let mut buffer = [0; 4];
                text.extend_from_slice(scalar.encode_utf8(&mut buffer).as_bytes());
            }
            let engine = || Engine::new(Size { cols: 37, rows: 6 }, Options::default());
            let mut whole = engine();
            whole.feed(&text);
            let mut bytewise = engine();
            for byte in &text {
                bytewise.feed(&[*byte]);
            }
            assert_eq!(whole.cursor(), bytewise.cursor(), "{text:02x?}");
            for row in 0..6 {
                assert_eq!(
                    whole.viewport_row(row).unwrap().snapshot(),
                    bytewise.viewport_row(row).unwrap().snapshot(),
                    "row {row} of {text:02x?}"
                );
            }
        }
    }
}
