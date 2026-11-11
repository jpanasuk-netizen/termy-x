//! Trends Desk domain for Termy X.
//!
//! Live posts come from OpenCLI only. The desktop window renders them.
//! A failed fetch keeps the last saved report labeled cached and does not
//! write a fixture success. Publishing stays on the existing confirm path.

mod cards;
mod domain;
mod fetch;
mod pack;
mod parse;
mod store;

pub use cards::{
    COMPANION_HEIGHT, COMPANION_WIDTH, DOCK_WIDTH, DeskFrame, EditPopout, ReplyPhase, SubjectPage,
    TakeDraft, TakeFrame, TakeProvider, TweetCard, WINDOW_PAD, apply_send_result, card_columns,
    card_from_post, char_count, close_editor, collapse_dock, commit_send, draft_take, edit_reply,
    editor_send, expand_right, expand_to_full, frame_from_parts, layout_takes, move_selection,
    open_editor, open_width, page_index, page_label, remember_width, request_send, scroll_by,
    scroll_of, subject_pages, toggle_dock, tone_variants, turn_page, wrap_token,
};
pub use domain::{
    ChangeTag, ColumnView, DeskSettings, GPU_NOTE, IMAGE_QUOTE_NOTE, Nav, OutcomeKind, Phase,
    RunOutcome, SourcePost, StoredReport, Surface, Topic, VISIBLE_NAME, chicago_stamp,
    column_count_for_width, column_from_cache, column_from_outcome, column_loading, daily_due,
    default_topics, diff_urls, draft_text_from_title, filter_posts, fixture_column, outcome_error,
    reply_dry_run_args, search_query, slug_id, surface_after_nav, topic_id_ok,
};
pub use fetch::{
    DraftPrep, InspectedPost, InspectedReply, OpenCliDesk, SEARCH_TIMEOUT, SessionState,
    SessionStatus, handoff_draft_to_termy_x, post_reply_opencli, prepare_draft, prepare_draft_file,
    refuse_unsafe_cli_name, run_topics, search_args,
};
pub use pack::{
    CardFilter, ComposerRead, DeskKey, HitItem, PackFile, ReplyMark, TakeTone, apply_filters,
    desk_key, flip_variant, heat_line, heat_score, hud_line, load_pack, new_ids, provider_drafts,
    read_composer, refresh_due, refresh_is_search_only, replied_badge, save_pack, sends_on_key,
    sort_by_heat, three_variants,
};
pub use parse::{
    ParseError, as_post_list, parse_json_blob, posts_from_stdout, unwrap_opencli_data,
};
pub use store::{
    DATA_DIR_ENV, DATA_FOLDER, DIR_NAME, DeskStore, drafts_file, termy_xl_data_root,
    within_termy_x_config,
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
}

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::config::XConfig;
use crate::io::CommandRunner;

pub fn isolated_opencli_config(data_root: &std::path::Path) -> XConfig {
    XConfig {
        config_dir: data_root.to_path_buf(),
        official_enabled: false,
        ..XConfig::default()
    }
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
