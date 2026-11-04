use super::super::types::{Color, CursorShape};
use super::*;

pub(super) fn grid(cols: usize, rows: usize, history: usize) -> Grid {
    Grid::new(Size { cols, rows }, history)
}

pub(super) fn text(grid: &Grid, row: usize) -> String {
    grid.visible_row(row)
        .unwrap()
        .cells()
        .iter()
        .map(|cell| cell.character)
        .collect()
}

pub(super) fn print(grid: &mut Grid, text: &str) {
    for character in text.chars() {
        match character {
            '\r' => grid.carriage_return(),
            '\n' => grid.linefeed(),
            character => grid.put_char(character),
        }
    }
}

mod damage;

#[test]
fn wrapping_is_deferred_until_the_next_printable_character() {
    let mut grid = grid(4, 2, 4);
    grid.write_ascii(b"abcd");
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 3));
    assert!(!grid.row(0).unwrap().wrapped);
    grid.put_char('e');
    assert_eq!((grid.cursor.row, grid.cursor.col), (1, 1));
    assert_eq!(text(&grid, 0), "abcd");
    assert_eq!(text(&grid, 1), "e   ");
    assert!(grid.row(0).unwrap().wrapped);
}

#[test]
fn scrolling_recycles_a_bounded_set_of_cell_buffers() {
    let mut grid = grid(12, 3, 4);
    for _ in 0..16 {
        print(&mut grid, "row\r\n");
    }
    let mut pointers = grid
        .history
        .iter()
        .chain(&grid.primary.rows)
        .map(|row| row.cells().as_ptr())
        .collect::<Vec<_>>();
    pointers.sort();
    assert_eq!(pointers.len(), 7);
    for _ in 0..1000 {
        print(&mut grid, "next\r\n");
    }
    let mut after = grid
        .history
        .iter()
        .chain(&grid.primary.rows)
        .map(|row| row.cells().as_ptr())
        .collect::<Vec<_>>();
    after.sort();
    assert_eq!(
        after, pointers,
        "steady scrolling must reuse evicted row buffers"
    );
    assert_eq!(grid.history_size(), 4);
}

#[test]
fn short_composed_lines_recycle_only_the_occupied_prefix() {
    let mut grid = grid(120, 2, 2);
    for _ in 0..32 {
        grid.write_ascii(b"e");
        grid.put_char('\u{301}');
        grid.write_ascii(b" a");
        grid.put_char('\u{308}');
        grid.carriage_return();
        grid.linefeed();
    }
    for row in grid.history.iter().chain(&grid.primary.rows) {
        assert!(row.occupied <= 3);
        assert!(row.line()[3..].iter().all(|cell| cell == &Cell::default()));
        if row.occupied != 0 {
            assert_eq!(row.line().combining(0), "\u{301}");
            assert_eq!(row.line().combining(2), "\u{308}");
        }
    }
    assert_eq!(grid.primary.rows[1].occupied, 0);
    assert_eq!(grid.history_size(), 2);
}

#[test]
fn recycled_rows_clear_metadata_and_repaint_the_entire_background() {
    use std::sync::Arc;

    use crate::terminal_engine::{CellExtra, Hyperlink};

    let mut grid = grid(8, 1, 0);
    grid.pen.extra = Some(Arc::new(CellExtra {
        combining: String::new(),
        hyperlink: Some(Arc::new(Hyperlink {
            id: "retained snapshot".into(),
            uri: "https://example.test".into(),
        })),
        ..CellExtra::default()
    }));
    grid.write_ascii(b"e");
    grid.put_char('\u{301}');
    let snapshot = grid.primary.rows[0].cells[0];
    let snapshot_extra = grid.primary.rows[0].extra(0).cloned().unwrap();
    grid.pen.extra = None;

    // Changing the erase background invalidates even the untouched suffix.
    for background in [Color::indexed(4), Color::indexed(4), Color::DEFAULT] {
        grid.pen.style.background = background;
        grid.carriage_return();
        grid.linefeed();
        let row = &grid.primary.rows[0];
        let blank = Cell {
            style: Style {
                background,
                ..Style::default()
            },
            ..Cell::default()
        };
        assert!(row.cells().iter().all(|cell| cell == &blank));
        assert!(row.extras.is_empty());
        assert_eq!(row.occupied, 0);
        assert!(!row.wrapped);
    }
    assert_eq!(snapshot.character, 'e');
    assert_eq!(snapshot_extra.combining, "\u{301}");
    assert_eq!(
        snapshot_extra.hyperlink.as_ref().unwrap().id,
        "retained snapshot"
    );
}

#[test]
fn combining_on_blank_cells_and_selective_erases_survive_until_recycling() {
    let mut grid = grid(12, 1, 0);
    grid.pen.style.set(Style::PROTECTED, true);
    grid.put_char('p');
    grid.pen.style.set(Style::PROTECTED, false);
    grid.goto(0, 7);
    grid.put_char('\u{301}');
    assert_eq!(grid.primary.rows[0].line().combining(6), "\u{301}");

    grid.pen.style.background = Color::indexed(5);
    grid.goto(0, 9);
    grid.erase_line(1, true);
    assert_eq!(grid.primary.rows[0].cells[0].character, 'p');
    assert_eq!(
        grid.primary.rows[0].cells[9].style.background,
        Color::indexed(5)
    );

    grid.pen.style.background = Color::DEFAULT;
    grid.linefeed();
    assert!(
        grid.primary.rows[0]
            .cells
            .iter()
            .all(|cell| cell == &Cell::default())
    );
}

#[test]
fn viewport_remains_anchored_while_new_output_enters_history() {
    let mut grid = grid(5, 2, 10);
    print(&mut grid, "one\r\ntwo\r\nthree\r\n");
    assert!(grid.scroll_display(2));
    let before = [text(&grid, 0), text(&grid, 1)];
    print(&mut grid, "four\r\n");
    assert_eq!([text(&grid, 0), text(&grid, 1)], before);
    assert_eq!(grid.display_offset(), 3);
    assert_eq!(grid.row(-1).unwrap().line()[0].character, 't');
}

