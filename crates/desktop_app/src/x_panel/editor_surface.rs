//! Shaped draft editor for Compose, Reply, Quote, and the shared single-line
//! fields (Reply Guy topic, search, and the other boxes that use the same
//! control). Hit testing uses the same shaped glyph positions that are painted,
//! so a click lands on the visual character, including soft-wrapped rows.

use super::{EditorId, InputTarget, TextField, XPanel, soft_wrap_ranges};
use gpui_kit::{
    Bounds, ContentMask, Context, Font, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Styled,
    TextAlign, TextRun, canvas, div, fill, point, px, size,
};
use termy_x::draft_edit::{
    DraftCaret, Motion, PaintedLine, grapheme_ranges, hit_test, hit_test_uniform,
    is_navigation_key, line_index,
};
use termy_x::text::{EntityKind, entity_spans};

pub(super) const COMPOSE_FONT: f32 = 14.0;
pub(super) const COMPOSE_LINE: f32 = 22.0;
pub(super) const FIELD_FONT: f32 = 13.0;
pub(super) const FIELD_LINE: f32 = 20.0;

pub(super) struct EditorPaint {
    pub text: String,
    pub lines: Vec<PaintedLine>,
}

struct EditorSnap {
    text: String,
    caret: usize,
    selection: Option<(usize, usize)>,
    placeholder: String,
    focused: bool,
    max_chars: usize,
    font_px: f32,
    line_h: f32,
    single_line: bool,
    masked: bool,
}

pub(super) fn navigation_motion(key: &str, word: bool) -> Option<Motion> {
    if !is_navigation_key(key) {
        return None;
    }
    Some(match key {
        "left" if word => Motion::WordLeft,
        "right" if word => Motion::WordRight,
        "left" => Motion::Left,
        "right" => Motion::Right,
        "up" => Motion::Up,
        "down" => Motion::Down,
        "home" if word => Motion::DocStart,
        "end" if word => Motion::DocEnd,
        "home" => Motion::Home,
        "end" => Motion::End,
        _ => return None,
    })
}

fn input_target_for(field: TextField) -> InputTarget {
    match field {
        TextField::Search => InputTarget::Search,
        TextField::Reply => InputTarget::Reply,
        TextField::Quote => InputTarget::Quote,
        TextField::Trends => InputTarget::Trends,
        TextField::Research => InputTarget::Research,
        TextField::Lookup => InputTarget::Lookup,
        TextField::FeedQuery => InputTarget::FeedQuery,
        TextField::FeedFilter => InputTarget::FeedFilter,
        TextField::QueueText => InputTarget::QueueText,
        TextField::QueueReplyTo => InputTarget::QueueReplyTo,
        TextField::Analyze => InputTarget::Analyze,
        TextField::RgTopic => InputTarget::RgTopic,
        TextField::RgKey => InputTarget::RgKey,
    }
}

