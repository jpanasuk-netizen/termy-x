# Termy XL master to-do (from Jeremy, 2026-11-12)

Goal: clean up everything and make every item work correctly, verified. Tick an item only when a headless test covers it.

Hard rules: Termy X binary, shortcuts, and `%APPDATA%\termy` are never modified. Nothing posts except a card's Send button. No paid official X API. No test posts. Secrets are never logged or committed. Must compile and pass tests on `x86_64-pc-windows-msvc`. CI stays green, including file-size limits.

Evidence command: `cargo test -p termy_x --lib trends_desk::`

## A. Window, dock, launch

- [x] 1. Single instance. A second launch focuses the existing window. `focus_steps` restores a minimized window, then raises it.
- [x] 2. Opens straight to the locked width (687 logical px), with no step-growth animation. `expand_locked` and `snap_tracked` ignore a saved width.
- [x] 3. The slim XL tab sits on Termy X's right edge and the placement math follows a move, a resize, and a minimize. `follow_tracks_a_moved_termy_window`. A live HWND on a Windows desktop was not exercised here.
- [x] 4. Expanded placement stays on that same edge at 687. Collapsed placement is the tab width. Same test.
- [x] 5. A missing or minimized Termy X uses the standalone right half. Clearing minimized reattaches. Same test.
- [x] 6. Docked state round-trips in settings, and the open width stays 687. `dock_state_round_trips_in_settings`.
- [x] 7. The placed window is clamped inside 1920, 1536, and 1280 work areas. Same follow test. Pixel clipping on a real 125% display was not viewed.
- [x] 8. Exit clears the refresh, follow, daily, and Hermes timers, the close hook, and the child process. `exit_clears_timers_hooks_and_the_child_process`.
- [x] 9. The install script points shortcuts at `trends_desk.exe`, refuses `Termy.lnk` / `Termy X.lnk`, and leaves `termy-7154b8a5.exe` shortcuts alone. `install_script_refuses_termy_x_and_checks_hashes`.

## B. Subjects, pages, search

- [x] 10. Each subject has its own page. `subject_pages_stay_in_bounds_and_map_by_id`.
- [x] 11. Page turns stop at the first and last page. `turn_page` in that test.
- [x] 12. Position label, for example `2 / 3 · Finance`. `page_label` in that test.
- [x] 13. Search returns the column's posts, and empty or error searches are not fresh. `search_json_does_not_post_and_queue_only_writes_a_draft`, `empty_and_error_do_not_write_and_are_not_fresh`.
- [x] 14. Auto-refresh is allowed while a model is halted, and new ids are not duplicates. `should_auto_refresh`, `new_ids`.
- [x] 15. Pressing Refresh starts one search and does not post. A second press while busy does not start another. `pressing_refresh_starts_one_search_and_does_not_post`. The button calls `press_refresh`.
- [x] 16. A missing OpenCLI search is an error and does not invent posts. `missing_opencli_is_an_error_without_fixture_posts`.
- [x] 17. Cards are de-duplicated by id. `new_ids`.

## C. Gold cards

- [x] 18. Take cards lay out in more columns as the window widens. `each_take_is_its_own_card_inside_the_window`.
- [x] 19. Cards stay inside the window and long tokens wrap. `wrap_token` in that test.
- [x] 20. A scroll moves the card and the following click still selects it. `a_scroll_does_not_swallow_the_following_click`. Offsets stay per card (`scroll_offsets_are_per_card`).
- [x] 21. One card is selected at a time. Empty space clears it. Selected colors are pink `0xff4fa3` and cyan `0x7af6ff`. `click_selects_one_card_and_reply_stays_a_draft_until_send`.
- [x] 22. Keyboard selection moves one card and does not send. `selection_moves_one_card_and_does_not_send`.
- [x] 23. Send is a separate explicit step from the draft. `editor_send` in the click test.
- [x] 24. Media URLs stay on the card. `card_keeps_author_text_media_metrics_time_and_link`. Opening the browser was not automated.
- [x] 25. Author, handle, time, metrics, and the status link stay on the card. Same test.

## D. Reply, Edit, Draft pop-out editor

- [x] 26. Reply opens an editor for that card id. The pop-out border uses the same pink as the selection. Click test.
- [x] 27. The editor carries the draft, three variants, and a provider. Click test and `read_composer`.
- [x] 28. The three variants differ on the model path and the fallback path. `variants_are_pairwise_different_on_the_model_and_fallback_paths`, `hermes_takes_are_three_distinct_lines_and_402_stays_a_fallback`.
- [x] 29. Variant index wraps. `flip_variant` in `tone_reaches_free_variants_and_treg_errors_stay_errors`.
- [x] 30. Character counting uses the X weighted rule, and a URL weighs 23. `a_url_of_any_length_weighs_23` in `text`. The compose card uses `char_count`.
- [x] 31. Closing the editor keeps the draft and does not send. `closing_the_editor_keeps_the_draft_and_does_not_send`.
- [x] 32. Send requires an explicit click. A failed send stays failed. `typing_and_a_failed_send_do_not_drop_text_or_auto_post`.
- [x] 33. A pending send is busy, so a second send is refused. `send_is_busy` in the click test.
- [x] 34. Only an explicit queue writes a draft row. An empty queue does not. `search_json_does_not_post_and_queue_only_writes_a_draft`.

