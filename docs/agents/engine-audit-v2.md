# Terminal engine audit v2: work list

Audit date: 2026-10-12, against `main` @ `225447ea` (after PR #414 landed every item in [engine-audit.md](engine-audit.md)).

Scope: same as v1, plus the code PR #414 added:
- `crates/core/src/terminal_engine/**`
- `crates/core/src/runtime/**`
- `crates/core/src/kitty_graphics.rs` and `kitty_graphics/**`
- `crates/core/src/keyboard.rs` (legacy encoder)
- `crates/pty` and `packages/pty`
- desktop snapshot/render call sites

IDs continue the v1 numbering (v1 ended at CPU-7, MEM-8, COR-4, LOW-24) so a commit that says `COR-5` is unambiguous. `VT-n` is new: xterm/VT conformance.

Items marked **measured** were reproduced with a temporary test or probe on macOS. Those probes were not committed. Items marked **plausible** come from reading the code. Linux and Windows were not compiled locally, because the cross C compiler needed by `ring` isn't installed, so CI has to cover them.

## Progress

Work branch: `fix/engine-audit-v2`, [PR #415](https://github.com/lassejlv/termy/pull/415). Throughput gate (`terminal_engine_bench`, 64 MiB, alternating pairs): every case within ~2% of `main`, steady state still allocation-free. Statuses: ✅ fixed + tested · 🟡 partial · ⏳ todo · ⏭ skipped · ⏸ deferred.

| ID | Status | Notes |
|---|---|---|
| COR-5 | ✅ | `Engine::commit_upload` releases the history read cache before committing, and `release_history_read_cache` clamps `end` to `history.len()` as a backstop. Test: `committing_an_upload_releases_history_read_while_it_decoded` (fails without the fix). |
| COR-6 | ✅ | New `signal_socket_pair` (sets `SO_NOSIGPIPE` on Apple) and `send_signal_byte` (`send` with `MSG_NOSIGNAL` elsewhere) in `transport/unix/master.rs`, used for the writer wake socket and the watcher→reader shutdown socket (the watcher could also write after the reader exited). Test: `dropping_after_the_child_exited_does_not_raise_sigpipe` runs in an isolated helper with SIGPIPE at `SIG_DFL`; without the fix the helper dies with 141. |
| CPU-8 | ✅ | `a=c` composes in place when the destination frame is unshared (a frame a snapshot still holds is copied first). `a=c`/`a=f` count written pixels; past `ANIMATION_PIXELS_PER_FEED` (4M) the parser pauses through the deferred-upload hook, so the runtime releases the lock between heavy edits (hydration and plain `Engine::feed` aren't paced). `update_frame` fills the background per pixel. Release, audit repro (4096², 341 compositions): 2.66 s → ~2.15 s total, longest lock hold ~8–12 ms instead of the whole run; one 4096² compose is still ~6 ms (memory-bound). Tests: `compositions_write_into_unshared_frames_in_place`, `heavy_animation_edits_pause_deferring_feeds`. |
| CPU-9 | ✅ | New `OriginResolver` (`placement.rs`) replaces `resolve_render_origin`: a parent index built on the first relative hop, plus each virtual parent's placeholder scan cached per command; used by deletes and render snapshots. Release: 4000 children + full placeholder screen, ~19 ms → ~0.33 ms per `a=d,d=q`. Test: `positional_deletes_resolve_a_shared_virtual_parent_once`. |
| MEM-9 | ✅ | `GraphicsImage::from_rgba` trims spare capacity; zlib output is pre-sized to the known raw size (capped at 1032× the compressed length). Test: `stored_images_hold_no_more_memory_than_they_are_charged` (chunked + `o=z`, 1025×1024; 6.29 MB held vs 4.20 MB charged without the trim). |
| CPU-10 | ✅ | Cache bookkeeping moved into a GPUI-free `TextureCache<T>`. Other generations of an id are evicted eagerly only when the inserted placement isn't playing; frames of a playing animation stay cached within the existing 256-entry / 128 MiB LRU. Pruning by snapshot generations (the doc's suggestion) wouldn't work: a snapshot only carries the current frame's generation. ⚠ Manual frame switches on a stopped animation still rebuild. Tests: `kitty_animation_frames_keep_their_textures_while_playing`, `kitty_retransmission_drops_older_generations_of_the_id`, `kitty_texture_cache_evicts_least_recently_used_over_budget`. |
| VT-1 | ✅ ⚠ behavior change | DECSC/DECRC, `CSI s`/`u`, 1048 and 47/1047/1049 switches only save/restore the position; visibility, shape and blink stay live, and a fresh alternate screen just homes the cursor. A program that hides the cursor on the alternate screen and never shows it keeps it hidden after exit (xterm does the same). Tests: `save_restore_and_screen_switches_keep_cursor_visibility_shape_and_blink`, `restore_and_alternate_screens_keep_the_configured_default_cursor_shape`; `saved_cursor_restores_style_wrap_and_origin` extended. |
| VT-2 | ✅ ⚠ behavior change | Resetting any of 9/1000/1002/1003 turns tracking off; encoding resets still clear only their own mode. `unrelated_mouse_mode_resets_preserve_active_tracking_and_encoding` encoded the old behavior and is replaced by `resetting_any_mouse_tracking_mode_turns_tracking_off` + `unrelated_mouse_encoding_reset_preserves_active_encoding`. |
| VT-3 | ✅ ⚠ API change | 1015 and 1016 are rejected (DECRQM answers 0, the current encoding is kept). 1016 needs pixel coordinates that `TerminalMousePosition` doesn't carry; 1015 is deprecated. The never-set `MouseEncoding::Urxvt`/`SgrPixels` variants are removed (custom_backend, wasm input updated). Test: `unsupported_urxvt_and_sgr_pixel_mouse_modes_are_rejected`. |
| VT-4 | ✅ ⚠ behavior change | Legacy encoder moved to new `keyboard/legacy.rs` (keyboard.rs 1673 → 1544 lines). Modified cursor/Home/End → `CSI 1;m X`, F1–F4 → `CSI 1;m P..S`, F5–F12/Insert/Delete/PgUp/PgDn → `CSI n;m ~`, Insert → `\e[2~`; Alt and Super count toward `m`. Keypad keys named `kp_*` honor DECKPAM (`TerminalKeyboardMode::with_application_keypad`, not part of the mux wire format). On Linux/Windows Alt+text now gets the ESC prefix (Alt+b → `\eb`); macOS Option text unchanged. Limitation: GPUI doesn't report keypad keys distinctly, so DECKPAM only works for FFI/web hosts sending `kp_*`. Tests in `keyboard::legacy::tests` (14); `non_macos_secondary_shortcuts_do_not_remap_in_alternate_screen` updated deliberately. |
| COR-7 | ✅ | `queue_reader_protocol_reply` no longer reports a closed writer to the reader, and the reader no longer SIGKILLs on it: the writer only closes during drop or after the child is gone, and drop's SIGHUP grace + escalation own shutdown. Live replies from the host (`Transport::write_protocol_reply_owned`) keep their `alive`-checked teardown. Test: `a_reply_during_the_drop_grace_keeps_the_hup_handler_running` (HUP trap prints DA1, sleeps, writes a marker; fails without the fix). `unqueueable_reader_reply_is_dropped_and_keeps_the_writer_open` updated for the `()` return. |
| COR-8 | ✅ | The PTY exit closure calls `notify_tab_chrome()` instead of `notify()`, so hidden tabs wake for the exit (it clears program status). Test: `hidden_terminal_wakes_the_host_when_its_child_exits` (fails without the fix). |
| COR-9 | ✅ | `NativePty::kill` no longer un-pauses output; a `pause()`/XOFF stays in effect and the exit is delivered after `resume()`. Test (vitest): `a signal that does not end the program keeps output paused` (fails without the fix: `expected 'winch\r\n' to be ''`). |
| COR-10 | ✅ | A restart is detected before the size check/base64 decode; the abandoned upload is dropped and the new command is checked and answered as itself (a single-chunk restart now also decodes off the lock). Test: `a_restarting_upload_is_checked_and_answered_as_itself`. |
| COR-11 | ✅ | Orphan cleanup no longer frees named images; id-less ones are still released by `release_unplaced_anonymous_images`. Test: `orphaned_relative_placements_keep_their_named_images`. |
| COR-12 | ✅ ⚠ behavior change | Matches kitty's `graphics.c`: re-transmit removes the old placements first, then the parent check fails with `ENOPARENT` (image stored, result marked changed). Test: `resending_an_image_relative_to_its_own_placement_has_no_parent`. |
| COR-13 | ✅ | Replies take `i=` only from `i` and send `I=` on its own (`a=p,I=5` → `\e_GI=5;ENOENT…`); order `i`, `I`, `p`. Test: `replies_report_image_numbers_as_numbers`. |
| COR-14 | ✅ | `wait_for_child` returns `Option<c_int>`; the watcher reports `ChildExit::default()` (unknown) when `waitpid` fails instead of `code: Some(0)`. Test: `a_failed_wait_reports_an_unknown_exit_instead_of_success`. |
| COR-15 | ✅ | PTY feeds now queue each slice's replies to the transport while still holding the engine lock (`Shared::write_replies_locked`), and the sync watchdog writes its replies before releasing the lock too, so the two feeders can't reorder replies. Display terminals and reads before the transport is registered keep the old return-at-end path. Test: `pty_replies_reach_the_child_in_order` (end-to-end round trip; the race itself isn't deterministic to reproduce). |
| COR-16 | ✅ (local) | New `Terminal::new_with_launch_restoring_output` replays the saved buffer before `Transport::spawn` (`CustomBackend::new_with_launch_and_wakeup_notifier` takes `restored_output`), and the desktop's restored panes use it through `Terminal::new_native_restoring`. Session-host panes still hydrate right after spawning: passing the buffer in `PaneLaunch` changes the bincode wire format and needs a mux `VERSION` bump, which would lock new app versions out of a running older host. Test: `restored_output_precedes_the_new_sessions_first_output`. |
| CPU-11 | ✅ | A full table rescans only after new-link requests since the last rescan reach 1/16 of its size (amortized O(16) per OSC 8); a link is dropped only when the table holds its sole reference. Scratch bench (4 links/row, 20k history, full table): 43.6 µs → 0.35 µs per OSC 8, none rejected. Test: `full_table_rescans_are_amortized_and_never_evict_referenced_links`. |
| MEM-10 | ✅ | History rows are charged for per-cell `CellExtra` heap (incl. combining text); the charge is stored on the row (fits existing padding, row stays 48 B) so the byte count matches a recount. Writers set a per-row metadata flag in `put_char` only when the pen carries metadata (not in `write_ascii`); a flagged row entering history is charged an upper bound, measured exactly when packed. Combining-heavy output: 78.1 MiB / 8738 rows → 22.7 MiB / 2510 rows against the 24 MiB budget. Tests: `heap_bytes_count_cell_metadata_once_per_shared_run`, `combining_heavy_history_counts_its_metadata_against_the_budget`, plus a fuzz assertion. |
| CPU-12 | ✅ | Crossing the budget packs the oldest pending rows 256 at a time and stops once under budget. Worst 4 KiB feed, plain output: ~2.4 ms → 90–150 µs (p99.9 1.25 ms → 72 µs); linked+colored pen: 4.2 ms → ~160 µs, but that case's throughput is ~3.5% lower (85 vs 88 MiB/s) from measuring metadata at pack time (not in the CI gate). Test: `crossing_the_byte_budget_packs_one_chunk_of_the_oldest_queued_rows`. |
| VT-5 | ✅ | Leaving 1049 emits `GridEffect::Clear { alternate: true, history_size: 0 }`. Like `CSI 2J`, it keeps virtual (Unicode-placeholder) placements; kitty drops all alternate-screen images, which would need a new effect variant. Test: `leaving_1049_clears_alternate_screen_images_with_its_text` (fails without the fix). |
| VT-6 | ✅ | `file://` (scheme case-insensitive) with a bad `%` escape or non-UTF-8 keeps the raw path; other `scheme://` URLs are ignored; bare paths (incl. `C:\...`) pass through. Test: `osc7_file_urls_are_percent_decoded_and_other_schemes_ignored`. |
| VT-7 | ✅ | `move_cursor`: up stops at the top margin only when starting at/below it, down stops at the bottom margin only when starting at/above it; origin mode still clamps. Test: `relative_vertical_motion_stops_at_margins_only_from_their_side`. |
| VT-8 | ✅ | Any G0/G1 designation other than `0` selects ASCII; DEC graphics `_` prints a space. Test: `other_charset_designations_select_ascii_and_dec_graphics_underscore_is_blank`. |
| VT-9 | ✅ | DECRC and 1049 exit keep the live OSC 8 link; style and SGR 58 still come from the save. Test: `restore_cursor_keeps_the_current_hyperlink_and_restores_underline_color`. |
| VT-10 | ✅ ⚠ behavior change | OSC 12 always replies: program-set cursor color, then configured `query_colors.cursor`, then the foreground. `cursor_queries_require_live_override` → `cursor_queries_prefer_live_override_then_configured_cursor`; new `cursor_color_query_falls_back_to_foreground`. |
| VT-11 | ✅ | `?3` reports its own flag (width never changes; RIS clears it); `?1048` reports whether the current screen has a saved cursor. Neither is in XTSAVE/XTRESTORE. Test: `column_and_save_cursor_modes_are_reported`. |
| VT-12 | ✅ | Tab stops are kept up to the widest width seen; only columns beyond it get default stops; `CSI 3 g` clears hidden ones too. Test: `resize_keeps_tab_stops_in_regained_columns`. |
| LOW-25 | ✅ | Denylist adds `COLORTERM` (Termy's own override still wins, and `colorterm` off now really omits it), `LC_TERMINAL`, `LC_TERMINAL_VERSION`, `KITTY_INSTALLATION_DIR`, `WEZTERM_EXECUTABLE[_DIR]`, `WEZTERM_CONFIG_DIR/FILE`. Matching is ASCII case-insensitive on Windows. Tests: `child_environment_drops_the_parent_terminals_identity` (extended; `LANG`/`LC_ALL` kept), `parent_terminal_variables_match_case_insensitively` (Windows, CI only). |
| LOW-26 | ✅ | If moving the status endpoint above stderr fails, `attach_child_terminal` returns the original endpoint (nothing is redirected yet), so the parent gets a `ControllingTerminal` failure record instead of a clean EOF. Test: `a_child_setup_failure_is_reported_instead_of_looking_like_a_spawn` (macOS, isolated helper: RLIMIT_NOFILE 64, table filled, stdin/stdout freed for the status pair). Without the fix the spawn instead fails later with `Too many open files`. |
| LOW-27 | ✅ | `WaiterAnnouncement` RAII guard in `lock.rs` withdraws the waiter count on drop, including unwinding. Test: `a_panicking_waiter_withdraws_its_announcement`. |
| LOW-28 | ✅ | Local `with_render_damage` validates spans like the remote backend (row < rows, left ≤ right < cols) and returns false without visiting. `visit_viewport_ranges_at_generation` keeps clipping (`partial_reads_clip_dirty_ranges_to_the_viewport`). Test: `damage_cells_reject_spans_outside_the_viewport`. |
| LOW-29 | ✅ | The parent blocks every signal (`pthread_sigmask`) around `fork` and restores its mask right after, so no parent handler runs in the child before the reset. The child resets every disposition (1..=64 on Linux/Android, 1..=31 elsewhere; not KILL/STOP) before clearing its mask, so `SIG_IGN` on SIGTSTP/SIGTTOU/SIGUSR1/… no longer survives exec. Unused signal constants removed. Test: `child_resets_inherited_ignored_signals` now also covers SIGUSR1 (fails with the old fixed list). |
| LOW-30 | ✅ / ⏭ | `wait_for_pty_writable` returns BrokenPipe when the master reports POLLHUP/POLLERR without POLLOUT, instead of letting the writer retry until the reader closes it. POLLERR fails even with POLLOUT, because Linux reports a pipe whose reader is gone as `POLLERR | POLLOUT`. Test: `waiting_on_a_hung_up_descriptor_fails_instead_of_spinning`. Skipped: suppressing the wake byte while the writer sits in `recv`. A correct version needs a Dekker-style handshake between the writer's poll and the command channel (otherwise a resize/close can wait for the 1 s backstop), and it only saves one nonblocking `send` per reply/resize. |
| LOW-31 | ✅ | `Output::flush` resets `scheduled` when the callback is gone or the threadsafe call fails, so a failed flush can't stop delivery for good. The paste-splitting half doesn't apply: the Node addon has no engine and never generates protocol replies, so nothing can be spliced into a chunked write. |
| LOW-32 | ✅ | Intern lookups borrow the request (no two-String allocation) and the table keeps one copy of each link's text; reopening the pen's current link reuses its `Arc<CellExtra>`; wide-glyph spacers reuse one cached extra when the pen has link + underline color. Test: `reopened_links_and_wide_spacers_reuse_shared_metadata`. |
| LOW-33 | ✅ | New `wide_mixed_sequences_preserve_grid_invariants_while_packing_rows`: 20-column fuzz with an underline-color pen and forced compaction; asserts rows pack, metadata flags are set and history charges are exact. Verified it reaches `Row::retain` by temporarily panicking there. |
| COR-17 | ✅ | Found while fixing: `Cell`'s const size assert (`== 24`) broke every wasm32 build since #414 (4-byte pointer → 20 B). Now `size_of::<usize>() + 16`. `cargo check -p termy_wasm --target wasm32-unknown-unknown` passes. Only `npm-publish.yml` builds wasm, so CI didn't catch it; consider a wasm check in CI. |
| PROTO-1 | ✅ | `CSI > q` / `CSI > 0 q` reply `DCS > | Termy <version> ST`. The version is termy_core's `CARGO_PKG_VERSION` (0.2.0, not the app's 1.0.0-rc.3), the only version the engine has; a host setter would be needed for the app version. Note `TERM_PROGRAM` says `ghostty` for compatibility while XTVERSION says Termy; previously there was no reply at all. Test: `xtversion_reports_the_engine_name_and_version`. |
| PROTO-2 | ✅ | Confirmed the engine clusters graphemes (UAX #29 via `unicode_segmentation` in `grid/print.rs`); DECRQM `?2027` answers 3 (permanently set), set/reset is a no-op, XTSAVE skips it. Test: `grapheme_clustering_mode_is_permanently_set`. |
| PROTO-3 | ✅ ⚠ behavior change | OSC 4/10/11/12 replies end with BEL when the query did (previously always ST), via new `State::osc_with_terminator`. Six tests that expected ST after BEL queries were updated (`cursor_color_query_falls_back_to_foreground`, five `query_colors` tests, one `replies` test). Test: `color_queries_echo_the_query_terminator`. |
| PROTO-4 | ✅ | XTGETTCAP adds `Smulx` (`\E[4:%p1%dm`), `Setulc` (kitty's value) and `Ms` (`\E]52;%p1%s;%p2%s\007`), each checked against what the engine supports. Test: `capability_queries_advertise_styled_underlines_and_clipboard_writes`. |
| PROTO-5 | ✅ | New `Modes::reverse_video` (DECRQM, XTSAVE, RIS); a toggle bumps `palette_revision` and marks full damage. Rendering: `State::palette()` swaps the resolved default colors (program override, else the host's query colors), so desktop, FFI and mux hosts get it with no renderer change; wasm adds `mode_bits::REVERSE_VIDEO` and swaps in `dynamic_color_overrides` once `set_query_colors` was called. Tests: `reverse_video_mode_is_tracked_reported_and_repaints`, `reverse_video_swaps_the_resolved_default_colors`. |
| PROTO-6 | ✅ | `ESC # 8` → `Grid::screen_alignment_test`: resets top/bottom margins, fills with `E` (pen colors only, like Ghostty), homes, clears images like ED 2, full damage; origin mode stays (xterm). Test: `screen_alignment_fills_with_e_resets_margins_and_homes`. |
| PROTO-7 | ✅ | Branch `feat/declrmm-margins` (stacked on #415). `?69` (DECLRMM) gates `CSI Pl;Pr s` (DECSLRM); otherwise `CSI s` stays SCOSC. Margins live in `grid/margins.rs`; while the mode is off they span the screen, so the print paths add one compare per run (throughput gate: every case within noise of #415). Printing wraps at the right margin to the left margin (from right of the margin it wraps at the screen edge, like xterm); rows wrapped inside margins aren't marked `wrapped`, so they don't reflow as one line. LF/IND/RI scroll only inside both margin pairs; IL/DL/SU/SD/ICH/DCH stay inside the margins, and ICH/DCH/IL/DL do nothing outside them. CR/BS/CUF/CUB/HT/CBT stop at a margin from its side; CUP/CHA/HPA and CPR are margin-relative in origin mode (CHA/VPA no longer go through CR/CUP). DECRQM `?69`, DECRQSS `s`, XTSAVE. Resize, screen switches, DECCOLM and DECALN reset the margins; DECSTR and RIS also turn the mode off. Scrolling inside narrower margins doesn't add to scrollback and leaves kitty placements in place. Wide pairs straddling a margin are blanked before cells move. Tests: `dispatch/margin_tests.rs` (7), `mixed_sequences_inside_left_right_margins_preserve_grid_invariants` (fuzz; verified it reaches `scroll_rect`). |
| PROTO-8 | ✅ ⚠ behavior change | `CSI 0 SP q` (and `CSI SP q`) restore the configured default shape and the startup (steady) blink instead of forcing blink on. The `cursor_blink` setting isn't passed into the engine, so "default blink" is the engine's startup value. Test: `cursor_style_zero_restores_the_configured_default_without_blinking`. |
| PROTO-9 | ✅ ⚠ behavior change | `?1049h` while already on the alternate screen (via 47/1047/1049) saves the cursor into the alternate screen's slot and clears it (`erase_display(2)`), like xterm; leaving 1049 still restores the primary's saved cursor. Test: `entering_1049_from_another_alternate_mode_saves_the_cursor_and_clears`. |
| PROTO-10 | ✅ | X10 (`?9`) now maps to an enabled `TerminalMouseMode` with no reporting flag (`report_click` is only set for 1000), exposed as `TerminalMouseMode::presses_only()`; the encoder drops releases and modifiers in that mode. No new field, so the mux wire format is unchanged; an older host still sends `report_click: true` for X10 and keeps the old behavior. The inspector labels it `press`. Tests: `x10_mode_reports_presses_without_modifiers_and_no_releases` (also checks tmux `?1002`-only still reports releases), `mouse_mode_maps_x10_to_presses_only`. |
| CI-1 | ✅ | New `wasm-check` job in `.github/workflows/architecture-checks.yml`: `cargo check --locked -p termy_wasm --target wasm32-unknown-unknown` on every PR and push to main. |

## How to work this list

Same rules as v1 (see "How to work this list" in [engine-audit.md](engine-audit.md)):
- Reference the ID in commits/PRs.
- Line numbers are as of `225447ea`; locate code by the named symbol.
- Add a regression test next to the code for every fix.
- Run `cargo fmt`, clippy, `cargo test -p termy_core`, `just check`, `just check-boundaries`, `just check-file-sizes`.
- Update this Progress table as items land.

## Priority order

| ID | Sev | Area | One-line summary |
|---|---|---|---|
| COR-5 | High | grid + runtime | Panic: history read cache not released before a deferred upload commit scrolls |
| COR-6 | High (FFI hosts) | transport/unix | Dropping a Transport after the child exited raises SIGPIPE on macOS |
| CPU-8 | Med-High | kitty animation | `a=c` compose runs unbounded under the engine lock |
| CPU-9 | Med | kitty deletion | Positional deletes are O(P·(P+C)) per command |
| MEM-9 | Med | media | Image quota undercounts real memory by up to 2× (spare `Vec` capacity) |
| CPU-10 | Med | desktop | Animated images rebuild their texture every frame |
| VT-1 | Med | grid | DECSC/DECRC and screen switches save cursor visibility/shape/blink |
| VT-2 | Low-Med | dispatch | Resetting one mouse tracking mode leaves another one active |
| VT-3 | Low-Med | runtime | Mouse modes 1015/1016 reported as supported but encoded wrongly |
| VT-4 | Med | keyboard | Legacy key encoder ignores modifiers on cursor/function keys and DECKPAM |
| COR-17 | High (wasm) | types | wasm32 build broken by the `Cell` size assert (found while fixing) |
| … | Low–Med | various | See the sections below |

---

## High

### COR-5: Panic after a deferred upload commit scrolls history (measured) — regression from CPU-3
- **Where:** `Grid::release_history_read_cache` (`grid.rs:181-192`), `Engine::commit_upload` (`terminal_engine/mod.rs:141-145`), `runtime/custom_backend/lock.rs:63-71`, cursor advance in `graphics.rs:286-292`.
- **Problem:** borrowed history reads (`Engine::line(-k)` on a packed row) record `history_read_start..end`, which every history-mutating entry point releases first. `Shared::feed` drops the engine lock while an upload decodes; the UI's `hyperlink_at` can call `engine.line(-k)` in that window. `commit_upload` then advances the cursor with linefeeds without releasing the range. With history at the byte budget, eviction can shorten the deque, and the next `prepare_output` calls `history.range_mut(start..end)` with `end > len`.
- **Repro:** 400×2 engine, fill history to the byte budget with unpackable rows, `feed_deferring_uploads(ESC_Ga=T,f=32,s=1,v=1,i=7,c=1,r=2;AQID/w==ESC\ …)`, `take_deferred_upload()`, `line(-1)`, `commit_upload(...)`, feed the rest → `range end index 5489 out of range for slice of length 5421`.
- **Fix:** release the history read cache at the top of `Engine::commit_upload`; clamp `end` to `history.len()` in `release_history_read_cache` as a backstop.

### COR-6: SIGPIPE when a Transport is dropped after its child exited (measured, macOS) — regression from LOW-1
- **Where:** `WriterHandle::wake` (`unix.rs:415-420`), called by `close()` (`unix.rs:408-413`), `Drop` (`unix.rs:1463-1474`), reader (`unix.rs:901`).
- **Problem:** the wake socket's read end belongs to the writer thread and is dropped when it exits. A later `close()` writes into a socket with no reader. std's `UnixStream::pair` doesn't set `SO_NOSIGPIPE` on macOS, so the process gets SIGPIPE.
- **Who:** Swift/C hosts of libtermy with default SIGPIPE disposition. The Rust desktop app/CLI (std ignores SIGPIPE), Node, and Linux (`MSG_NOSIGNAL`) are not affected.
- **Fix:** set `SO_NOSIGPIPE` on Apple targets for the wake pair (and any other socketpair the transport writes to), or send with `MSG_NOSIGNAL` where available. Test with SIGPIPE at `SIG_DFL`.

## Medium

### CPU-8: Animation compose (`a=c`) is unbounded under the engine lock (measured)
- **Where:** `kitty_graphics.rs:402-417` → `kitty_graphics/animation.rs:50-62` → `media/animation.rs:162-214`.
- **Problem:** each command copies the whole destination frame (`to_vec`) and blends it inline. 4096×4096 root (`o=z`, ~64 KiB payload) + one `a=f` frame, then 8 KiB of `\e_Ga=c,i=1,r=1,c=2,q=2\e\\` (341 commands) took 2.66 s in one `Engine::feed`. `a=f` costs ~2 ms per 45-byte command (`update_frame`, byte-by-byte background fill), but goes through the deferred path.
- **Fix:** defer/pause the parser after heavy animation edits like uploads, or cap pixel work per feed; compose in place instead of `to_vec`.

### CPU-9: Positional deletes are O(P·(P+C)) per command (measured) — leftover of CPU-6
- **Where:** `kitty_graphics/deletion.rs:54-108` calls `resolve_render_origin` (`placement.rs:55-142`) per placement.
- **Problem:** 4000 placements relative to one virtual placement, screen full of placeholders: each 30-byte `a=d,d=q,x=1,y=1,z=999` took ~19 ms, ~5 s per 8 KiB slice.
- **Fix:** resolve origins once per command (memo map).

### MEM-9: Image quota undercounts memory up to 2× (measured) — regression from MEM-3
- **Where:** `media/image.rs:29-34` keeps the incoming `Vec` with spare capacity. Sources: chunk assembly `extend_from_slice` (`kitty_graphics.rs:503`), zlib `read_to_end` (`kitty_graphics/upload.rs:140-143`).
- **Measured (1025×1024, f=32):** chunked charged 4.20 MB / held 6.29 MB; `o=z` charged 4.20 MB / held 8.39 MB.
- **Fix:** `shrink_to_fit` in `from_rgba`, and pre-size inflate output when the raw size is known.

### CPU-10: Desktop rebuilds animated image textures every frame (plausible) — regression from MEM-7
- **Where:** `crates/desktop_app/src/terminal_view/kitty_images.rs:71-80`.
- **Problem:** on insert the cache drops every other generation of the same image id. Animation frames share the id with per-frame generations, so each frame change discards the others' textures and re-runs `padded_bgra` + atlas upload.
- **Fix:** prune against generations present in the current snapshot.

### VT-1: DECSC/DECRC saves cursor visibility, shape and blink (measured)
- **Where:** `SavedCursor.cursor: Cursor` (`grid.rs:79-85`), restore at `grid.rs:958`, `grid.rs:1004`, alternate screen init `grid.rs:95-107`.
- **Repro:** `\e7\e[?25l\e8` shows the cursor again; `\e7\e[5 q\e8` restores a block; with a Beam default, bare `\e8` or `\e[?47h` gives a Block.
- **Fix:** save/restore only position (+ pen/charset/origin/autowrap as today); keep visibility/shape/blink live.

### VT-2: Resetting one mouse mode leaves another active (measured)
- **Where:** `dispatch.rs:151-157`.
- **Repro:** `\e[?1003h\e[?1000l` leaves motion tracking on (xterm/kitty/Ghostty turn tracking off).
- **Fix:** reset of any of 9/1000/1002/1003 sets tracking to none.

### VT-3: Mouse modes 1015/1016 accepted but not implemented (plausible)
- **Where:** `runtime/custom_backend.rs:1188-1192`, `mouse_protocol.rs`, DECRQM in `queries.rs:27-28`.
- **Problem:** 1016 encodes SGR with cell coordinates; 1015 encodes as X10. DECRQM says supported, so apps that probe (notcurses) misbehave.
- **Fix:** implement both, or reject them and report 0.

### VT-4: Legacy key encoder ignores modifiers and DECKPAM (measured)
- **Where:** `keyboard.rs:207-333`; `modes.application_keypad` (`custom_backend.rs:1166-1176`) is never read.
- **Repro:** Ctrl+Up → `\e[A` (xterm `\e[1;5A`), Shift+Right → `\e[C`, Ctrl+Delete → `\e[3~`, Shift+F5 → nothing, Insert → nothing.
- **Fix:** xterm modifier encoding (`CSI 1;m X`, `CSI n;m ~`), Insert `\e[2~`, keypad application mode.

### COR-7: Protocol reply during the drop grace period SIGKILLs the shell (measured)
- **Where:** `unix.rs:884-891`, `1340-1358` → `force_child_shutdown` (`1436-1443`).
- **Problem:** after `Drop`, the writer is closed; any reply gets BrokenPipe and the reader SIGKILLs the process group, skipping the SIGHUP grace (breaks shells' HUP traps). It also doesn't check `reader_alive`, so it may signal a reused pid/pgid.
- **Fix:** on BrokenPipe drop the reply; leave shutdown to `Drop`'s escalation and the watcher.

### COR-8: Hidden terminal's exit never wakes the host (measured)
- **Where:** exit closure in `runtime/custom_backend.rs:646-660` only calls `exit.notify()`, a no-op while wakeups are disabled.
- **Effect:** a background tab keeps showing "working" until some unrelated drain.
- **Fix:** `exit.notify_tab_chrome()`; also schedule maintenance after the exit commit.

### COR-9: Node `kill()` un-pauses output permanently (measured) — regression from LOW-24
- **Where:** `NativePty::kill` (`crates/pty/src/lib.rs:287`) calls `set_paused(false)` for every signal, including SIGWINCH/SIGUSR1/SIGSTOP.
- **Fix:** don't touch the paused state; let the exit be delivered once the child is gone even while paused.

### CPU-11: Hyperlink table rescans aren't amortized (measured)
- **Where:** `hyperlinks.rs:42-57`. `skip_rescans` is only set when a rescan frees nothing.
- **Measured:** ~65k live links + scrolling frees a few per line → 27.4 µs per OSC 8 (70× baseline).
- **Fix:** rescan only after a number of requests proportional to the table size.

### COR-17: wasm32 build broken by the `Cell` size assert (measured) — regression from MEM-6
- **Where:** `const _: () = assert!(… size_of::<Cell>() == 24)` in `terminal_engine/types.rs`.
- **Problem:** `Option<Arc<CellExtra>>` is 4 bytes on wasm32, so `Cell` is 20 B and the assert fails; `cargo check -p termy_wasm --target wasm32-unknown-unknown` errors with E0080.
- **Fix:** assert `size_of::<usize>() + 16`; add a wasm check to CI.

## Low / Low-Med

### COR-10: Restarting upload checked against the abandoned one (measured) — regression from MEM-8
- **Where:** `kitty_graphics.rs:437-474` size check/base64 decode use the old pending length and reply with the old command before the restart is detected (476-495).
- **Fix:** detect the restart first, then size-check against zero pending and reply with the new command.

### COR-11: Orphan cleanup frees named images (measured)
- **Where:** `placement.rs:178-182`.
- **Repro:** `a=T,i=1,p=1`; `a=T,i=2,P=1,Q=1`; `a=d,d=i,i=1` → `a=p,i=2` replies `ENOENT`.
- **Fix:** remove the loop; `release_unplaced_anonymous_images` covers anonymous images.

### COR-12: Re-sending an image relative to its own old placement (measured)
- **Where:** `commit_upload` checks the parent (`kitty_graphics.rs:1023`) before `remove_image` (1041) removes it. Replies OK but the placement is orphaned.
- **Fix:** validate the parent after removals, or reply `ENOPARENT`.

### COR-13: Replies report image number `I` as id `i` (measured)
- **Where:** `kitty_graphics.rs:1422-1425`. `a=p,I=5` → `\e_Gi=5,I=5;ENOENT…`; kitty sends `\e_GI=5;…`.

### COR-14: Exit status reported as 0 when `waitpid` fails (measured)
- **Where:** `wait_for_child` (`unix.rs:1415-1425`) returns 0 on ECHILD (host ignores SIGCHLD or reaps with `waitpid(-1)`).
- **Fix:** return `Option<c_int>`; report unknown exit.

### COR-15: Protocol replies can be reordered across a sync timeout (plausible)
- **Where:** `lock.rs:73-76` holds the replies of all slices until the read ends; `maintenance.rs:97-105` writes sync-timeout commit replies directly.
- **Fix:** queue each slice's replies to the transport at the end of that slice.

### COR-16: Hydrate into a live session drops the session's replies (plausible)
- **Where:** desktop `persistence.rs:725` hydrates after spawning; hydrate mode discards events/replies and acts as a second feeder.
- **Fix:** hydrate before the reader starts.

### MEM-10: History byte budget ignores `CellExtra` heap (measured)
- **Where:** `Row::heap_bytes` (`row.rs:136-145`). Combining-heavy rows: 141 MiB live vs 24 MiB budget.
- **Fix:** count `combining.capacity()` + the extra's allocation for extras stored in the row.

### CPU-12: Crossing the history budget packs all pending rows at once (measured)
- **Where:** `enforce_history_bytes` → `compact_pending_history(usize::MAX)` (`history.rs:39-53`). 6.1 ms spike about every 12k lines.
- **Fix:** pack the oldest pending rows in chunks until under budget.

### VT-5: Leaving 1049 leaves alternate-screen kitty placements (measured) — regression from LOW-8
- **Where:** `grid.rs:1009-1013`. Also keeps id-less images alive past MEM-7's release.
- **Fix:** emit `GridEffect::Clear { alternate: true, .. }` when freeing the alternate screen.

### VT-6: OSC 7 drops the cwd for a literal `%` (measured) — regression from LOW-13
- **Where:** `dispatch.rs:739-748`, `percent_decode` (`dispatch.rs:827`). Also emits non-`file://` URLs raw.
- **Fix:** fall back to the raw path on malformed escapes; ignore schemes other than `file`.

### VT-7: CUU/CUD margin limits differ from xterm (measured)
- **Where:** `grid.rs:597-603`. `\e[3;5r\e[9;1H\e[20A` → row 0 (xterm 2); `\e[3;5r\e[1;1H\e[20B` → row 9 (xterm 4).

### VT-8: Charset designation gaps (measured)
- **Where:** `dispatch.rs:496-497`, `934-944`. Designations other than `0`/`B` are ignored (`\e(0\e(Aq` still prints `─`); DEC graphics `_` should be blank.

### VT-9: DECRC restores the OSC 8 hyperlink (measured)
- **Where:** `grid.rs:961`. Same across 1049 exit.

### VT-10: OSC 12 query gets no reply when no cursor color is set (measured)
- **Where:** `dispatch.rs:373-377`. xterm falls back to the foreground.

### VT-11: DECRQM mismatches for `?3` and `?1048` (measured)
- **Where:** `queries.rs:10-32`.

### VT-12: Tab stops overwritten when a resize gives columns back (measured)
- **Where:** `grid.rs:1196-1200`.

### LOW-25: Env filter gaps (plausible)
- `transport/mod.rs:168-203`: `COLORTERM`, `LC_TERMINAL`, `LC_TERMINAL_VERSION`, `KITTY_INSTALLATION_DIR`, `WEZTERM_EXECUTABLE`/`WEZTERM_CONFIG_*` leak; Windows matching is case-sensitive.

### LOW-26: EMFILE in the child looks like a successful spawn (plausible)
- `child.rs:110-114`, `unix.rs:728-733`: exit 126 without a status record.

### LOW-27: `LockWaiters` count isn't released by a guard
- `lock.rs:19-24`. A panic leaves the count raised and the feeder yields 2 ms per slice forever.

### LOW-28: Local `TerminalDamageCells::visit` never returns false
- `viewport.rs:91-94` clamps instead of rejecting out-of-viewport spans, against the documented contract.

### LOW-29: Signals around fork
- Block signals in the parent around `fork`; reset every signal disposition, not just 7 (`SIG_IGN` on SIGTSTP/SIGTTIN/SIGTTOU/SIGXFSZ survives exec).

### LOW-30: Writer poll ignores POLLHUP/POLLERR; wake writes while idle
- `master.rs:4-46`; `wake()` writes even when the writer sits in `recv`.

### LOW-31: Node 1 MiB write chunking can split a paste
- `packages/pty/src/terminal.ts`: a reply can land inside a >1 MiB bracketed paste (undoes COR-1). Also reset `Output::flush`'s `scheduled` flag on failure.

### LOW-32: Allocation churn in links
- `hyperlinks.rs:33` allocates two `String`s per lookup; `dispatch.rs:779` rebuilds `Arc<CellExtra>` when reopening the pen's link; `types.rs:178-190` allocates a spacer extra per wide glyph.

### LOW-33: Grid fuzz never reaches packing
- The in-repo fuzz runs at 13 columns, below the `cols < 16` cutoff for `Row::retain`. Add a 16+ column run.

## Protocol gaps (not bugs)
- **PROTO-1:** XTVERSION (`CSI > q`) gets no reply.
- **PROTO-2:** DECRQM `?2027` (grapheme clustering) should report permanently set.
- **PROTO-3:** OSC 4/10/11/12 replies should echo the query's terminator (BEL vs ST).
- **PROTO-4:** XTGETTCAP lacks `Smulx`, `Setulc`, `Ms`.
- **PROTO-5:** DECSCNM (`?5`, reverse-video screen) is unsupported.
- **PROTO-6:** DECALN (`ESC # 8`) is unsupported.
- **PROTO-7:** DECSLRM/DECLRMM (`?69`, left/right margins) is unsupported.
- **PROTO-8:** `CSI 0 SP q` sets blink on, while the startup cursor is steady; 0 should restore the configured default.
- **PROTO-9:** `?1049h` while already on the alternate screen via 47 neither saves the cursor nor clears (xterm does both).
- **PROTO-10:** X10 mode 9 reports button releases (X10 reports presses only).
- **CI-1:** CI never builds wasm32 except at npm publish (how COR-17 slipped through).

## Verified OK (don't re-audit)
In addition to v1's list:
- `history_bytes` bookkeeping on push/pop/clear/trim/compaction/resize/reflow/alt screen (invariant + saturation fuzz).
- `Row::reflow_in_place` matches the logical-line path (differential fuzz, 400 seeds).
- `write_ascii` and REP ASCII match per-char writes, incl. DECAWM toggles and pending wrap.
- SGR 58 survives pack/unpack, reflow, spacers; hyperlinks can't be evicted while referenced.
- Lock order `state` → `sync_signal`; feeder yield is bounded; hydration replays stay atomic; slicing can't expose half a sync frame.
- Fork child path: no allocation/locks between fork and exec; `setsid` before `TIOCSCTTY`; exec failure reported.
- Motion half-backlog counters can't drift; protocol replies can't starve (worst case one write entry).
- kitty quota counters can't drift or go negative; prepare/commit can't collide ids.
- COR-3 recovery can't truncate sixel/kitty/OSC 52; LOW-10 sniffing works across chunks.
- RIS/DECSTR reset sets are complete.