fn hex_hsla(hex: u32, alpha: f32) -> Hsla {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d < 1.0e-6 {
        return Hsla {
            h: 0.0,
            s: 0.0,
            l,
            a: alpha,
        };
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut h = if (max - r).abs() < 1.0e-6 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < 1.0e-6 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h /= 6.0;
    Hsla { h, s, l, a: alpha }
}

impl XPanel {
    pub(super) fn active_editor(&self) -> Option<EditorId> {
        if self.bearer_mode {
            return Some(EditorId::Bearer);
        }
        match self.input_target {
            InputTarget::Compose => Some(EditorId::Compose),
            InputTarget::RgKey => Some(EditorId::Field(TextField::RgKey)),
            InputTarget::Reply => Some(EditorId::Field(TextField::Reply)),
            InputTarget::Quote => Some(EditorId::Field(TextField::Quote)),
            other => super::single_line_field_for(other).map(EditorId::Field),
        }
    }

    fn editor_is_single(&self, id: EditorId) -> bool {
        !matches!(
            id,
            EditorId::Compose | EditorId::Field(TextField::Reply | TextField::Quote)
        )
    }

    fn editor_wrap_chars(&self, id: EditorId) -> usize {
        if self.editor_is_single(id) {
            // One visual row per hard line. Enter does not insert a break
            // in these fields; a pasted newline still moves with Up/Down.
            usize::MAX / 4
        } else {
            self.editor_max_chars()
        }
    }

    fn editor_lines(&self, id: EditorId, text: &str) -> Vec<(usize, usize)> {
        soft_wrap_ranges(text, self.editor_wrap_chars(id))
    }

    pub(super) fn editor_text(&self, id: EditorId) -> &str {
        match id {
            EditorId::Compose => self.compose.as_str(),
            EditorId::Field(TextField::Search) => self.search_input.as_str(),
            EditorId::Field(TextField::Reply) => self.reply_text.as_str(),
            EditorId::Field(TextField::Quote) => self.quote_text.as_str(),
            EditorId::Field(TextField::Trends) => self.trends_filter.as_str(),
            EditorId::Field(TextField::Research) => self.research_input.as_str(),
            EditorId::Field(TextField::Lookup) => self.lookup.as_str(),
            EditorId::Field(TextField::FeedQuery) => self.desk.feed_query.as_str(),
            EditorId::Field(TextField::FeedFilter) => self.desk.feed_filter.as_str(),
            EditorId::Field(TextField::QueueText) => self.desk.queue_text.as_str(),
            EditorId::Field(TextField::QueueReplyTo) => self.desk.queue_reply_to.as_str(),
            EditorId::Field(TextField::Analyze) => self.desk.analyze_handle.as_str(),
            EditorId::Field(TextField::RgTopic) => self.desk.rg_topic.as_str(),
            EditorId::Field(TextField::RgKey) => self.desk.rg_key_input.as_str(),
            EditorId::Bearer => self.bearer_input.as_str(),
        }
    }

    fn buffer_mut(&mut self, id: EditorId) -> &mut String {
        match id {
            EditorId::Compose => &mut self.compose,
            EditorId::Field(field) => self
                .field_text_mut(field)
                .expect("every draft field has a string"),
            EditorId::Bearer => &mut self.bearer_input,
        }
    }

    fn draft_seed(&self, id: EditorId) -> Option<DraftCaret> {
        match id {
            EditorId::Compose => Some(DraftCaret {
                caret: self.caret,
                anchor: self.sel_anchor,
                preferred_col: self.compose_preferred,
            }),
            EditorId::Field(field) => self.field_carets.get(&field).copied(),
            EditorId::Bearer => Some(self.bearer_caret),
        }
    }

    fn store_draft(&mut self, id: EditorId, draft: DraftCaret) {
        match id {
            EditorId::Compose => {
                self.caret = draft.caret;
                self.sel_anchor = draft.anchor;
                self.compose_preferred = draft.preferred_col;
            }
            EditorId::Field(field) => {
                self.field_carets.insert(field, draft);
            }
            EditorId::Bearer => self.bearer_caret = draft,
        }
    }

    pub(super) fn edit_buffer(
        &mut self,
        id: EditorId,
        f: impl FnOnce(&mut String, &mut DraftCaret),
    ) {
        let seed = self.draft_seed(id);
        let saved = {
            let text = self.buffer_mut(id);
            let mut draft = seed.unwrap_or_else(|| DraftCaret::at_end(text.len()));
            draft.clamp(text);
            f(text, &mut draft);
            draft
        };
        self.store_draft(id, saved);
    }

    pub(super) fn editor_motion(&mut self, id: EditorId, motion: Motion, extend: bool) {
        let lines = {
            let text = self.editor_text(id).to_string();
            self.editor_lines(id, &text)
        };
        self.edit_buffer(id, |text, draft| {
            draft.move_by(text, &lines, motion, extend);
        });
        self.reveal_editor(id);
    }

    pub(super) fn editor_insert(&mut self, id: EditorId, insert: &str) {
        self.mention_dirty_at = Some(std::time::Instant::now());
        self.edit_buffer(id, |text, draft| draft.insert(text, insert));
        self.reveal_editor(id);
    }

    pub(super) fn editor_backspace(&mut self, id: EditorId, word: bool) {
        self.mention_dirty_at = Some(std::time::Instant::now());
        self.edit_buffer(id, |text, draft| {
            if word {
                draft.delete_word_backward(text);
            } else {
                draft.backspace(text);
            }
        });
        self.reveal_editor(id);
    }

    pub(super) fn editor_delete_forward(&mut self, id: EditorId) {
        self.mention_dirty_at = Some(std::time::Instant::now());
        self.edit_buffer(id, |text, draft| draft.delete_forward(text));
        self.reveal_editor(id);
    }

    pub(super) fn editor_select_all(&mut self, id: EditorId) {
        self.edit_buffer(id, |text, draft| draft.select_all(text));
    }

    pub(super) fn editor_selection_text(&self, id: EditorId) -> Option<String> {
        let text = self.editor_text(id);
        let mut draft = self
            .draft_seed(id)
            .unwrap_or_else(|| DraftCaret::at_end(text.len()));
        draft.clamp(text);
        draft.selected_text(text).map(str::to_string)
    }

    pub(super) fn editor_cut(&mut self, id: EditorId) -> Option<String> {
        let selected = self.editor_selection_text(id)?;
        self.edit_buffer(id, |text, draft| {
            if let Some((start, end)) = draft.selection() {
                text.replace_range(start..end, "");
                draft.caret = start;
                draft.anchor = None;
                draft.preferred_col = None;
            }
        });
        self.reveal_editor(id);
        Some(selected)
    }

    pub(super) fn reveal_editor(&mut self, id: EditorId) {
        match id {
            EditorId::Compose => self.follow_compose_caret(),
            EditorId::Field(TextField::Reply) => self.follow_small_caret(TextField::Reply),
            EditorId::Field(TextField::Quote) => self.follow_small_caret(TextField::Quote),
            _ => {}
        }
    }

    pub(super) fn follow_small_caret(&mut self, field: TextField) {
        let id = EditorId::Field(field);
        let text = self.editor_text(id).to_string();
        let lines = self.editor_lines(id, &text);
        let caret = self
            .draft_seed(id)
            .map_or(text.len(), |draft| draft.caret.min(text.len()));
        let line = line_index(&lines, caret);
        let handle = match field {
            TextField::Reply => &self.reply_scroll,
            TextField::Quote => &self.quote_scroll,
            _ => return,
        };
        let line_h = FIELD_LINE;
        let view_h = 44.0_f32;
        let cur: f32 = handle.offset().y.into();
        let top = -cur;
        let caret_top = line as f32 * line_h;
        let mut new_top = top;
        if caret_top < top {
            new_top = caret_top;
        } else if caret_top + line_h > top + view_h {
            new_top = caret_top + line_h - view_h;
        }
        if (new_top - top).abs() > 0.5 {
            handle.set_offset(point(px(0.0), px(-new_top)));
        }
    }

    fn focus_editor(
        &mut self,
        id: EditorId,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.card_copy_focus = false;
        self.card_text_selection = None;
        self.card_drag_start = None;
        match id {
            EditorId::Compose => {
                self.input_target = InputTarget::Compose;
                self.focus_compose_editor();
            }
            EditorId::Bearer => {
                self.bearer_mode = true;
            }
            EditorId::Field(field) => {
                self.input_target = input_target_for(field);
                self.compose_focused = false;
            }
        }
        self.focus.focus(window, cx);
        self.pending_focus = false;
    }

    fn editor_hit_index(&self, id: EditorId, at: gpui_kit::Point<Pixels>) -> usize {
        let text = self.editor_text(id);
        if text.is_empty() {
            return 0;
        }
        let x: f32 = at.x.into();
        let y: f32 = at.y.into();
        if let Some(paint) = self.editor_paint.get(&id)
            && paint.text == text
        {
            return hit_test(&paint.lines, x, y);
        }
        let lines = self.editor_lines(id, text);
        let origin = self.editor_origin(id);
        hit_test_uniform(
            text,
            &lines,
            origin.0,
            origin.1,
            x,
            y,
            if self.editor_is_single(id) {
                FIELD_LINE
            } else if matches!(id, EditorId::Compose) {
                COMPOSE_LINE
            } else {
                FIELD_LINE
            },
            7.2,
        )
    }

    fn editor_origin(&self, id: EditorId) -> (f32, f32) {
        let bounds = match id {
            EditorId::Compose => self.compose_scroll.bounds(),
            EditorId::Field(TextField::Reply) => self.reply_scroll.bounds(),
            EditorId::Field(TextField::Quote) => self.quote_scroll.bounds(),
            _ => return (0.0, 0.0),
        };
        let pad = match id {
            EditorId::Compose => 0.0,
            _ => 8.0,
        };
        (
            f32::from(bounds.origin.x) + pad,
            f32::from(bounds.origin.y) + pad,
        )
    }

    pub(super) fn editor_pointer_down(
        &mut self,
        id: EditorId,
        event: &MouseDownEvent,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_editor(id, window, cx);
        let index = self.editor_hit_index(id, event.position);
        let shift = event.modifiers.shift;
        let clicks = event.click_count;
        let single = self.editor_is_single(id);
        let lines = {
            let text = self.editor_text(id).to_string();
            self.editor_lines(id, &text)
        };
        self.edit_buffer(id, |text, draft| {
            if clicks >= 3 {
                if single {
                    draft.select_all(text);
                } else {
                    draft.select_line_at(text, &lines, index);
                }
            } else if clicks == 2 {
                // Single-line fields keep double-click = select all
                // (Reply Guy topic, search, and the other shared boxes).
                if single {
                    draft.select_all(text);
                } else {
                    draft.select_word_at(text, index);
                }
            } else if shift {
                draft.place(text, index, true);
            } else {
                draft.drag_anchor(text, index);
            }
        });
        self.text_drag = (clicks == 1).then_some(id);
        self.reveal_editor(id);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn editor_pointer_move(
        &mut self,
        id: EditorId,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) {
        if self.text_drag != Some(id) || !event.dragging() {
            return;
        }
        let index = self.editor_hit_index(id, event.position);
        self.edit_buffer(id, |text, draft| draft.drag_to(text, index));
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn editor_pointer_up(&mut self, id: EditorId, cx: &mut Context<Self>) {
        if self.text_drag == Some(id) {
            self.text_drag = None;
            cx.stop_propagation();
            cx.notify();
        }
    }

    pub(super) fn editor_canvas(
        &self,
        id: EditorId,
        focused: bool,
        placeholder: &str,
        font_px: f32,
        line_h: f32,
        masked: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let text = self.editor_text(id).to_string();
        let mut draft = self
            .draft_seed(id)
            .unwrap_or_else(|| DraftCaret::at_end(text.len()));
        draft.clamp(&text);
        let max_chars = self.editor_wrap_chars(id);
        let single_line = self.editor_is_single(id);
        let ranges = soft_wrap_ranges(&text, max_chars);
        let content_h = if single_line {
            line_h
        } else {
            (ranges.len().max(1) as f32) * line_h
        };
        let snap = EditorSnap {
            text,
            caret: draft.caret,
            selection: draft.selection(),
            placeholder: placeholder.to_string(),
            focused,
            max_chars,
            font_px,
            line_h,
            single_line,
            masked,
        };
        let paint_entity = cx.entity();
        div()
            .relative()
            .w_full()
            .h(px(content_h))
            .child(
                canvas(
                    move |bounds, window, _cx| paint_editor(bounds, window, &snap),
                    move |_bounds, painted, window, cx| {
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds: painted.clip,
                            }),
                            |window| {
                                for quad in &painted.quads {
                                    window.paint_quad(quad.clone());
                                }
                                for (origin, line_h, line) in &painted.glyphs {
                                    line.paint(*origin, *line_h, TextAlign::Left, None, window, cx)
                                        .expect("failed to paint draft text");
                                }
                            },
                        );
                        let recorded = EditorPaint {
                            text: painted.text,
                            lines: painted.lines,
                        };
                        paint_entity.update(cx, |panel, _cx| {
                            panel.editor_paint.insert(id, recorded);
                        });
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .cursor_text()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            this.editor_pointer_down(id, event, window, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                        this.editor_pointer_move(id, event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _event: &MouseUpEvent, _, cx| {
                            this.editor_pointer_up(id, cx);
                        }),
                    ),
            )
            .into_any_element()
    }
}

struct PaintOut {
    text: String,
    lines: Vec<PaintedLine>,
    quads: Vec<gpui_kit::PaintQuad>,
    glyphs: Vec<(gpui_kit::Point<Pixels>, Pixels, gpui_kit::ShapedLine)>,
    clip: Bounds<Pixels>,
}

fn paint_editor(
    bounds: Bounds<Pixels>,
    window: &mut gpui_kit::Window,
    snap: &EditorSnap,
) -> PaintOut {
    let font = Font {
        family: "sans-serif".into(),
        weight: FontWeight::NORMAL,
        ..gpui_kit::font("")
    };
    let mask = snap
        .masked
        .then(|| termy_x::draft_edit::MaskMap::new(&snap.text));
    let display_text = mask
        .as_ref()
        .map(termy_x::draft_edit::MaskMap::display)
        .unwrap_or_else(|| snap.text.clone());
    let real_at = |display_offset: usize| {
        mask.as_ref()
            .map(|map| map.display_to_real(display_offset))
            .unwrap_or(display_offset)
    };
    let display_at = |real_offset: usize| {
        mask.as_ref()
            .map(|map| map.real_to_display(real_offset))
            .unwrap_or(real_offset)
    };
    let text_color = hex_hsla(0xf8fbff, 1.0);
    let entity_color = hex_hsla(0x1d9bf0, 1.0);
    let placeholder_color = hex_hsla(0x93a4c3, 1.0);
    let selection_color = hex_hsla(0x0a84ff, 0.45);
    let caret_color = hex_hsla(0x7af6ff, 1.0);
    let ranges = soft_wrap_ranges(&display_text, snap.max_chars);
    let entities = if snap.masked {
        Vec::new()
    } else {
        entity_spans(&snap.text)
    };
    let caret = display_at(snap.caret.min(snap.text.len()));
    let caret_row = line_index(&ranges, caret);

    let mut shaped: Vec<Option<gpui_kit::ShapedLine>> = Vec::with_capacity(ranges.len());
    let mut caret_local_x = 0.0_f32;
    for (idx, &(start, end)) in ranges.iter().enumerate() {
        let slice = display_text.get(start..end).unwrap_or("");
        if slice.is_empty() {
            shaped.push(None);
            continue;
        }
        let runs = text_runs(slice, start, &entities, &font, text_color, entity_color);
        let line = window.text_system().shape_line(
            slice.to_string().into(),
            px(snap.font_px),
            &runs,
            None,
        );
        if idx == caret_row {
            let local = caret.saturating_sub(start).min(slice.len());
            caret_local_x = f32::from(line.x_for_index(local));
        }
        shaped.push(Some(line));
    }

    let view_w: f32 = bounds.size.width.into();
    let (x_shift, y_shift) = if snap.single_line {
        let y_shift = caret_row as f32 * snap.line_h;
        let x_shift = if caret_local_x > view_w - 16.0 {
            (caret_local_x - (view_w - 16.0)).max(0.0)
        } else {
            0.0
        };
        (x_shift, y_shift)
    } else {
        (0.0, 0.0)
    };

    let left0: f32 = bounds.left().into();
    let top0: f32 = bounds.top().into();
    let mut lines = Vec::with_capacity(ranges.len());
    let mut quads = Vec::new();
    let mut glyphs = Vec::new();

    for (idx, (&(start, end), line)) in ranges.iter().zip(shaped).enumerate() {
        let line_left = left0 - x_shift;
        let line_top = top0 + idx as f32 * snap.line_h - y_shift;
        let slice = display_text.get(start..end).unwrap_or("");
        let mut stops = Vec::new();
        stops.push((real_at(start), line_left));
        if let Some(line) = line.as_ref() {
            for (_rel, rel_end) in grapheme_ranges(slice) {
                let x = line_left + f32::from(line.x_for_index(rel_end));
                stops.push((real_at(start + rel_end), x));
            }
        }
        let selection = snap
            .selection
            .map(|(sel_a, sel_b)| (display_at(sel_a), display_at(sel_b)));
        if snap.focused
            && let Some((sel_a, sel_b)) = selection
        {
            let from = sel_a.max(start).min(end);
            let to = sel_b.max(start).min(end);
            if from < to {
                let x0 = line
                    .as_ref()
                    .map_or(0.0, |line| f32::from(line.x_for_index(from - start)));
                let x1 = line
                    .as_ref()
                    .map_or(0.0, |line| f32::from(line.x_for_index(to - start)));
                quads.push(fill(
                    Bounds::from_corners(
                        point(px(line_left + x0), px(line_top)),
                        point(px(line_left + x1.max(x0 + 1.0)), px(line_top + snap.line_h)),
                    ),
                    selection_color,
                ));
            }
        }
        if let Some(line) = line {
            glyphs.push((point(px(line_left), px(line_top)), px(snap.line_h), line));
        }
        if snap.focused && snap.selection.is_none() && idx == caret_row {
            let caret_x = line_left + caret_local_x;
            quads.push(fill(
                Bounds::new(
                    point(px(caret_x), px(line_top + 2.0)),
                    size(px(2.0), px((snap.line_h - 4.0).max(8.0))),
                ),
                caret_color,
            ));
        }
        lines.push(PaintedLine {
            top: line_top,
            bottom: line_top + snap.line_h,
            stops,
        });
    }

    if snap.text.is_empty() && !snap.placeholder.is_empty() {
        let runs = vec![TextRun {
            len: snap.placeholder.len(),
            font,
            color: placeholder_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        let line = window.text_system().shape_line(
            snap.placeholder.clone().into(),
            px(snap.font_px),
            &runs,
            None,
        );
        let x = if snap.focused { 8.0 } else { 0.0 };
        glyphs.push((point(px(left0 + x), px(top0)), px(snap.line_h), line));
        if lines.is_empty() {
            lines.push(PaintedLine {
                top: top0,
                bottom: top0 + snap.line_h,
                stops: vec![(0, left0)],
            });
        }
    }

    PaintOut {
        text: snap.text.clone(),
        lines,
        quads,
        glyphs,
        clip: bounds,
    }
}

fn text_runs(
    slice: &str,
    abs_start: usize,
    entities: &[(usize, usize, EntityKind)],
    font: &Font,
    text_color: Hsla,
    entity_color: Hsla,
) -> Vec<TextRun> {
    let abs_end = abs_start + slice.len();
    let mut cuts = vec![0usize, slice.len()];
    for (start, end, _) in entities {
        for edge in [*start, *end] {
            if edge > abs_start && edge < abs_end {
                cuts.push(edge - abs_start);
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut runs = Vec::new();
    for pair in cuts.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if from >= to {
            continue;
        }
        let in_entity = entities
            .iter()
            .any(|(start, end, _)| abs_start + from >= *start && abs_start + to <= *end);
        runs.push(TextRun {
            len: to - from,
            font: font.clone(),
            color: if in_entity { entity_color } else { text_color },
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    if runs.is_empty() {
        runs.push(TextRun {
            len: slice.len(),
            font: font.clone(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    runs
}