#[test]
fn partial_scroll_regions_never_enter_history() {
    let mut grid = grid(4, 4, 10);
    print(&mut grid, "AAAA\r\nBBBB\r\nCCCC\r\nDDDD");
    grid.set_scroll_region(1, 3);
    grid.goto(2, 0);
    grid.linefeed();
    assert_eq!(text(&grid, 0), "AAAA");
    assert_eq!(text(&grid, 1), "CCCC");
    assert_eq!(text(&grid, 2), "    ");
    assert_eq!(text(&grid, 3), "DDDD");
    assert_eq!(grid.history_size(), 0);
    grid.goto(1, 0);
    grid.reverse_index();
    assert_eq!(text(&grid, 1), "    ");
    assert_eq!(text(&grid, 2), "CCCC");
}

#[test]
fn wide_character_overwrite_cleans_both_halves() {
    let mut grid = grid(6, 2, 0);
    print(&mut grid, "a界b");
    assert_eq!(grid.row(0).unwrap().line()[1].flags, Cell::WIDE);
    assert_eq!(grid.row(0).unwrap().line()[2].flags, Cell::WIDE_SPACER);
    grid.goto(0, 2);
    grid.put_char('x');
    assert_eq!(text(&grid, 0), "a xb  ");
    assert!(
        grid.row(0)
            .unwrap()
            .cells()
            .iter()
            .all(|cell| cell.flags == 0)
    );
    grid.goto(0, 3);
    grid.put_char('界');
    grid.goto(0, 3);
    grid.erase_chars(1);
    assert_eq!(text(&grid, 0), "a x   ");
}

#[test]
fn scalar_overwrites_repair_wide_boundaries_and_damage() {
    for (column, character, expected, start, end) in [
        (1, 'é', "aé 界 z  ", 1, 3),
        (2, 'é', "a é界 z  ", 1, 4),
        (0, '語', "語  界 z  ", 0, 3),
        (1, '語', "a語 界 z  ", 1, 4),
        (2, '語', "a 語  z  ", 1, 5),
    ] {
        let mut grid = grid(8, 2, 0);
        print(&mut grid, "a界界z");
        grid.goto(0, column);
        grid.take_damage();
        grid.put_char(character);
        assert_eq!(text(&grid, 0), expected, "{character} at {column}");
        assert_eq!(
            grid.take_damage(),
            Damage::Partial(vec![DirtySpan { row: 0, start, end }]),
            "{character} at {column}"
        );
        let cells = grid.row(0).unwrap().cells();
        for (column, cell) in cells.iter().enumerate() {
            if cell.flags & Cell::WIDE != 0 {
                assert_eq!(cells[column + 1].flags, Cell::WIDE_SPACER);
            } else if cell.flags & Cell::WIDE_SPACER != 0 {
                assert!(column > 0);
                assert_eq!(cells[column - 1].flags, Cell::WIDE);
            }
        }
    }
}

#[test]
fn wide_overwrite_preserves_styles_and_owns_only_its_base_metadata() {
    use super::super::types::{CellExtra, Hyperlink};
    use std::sync::Arc;

    let mut grid = grid(8, 2, 0);
    grid.pen.style.foreground = Color::indexed(1);
    grid.pen.style.background = Color::indexed(2);
    grid.pen.style.attributes = Style::BOLD;
    grid.pen.extra = Some(Arc::new(CellExtra {
        hyperlink: Some(Arc::new(Hyperlink {
            id: "old".into(),
            uri: "https://old.test".into(),
        })),
        ..CellExtra::default()
    }));
    print(&mut grid, "a界\u{301}界\u{308}z");
    let before_cells = grid.row(0).unwrap().cells().to_vec();
    let before_extras = grid.row(0).unwrap().extras.clone();
    let before = Line::new(&before_cells, &before_extras);
    let extra = Arc::new(CellExtra {
        hyperlink: Some(Arc::new(Hyperlink {
            id: "new".into(),
            uri: "https://new.test".into(),
        })),
        ..CellExtra::default()
    });
    grid.pen.style.foreground = Color::indexed(3);
    grid.pen.style.background = Color::indexed(4);
    grid.pen.style.attributes = Style::ITALIC;
    grid.pen.extra = Some(Arc::clone(&extra));
    grid.goto(0, 2);
    grid.put_char('語');

    let row = grid.row(0).unwrap();
    let cells = row.cells();
    let blank = Cell {
        style: Style {
            background: Color::indexed(4),
            ..Style::default()
        },
        ..Cell::default()
    };
    assert_eq!(cells[1], blank);
    assert_eq!(cells[4], blank);
    assert!(row.extra(1).is_none() && row.extra(4).is_none());
    assert_eq!(cells[2].style, grid.pen.style);
    assert_eq!(cells[3].style, grid.pen.style);
    assert!(Arc::ptr_eq(row.extra(2).unwrap(), &extra));
    assert!(row.extra(3).is_none());
    assert_eq!(Arc::strong_count(&extra), 3);
    for column in [0, 5, 6, 7] {
        assert_eq!(cells[column], before[column]);
        assert_eq!(row.line().extra(column), before.extra(column));
    }
    assert_eq!(before.combining(1), "\u{301}");
    assert_eq!(before.combining(3), "\u{308}");
    assert_eq!(before.hyperlink(1).unwrap().id, "old");
}

#[test]
fn wide_wrap_and_combining_marks_follow_the_base_cell() {
    let mut grid = grid(4, 2, 4);
    print(&mut grid, "abc界\u{301}");
    assert_eq!(
        grid.row(0).unwrap().line()[3].flags,
        Cell::LEADING_WIDE_SPACER
    );
    assert!(grid.row(0).unwrap().wrapped);
    assert_eq!(grid.row(1).unwrap().line()[0].character, '界');
    assert_eq!(grid.row(1).unwrap().line().combining(0), "\u{301}");
    assert_eq!(grid.row(1).unwrap().line().combining(1), "");
    grid.resize(Size { cols: 6, rows: 2 });
    assert_eq!(text(&grid, 0), "abc界  ");
    assert_eq!(grid.row(0).unwrap().line().combining(3), "\u{301}");
}

