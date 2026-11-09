//! DECLRMM (`?69`) and DECSLRM left/right margins.

use super::super::{Engine, Options, Size};

fn engine(cols: usize, rows: usize) -> Engine {
    Engine::new(
        Size { cols, rows },
        Options {
            scrollback_history: 10,
            ..Default::default()
        },
    )
}

fn replies(engine: &mut Engine) -> Vec<u8> {
    let mut bytes = Vec::new();
    engine.drain_replies(&mut bytes);
    bytes
}

fn text(engine: &Engine, row: usize) -> String {
    engine
        .viewport_row(row)
        .unwrap()
        .iter()
        .map(|cell| cell.character)
        .collect()
}

fn position(engine: &Engine) -> (usize, usize) {
    (engine.cursor().row, engine.cursor().col)
}

/// Three rows of `0123456789`, then margins at columns 3–6 (1-based).
fn digits_with_margins() -> Engine {
    let mut engine = engine(10, 3);
    engine.feed(b"0123456789\r\n0123456789\r\n0123456789\x1b[?69h\x1b[3;6s");
    engine
}

#[test]
fn csi_s_sets_left_right_margins_only_while_declrmm_is_set() {
    let mut engine = engine(10, 3);
    // Without DECLRMM, `CSI s` saves the cursor.
    engine.feed(b"\x1b[2;4H\x1b[3;6s\x1b[H\x1b[u");
    assert_eq!(position(&engine), (1, 3));
    engine.feed(b"\x1b[?69$p\x1bP$qs\x1b\\");
    assert_eq!(replies(&mut engine), b"\x1b[?69;2$y\x1bP1$r1;10s\x1b\\");

    // Like DECSTBM, DECSLRM homes the cursor.
    engine.feed(b"\x1b[?69h\x1b[2;4H\x1b[3;6s");
    assert_eq!(position(&engine), (0, 0));
    engine.feed(b"\x1b[?69$p\x1bP$qs\x1b\\");
    assert_eq!(replies(&mut engine), b"\x1b[?69;1$y\x1bP1$r3;6s\x1b\\");

    // A margin narrower than two columns is ignored.
    engine.feed(b"\x1b[5;5s\x1bP$qs\x1b\\");
    assert_eq!(replies(&mut engine), b"\x1bP1$r3;6s\x1b\\");

    // Resetting DECLRMM resets the margins, and `CSI s` saves again.
    engine.feed(b"\x1b[?69l\x1bP$qs\x1b\\\x1b[3;5H\x1b[s\x1b[H\x1b[u");
    assert_eq!(replies(&mut engine), b"\x1bP1$r1;10s\x1b\\");
    assert_eq!(position(&engine), (2, 4));
}

#[test]
fn text_wraps_and_scrolls_inside_the_left_right_margins() {
    let mut engine = digits_with_margins();
    engine.feed(b"\x1b[1;3Habcdefg");
    assert_eq!(text(&engine, 0), "01abcd6789");
    assert_eq!(text(&engine, 1), "01efg56789");
    assert_eq!(position(&engine), (1, 5));
    // Wrapping inside margins doesn't join rows into one logical line.
    assert!(!engine.viewport_row_wrapped(0));

    // At the bottom margin only the margin columns scroll, and nothing
    // enters history.
    engine.feed(b"hijklmno");
    assert_eq!(text(&engine, 0), "01efgh6789");
    assert_eq!(text(&engine, 1), "01ijkl6789");
    assert_eq!(text(&engine, 2), "01mno 6789");
    assert_eq!(engine.history_size(), 0);

    // Outside the margins, a line feed at the bottom margin doesn't scroll.
    engine.feed(b"\x1b[3;9H\n");
    assert_eq!(text(&engine, 0), "01efgh6789");
    assert_eq!(position(&engine), (2, 8));
}

#[test]
fn wide_glyphs_wrap_at_the_right_margin_and_never_straddle_it() {
    let mut engine = engine(10, 3);
    engine.feed("\x1b[?69h\x1b[3;6s\x1b[1;6H界".as_bytes());
    assert_eq!(engine.viewport_row(0).unwrap()[5].character, ' ');
    assert_eq!(engine.viewport_row(1).unwrap()[2].character, '界');
    assert!(!engine.viewport_row_wrapped(0));

    // A pair straddling the left margin can't move by half, so scrolling the
    // margin columns blanks it.
    let mut engine = engine_with_wide_pair_on_the_left_margin();
    engine.feed(b"\x1b[2;3H\x1b[M");
    let row = engine.viewport_row(1).unwrap();
    assert_eq!(row[1].character, ' ');
    assert!(row.iter().all(|cell| cell.flags == 0));
}

