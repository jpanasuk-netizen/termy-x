use super::*;

fn rendered_rows(grid: &Grid) -> Vec<Vec<(Cell, bool)>> {
    (0..grid.size.rows)
        .map(|row| {
            let row = grid.visible_row(row).unwrap();
            row.cells()
                .iter()
                .copied()
                .enumerate()
                .map(|(col, cell)| (cell, row.wrapped && col + 1 == grid.size.cols))
                .collect()
        })
        .collect()
}

fn replay_damage(grid: &mut Grid, cached: &mut Vec<Vec<(Cell, bool)>>) -> usize {
    let (damage, scrolls) = grid.take_render_damage();
    let actual = rendered_rows(grid);
    if damage == Damage::Full {
        assert!(scrolls.is_empty());
        *cached = actual;
        return 0;
    }
    for scroll in &scrolls {
        let rows = &mut cached[scroll.top..scroll.bottom];
        if scroll.lines > 0 {
            rows.rotate_left(scroll.lines as usize);
        } else {
            rows.rotate_right(scroll.lines.unsigned_abs() as usize);
        }
    }
    let Damage::Partial(spans) = damage else {
        unreachable!();
    };
    for span in spans {
        cached[span.row][span.start..span.end]
            .clone_from_slice(&actual[span.row][span.start..span.end]);
    }
    assert_eq!(*cached, actual);
    scrolls.len()
}

#[test]
fn ordered_scroll_damage_replays_edits_before_between_and_after_scrolls() {
    let mut grid = grid(8, 5, 10);
    print(&mut grid, "one\r\ntwo\r\nthree\r\nfour\r\nfive");
    let mut cached = Vec::new();
    replay_damage(&mut grid, &mut cached);
    grid.goto(2, 1);
    print(&mut grid, "界");
    grid.scroll_up(1);
    grid.goto(4, 0);
    print(&mut grid, "new");
    grid.set_scroll_region(1, 4);
    grid.scroll_down(1);
    grid.goto(1, 3);
    print(&mut grid, "z\u{301}");
    grid.scroll_up(2);
    grid.goto(2, 2);
    grid.insert_lines(1);
    assert_eq!(replay_damage(&mut grid, &mut cached), 4);
    assert_eq!(grid.take_render_damage(), (Damage::Partial(vec![]), vec![]));
}

#[test]
fn hidden_cursor_deferred_wrap_damages_the_previous_row_metadata() {
    for next in ["X", "界"] {
        let mut grid = grid(4, 3, 10);
        grid.cursor.visible = false;
        grid.write_ascii(b"abcd");
        let mut cached = Vec::new();
        replay_damage(&mut grid, &mut cached);
        assert!(!cached[0][3].1);

        print(&mut grid, next);
        assert!(grid.row(0).unwrap().wrapped);
        replay_damage(&mut grid, &mut cached);
        assert!(cached[0][3].1);
    }
}

#[test]
fn scroll_damage_coalesces_and_falls_back_at_a_bounded_record_count() {
    let mut grid = grid(8, 5, 0);
    grid.take_damage();
    for _ in 0..100 {
        grid.scroll_up(1);
    }
    let (damage, scrolls) = grid.take_render_damage();
    assert_ne!(damage, Damage::Full);
    assert_eq!(
        scrolls,
        vec![ViewportScroll {
            top: 0,
            bottom: 5,
            lines: 5
        }]
    );
    for _ in 0..MAX_SCROLL_DAMAGE {
        grid.scroll_up(1);
        grid.scroll_down(1);
    }
    assert_eq!(grid.take_render_damage(), (Damage::Full, vec![]));
    assert_eq!(grid.pending_scrolls.capacity(), MAX_SCROLL_DAMAGE);
}

#[test]
fn cell_only_damage_covers_scrolls_and_full_invalidations_discard_them() {
    let mut grid = grid(8, 5, 10);
    print(&mut grid, "one\r\ntwo\r\nthree\r\nfour\r\nfive");
    grid.take_damage();
    grid.scroll_up(1);
    let Damage::Partial(spans) = grid.take_damage() else {
        panic!("expected spans")
    };
    assert_eq!(
        spans,
        (0..5)
            .map(|row| DirtySpan {
                row,
                start: 0,
                end: 8
            })
            .collect::<Vec<_>>()
    );
    grid.scroll_up(1);
    grid.set_alternate(true, true, true);
    assert_eq!(grid.take_render_damage(), (Damage::Full, vec![]));
    grid.set_alternate(false, false, true);
    grid.take_damage();
    assert!(grid.scroll_display(1));
    grid.take_damage();
    grid.scroll_up(1);
    assert_eq!(grid.take_render_damage(), (Damage::Full, vec![]));
}

#[test]
fn randomized_scroll_damage_reconstructs_cell_and_wrap_metadata() {
    let mut grid = grid(9, 6, 12);
    let mut cached = Vec::new();
    replay_damage(&mut grid, &mut cached);
    let mut random = 0xabcddcba12344321_u64;
    let mut scrolls = 0;
    for _ in 0..500 {
        for _ in 0..7 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = (random >> 32) as usize;
            match value % 12 {
                0 => grid.goto(value / 12 % grid.size.rows, value / 100 % grid.size.cols),
                1 => grid.scroll_up(1 + value % 3),
                2 => grid.scroll_down(1 + value % 3),
                3 => print(&mut grid, "A界e\u{301}"),
                4 => grid.insert_lines(1 + value % 2),
                5 => grid.delete_lines(1 + value % 2),
                6 => grid.linefeed(),
                7 => grid.set_scroll_region(value / 12 % 3, 4 + value % 3),
                8 => grid.erase_chars(1 + value % 3),
                9 => grid.carriage_return(),
                10 => grid.delete_chars(1 + value % 3),
                _ => print(&mut grid, "123456789012345"),
            }
        }
        scrolls += replay_damage(&mut grid, &mut cached);
    }
    assert!(scrolls > 100);
}