#[test]
fn combining_sequences_are_bounded_and_owned_by_each_cell() {
    let mut grid = grid(4, 2, 0);
    grid.put_char('e');
    for _ in 0..1000 {
        grid.put_char('\u{301}');
    }
    assert!(grid.row(0).unwrap().line().combining(0).len() <= 256);
    grid.put_char('a');
    assert_eq!(grid.row(0).unwrap().line().combining(1), "");
}

#[test]
fn single_column_wide_input_always_makes_progress() {
    let mut grid = grid(1, 2, 2);
    print(&mut grid, "界界界界");
    assert_eq!(grid.history_size(), 2);
    assert_eq!(text(&grid, 0), "界");
    assert_eq!(text(&grid, 1), "界");
    grid.resize(Size { cols: 2, rows: 2 });
    assert_eq!(grid.size().cols, 2);
}

#[test]
fn editing_repairs_cut_wide_glyphs_and_preserves_other_cells() {
    let mut grid = grid(8, 2, 0);
    print(&mut grid, "ab界cd");
    grid.goto(0, 3);
    grid.insert_chars(1);
    assert_eq!(text(&grid, 0), "ab   cd ");
    grid.goto(0, 2);
    grid.delete_chars(3);
    assert_eq!(text(&grid, 0), "abcd    ");
    assert!(
        grid.row(0)
            .unwrap()
            .cells()
            .iter()
            .all(|cell| cell.flags == 0)
    );
}

#[test]
fn editing_a_wrapped_row_drops_its_leading_wide_spacer_and_edge_glyph() {
    let mut grid = grid(4, 2, 0);
    print(&mut grid, "abc界");
    assert_ne!(
        grid.screen().rows[0].cells[3].flags & Cell::LEADING_WIDE_SPACER,
        0
    );
    // The leading spacer lands mid-row and the row stops wrapping.
    grid.goto(0, 0);
    grid.delete_chars(1);
    assert_eq!(text(&grid, 0), "bc  ");
    assert!(
        grid.screen().rows[0]
            .cells
            .iter()
            .all(|cell| cell.flags == 0)
    );
    // A glyph pushed into the last column loses its spacer.
    grid.goto(1, 0);
    grid.insert_chars(3);
    assert_eq!(text(&grid, 1), "    ");
    assert!(
        grid.screen().rows[1]
            .cells
            .iter()
            .all(|cell| cell.flags == 0)
    );
}

#[test]
fn selective_erasure_respects_protection_and_background() {
    let mut grid = grid(6, 2, 0);
    grid.pen.style.attributes = Style::PROTECTED;
    print(&mut grid, "界");
    grid.pen.style.attributes = 0;
    print(&mut grid, "abc");
    grid.pen.style.background = Color::indexed(4);
    grid.erase_line(2, true);
    assert_eq!(text(&grid, 0), "界     ");
    assert_eq!(grid.row(0).unwrap().line()[0].flags, Cell::WIDE);
    assert_eq!(
        grid.row(0).unwrap().line()[2].style.background,
        Color::indexed(4)
    );
    grid.erase_line(2, false);
    assert!(
        grid.row(0)
            .unwrap()
            .cells()
            .iter()
            .all(|cell| cell.flags == 0)
    );
}

#[test]
fn damage_spans_cover_cursor_motion_and_changed_glyph_halves() {
    let mut grid = grid(8, 3, 0);
    assert_eq!(grid.take_damage(), Damage::Full);
    grid.write_ascii(b"abc");
    assert_eq!(
        grid.take_damage(),
        Damage::Partial(vec![DirtySpan {
            row: 0,
            start: 0,
            end: 4
        }])
    );
    grid.goto(1, 5);
    assert_eq!(
        grid.take_damage(),
        Damage::Partial(vec![
            DirtySpan {
                row: 0,
                start: 3,
                end: 4
            },
            DirtySpan {
                row: 1,
                start: 5,
                end: 6
            }
        ])
    );
    assert_eq!(grid.take_damage(), Damage::Partial(vec![]));
}

#[test]
fn scalar_damage_tracks_visible_cursors_and_wraps_between_rows() {
    for (character, width) in [('é', 1), ('界', 2)] {
        for visible in [false, true] {
            let mut grid = grid(6, 2, 0);
            grid.cursor.visible = visible;
            grid.goto(0, 2);
            grid.take_damage();
            grid.put_char(character);
            assert_eq!(
                grid.take_damage(),
                Damage::Partial(vec![DirtySpan {
                    row: 0,
                    start: 2,
                    end: 2 + width + usize::from(visible),
                }])
            );
        }
    }

    let mut grid = grid(4, 2, 0);
    print(&mut grid, "abcd");
    grid.take_damage();
    grid.put_char('é');
    assert_eq!(
        grid.take_damage(),
        Damage::Partial(vec![
            DirtySpan {
                row: 0,
                start: 0,
                end: 4,
            },
            DirtySpan {
                row: 1,
                start: 0,
                end: 2,
            },
        ])
    );
    assert!(grid.row(0).unwrap().wrapped);
}

#[test]
fn erasing_one_column_of_a_wide_glyph_damages_both_columns() {
    let mut grid = grid(8, 2, 0);
    grid.put_char('界');
    grid.goto(0, 0);
    grid.take_damage();
    grid.erase_chars(1);
    assert_eq!(
        grid.take_damage(),
        Damage::Partial(vec![DirtySpan {
            row: 0,
            start: 0,
            end: 2
        }])
    );
    assert_eq!(text(&grid, 0), "        ");
}

#[test]
fn alternate_screen_restores_primary_and_never_retains_history() {
    let mut grid = grid(6, 3, 5);
    print(&mut grid, "hello\r\nworld");
    let primary = (0..3).map(|row| text(&grid, row)).collect::<Vec<_>>();
    let cursor = grid.cursor;
    grid.set_alternate(true, true, true);
    for _ in 0..10 {
        print(&mut grid, "alt\r\n");
    }
    assert_eq!(grid.history_size(), 0);
    assert!(!grid.scroll_display(1));
    grid.set_alternate(false, false, true);
    assert_eq!(
        (0..3).map(|row| text(&grid, row)).collect::<Vec<_>>(),
        primary
    );
    assert_eq!(grid.cursor, cursor);
}

