//! One inventory of what this Termy X build actually contains.
//! Windows worktrees that were never pushed (`termyx-wt-fixes` chrome.rs,
//! `tv-all` staged) are not in this tree and are not listed as shipped.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureState {
    /// In this branch and covered by a unit test or an existing control.
    InThisBuild,
    /// Named so it is not mistaken for a missing merge.
    NotASeparateApp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    pub id: &'static str,
    pub state: FeatureState,
    pub note: &'static str,
}

pub fn features() -> &'static [Feature] {
    &FEATURES
}

const FEATURES: &[Feature] = &[
    Feature {
        id: "caret",
        state: FeatureState::InThisBuild,
        note: "Every editable text field, including masked keys. The terminal PTY is not a text box.",
    },
    Feature {
        id: "reply-guy",
        state: FeatureState::InThisBuild,
        note: "Topic Fetch, Popular/Newest re-rank, Trends, Grok drafts, saved Treg/Grok keys. One Treg page, max 50, no cursor.",
    },
    Feature {
        id: "home-pages",
        state: FeatureState::InThisBuild,
        note: "For you and Following load one 20-post page. Load more continues with the cursor. No fixture swap when a live read fails.",
    },
    Feature {
        id: "follow",
        state: FeatureState::InThisBuild,
        note: "Follow and Unfollow on profile and post authors. Confirm first. OpenCLI only. Nothing is sent in tests.",
    },
    Feature {
        id: "compose-confirm",
        state: FeatureState::InThisBuild,
        note: "Compose, reply, and quote still confirm before a post.",
    },
    Feature {
        id: "layout",
        state: FeatureState::InThisBuild,
        note: "Side panel, quit guard, and the existing window live in this same binary.",
    },
    Feature {
        id: "one-binary",
        state: FeatureState::NotASeparateApp,
        note: "One termy.exe. No second shortcut and no preview app.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_lists_one_line_of_features() {
        let mut ids = std::collections::BTreeSet::new();
        for feature in features() {
            assert!(ids.insert(feature.id), "duplicate {}", feature.id);
            assert!(!feature.note.is_empty());
        }
        for required in ["caret", "reply-guy", "home-pages", "follow", "one-binary"] {
            assert!(features().iter().any(|feature| feature.id == required));
        }
    }
}
