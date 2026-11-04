use super::*;

impl TerminalView {
    fn native_resize_overlap_cells(a_start: u16, a_end: u16, b_start: u16, b_end: u16) -> u16 {
        let start = a_start.max(b_start);
        let end = a_end.min(b_end);
        end.saturating_sub(start)
    }

    pub(in super::super) fn begin_pane_resize_drag(
        &mut self,
        pane_id: &str,
        axis: PaneResizeAxis,
        edge: PaneResizeEdge,
        position: gpui_kit::Point<Pixels>,
    ) -> bool {
        let (x, y) = self.terminal_content_position(position);
        self.pane_resize_drag = Some(PaneResizeDragState {
            pane_id: pane_id.to_string(),
            axis,
            edge,
            start_x: x,
            start_y: y,
            applied_steps: 0,
        });
        true
    }

    pub(super) fn finish_pane_resize_drag(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pane_resize_drag.take().is_none() {
            return false;
        }
        self.pane_resize_blocked = false;
        self.last_terminal_resize_signature = None;
        self.last_resize_applied_at = None;
        if self.runtime_kind() == RuntimeKind::Native {
            self.schedule_persist_native_workspace(cx);
        }
        true
    }

    pub(super) fn pane_resize_hit_test(
        &self,
        position: gpui_kit::Point<Pixels>,
        window: &Window,
    ) -> Option<PaneResizeDragState> {
        let tab = self.session.tabs.get(self.session.active_tab)?;
        let content_bounds = self.terminal_content_bounds(window)?;
        let (x, y) = self.terminal_content_position(position);
        self.native_pane_dividers(tab, content_bounds)
            .into_iter()
            .filter_map(|divider| {
                divider
                    .hit_distance(x, y)
                    .map(|distance| (distance, divider))
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, divider)| PaneResizeDragState {
                pane_id: divider.pane_id,
                axis: divider.axis,
                edge: divider.edge,
                start_x: x,
                start_y: y,
                applied_steps: 0,
            })
    }