#[test]
fn leaving_the_1049_alternate_screen_releases_it() {
    let mut grid = grid(6, 3, 5);
    grid.set_alternate(true, false, false);
    print(&mut grid, "kept");
    grid.set_alternate(false, false, false);
    assert!(grid.alternate.is_some());
    grid.set_alternate(true, false, false);
    assert_eq!(text(&grid, 0), "kept  ");
    grid.set_alternate(false, true, true);
    assert!(grid.alternate.is_none());
    grid.set_alternate(true, true, true);
    assert_eq!(text(&grid, 0), "      ");
}

#[test]
fn clearing_and_trimming_history_release_its_capacity() {
    let mut grid = grid(4, 2, 5000);
    for _ in 0..5000 {
        print(&mut grid, "x\r\n");
    }
    grid.set_history_limits(10, DEFAULT_HISTORY_BYTES);
    assert!(grid.history.capacity() <= 2 * 64);
    grid.set_history_limits(5000, DEFAULT_HISTORY_BYTES);
    for _ in 0..5000 {
        print(&mut grid, "x\r\n");
    }
    assert!(grid.clear_scrollback());
    assert_eq!(grid.history.capacity(), 0);
}

#[test]
fn saved_cursor_restores_style_wrap_and_origin() {
    let mut grid = grid(4, 3, 0);
    grid.pen.style.foreground = Color::indexed(1);
    grid.cursor.shape = CursorShape::Beam;
    grid.write_ascii(b"abcd");
    grid.save_cursor();
    grid.goto(2, 1);
    grid.pen.style.foreground = Color::DEFAULT;
    // Shape, visibility and blinking are live state, not part of the save.
    grid.cursor.shape = CursorShape::Underline;
    grid.cursor.visible = false;
    grid.restore_cursor();
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 3));
    assert_eq!(grid.cursor.shape, CursorShape::Underline);
    assert!(!grid.cursor.visible);
    assert_eq!(grid.pen.style.foreground, Color::indexed(1));
    grid.put_char('e');
    assert_eq!((grid.cursor.row, grid.cursor.col), (1, 1));
}

#[test]
fn tabs_origin_and_relative_motion_use_the_configured_region() {
    let mut grid = grid(20, 6, 0);
    grid.tab();
    assert_eq!(grid.cursor.col, 8);
    grid.clear_tab(false);
    grid.carriage_return();
    grid.tab();
    assert_eq!(grid.cursor.col, 16);
    grid.clear_tab(true);
    grid.goto(0, 3);
    grid.set_tab();
    grid.goto(0, 10);
    grid.backtab();
    assert_eq!(grid.cursor.col, 3);
    grid.set_scroll_region(2, 5);
    grid.origin_mode = true;
    grid.goto(0, 0);
    assert_eq!(grid.cursor.row, 2);
    grid.move_cursor(999, 999);
    assert_eq!((grid.cursor.row, grid.cursor.col), (4, 19));
    grid.move_cursor(-999, -999);
    assert_eq!((grid.cursor.row, grid.cursor.col), (2, 0));
}

#[test]
fn relative_vertical_motion_stops_at_margins_only_from_their_side() {
    // Rows 2..=4 form the region. Like xterm, CUU stops at the top margin
    // from anywhere below it and CUD at the bottom margin from above it.
    for (start, delta, expected) in [
        (8, -20, 2),
        (0, 20, 4),
        (3, -20, 2),
        (3, 20, 4),
        (1, -20, 0),
        (6, 20, 9),
    ] {
        let mut grid = grid(4, 10, 0);
        grid.set_scroll_region(2, 5);
        grid.goto(start, 0);
        grid.move_cursor(delta, 0);
        assert_eq!(grid.cursor.row, expected, "{start} {delta}");
    }
}

#[test]
fn resize_keeps_tab_stops_in_regained_columns() {
    let mut grid = grid(20, 2, 0);
    grid.goto(0, 8);
    grid.clear_tab(false);
    grid.goto(0, 12);
    grid.set_tab();
    grid.resize(Size { cols: 6, rows: 2 });
    grid.resize(Size { cols: 30, rows: 2 });
    let stops = |grid: &mut Grid| {
        grid.carriage_return();
        let mut stops = Vec::new();
        for _ in 0..4 {
            grid.tab();
            stops.push(grid.cursor.col);
        }
        stops
    };
    // Columns past the previous widest width (20) get default stops.
    assert_eq!(stops(&mut grid), [12, 16, 24, 29]);
    grid.clear_tab(true);
    grid.resize(Size { cols: 10, rows: 2 });
    grid.resize(Size { cols: 30, rows: 2 });
    assert_eq!(stops(&mut grid), [29, 29, 29, 29]);
}

#[test]
fn height_resize_keeps_cursor_visible_and_recovers_history_on_growth() {
    let mut grid = grid(6, 4, 10);
    print(&mut grid, "one\r\ntwo\r\nthree\r\nfour");
    grid.resize(Size { cols: 6, rows: 2 });
    assert_eq!(
        (text(&grid, 0), text(&grid, 1)),
        ("three ".into(), "four  ".into())
    );
    assert_eq!(grid.history_size(), 2);
    assert_eq!(grid.cursor.row, 1);
    grid.resize(Size { cols: 6, rows: 4 });
    assert_eq!(text(&grid, 0), "one   ");
    assert_eq!(text(&grid, 3), "four  ");
    assert_eq!(grid.cursor.row, 3);
    assert_eq!(grid.history_size(), 0);
}

#[test]
fn limits_bound_dimensions_and_scrollback_storage() {
    let grid = grid(usize::MAX, usize::MAX, usize::MAX);
    assert!(grid.size.cols * grid.size.rows <= Size::MAX_CELLS);
    // History memory is bounded by its byte limit alone, independent of width
    // and of the row limit.
    assert_eq!(grid.history_limit, usize::MAX);
    assert_eq!(grid.history_byte_limit, DEFAULT_HISTORY_BYTES);
}