## E. Providers (takes and drafts)

- [x] 35. A 401 or 402 is recognized as a hard stop for the free model. `blocks_free_model` is what the take loop uses after one retry. The desk logs a failure once. `should_log_model_failure`.
- [x] 36. A Treg failure stays an error and is not replaced with a template. `provider_selection_does_not_hide_a_treg_failure`.
- [x] 37. Hermes defaults to `http://127.0.0.1:8642/v1` and model `hermes-agent`. The key prefers the saved value, otherwise `API_SERVER_KEY`. `/health` and `/v1/models` status lines are tested. `hermes_model_defaults_and_a_bare_vyce_name_is_sent_with_provider`, `hermes_key_prefers_the_saved_value_and_stays_out_of_errors`.
- [x] 38. `desk.provider` is written by `save_take_provider` into the Termy XL config directory. The takes test saves `hermes` there. The store refuses the Termy X config tree. `isolated_store_refuses_the_termy_x_config_tree`.
- [x] 39. Status lines cover up, missing key (401), credits (402), and unreachable. `hermes_models_line` and `hermes_pill`.
- [x] 40. A server that accepts and never answers is called on a worker. The caller returns immediately and the worker reports `timed out`. `a_hung_server_does_not_block_the_caller`.

## F. Agent panes (Grok, Hermes, Agy, Muse)

- [x] 41. Defaults are `grok`, `hermes`, `agy`, `muse`. A missing binary is one line. `x_toml_wins_and_a_missing_binary_is_one_line`.
- [x] 42. Drafting returns three distinct variants. `variants_are_pairwise_different_on_the_model_and_fallback_paths`.
- [x] 43. Use-as-draft copies typed text, then later agent output. `the_pane_can_take_typed_text_without_posting`.
- [x] 44. One or several open agent panes do not produce a quit dialog. `busy_agent_panes_do_not_open_a_quit_dialog`. Closing the window runs the same shutdown as item 8.

## G. termy-xcli for Grok Bot and Hermes

- [x] 45. `desk search`, `desk takes`, `desk drafts list|save`, and `desk queue-reply` return JSON and do not post. `search_json_does_not_post_and_queue_only_writes_a_draft`, `hermes_takes_are_three_distinct_lines_and_402_stays_a_fallback`.
- [x] 46. The JSON always includes `posted: false`. Commands and examples are in `docs/termy-xl-agents.md`.
- [x] 47. The Drafts page re-reads `%APPDATA%\Termy XL` drafts on each paint, so a CLI queue shows up on the next frame. The file write is the queue test.

## H. Settings, data, logs

- [x] 48. Settings and the takes provider write under the Termy XL directory. The store refuses `%APPDATA%\termy`. `isolated_store_refuses_the_termy_x_config_tree`.
- [x] 49. Log lines are redacted and the file rotates. `secrets_are_stripped_and_the_file_rotates`.
- [x] 50. Empty settings JSON loads the defaults. `dock_state_round_trips_in_settings`.
- [x] 51. The installer keeps the newest `*.bak` and the newest `backup-*` folder, deletes the older ones, and logs `removed`. `prune_keeps_the_newest_bak_and_backup_folder`. The data folder is described in `docs/termy-xl-agents.md`.
- [x] 52. `scripts/check-desk-clippy.sh` runs `cargo clippy -p termy_x --lib` and `-p termy --lib` with `-D warnings` and no allow-list, and fails if a desk file is named.
- [x] 53. Desk sources stay at or under 1500 lines. `scripts/check-file-sizes.sh`.
- [x] 54. Headless tests cover selection, the editor, distinct variants, fake providers, xcli JSON, and dock math. This file lists them.
- [x] 55. The install script refuses Termy X shortcuts, hashes before copy, and writes a `.bak`. `install_script_refuses_termy_x_and_checks_hashes`.
- [x] 56. This file is the checklist. `docs/termy-xl-agents.md` is the agent skill.

## Windows-only, not ticked as visually verified

- The tab actually glued to a moving Termy X window, including DPI and a second monitor.
- The pink card and pink compose card as drawn by GPUI.
- Shortcut icons on the Desktop and Start Menu after the install script runs.
