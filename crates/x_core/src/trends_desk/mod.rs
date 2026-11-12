//! Trends Desk domain for Termy X.
//!
//! Live posts come from OpenCLI only. The desktop window renders them.
//! A failed fetch keeps the last saved report labeled cached and does not
//! write a fixture success. Publishing stays on the existing confirm path.

mod agents;
mod cards;
mod desk_cmd;
mod domain;
mod fetch;
mod lifecycle;
mod log;
mod media;
mod pack;
mod parse;
mod snap;
mod store;
mod takes;

pub use agents::{
    AgentPane, DraftRoute, HERMES_BASE, HERMES_HEALTH_PATH, HERMES_MODEL, HermesAuth, Launch,
    PromptCli, api_server_key_from_env_file, choose_hermes_key, default_command,
    draft_hermes_variants, execute_draft, hermes_auth, hermes_body, hermes_health_request,
    hermes_health_url, hermes_models_line, hermes_models_request, hermes_pill, known_hermes_exe,
    missing_line, plan_agent_draft, plan_hermes_draft, redact_secret, reply_prompt, resolve_launch,
    seed_agent_block, variants_from_draft, write_hermes_settings,
};
pub use cards::{
    COMPANION_HEIGHT, COMPANION_WIDTH, DOCK_WIDTH, DeskFrame, EditPopout, LOCKED_OPEN_WIDTH,
    ReplyPhase, SELECT_CYAN, SELECT_PINK, SubjectPage, TAKE_CARD_WIDTH, TakeDraft, TakeFrame,
    TakeProvider, TweetCard, WINDOW_PAD, apply_send_result, card_columns, card_from_post,
    char_count, close_editor, collapse_dock, commit_send, draft_take, edit_reply, editor_send,
    expand_locked, expand_right, expand_to_full, frame_from_parts, layout_takes, move_selection,
    offline_take, open_editor, open_width, page_index, page_label, remember_width, request_send,
    scroll_by, scroll_of, select_take, selection_after_card, selection_after_empty,
    selection_colors, send_is_busy, short_take_note, subject_pages, take_column_count,
    takes_for_posts, toggle_dock, tone_variants, turn_page, wrap_token,
};
pub use desk_cmd::{
    find_cached_post, list_drafts, queue_reply, save_draft, search_subject, takes_for_tweet,
    topic_by_subject,
};
pub use domain::{
    ChangeTag, ColumnView, DeskSettings, GPU_NOTE, IMAGE_QUOTE_NOTE, Nav, OutcomeKind, Phase,
    RunOutcome, SourcePost, StoredReport, Surface, Topic, VISIBLE_NAME, chicago_stamp,
    column_count_for_width, column_from_cache, column_from_outcome, column_loading,
    column_loading_keep, daily_due, default_topics, diff_urls, draft_text_from_title, filter_posts,
    fixture_column, idea_from, outcome_error, reply_dry_run_args, search_query, shift_index,
    slug_id, surface_after_nav, topic_id_ok,
};
pub use fetch::{
    DraftPrep, InspectedPost, InspectedReply, OpenCliDesk, SEARCH_TIMEOUT, SessionState,
    SessionStatus, handoff_draft_to_termy_x, post_reply_opencli, prepare_draft, prepare_draft_file,
    refuse_unsafe_cli_name, run_topics, search_args,
};
pub use lifecycle::{
    DeskSession, EscapeEffect, OffUiRequest, PruneLog, RefreshPress, child_is_gone,
    click_after_scroll, escape_effect, escape_requests_quit, focus_loss_closes_the_window,
    press_refresh, prune_backups, quit_dialog_for_agents, reap_child, start_off_ui,
};
pub use log::{
    append_desk_log, exit_log_line, exit_reason, install_panic_log, note_exit_reason,
    panic_log_line, redact_log, remember_exit, should_log_model_failure,
};
pub use media::{
    CLOSE_HIT, CLOSE_ICON, MediaClick, QuotedTweet, TileBox, cache_name, cached_file,
    card_click_drafts, click_drafts_a_card, close_hit_box, grid_inside, items_from_post,
    media_cache_dir, media_click, play_badge, player_origin, preview_url, quoted_from_post,
    read_cached, tile_grid, write_cached,
};
pub use pack::{
    CardFilter, ComposerRead, DeskKey, HitItem, PAGE_BAR_H, PAGE_BAR_HIT, PackFile, ReplyMark,
    SearchEdit, TakeTone, apply_filters, desk_key, distinct_variants, flip_variant, format_clock,
    format_post_time, heat_badge, heat_line, heat_score, hud_line, load_pack, local_offset_secs,
    new_ids, next_click_reaches, next_refresh_text, page_after_key, provider_drafts, read_composer,
    refresh_due, refresh_is_search_only, replied_badge, save_pack, search_edit, search_label,
    search_page_name, sends_on_key, should_auto_refresh, sort_by_heat,
};
pub use parse::{
    ParseError, as_post_list, parse_json_blob, posts_from_stdout, unwrap_opencli_data,
};
pub use snap::{
    SeenWindow, Snap, SnapSource, WindowRect, WriteGate, XlArea, area_gate, attach_plan,
    clamp_outer, is_termy_x_title, pick_termy_host, snap_beside, snap_tracked, termy_areas,
};
pub use store::{
    DATA_DIR_ENV, DATA_FOLDER, DIR_NAME, DeskStore, drafts_file, termy_xl_data_root,
    within_termy_x_config,
};
pub use takes::{
    CLICK_TO_DRAFT, DeskEvent, HONEST_PREFIX, TAKE_CHAIN, accept_lines, admit_grok, agent_work_dir,
    agy_credit_error, apply_hidden_command, background_grok_spawn, chain_order, choose_chain,
    halt_takes, honest_failure, interactive_command, is_provider_failure, is_template_angle,
    is_termy_x_image, merge_attempts, muse_needs_trust, muse_trust_line, resume_takes,
    retain_grok_jobs, run_take_chain, settle_variants, take_spawns, takes_halted, with_grok_stub,
};