#[test]
fn mixed_edit_resize_and_scroll_sequences_preserve_grid_invariants() {
    for initial in [0xd9a3_4871_u32, 0x1234_5678, 0x9e37_79b9, 0x0bad_f00d] {
        mixed_edit_sequence_preserves_grid_invariants(initial, 13, false);
    }
}

/// At 16+ columns, quiet intervals make scrolling rows pack and recycle
/// through `Row::retain`, and pen metadata reaches the packed form.
#[test]
fn wide_mixed_sequences_preserve_grid_invariants_while_packing_rows() {
    for initial in [0x2545_f491_u32, 0x6c07_8965, 0xdead_beef] {
        mixed_edit_sequence_preserves_grid_invariants(initial, 20, false);
    }
}

/// DECSLRM margins move cells within rows instead of rotating whole rows.
#[test]
fn mixed_sequences_inside_left_right_margins_preserve_grid_invariants() {
    for initial in [0x5bd1_e995_u32, 0x85eb_ca6b, 0xc2b2_ae35] {
        mixed_edit_sequence_preserves_grid_invariants(initial, 20, true);
    }
}

fn mixed_edit_sequence_preserves_grid_invariants(mut seed: u32, cols: usize, margins: bool) {
    let wide = cols >= 16;
    let mut grid = grid(cols, 5, 11);
    grid.set_lr_margin_mode(margins);
    let mut packed = false;
    for _ in 0..3000 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let count = (seed >> 8) as usize % 8 + 1;
        grid.pen.style.background = if seed & 0x80 != 0 {
            Color::indexed((seed >> 16) as u8)
        } else {
            Color::DEFAULT
        };
        grid.insert_mode = seed & 0x0100_0000 != 0;
        if wide {
            grid.pen.set_underline_color(if seed & 0x0200_0000 != 0 {
                Color::indexed(1)
            } else {
                Color::DEFAULT
            });
            if seed & 0x0c00_0000 == 0 {
                grid.compact_history(count);
            }
        }
        // Resizes and screen switches reset the margins, so keep setting them.
        if margins && seed >> 29 == 0 {
            let left = (seed >> 4) as usize % 6;
            grid.set_lr_margins(left, left + 2 + (seed >> 12) as usize % 10);
        }
        match seed % 15 {
            0 => grid.put_char('界'),
            1 => grid.put_char('\u{301}'),
            2 => grid.write_ascii(b"abcdef"),
            3 => grid.insert_chars(count),
            4 => grid.delete_chars(count),
            5 => grid.erase_chars(count),
            6 => grid.linefeed(),
            7 => grid.reverse_index(),
            8 => grid.goto(count % grid.size.rows, count % grid.size.cols),
            9 => grid.insert_lines(count),
            10 => grid.delete_lines(count),
            11 => grid.resize(Size {
                cols: if wide { cols - 4 + count } else { count + 1 },
                rows: 3 + count % 4,
            }),
            12 => {
                grid.scroll_display(count as i32 - 4);
            }
            13 => grid.set_alternate(!grid.alternate_active, true, true),
            _ => grid.erase_display(2, false),
        }
        assert!(grid.cursor.col < grid.size.cols);
        assert!(grid.cursor.row < grid.size.rows);
        assert_eq!(grid.screen().rows.len(), grid.size.rows);
        assert!(grid.history.len() <= grid.history_limit);
        let bytes: usize = grid.history.iter().map(history::history_charge).sum();
        assert_eq!(grid.history_bytes, bytes);
        assert!(bytes <= DEFAULT_HISTORY_BYTES);
        packed |= grid.history.iter().any(Row::is_packed);
        // Rows awaiting compaction are charged at least their metadata, and
        // compacted rows exactly what they reference.
        let measured = grid.history.len() - grid.pending_compaction.min(grid.history.len());
        for (index, row) in grid.history.iter().enumerate() {
            let units = row.count_extra_units();
            if index < measured {
                assert_eq!(row.extra_units, units);
            } else {
                assert!(row.extra_units >= units);
            }
        }
        for row in grid
            .history
            .iter()
            .chain(&grid.primary.rows)
            .chain(grid.alternate.iter().flat_map(|screen| &screen.rows))
        {
            assert_eq!(row.cells().len(), grid.size.cols);
            assert!(row.occupied <= row.cells().len());
            // The metadata table is sorted, one entry per column, and holds
            // only cells of the row's text: past `occupied` are erase blanks.
            assert!(row.extras.windows(2).all(|pair| pair[0].0 < pair[1].0));
            assert!(row.extras.last().is_none_or(|(col, _)| *col < row.occupied));
            let blank = Cell {
                style: Style {
                    background: row.clear_background,
                    ..Style::default()
                },
                ..Cell::default()
            };
            assert!(row.line()[row.occupied..].iter().all(|cell| cell == &blank));
            for (col, cell) in row.cells().iter().enumerate() {
                if cell.flags & Cell::WIDE != 0 {
                    assert!(col + 1 < row.cells().len());
                    assert_ne!(row.line()[col + 1].flags & Cell::WIDE_SPACER, 0);
                }
                if cell.flags & Cell::WIDE_SPACER != 0 {
                    assert!(col > 0);
                    assert_ne!(row.line()[col - 1].flags & Cell::WIDE, 0);
                }
                if cell.flags & Cell::LEADING_WIDE_SPACER != 0 {
                    assert_eq!(col + 1, row.cells().len());
                    assert!(row.wrapped);
                }
            }
        }
    }
    assert_eq!(packed, wide);
}