fn engine_with_wide_pair_on_the_left_margin() -> Engine {
    let mut engine = engine(10, 3);
    engine.feed("\x1b[2;2H界\x1b[?69h\x1b[3;6s".as_bytes());
    engine
}

#[test]
fn character_and_line_editing_stays_inside_the_margins() {
    let mut engine = digits_with_margins();
    engine.feed(b"\x1b[1;4H\x1b[2@");
    assert_eq!(text(&engine, 0), "012  36789");
    engine.feed(b"\x1b[2;4H\x1b[P");
    assert_eq!(text(&engine, 1), "01245 6789");
    // Outside the margins ICH and DCH do nothing.
    engine.feed(b"\x1b[3;1H\x1b[@\x1b[3;8H\x1b[P");
    assert_eq!(text(&engine, 2), "0123456789");

    engine.feed(b"\x1b[2;4H\x1b[L");
    assert_eq!(text(&engine, 1), "01    6789");
    assert_eq!(text(&engine, 2), "01245 6789");
    // IL and DL leave the cursor on the left margin.
    assert_eq!(position(&engine), (1, 2));
    engine.feed(b"\x1b[M");
    assert_eq!(text(&engine, 1), "01245 6789");
    assert_eq!(text(&engine, 2), "01    6789");

    // SU and SD scroll the margin columns of the scroll region.
    engine.feed(b"\x1b[S");
    assert_eq!(text(&engine, 0), "01245 6789");
    engine.feed(b"\x1b[T");
    assert_eq!(text(&engine, 0), "01    6789");
    assert_eq!(text(&engine, 1), "01245 6789");
    assert_eq!(engine.history_size(), 0);
}

#[test]
fn cursor_motion_respects_the_margins_from_inside_them() {
    let mut engine = engine(10, 3);
    engine.feed(b"\x1b[?69h\x1b[3;6s");
    let mut at = |input: &[u8]| {
        engine.feed(input);
        engine.cursor().col
    };
    // CR goes to the left margin, or column 0 from left of it.
    assert_eq!(at(b"\x1b[1;4H\r"), 2);
    assert_eq!(at(b"\x1b[1;2H\r"), 0);
    // CUF/CUB stop at the margins, except toward the screen edge outside.
    assert_eq!(at(b"\x1b[1;4H\x1b[9C"), 5);
    assert_eq!(at(b"\x1b[9D"), 2);
    assert_eq!(at(b"\x1b[1;8H\x1b[9C"), 9);
    assert_eq!(at(b"\x1b[1;2H\x1b[9D"), 0);
    // BS stops at the left margin; HT stops at the right one.
    assert_eq!(at(b"\x1b[1;3H\x08"), 2);
    assert_eq!(at(b"\x1b[1;3H\t"), 5);
    assert_eq!(at(b"\x1b[1;4H\x1b[Z"), 2);
    // CHA is absolute outside origin mode.
    assert_eq!(at(b"\x1b[1;4H\x1b[1G"), 0);

    // In origin mode, CUP, CHA and CPR are relative to the margins.
    assert_eq!(at(b"\x1b[?6h"), 2);
    assert_eq!(at(b"\x1b[1;9H"), 5);
    assert_eq!(at(b"\x1b[2G"), 3);
    engine.feed(b"\x1b[2;3H\x1b[6n");
    assert_eq!(replies(&mut engine), b"\x1b[2;3R");
}

#[test]
fn long_repeats_inside_margins_match_printing_each_character() {
    for count in 200..204 {
        let mut repeated = engine(10, 3);
        repeated.feed(format!("\x1b[?69h\x1b[3;6s\x1b[1;9Hx\x1b[{count}b").as_bytes());
        let mut printed = engine(10, 3);
        printed.feed(b"\x1b[?69h\x1b[3;6s\x1b[1;9H");
        printed.feed("x".repeat(count + 1).as_bytes());
        for row in 0..3 {
            assert_eq!(text(&repeated, row), text(&printed, row), "{count}");
        }
        assert_eq!(position(&repeated), position(&printed), "{count}");
    }
}

#[test]
fn resets_turn_declrmm_off() {
    for reset in [&b"\x1b[!p"[..], b"\x1bc"] {
        let mut engine = engine(10, 3);
        engine.feed(b"\x1b[?69h\x1b[3;6s");
        engine.feed(reset);
        engine.feed(b"\x1b[?69$p\x1bP$qs\x1b\\");
        assert_eq!(
            replies(&mut engine),
            b"\x1b[?69;2$y\x1bP1$r1;10s\x1b\\",
            "{reset:?}"
        );
    }
}
