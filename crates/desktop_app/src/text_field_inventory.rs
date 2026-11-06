//! App-wide inventory of editable text fields.
//!
//! The terminal PTY is not a text box. Every other place a person types is
//! listed here with the editor that owns it, so a caret gap cannot hide in
//! an unnamed field.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// GPUI `TextInputState` / `TextInputElement`: click uses the shaped
    /// line, arrows and Home/End edit, Shift extends, clipboard replaces.
    NativeTextInput,
    /// Tab rename, workspace rename, terminal find, command palette.
    NativeInlineInput,
    /// X panel draft editor, including soft-wrapped compose/reply/quote.
    DraftEditor,
    /// Same caret model, painted as one bullet per grapheme. The secret
    /// is not drawn or logged. An explicit copy puts the selection on the
    /// clipboard because that is the edit the person asked for.
    MaskedCaret,
    /// Not free text. Documented so it is not mistaken for a missed box.
    NotATextBox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldRecord {
    pub id: &'static str,
    pub surface: &'static str,
    pub kind: FieldKind,
    pub notes: &'static str,
}

pub fn inventory() -> &'static [FieldRecord] {
    &INVENTORY
}

const INVENTORY: &[FieldRecord] = &[
    FieldRecord {
        id: "settings.text",
        surface: "Settings",
        kind: FieldKind::NativeTextInput,
        notes: "Font, shell, title formats, and the other free-text settings. Click, Shift+arrows, Home/End (line), Up/Down across hard lines, clipboard. Long lines pan instead of soft-wrapping.",
    },
    FieldRecord {
        id: "settings.search",
        surface: "Settings sidebar search",
        kind: FieldKind::NativeTextInput,
        notes: "Single-line search. Same native caret as settings text.",
    },
    FieldRecord {
        id: "settings.ssh",
        surface: "Settings SSH form",
        kind: FieldKind::NativeTextInput,
        notes: "Display name, host, port, username, identity file.",
    },
    FieldRecord {
        id: "settings.ssh.secret",
        surface: "Settings SSH secret",
        kind: FieldKind::MaskedCaret,
        notes: "One bullet per grapheme. Hit testing maps the bullet back to the real offset. The secret is not drawn.",
    },
    FieldRecord {
        id: "settings.plugin",
        surface: "Settings plugin values",
        kind: FieldKind::NativeTextInput,
        notes: "Plugin setting strings. Enter commits. Shift+arrows select.",
    },
    FieldRecord {
        id: "plugin.text",
        surface: "Plugin UI text input",
        kind: FieldKind::NativeTextInput,
        notes: "Single-line and multiline plugin controls. Enter submits or inserts a newline in multiline. Up/Down follow hard lines.",
    },
    FieldRecord {
        id: "plugin.list-filter",
        surface: "Plugin UI list filter",
        kind: FieldKind::NativeTextInput,
        notes: "Focusing the filter promotes it to the same text input. Up/Down still move the highlighted row. Left/Right/Home/End/Backspace edit the query.",
    },
    FieldRecord {
        id: "inline.rename-tab",
        surface: "Tab rename",
        kind: FieldKind::NativeInlineInput,
        notes: "Click, Shift+arrows, Home/End, clipboard. Up/Down move across hard lines when a newline is present.",
    },
    FieldRecord {
        id: "inline.rename-workspace",
        surface: "Workspace rename",
        kind: FieldKind::NativeInlineInput,
        notes: "Same inline editor as tab rename.",
    },
    FieldRecord {
        id: "inline.terminal-search",
        surface: "Terminal find",
        kind: FieldKind::NativeInlineInput,
        notes: "Search field over the terminal. It does not replace PTY input.",
    },
    FieldRecord {
        id: "inline.command-palette",
        surface: "Command palette",
        kind: FieldKind::NativeInlineInput,
        notes: "Palette query, including plugin prompts inside the palette.",
    },
    FieldRecord {
        id: "x.compose",
        surface: "Termy X compose",
        kind: FieldKind::DraftEditor,
        notes: "Soft-wrapped. Enter inserts a newline. Publish still confirms before any post.",
    },
    FieldRecord {
        id: "x.reply",
        surface: "Termy X reply",
        kind: FieldKind::DraftEditor,
        notes: "Soft-wrapped. Enter inserts a newline and does not post.",
    },
    FieldRecord {
        id: "x.quote",
        surface: "Termy X quote",
        kind: FieldKind::DraftEditor,
        notes: "Same editor as reply.",
    },
    FieldRecord {
        id: "x.search",
        surface: "Termy X search",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter runs search.",
    },
    FieldRecord {
        id: "x.trends",
        surface: "Termy X trends filter",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter searches.",
    },
    FieldRecord {
        id: "x.research",
        surface: "Termy X research",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter runs research.",
    },
    FieldRecord {
        id: "x.lookup",
        surface: "Termy X lookup",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter looks up the handle.",
    },
    FieldRecord {
        id: "x.feed-query",
        surface: "Termy X feed topic",
        kind: FieldKind::DraftEditor,
        notes: "Single line.",
    },
    FieldRecord {
        id: "x.feed-filter",
        surface: "Termy X feed filter",
        kind: FieldKind::DraftEditor,
        notes: "Single line.",
    },
    FieldRecord {
        id: "x.queue-text",
        surface: "Termy X queue draft",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter queues; it does not send.",
    },
    FieldRecord {
        id: "x.queue-reply",
        surface: "Termy X queue reply target",
        kind: FieldKind::DraftEditor,
        notes: "Single line.",
    },
    FieldRecord {
        id: "x.analyze",
        surface: "Termy X analyze handle",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter runs the read.",
    },
    FieldRecord {
        id: "x.reply-guy-topic",
        surface: "Reply Guy topic",
        kind: FieldKind::DraftEditor,
        notes: "Single line. Enter fetches. Double-click selects all.",
    },
    FieldRecord {
        id: "x.reply-guy-key",
        surface: "Reply Guy Treg/Grok key",
        kind: FieldKind::MaskedCaret,
        notes: "Bullets scroll inside the fixed field so a long key cannot push Save off screen. Enter and paste still save. The secret is not logged.",
    },
    FieldRecord {
        id: "x.bearer",
        surface: "X developer key",
        kind: FieldKind::MaskedCaret,
        notes: "Enter saves, Esc cancels. Caret edits the real string through the bullet row.",
    },
    FieldRecord {
        id: "terminal.pty",
        surface: "Terminal shell",
        kind: FieldKind::NotATextBox,
        notes: "Upstream PTY. Arrow keys belong to the shell unless a text field above is focused.",
    },
    FieldRecord {
        id: "prompt.buttons",
        surface: "Linux modal prompt",
        kind: FieldKind::NotATextBox,
        notes: "Button dialog. It has no text field.",
    },
    FieldRecord {
        id: "settings.keybind-capture",
        surface: "Settings keybind capture",
        kind: FieldKind::NotATextBox,
        notes: "Records one shortcut chord. It is not a free-text editor.",
    },
    FieldRecord {
        id: "x.confirm",
        surface: "Termy X confirm",
        kind: FieldKind::NotATextBox,
        notes: "Confirm buttons. Nothing posts until the person confirms.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_named_once_and_end_only_masks_are_gone() {
        let mut seen = std::collections::BTreeSet::new();
        for field in inventory() {
            assert!(seen.insert(field.id), "duplicate field {}", field.id);
            assert!(!field.notes.is_empty(), "{}", field.id);
            assert!(
                !field.notes.to_ascii_lowercase().contains("end-only"),
                "{} still documents an end-only caret",
                field.id
            );
        }
        for required in [
            "settings.text",
            "settings.ssh.secret",
            "plugin.list-filter",
            "inline.rename-tab",
            "inline.terminal-search",
            "inline.command-palette",
            "x.compose",
            "x.reply",
            "x.quote",
            "x.reply-guy-topic",
            "x.reply-guy-key",
            "x.bearer",
            "terminal.pty",
        ] {
            assert!(
                inventory().iter().any(|field| field.id == required),
                "missing {required}"
            );
        }
        assert!(inventory().iter().any(|field| {
            field.id == "x.reply-guy-key" && field.kind == FieldKind::MaskedCaret
        }));
        assert!(
            inventory()
                .iter()
                .any(|field| field.id == "terminal.pty" && field.kind == FieldKind::NotATextBox)
        );
    }
}