#[test]
fn effects_are_dormant_without_graphics_and_coalesce_contiguous_scrolls() {
    let mut grid = grid(8, 3, 4);
    for _ in 0..20 {
        print(&mut grid, "row\r\n");
    }
    assert!(grid.effects.is_empty());
    assert_eq!(grid.effects.capacity(), 0);
    grid.set_effect_tracking(true);
    for _ in 0..1000 {
        print(&mut grid, "row\r\n");
    }
    assert_eq!(
        grid.effects,
        vec![GridEffect::Scroll {
            alternate: false,
            top: 0,
            bottom: 3,
            lines: 1000,
            retains_history: true,
            history_before: 4,
            history_after: 4
        }]
    );
    let capacity = grid.effects.capacity();
    let mut output = Vec::new();
    grid.drain_effects(&mut output);
    assert_eq!(output.len(), 1);
    assert_eq!(grid.effects.capacity(), capacity);
    assert!(grid.effects.is_empty());
}

#[test]
fn effect_order_preserves_opposing_scrolls_partial_regions_and_clears() {
    let mut grid = grid(8, 5, 10);
    grid.set_effect_tracking(true);
    grid.scroll_up(2);
    grid.scroll_up(1);
    grid.scroll_down(1);
    grid.set_scroll_region(1, 4);
    grid.goto(2, 0);
    grid.delete_lines(1);
    grid.insert_lines(2);
    grid.erase_display(2, false);
    grid.clear_scrollback();
    grid.set_alternate(true, true, true);
    grid.scroll_up(1);
    grid.reset();
    assert_eq!(
        grid.effects,
        vec![
            GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: 5,
                lines: 3,
                retains_history: true,
                history_before: 0,
                history_after: 3
            },
            GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: 5,
                lines: -1,
                retains_history: false,
                history_before: 3,
                history_after: 3
            },
            GridEffect::Scroll {
                alternate: false,
                top: 2,
                bottom: 4,
                lines: 1,
                retains_history: false,
                history_before: 3,
                history_after: 3
            },
            GridEffect::Scroll {
                alternate: false,
                top: 2,
                bottom: 4,
                lines: -2,
                retains_history: false,
                history_before: 3,
                history_after: 3
            },
            GridEffect::Clear {
                alternate: false,
                history_size: 3
            },
            GridEffect::ClearHistory { removed: 3 },
            GridEffect::Clear {
                alternate: true,
                history_size: 0
            },
            GridEffect::Scroll {
                alternate: true,
                top: 0,
                bottom: 5,
                lines: 1,
                retains_history: false,
                history_before: 0,
                history_after: 0
            },
            GridEffect::Reset,
        ]
    );
    assert!(
        grid.track_effects,
        "reset must still report future graphics changes"
    );
    grid.set_effect_tracking(false);
    grid.scroll_up(1);
    assert!(grid.effects.is_empty());
}

#[test]
fn saturated_history_scroll_effects_distinguish_line_deletion() {
    let mut grid = grid(8, 3, 2);
    grid.scroll_up(2);
    grid.set_effect_tracking(true);
    grid.scroll_up(1);
    grid.goto(0, 0);
    grid.delete_lines(1);
    assert_eq!(
        grid.effects,
        vec![
            GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: 3,
                lines: 1,
                retains_history: true,
                history_before: 2,
                history_after: 2
            },
            GridEffect::Scroll {
                alternate: false,
                top: 0,
                bottom: 3,
                lines: 1,
                retains_history: false,
                history_before: 2,
                history_after: 2
            },
        ]
    );
}

#[test]
fn history_clear_and_reset_release_resize_suppression() {
    let mut grid = grid(8, 3, 8);
    grid.erase_display(2, false);
    assert!(grid.clear_anchor);
    grid.erase_display(3, false);
    assert!(!grid.clear_anchor);
    grid.erase_display(2, false);
    grid.reset();
    assert!(!grid.clear_anchor);
}

#[test]
fn invalid_scroll_margins_preserve_the_previous_region_and_cursor() {
    let mut grid = grid(8, 6, 8);
    grid.set_scroll_region(1, 5);
    grid.goto(3, 4);
    grid.set_scroll_region(5, 1);
    grid.set_scroll_region(3, 4);
    grid.set_scroll_region(100, 200);
    assert_eq!(grid.scroll_region(), (1, 5));
    assert_eq!((grid.cursor.row, grid.cursor.col), (3, 4));
}

#[test]
fn inserting_and_deleting_lines_return_to_the_left_margin() {
    // VT510: "The cursor is reset to the first column" (xterm, kitty, Ghostty).
    let mut grid = grid(8, 5, 8);
    print(&mut grid, "first\r\nsecond\r\nthird");
    grid.goto(1, 4);
    grid.insert_lines(1);
    assert_eq!((grid.cursor.row, grid.cursor.col), (1, 0));
    assert_eq!(text(&grid, 1), "        ");
    grid.goto(1, 4);
    grid.delete_lines(1);
    assert_eq!((grid.cursor.row, grid.cursor.col), (1, 0));
    assert_eq!(text(&grid, 1), "second  ");
}

#[test]
fn alternate_width_growth_and_height_shrink_preserve_only_surviving_rows() {
    let mut grid = grid(1, 4096, 0);
    grid.set_alternate(true, true, true);
    grid.put_char('a');
    grid.goto(4095, 0);
    grid.put_char('z');
    grid.resize(Size {
        cols: 4096,
        rows: 1,
    });
    assert!(grid.alternate_active);
    assert_eq!(grid.screen().rows.len(), 1);
    assert_eq!(grid.screen().rows[0].cells.len(), 4096);
    assert_eq!(grid.screen().rows[0].cells[0].character, 'a');
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 0));
    assert!(!grid.pending_wrap);
    grid.set_alternate(false, false, true);
    assert_eq!(grid.primary.rows.len(), 1);
    assert_eq!(grid.primary.rows[0].cells.len(), 4096);
}