    pub(in crate::terminal_view) fn native_resize_pane_step(
        &mut self,
        pane_id: &str,
        axis: PaneResizeAxis,
        edge: PaneResizeEdge,
        divider_delta: i16,
    ) -> PaneResizeResult {
        fn overlaps_any(span_start: u16, span_end: u16, spans: &[(u16, u16)]) -> bool {
            spans.iter().any(|(start, end)| {
                TerminalView::native_resize_overlap_cells(span_start, span_end, *start, *end) > 0
            })
        }

        if divider_delta == 0 {
            return PaneResizeResult::NoChange;
        }

        let (
            tab_id,
            total_cols,
            total_rows,
            target_id,
            target_left,
            target_top,
            target_width,
            target_height,
        ) = {
            let Some(tab) = self.session.tabs.get(self.session.active_tab) else {
                return PaneResizeResult::NoChange;
            };
            let Some(target_index) = tab.panes.iter().position(|pane| pane.id == pane_id) else {
                return PaneResizeResult::NoChange;
            };
            let Some(target) = tab.panes.get(target_index) else {
                return PaneResizeResult::NoChange;
            };
            (
                tab.id,
                tab.panes
                    .iter()
                    .map(|pane| pane.left.saturating_add(pane.width))
                    .max()
                    .unwrap_or(target.width)
                    .max(1),
                tab.panes
                    .iter()
                    .map(|pane| pane.top.saturating_add(pane.height))
                    .max()
                    .unwrap_or(target.height)
                    .max(1),
                target.id.clone(),
                target.left,
                target.top,
                target.width,
                target.height,
            )
        };

        if self.ensure_native_layout_tree_for_tab_id(tab_id)
            && let Some(tree) = self.session.native_pane_layout_trees.get_mut(&tab_id)
        {
            let min_extent = match axis {
                PaneResizeAxis::Horizontal => NativeLayout::native_min_extent_allowed(
                    total_cols,
                    NativeLayout::native_tree_leaf_count(&tree.root),
                    NativeLayout::native_pane_min_extent_for_axis(PaneResizeAxis::Horizontal),
                ),
                PaneResizeAxis::Vertical => NativeLayout::native_min_extent_allowed(
                    total_rows,
                    NativeLayout::native_tree_leaf_count(&tree.root),
                    NativeLayout::native_pane_min_extent_for_axis(PaneResizeAxis::Vertical),
                ),
            };
            let result = NativeLayout::native_adjust_tree_split(
                &mut tree.root,
                target_id.as_str(),
                axis,
                edge,
                divider_delta,
                NativePaneRect {
                    left: 0,
                    top: 0,
                    width: total_cols,
                    height: total_rows,
                },
                min_extent,
            );
            if result != PaneResizeResult::NoChange {
                self.apply_native_layout_tree_to_tab(tab_id, total_cols, total_rows);
                return result;
            }
        }

        let Some(tab) = self.session.tabs.get_mut(self.session.active_tab) else {
            return PaneResizeResult::NoChange;
        };
        debug_assert_eq!(tab.id, tab_id);

        match axis {
            PaneResizeAxis::Horizontal => {
                let boundary = match edge {
                    PaneResizeEdge::Left => target_left,
                    PaneResizeEdge::Right => target_left.saturating_add(target_width),
                    PaneResizeEdge::Top | PaneResizeEdge::Bottom => {
                        return PaneResizeResult::NoChange;
                    }
                };
                let mut spans = vec![(target_top, target_top.saturating_add(target_height))];
                let mut left_indices = Vec::<usize>::new();
                let mut right_indices = Vec::<usize>::new();

                loop {
                    let mut changed = false;
                    for (index, pane) in tab.panes.iter().enumerate() {
                        let pane_top = pane.top;
                        let pane_bottom = pane.top.saturating_add(pane.height);
                        if !overlaps_any(pane_top, pane_bottom, &spans) {
                            continue;
                        }

                        let pane_left = pane.left;
                        let pane_right = pane.left.saturating_add(pane.width);
                        if pane_right == boundary && !left_indices.contains(&index) {
                            left_indices.push(index);
                            spans.push((pane_top, pane_bottom));
                            changed = true;
                        }
                        if pane_left == boundary && !right_indices.contains(&index) {
                            right_indices.push(index);
                            spans.push((pane_top, pane_bottom));
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }

                if left_indices.is_empty() || right_indices.is_empty() {
                    return PaneResizeResult::NoChange;
                }
                let horizontal_lane_count =
                    Self::native_pane_lane_count_for_axis(&tab.panes, PaneResizeAxis::Horizontal);
                let min_width = NativeLayout::native_min_extent_allowed(
                    tab.panes
                        .iter()
                        .map(|pane| pane.left.saturating_add(pane.width))
                        .max()
                        .unwrap_or(0),
                    horizontal_lane_count,
                    NativeLayout::native_pane_min_extent_for_axis(PaneResizeAxis::Horizontal),
                );

                if divider_delta > 0 {
                    if right_indices.iter().any(|index| {
                        tab.panes[*index].width
                            < min_width.saturating_add(divider_delta.unsigned_abs())
                    }) {
                        return PaneResizeResult::BlockedByMinimum;
                    }
                    for index in left_indices {
                        tab.panes[index].width =
                            tab.panes[index].width.saturating_add(divider_delta as u16);
                    }
                    for index in right_indices {
                        tab.panes[index].left =
                            tab.panes[index].left.saturating_add(divider_delta as u16);
                        tab.panes[index].width =
                            tab.panes[index].width.saturating_sub(divider_delta as u16);
                    }
                } else {
                    let shrink = divider_delta.unsigned_abs();
                    if left_indices
                        .iter()
                        .any(|index| tab.panes[*index].width < min_width.saturating_add(shrink))
                    {
                        return PaneResizeResult::BlockedByMinimum;
                    }
                    for index in left_indices {
                        tab.panes[index].width = tab.panes[index].width.saturating_sub(shrink);
                    }
                    for index in right_indices {
                        tab.panes[index].left = tab.panes[index].left.saturating_sub(shrink);
                        tab.panes[index].width = tab.panes[index].width.saturating_add(shrink);
                    }
                }
            }
            PaneResizeAxis::Vertical => {
                let boundary = match edge {
                    PaneResizeEdge::Top => target_top,
                    PaneResizeEdge::Bottom => target_top.saturating_add(target_height),
                    PaneResizeEdge::Left | PaneResizeEdge::Right => {
                        return PaneResizeResult::NoChange;
                    }
                };
                let mut spans = vec![(target_left, target_left.saturating_add(target_width))];
                let mut top_indices = Vec::<usize>::new();
                let mut bottom_indices = Vec::<usize>::new();

                loop {
                    let mut changed = false;
                    for (index, pane) in tab.panes.iter().enumerate() {
                        let pane_left = pane.left;
                        let pane_right = pane.left.saturating_add(pane.width);
                        if !overlaps_any(pane_left, pane_right, &spans) {
                            continue;
                        }

                        let pane_top = pane.top;
                        let pane_bottom = pane.top.saturating_add(pane.height);
                        if pane_bottom == boundary && !top_indices.contains(&index) {
                            top_indices.push(index);
                            spans.push((pane_left, pane_right));
                            changed = true;
                        }
                        if pane_top == boundary && !bottom_indices.contains(&index) {
                            bottom_indices.push(index);
                            spans.push((pane_left, pane_right));
                            changed = true;
                        }
                    }
                    if !changed {
                        break;
                    }
                }

                if top_indices.is_empty() || bottom_indices.is_empty() {
                    return PaneResizeResult::NoChange;
                }
                let vertical_lane_count =
                    Self::native_pane_lane_count_for_axis(&tab.panes, PaneResizeAxis::Vertical);
                let min_height = NativeLayout::native_min_extent_allowed(
                    tab.panes
                        .iter()
                        .map(|pane| pane.top.saturating_add(pane.height))
                        .max()
                        .unwrap_or(0),
                    vertical_lane_count,
                    NativeLayout::native_pane_min_extent_for_axis(PaneResizeAxis::Vertical),
                );

                if divider_delta > 0 {
                    if bottom_indices.iter().any(|index| {
                        tab.panes[*index].height
                            < min_height.saturating_add(divider_delta.unsigned_abs())
                    }) {
                        return PaneResizeResult::BlockedByMinimum;
                    }
                    for index in top_indices {
                        tab.panes[index].height =
                            tab.panes[index].height.saturating_add(divider_delta as u16);
                    }
                    for index in bottom_indices {
                        tab.panes[index].top =
                            tab.panes[index].top.saturating_add(divider_delta as u16);
                        tab.panes[index].height =
                            tab.panes[index].height.saturating_sub(divider_delta as u16);
                    }
                } else {
                    let shrink = divider_delta.unsigned_abs();
                    if top_indices
                        .iter()
                        .any(|index| tab.panes[*index].height < min_height.saturating_add(shrink))
                    {
                        return PaneResizeResult::BlockedByMinimum;
                    }
                    for index in top_indices {
                        tab.panes[index].height = tab.panes[index].height.saturating_sub(shrink);
                    }
                    for index in bottom_indices {
                        tab.panes[index].top = tab.panes[index].top.saturating_sub(shrink);
                        tab.panes[index].height = tab.panes[index].height.saturating_add(shrink);
                    }
                }
            }
        }

        PaneResizeResult::Applied
    }

    pub(super) fn apply_pane_resize_drag(&mut self, position: gpui_kit::Point<Pixels>) -> bool {
        let Some(drag_state) = self.pane_resize_drag.as_ref() else {
            return false;
        };
        let pane_id = drag_state.pane_id.clone();
        let axis = drag_state.axis;
        let edge = drag_state.edge;
        let start_x = drag_state.start_x;
        let start_y = drag_state.start_y;
        let already_applied_steps = drag_state.applied_steps;

        let layout_cell_size = self.layout_cell_size();
        let axis_cell_pixels = match axis {
            PaneResizeAxis::Horizontal => {
                let width: f32 = layout_cell_size.width.into();
                width
            }
            PaneResizeAxis::Vertical => {
                let height: f32 = layout_cell_size.height.into();
                height
            }
        };
        if axis_cell_pixels <= f32::EPSILON {
            return false;
        }

        let (current_x, current_y) = self.terminal_content_position(position);
        let delta_pixels = match axis {
            PaneResizeAxis::Horizontal => current_x - start_x,
            PaneResizeAxis::Vertical => current_y - start_y,
        };
        let desired_steps = (delta_pixels / axis_cell_pixels).trunc() as i32;
        let step_delta = desired_steps - already_applied_steps;
        if step_delta == 0 {
            return false;
        }
        let (completed_steps, was_blocked) = match self.runtime_kind() {
            RuntimeKind::Tmux => {
                // Left/top drags invert the tmux resize direction relative to cursor delta,
                // because dragging toward the pane interior shrinks that edge.
                let positive_direction = match edge {
                    PaneResizeEdge::Left | PaneResizeEdge::Top => step_delta.is_negative(),
                    PaneResizeEdge::Right | PaneResizeEdge::Bottom => step_delta.is_positive(),
                };
                let cells = step_delta.unsigned_abs().min(u32::from(u16::MAX)) as u16;
                if self.tmux_resize_pane_by(pane_id.as_str(), axis, positive_direction, cells) {
                    (i32::from(cells), false)
                } else {
                    (0, false)
                }
            }
            RuntimeKind::Native => {
                let batched_delta = step_delta.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                match self.native_resize_pane_step(
                    pane_id.as_str(),
                    axis,
                    edge,
                    batched_delta as i16,
                ) {
                    PaneResizeResult::Applied => (batched_delta.unsigned_abs() as i32, false),
                    PaneResizeResult::NoChange => (0, false),
                    PaneResizeResult::BlockedByMinimum => {
                        let mut completed_steps = 0i32;
                        for _ in 0..batched_delta.unsigned_abs() {
                            match self.native_resize_pane_step(
                                pane_id.as_str(),
                                axis,
                                edge,
                                if batched_delta.is_positive() { 1 } else { -1 },
                            ) {
                                PaneResizeResult::Applied => completed_steps += 1,
                                PaneResizeResult::BlockedByMinimum | PaneResizeResult::NoChange => {
                                    break;
                                }
                            }
                        }
                        (completed_steps, true)
                    }
                }
            }
        };

        self.pane_resize_blocked = was_blocked;

        if completed_steps == 0 {
            return false;
        }

        let applied_delta = if step_delta.is_positive() {
            completed_steps
        } else {
            -completed_steps
        };
        if let Some(drag) = self.pane_resize_drag.as_mut() {
            drag.applied_steps += applied_delta;
        }
        true
    }
}