const SYNTHETIC_OPENCLI: &str = include_str!("synthetic_opencli.json");

/// Clearly labeled sample rows for the `--fixture` window. Not a live fetch.
pub fn synthetic_fixture_posts() -> Vec<SourcePost> {
    posts_from_stdout(SYNTHETIC_OPENCLI).unwrap_or_default()
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    #[test]
    fn synthetic_fixture_is_labeled_and_not_a_live_phase() {
        let posts = synthetic_fixture_posts();
        assert!(posts.len() >= 2);
        assert!(
            posts
                .iter()
                .all(|post| post.text.contains("SYNTHETIC FIXTURE"))
        );
        let view = fixture_column(&Topic::law(), posts);
        assert_eq!(view.phase, Phase::Fixture);
        assert_ne!(view.phase, Phase::Fresh);
        assert!(view.banner.contains("not a live fetch"));
    }

    #[test]
    fn xl_reads_termy_x_ai_settings_without_taking_its_directory() {
        let mut xl = XConfig::default();
        xl.config_dir = std::path::PathBuf::from("/xl");
        let mut termy = XConfig::default();
        termy.config_dir = std::path::PathBuf::from("/termy");
        termy.ai_base_url = "https://vyce.example/v1".into();
        termy.ai_model = "glm-5.3-flash".into();
        termy.ai_api_key = "secret-key".into();
        termy.ai_explicit = true;
        copy_ai_if_unset(&mut xl, &termy);
        assert_eq!(xl.ai_endpoint().0, "https://vyce.example/v1");
        assert_eq!(xl.ai_endpoint().1, "glm-5.3-flash");
        assert_eq!(xl.ai_api_key, "secret-key");
        assert_eq!(xl.config_dir, std::path::PathBuf::from("/xl"));
        xl.ai_explicit = true;
        xl.ai_base_url = "http://xl-local/v1".into();
        copy_ai_if_unset(&mut xl, &termy);
        assert_eq!(xl.ai_endpoint().0, "http://xl-local/v1");
    }
}

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::config::XConfig;
use crate::io::CommandRunner;

pub fn isolated_opencli_config(data_root: &std::path::Path) -> XConfig {
    let mut config = XConfig::load(data_root);
    config.official_enabled = false;
    if !config.ai_explicit {
        let shared = XConfig::load(&crate::config::default_config_dir());
        copy_ai_if_unset(&mut config, &shared);
    }
    config.config_dir = data_root.to_path_buf();
    config
}

/// Read Termy X's draft endpoint into XL when XL has none of its own.
/// Does not change `config_dir` and does not write the Termy X file.
pub fn copy_ai_if_unset(into: &mut XConfig, from: &XConfig) {
    if into.ai_explicit || !from.ai_explicit {
        return;
    }
    into.ai_base_url = from.ai_base_url.clone();
    into.ai_model = from.ai_model.clone();
    into.ai_key_env = from.ai_key_env.clone();
    if into.ai_api_key.trim().is_empty() {
        into.ai_api_key = from.ai_api_key.clone();
    }
    into.ai_explicit = true;
}

pub fn run_saved_topic(
    runner: Arc<dyn CommandRunner>,
    topic_id: &str,
    focus: Option<&str>,
    data_root: &std::path::Path,
) -> Result<Vec<RunOutcome>, String> {
    let store = DeskStore::open_isolated(data_root)?;
    let config = isolated_opencli_config(data_root);
    let mut settings = store.load_settings();
    if let Some(focus) = focus.map(str::trim).filter(|value| !value.is_empty()) {
        settings.focus = focus.to_string();
        store.save_settings(&settings)?;
    }
    let topics = store.load_topics();
    let selected: Vec<Topic> = if topic_id == "all" {
        topics
    } else {
        topics
            .into_iter()
            .filter(|topic| topic.id == topic_id)
            .collect()
    };
    if selected.is_empty() {
        return Err(format!("unknown column `{topic_id}`"));
    }
    let desk = OpenCliDesk::new(&config, runner);
    let cancel = AtomicBool::new(false);
    let outcomes = run_topics(&desk, &selected, &settings, &cancel);
    let now = crate::time::unix_now();
    for outcome in &outcomes {
        if outcome.kind.writes_report() {
            store.write_fresh(outcome, now)?;
        }
    }
    Ok(outcomes)
}

pub fn format_outcomes(outcomes: &[RunOutcome]) -> String {
    let mut text = String::new();
    for outcome in outcomes {
        text.push_str(&format!(
            "{}  {}  wrote={}\n",
            outcome.topic_id,
            outcome.kind.as_str(),
            outcome.kind.writes_report()
        ));
        if outcome.kind == OutcomeKind::Fresh {
            text.push_str(&outcome.markdown);
        } else {
            text.push_str(&outcome.message);
            text.push('\n');
        }
        text.push('\n');
    }
    text
}