#[test]
fn alternate_resize_restores_the_reflowed_primary_cursor_and_style() {
    for (before, after, input) in [
        (
            Size { cols: 8, rows: 3 },
            Size { cols: 4, rows: 3 },
            "abcdef",
        ),
        (
            Size { cols: 4, rows: 3 },
            Size { cols: 8, rows: 3 },
            "abcdef",
        ),
        (
            Size { cols: 8, rows: 3 },
            Size { cols: 4, rows: 3 },
            "abcdefgh",
        ),
        (Size { cols: 4, rows: 3 }, Size { cols: 8, rows: 3 }, "abcd"),
        (
            Size { cols: 8, rows: 4 },
            Size { cols: 8, rows: 2 },
            "one\r\ntwo\r\nthree",
        ),
        (
            Size { cols: 8, rows: 2 },
            Size { cols: 8, rows: 4 },
            "one\r\ntwo\r\nthree",
        ),
    ] {
        let mut direct = Grid::new(before, 10);
        let mut alternate = Grid::new(before, 10);
        for grid in [&mut direct, &mut alternate] {
            grid.pen.style.foreground = Color::indexed(1);
            print(grid, input);
        }
        alternate.set_alternate(true, true, true);
        alternate.pen.style.foreground = Color::indexed(2);
        direct.resize(after);
        alternate.resize(after);
        alternate.set_alternate(false, false, true);
        assert_eq!(alternate.cursor, direct.cursor, "{before:?} -> {after:?}");
        assert_eq!(alternate.pending_wrap, direct.pending_wrap);
        assert_eq!(alternate.pen, direct.pen);

        // Output must append to the same logical position after returning
        // from a fullscreen application, including a pending right-margin wrap.
        print(&mut direct, "X");
        print(&mut alternate, "X");
        assert_eq!(alternate.cursor, direct.cursor);
        assert_eq!(alternate.history_size(), direct.history_size());
        for line in -(direct.history_size() as i32)..after.rows as i32 {
            let actual = alternate.row(line).unwrap();
            let expected = direct.row(line).unwrap();
            assert_eq!(actual.cells(), expected.cells(), "line {line}");
            assert_eq!(actual.wrapped, expected.wrapped, "line {line}");
        }
    }
}

#[test]
fn alternate_resize_preserves_a_separately_saved_primary_cursor() {
    let mut grid = grid(8, 3, 10);
    grid.write_ascii(b"ab");
    grid.save_cursor();
    grid.write_ascii(b"cdef");
    // Mode 47 parks the primary screen without replacing its saved cursor.
    grid.set_alternate(true, false, false);
    grid.resize(Size { cols: 4, rows: 3 });
    grid.set_alternate(false, false, false);
    assert_eq!((grid.cursor.row, grid.cursor.col), (1, 2));
    grid.restore_cursor();
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 2));
    grid.write_ascii(b"X");
    assert_eq!(text(&grid, 0), "abXd");
    assert_eq!(text(&grid, 1), "ef  ");
}

pub(super) fn assert_history_compacted(grid: &mut Grid) {
    while grid.needs_compaction() {
        grid.compact_history(256);
    }
    let dense = grid
        .history
        .iter()
        .filter(|row| row.packed.is_none())
        .count();
    assert_eq!(
        dense,
        0,
        "{dense} of {} history rows stayed dense",
        grid.history.len()
    );
}

#[test]
fn height_shrink_keeps_older_history_in_the_compaction_window() {
    let mut grid = grid(120, 10, 100);
    for line in 0..30 {
        print(&mut grid, &format!("line {line}\r\n"));
    }
    grid.resize(Size { cols: 120, rows: 4 });
    assert!(grid.history_activity);
    assert_history_compacted(&mut grid);
    grid.resize(Size { cols: 120, rows: 8 });
    grid.resize(Size { cols: 120, rows: 3 });
    assert_history_compacted(&mut grid);
}

#[test]
fn forced_compaction_does_not_pack_rows_as_they_scroll() {
    let mut grid = grid(120, 4, 100);
    for line in 0..20 {
        print(&mut grid, &format!("line {line}\r\n"));
    }
    while grid.needs_compaction() {
        grid.compact_pending_history(256);
    }
    assert!(grid.history.iter().all(|row| row.packed.is_some()));
    print(&mut grid, "next\r\n");
    assert!(grid.history.back().unwrap().packed.is_none());
    grid.compact_history(256);
    print(&mut grid, "quiet\r\n");
    assert!(grid.history.back().unwrap().packed.is_some());
}

#[test]
fn fragmented_output_leaves_on_scroll_packing_after_a_bounded_burst() {
    for chunk_size in [1, 17, COMPACT_OUTPUT_BURST_BYTES - 1] {
        let mut grid = grid(120, 4, 100);
        for _ in 0..20 {
            print(&mut grid, "a short line\r\n");
        }
        grid.compact_history(usize::MAX);
        let burst = vec![b'x'; COMPACT_OUTPUT_BURST_BYTES];
        for chunk in burst.chunks(chunk_size) {
            grid.prepare_output(chunk.len());
            grid.write_ascii(chunk);
        }
        grid.prepare_output(8);
        print(&mut grid, "\r\nlast\r\n");
        assert!(
            grid.history.back().unwrap().packed.is_none(),
            "{chunk_size}"
        );

        // A subsequent quiet interval still packs isolated short log lines.
        grid.compact_history(usize::MAX);
        grid.prepare_output(7);
        print(&mut grid, "quiet\r\n");
        assert!(grid.history.back().unwrap().packed.is_some());
    }
}

#[test]
fn forced_compaction_does_not_restart_the_fragmented_output_budget() {
    let mut grid = grid(120, 4, 100);
    for _ in 0..20 {
        print(&mut grid, "a short line\r\n");
    }
    grid.compact_history(usize::MAX);
    grid.prepare_output(COMPACT_OUTPUT_BURST_BYTES / 2);
    grid.compact_pending_history(256);
    grid.prepare_output(COMPACT_OUTPUT_BURST_BYTES / 2);
    print(&mut grid, "continued\r\n");
    assert!(grid.history.back().unwrap().packed.is_none());
}

