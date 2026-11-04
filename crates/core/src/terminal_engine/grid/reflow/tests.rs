use super::super::history::{DEFAULT_HISTORY_BYTES, history_heap};
use super::super::tests::{assert_history_compacted, grid, print, text};
use super::super::*;

#[test]
fn wide_glyphs_recover_their_spacers_after_single_column_resize() {
    let mut grid = grid(4, 3, 8);
    print(&mut grid, "界e\u{301}");
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 3));
    grid.resize(Size { cols: 1, rows: 3 });
    grid.resize(Size { cols: 4, rows: 3 });
    let row = grid.row(0).unwrap().line();
    assert_eq!(row[0].character, '界');
    assert_eq!(row[0].flags, Cell::WIDE);
    assert_eq!(row[1].flags, Cell::WIDE_SPACER);
    assert_eq!(row[2].character, 'e');
    assert_eq!(row.combining(2), "\u{301}");
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 3));
}

#[test]
fn squeezed_wide_glyph_preserves_pending_wrap_and_spacer_cursor_mapping() {
    let mut grid = grid(2, 2, 4);
    grid.put_char('界');
    grid.resize(Size { cols: 1, rows: 2 });
    grid.resize(Size { cols: 2, rows: 2 });
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 1));
    assert!(grid.pending_wrap);
    grid.goto(0, 1);
    grid.resize(Size { cols: 1, rows: 2 });
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 0));
    assert!(!grid.pending_wrap);
}

#[test]
fn width_reflow_preserves_hard_breaks_soft_wraps_cursor_and_style() {
    let mut grid = grid(10, 6, 10);
    grid.pen.style.attributes = Style::BOLD;
    print(&mut grid, "abcdefghijklm\r\n$ ");
    grid.resize(Size { cols: 6, rows: 6 });
    assert_eq!(text(&grid, 0), "abcdef");
    assert_eq!(text(&grid, 1), "ghijkl");
    assert_eq!(text(&grid, 2), "m     ");
    assert_eq!(text(&grid, 3), "$     ");
    assert_eq!((grid.cursor.row, grid.cursor.col), (3, 2));
    assert_eq!(grid.row(1).unwrap().line()[0].style.attributes, Style::BOLD);
    assert!(grid.row(0).unwrap().wrapped);
    assert!(!grid.row(2).unwrap().wrapped);
    grid.resize(Size { cols: 10, rows: 6 });
    assert_eq!(text(&grid, 0), "abcdefghij");
    assert_eq!(text(&grid, 1), "klm       ");
    assert_eq!(text(&grid, 2), "$         ");
    assert_eq!((grid.cursor.row, grid.cursor.col), (2, 2));
}

#[test]
fn cleared_scrollback_remains_hidden_across_width_reflow() {
    let mut grid = grid(20, 6, 40);
    for index in 0..18 {
        print(&mut grid, &format!("HISTORY-{index:02}-abcdefghijk\r\n"));
    }
    grid.goto(0, 0);
    grid.erase_display(2, false);
    print(&mut grid, "$ ");
    for cols in [10, 20, 10, 20] {
        grid.resize(Size { cols, rows: 6 });
        assert!((0..6).all(|row| !text(&grid, row).contains("HISTORY")));
        assert!(grid.history_size() > 0);
    }
    grid.scroll_display(i32::MAX);
    assert!((0..6).any(|row| text(&grid, row).contains("HISTORY")));
}

#[test]
fn scrolled_viewport_tracks_its_logical_anchor_through_reflow() {
    let mut grid = grid(12, 3, 40);
    for index in 0..12 {
        print(&mut grid, &format!("row-{index:02}-abcdefgh\r\n"));
    }
    grid.scroll_display(i32::MAX);
    let prefix = text(&grid, 0)[..6].to_owned();
    grid.resize(Size { cols: 6, rows: 3 });
    assert_eq!(text(&grid, 0), prefix);
    grid.resize(Size { cols: 12, rows: 3 });
    assert!(text(&grid, 0).starts_with(&prefix));
    let before = text(&grid, 0);
    grid.resize(Size { cols: 12, rows: 2 });
    assert_eq!(text(&grid, 0), before);
}

#[test]
fn widening_packs_rewrapped_history_instead_of_evicting_it() {
    let mut grid = grid(1, 3, 2048);
    for index in 0..1024 {
        grid.put_char(char::from(b'a' + (index % 26) as u8));
        grid.carriage_return();
        grid.linefeed();
    }
    assert_eq!(grid.history_size(), 1022);
    assert!(grid.scroll_display(400));
    grid.resize(Size {
        cols: 4096,
        rows: 3,
    });
    // At 4096 columns the old cell cap kept only 256 rows. The widen evicts
    // nothing the old width kept, and what history takes fits the budget.
    assert_eq!(grid.history_size(), 1022);
    assert_eq!(grid.display_offset(), 400);
    let heap: usize = grid.history.iter().map(history_heap).sum();
    assert!(heap <= DEFAULT_HISTORY_BYTES);
    assert_eq!(
        grid.row(-1).unwrap().line()[0].character,
        char::from(b'a' + (1021 % 26) as u8)
    );
    assert_eq!((grid.cursor.row, grid.cursor.col), (2, 0));
    grid.resize(Size { cols: 1, rows: 3 });
    assert_eq!(grid.history_size(), 1022);
}