#[test]
fn borrowed_history_caches_release_before_output_trimming_and_clear() {
    use super::super::types::ExtraSlot;
    use std::sync::Arc;

    let mut grid = grid(40, 4, 100);
    for _ in 0..110 {
        print(&mut grid, "older line\r\n");
    }
    let mut marked = None;
    marked.push_combining('\u{301}');
    let extra = marked.unwrap();
    grid.pen.extra = Some(Arc::clone(&extra));
    print(&mut grid, "x\r\n");
    grid.pen.extra = None;
    for _ in 1..grid.size.rows {
        grid.linefeed();
    }
    grid.compact_history(usize::MAX);
    // Packed rows keep their metadata in their table: reads borrow it.
    assert_eq!(Arc::strong_count(&extra), 2);

    assert_eq!(grid.row_line(-1).unwrap().combining(0), "\u{301}");
    assert!(grid.has_history_read_cache());
    assert_eq!(Arc::strong_count(&extra), 2);
    grid.set_history_limits(2, DEFAULT_HISTORY_BYTES);
    assert!(!grid.has_history_read_cache());
    assert_eq!(Arc::strong_count(&extra), 2);

    grid.row_line(-1).unwrap();
    assert!(grid.has_history_read_cache());
    grid.prepare_output(1);
    assert!(!grid.has_history_read_cache());

    grid.scroll_display(1);
    assert_eq!(grid.visible_row_line(0).unwrap().combining(0), "\u{301}");
    assert!(grid.has_history_read_cache());
    grid.clear_scrollback();
    assert!(!grid.has_history_read_cache());
    assert_eq!(Arc::strong_count(&extra), 1);
    grid.prepare_output(1);
}

#[test]
fn history_cache_tracking_survives_sparse_reads_resize_and_reset() {
    let mut grid = grid(40, 4, 100);
    for line in 0..110 {
        print(&mut grid, &format!("line {line}\r\n"));
    }
    grid.compact_history(usize::MAX);
    let first = grid.row_line(-100).unwrap().snapshot();
    let last = grid.row_line(-1).unwrap().snapshot();
    grid.prepare_output(1);
    assert_eq!(grid.row_line(-100).unwrap().snapshot(), first);
    assert_eq!(grid.row_line(-1).unwrap().snapshot(), last);

    grid.resize(Size { cols: 40, rows: 6 });
    grid.compact_history(usize::MAX);
    grid.row_line(-1).unwrap();
    grid.resize(Size { cols: 80, rows: 4 });
    grid.compact_history(usize::MAX);
    grid.scroll_display(2);
    grid.visible_row_line(0).unwrap();
    assert!(grid.visible_row_line(usize::MAX).is_none());
    assert!(grid.row_line(i32::MIN).is_none());
    grid.reset();
    grid.prepare_output(1);
    assert_eq!(grid.history_size(), 0);
    assert_eq!(text(&grid, 0), " ".repeat(80));
}

#[test]
fn erase_to_end_of_line_shrinks_the_occupied_prefix() {
    let mut grid = grid(200, 2, 4);
    print(&mut grid, "abc");
    grid.erase_line(0, false);
    assert_eq!(grid.primary.rows[0].occupied, 3);

    // A different erase background becomes the clean suffix's background.
    grid.pen.style.background = Color::rgb(1, 2, 3);
    grid.carriage_return();
    grid.move_cursor(0, 1);
    grid.erase_line(0, false);
    let row = &grid.primary.rows[0];
    assert_eq!(row.occupied, 1);
    assert_eq!(row.clear_background, Color::rgb(1, 2, 3));
    assert_eq!(row.line()[0].character, 'a');
    assert!(
        row.line()[1..]
            .iter()
            .all(|cell| cell.style.background == Color::rgb(1, 2, 3))
    );
}

#[test]
fn ascii_runs_without_autowrap_match_scalar_output() {
    for prefix in ["", "ab", "中"] {
        for text in ["xyz", "0123456789", "0123456789abcdef"] {
            let mut fast = grid(8, 2, 4);
            let mut scalar = grid(8, 2, 4);
            for grid in [&mut fast, &mut scalar] {
                grid.autowrap = false;
                print(grid, prefix);
            }
            fast.write_ascii(text.as_bytes());
            print(&mut scalar, text);
            assert_eq!(text_of(&fast), text_of(&scalar), "{prefix:?} {text:?}");
            assert_eq!(fast.cursor, scalar.cursor);
            assert_eq!(fast.pending_wrap, scalar.pending_wrap);
            fast.write_ascii(b"Q");
            scalar.put_char('Q');
            assert_eq!(text_of(&fast), text_of(&scalar), "{prefix:?} {text:?}");
        }
    }
}

pub(super) fn text_of(grid: &Grid) -> Vec<String> {
    (0..grid.size.rows).map(|row| text(grid, row)).collect()
}

#[test]
fn restoring_a_cursor_saved_at_the_right_margin_keeps_its_pending_wrap() {
    let mut grid = grid(12, 3, 8);
    grid.set_lr_margin_mode(true);
    grid.set_lr_margins(0, 6);
    print(&mut grid, "abcdef");
    assert!(grid.pending_wrap);
    grid.save_cursor();
    grid.goto(2, 1);
    grid.restore_cursor();
    assert_eq!((grid.cursor.row, grid.cursor.col), (0, 5));
    assert!(grid.pending_wrap);
    print(&mut grid, "g");
    assert_eq!(text(&grid, 0), "abcdef      ");
    assert_eq!(text(&grid, 1), "g           ");
}

#[test]
fn fast_paths_write_after_cells_with_metadata() {
    let mut grid = grid(20, 2, 4);
    grid.pen.set_underline_color(Color::indexed(1));
    print(&mut grid, "ab");
    grid.pen.set_underline_color(Color::DEFAULT);
    // Metadata left of the cursor (a link earlier on the line) doesn't turn
    // either fast path away.
    assert!(grid.write_ascii_cell(b'x'));
    assert_eq!(grid.write_utf8_run("éé".as_bytes()).0, 4);
    // Writing over cells with metadata is left to the general path, which
    // drops it.
    grid.carriage_return();
    assert!(!grid.write_ascii_cell(b'y'));
    assert_eq!(grid.write_utf8_run("é".as_bytes()).0, 0);
    grid.put_char('y');
    let row = &grid.primary.rows[0];
    let columns: Vec<_> = row.extras.iter().map(|(col, _)| *col).collect();
    assert_eq!(columns, [1]);
    assert!(text(&grid, 0).starts_with("ybxéé"));
}