#[test]
fn a_transient_widen_keeps_the_requested_scrollback() {
    let mut grid = grid(120, 40, 10_000);
    for index in 0..12_000 {
        print(&mut grid, &format!("{index:05} a short history line\r\n"));
    }
    grid.compact_history(usize::MAX);
    assert_eq!(grid.history_size(), 10_000);
    // The cell cap allowed 8738 rows at 120 columns and 2621 at 400.
    grid.resize(Size {
        cols: 400,
        rows: 40,
    });
    assert_eq!(grid.history_size(), 10_000);
    grid.resize(Size {
        cols: 120,
        rows: 40,
    });
    assert_eq!(grid.history_size(), 10_000);
    assert!(grid.history_bytes <= DEFAULT_HISTORY_BYTES);
    assert!(text(&grid, 0).starts_with("11961"), "{}", text(&grid, 0));
}

#[test]
fn narrowing_one_long_logical_line_recycles_rows_and_preserves_its_tail() {
    let mut grid = grid(80, 3, 1000);
    let pattern = "abcdefghijklmnopqrstuvwxyz";
    let input = pattern.repeat(3000);
    grid.write_ascii(input.as_bytes());
    grid.resize(Size { cols: 1, rows: 3 });
    assert_eq!(grid.history_size(), 1000);
    assert!(grid.last_reflow_row_allocations <= 1000 + 3 + 1);
    assert_eq!(grid.primary.rows.len(), 3);
    assert_eq!(text(&grid, 0), "x");
    assert_eq!(text(&grid, 1), "y");
    assert_eq!(text(&grid, 2), "z");
    assert_eq!((grid.cursor.row, grid.cursor.col), (2, 0));
    assert!(grid.pending_wrap);
    assert_eq!(grid.row(-1).unwrap().line()[0].character, 'w');
}

#[test]
fn reflow_drops_below_cursor_rows_without_losing_the_cursor_window() {
    let mut grid = grid(80, 4, 5);
    grid.write_ascii(&vec![b'x'; 320]);
    grid.goto(0, 1);
    grid.resize(Size { cols: 1, rows: 4 });
    assert_eq!(grid.history_size(), 1);
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 0));
    assert!(grid.last_reflow_row_allocations <= 5 + 4 + 1);
    assert!((0..4).all(|row| text(&grid, row) == "x"));
}

#[test]
fn width_resize_defers_history_compaction_to_idle_steps() {
    let mut grid = grid(120, 4, 100);
    for line in 0..60 {
        print(&mut grid, &format!("line {line} {}\r\n", "x".repeat(140)));
    }
    assert_history_compacted(&mut grid);
    grid.history_activity = false;
    grid.resize(Size { cols: 100, rows: 4 });
    // Only a leading tail whose head was evicted is kept in place.
    assert!(grid.history.iter().skip(1).all(|row| row.packed.is_none()));
    assert!(grid.history_activity);
    assert!(grid.needs_compaction());
    assert_history_compacted(&mut grid);
}

#[test]
fn width_resize_keeps_fitting_history_rows_packed() {
    let mut grid = grid(120, 4, 100);
    print(&mut grid, &"w".repeat(130));
    print(&mut grid, "\r\n");
    for line in 0..60 {
        print(&mut grid, &format!("line {line}\r\n"));
    }
    assert_history_compacted(&mut grid);
    let before = (-(grid.history_size() as i32)..4)
        .map(|line| grid.row_line(line).unwrap().slice(0..20).snapshot())
        .collect::<Vec<_>>();
    for cols in [100, 120] {
        grid.resize(Size { cols, rows: 4 });
        let packed = grid.history.iter().filter(|row| row.packed.is_some());
        assert_eq!(packed.count(), grid.history_size().saturating_sub(2));
        assert!(grid.primary.rows.iter().all(|row| row.packed.is_none()));
        assert!(
            grid.history
                .iter()
                .chain(&grid.primary.rows)
                .all(|row| row.cells().len() == cols)
        );
    }
    let after = (-(grid.history_size() as i32)..4)
        .map(|line| grid.row_line(line).unwrap().slice(0..20).snapshot())
        .collect::<Vec<_>>();
    assert_eq!(after, before);
}
