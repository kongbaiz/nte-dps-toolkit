use super::*;

#[derive(Clone)]
pub(crate) struct SkillDamageSummary {
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) hits: u64,
    pub(crate) damage: f64,
}

#[derive(Clone)]
pub(crate) struct SkillCharacterSummary {
    pub(crate) char_id: u32,
    pub(crate) name: String,
    pub(crate) damage: f64,
    pub(crate) color: Color32,
}

pub(crate) fn aggregate_skill_characters(rows: &[SkillBreakdownRow]) -> Vec<SkillCharacterSummary> {
    let mut summaries = HashMap::<u32, SkillCharacterSummary>::new();
    for row in rows {
        let entry = summaries
            .entry(row.char_id)
            .or_insert_with(|| SkillCharacterSummary {
                char_id: row.char_id,
                name: row.char_name.clone(),
                damage: 0.0,
                color: Color32::WHITE,
            });
        entry.name.clone_from(&row.char_name);
        entry.damage += row.damage;
    }
    let mut rows = summaries.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.char_id.cmp(&right.char_id))
    });
    rows
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TimelineViewState {
    pub(crate) user_markers: Vec<f64>,
    pub(crate) drag_selection: Option<(f64, f64)>,
    pub(crate) zoom: Option<(f64, f64)>,
    drag_anchor: Option<f64>,
    context_time: Option<f64>,
}

impl TimelineViewState {
    fn view_range(&mut self, duration: f64) -> (f64, f64) {
        self.zoom = clamp_timeline_zoom(self.zoom, duration);
        self.zoom.unwrap_or((0.0, duration))
    }

    fn add_marker(&mut self, time: f64) {
        self.user_markers.push(time);
        self.user_markers.sort_by(f64::total_cmp);
    }

    fn remove_nearest_marker(&mut self, time: f64) {
        let Some(index) = self
            .user_markers
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (time - **left).abs().total_cmp(&(time - **right).abs())
            })
            .map(|(index, _)| index)
        else {
            return;
        };
        self.user_markers.remove(index);
    }
}

fn pointer_x_to_timeline_time(
    pointer_x: f32,
    plot_left: f32,
    plot_width: f32,
    view_range: (f64, f64),
) -> f64 {
    let progress = ((pointer_x - plot_left) / plot_width).clamp(0.0, 1.0) as f64;
    view_range.0 + (view_range.1 - view_range.0) * progress
}

fn timeline_time_to_pointer_x(
    time: f64,
    plot_left: f32,
    plot_width: f32,
    view_range: (f64, f64),
) -> f32 {
    let progress = ((time - view_range.0) / (view_range.1 - view_range.0)).clamp(0.0, 1.0);
    plot_left + plot_width * progress as f32
}

fn normalize_timeline_selection(start: f64, end: f64, duration: f64) -> Option<(f64, f64)> {
    if !start.is_finite() || !end.is_finite() || !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    let lower = start.min(end).clamp(0.0, duration);
    let upper = start.max(end).clamp(0.0, duration);
    (upper - lower > f64::EPSILON).then_some((lower, upper))
}

fn clamp_timeline_zoom(zoom: Option<(f64, f64)>, duration: f64) -> Option<(f64, f64)> {
    let (start, end) = zoom?;
    let range = normalize_timeline_selection(start, end, duration)?;
    (range.1 - range.0 < duration - f64::EPSILON).then_some(range)
}

fn intersect_timeline_range(start: f64, end: f64, view_range: (f64, f64)) -> Option<(f64, f64)> {
    let lower = start.max(view_range.0);
    let upper = end.min(view_range.1);
    (upper > lower).then_some((lower, upper))
}

fn timeline_time_is_visible(time: f64, view_range: (f64, f64)) -> bool {
    time >= view_range.0 && time <= view_range.1
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_timeline_chart(
    ui: &mut egui::Ui,
    series: &TimelineSeries,
    dps_view_mode: TimelineDpsViewMode,
    chart_height: f32,
    selected_char: &mut Option<u32>,
    dark_mode: bool,
    characters: &HashMap<u32, CharacterInfo>,
    view_state: &mut TimelineViewState,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), chart_height),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter().clone();
    painter.rect_filled(rect, 8.0, shadcn_card(dark_mode));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0_f32, shadcn_border(dark_mode)),
        egui::StrokeKind::Inside,
    );
    let duration = series
        .buckets
        .last()
        .map_or(series.bucket_seconds, |bucket| bucket.end_offset)
        .max(series.bucket_seconds)
        .max(0.001);
    let role_totals = timeline_top_roles(series, usize::MAX);
    let top_padding =
        if matches!(dps_view_mode, TimelineDpsViewMode::Characters) && !role_totals.is_empty() {
            92.0
        } else {
            24.0
        };
    let plot = egui::Rect::from_min_max(
        rect.min + egui::vec2(52.0, top_padding),
        rect.max - egui::vec2(12.0, 24.0),
    );
    if plot.width() <= 1.0 || plot.height() <= 1.0 {
        return;
    }
    let view_range = view_state.view_range(duration);
    let plot_painter = painter.with_clip_rect(plot);

    if response.secondary_clicked() {
        view_state.context_time = response
            .interact_pointer_pos()
            .filter(|pointer| plot.contains(*pointer))
            .map(|pointer| {
                pointer_x_to_timeline_time(pointer.x, plot.left(), plot.width(), view_range)
            });
    }
    if response.drag_started_by(egui::PointerButton::Primary) {
        let origin = ui.input(|input| input.pointer.press_origin());
        view_state.drag_anchor = origin
            .filter(|origin| plot.contains(*origin))
            .map(|origin| {
                pointer_x_to_timeline_time(origin.x, plot.left(), plot.width(), view_range)
            });
        view_state.drag_selection = None;
    }
    if (response.dragged_by(egui::PointerButton::Primary)
        || response.drag_stopped_by(egui::PointerButton::Primary))
        && let Some(anchor) = view_state.drag_anchor
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let current = pointer_x_to_timeline_time(pointer.x, plot.left(), plot.width(), view_range);
        view_state.drag_selection = normalize_timeline_selection(anchor, current, duration);
    }
    if response.drag_stopped_by(egui::PointerButton::Primary) {
        view_state.drag_anchor = None;
    }
    if let Some((start, end)) = view_state.drag_selection {
        view_state.drag_selection = normalize_timeline_selection(start, end, duration);
    }

    let team_max_dps = series
        .buckets
        .iter()
        .map(|bucket| bucket.dps)
        .fold(0.0, f64::max);
    let role_max_dps = series
        .buckets
        .iter()
        .flat_map(|bucket| bucket.role_damage.iter().map(|role| role.dps))
        .fold(0.0, f64::max);
    let max_dps = match dps_view_mode {
        TimelineDpsViewMode::Team => team_max_dps,
        TimelineDpsViewMode::Characters => role_max_dps.max(team_max_dps),
    }
    .max(1.0);
    let max_damage = series.total_damage.max(1.0);
    let grid_color = shadcn_border(dark_mode).gamma_multiply(0.7);
    let visible_duration = view_range.1 - view_range.0;
    for step in 0..=4 {
        let x = plot.left() + plot.width() * step as f32 / 4.0;
        painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.0_f32, grid_color),
        );
        let seconds = view_range.0 + visible_duration * step as f64 / 4.0;
        painter.text(
            egui::pos2(x, rect.bottom() - 12.0),
            egui::Align2::CENTER_CENTER,
            format!("{seconds:.0}s"),
            egui::FontId::monospace(10.0),
            ui.visuals().weak_text_color(),
        );
    }
    for step in 0..=3 {
        let y = plot.bottom() - plot.height() * step as f32 / 3.0;
        painter.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            Stroke::new(1.0_f32, grid_color),
        );
        let dps_value = max_dps * step as f64 / 3.0;
        painter.text(
            egui::pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_CENTER,
            format_number(dps_value),
            egui::FontId::monospace(9.0),
            ui.visuals().weak_text_color(),
        );
    }

    for interval in &series.time_stop_intervals {
        let Some((visible_start, visible_end)) =
            intersect_timeline_range(interval.start_offset, interval.end_offset, view_range)
        else {
            continue;
        };
        let left = timeline_time_to_pointer_x(visible_start, plot.left(), plot.width(), view_range);
        let right = timeline_time_to_pointer_x(visible_end, plot.left(), plot.width(), view_range);
        let band = egui::Rect::from_min_max(
            egui::pos2(left, plot.top()),
            egui::pos2(right, plot.bottom()),
        );
        plot_painter.rect_filled(band, 0.0, semantic_warning(dark_mode).gamma_multiply(0.16));
    }

    for marker in &series.markers {
        if !timeline_time_is_visible(marker.offset, view_range) {
            continue;
        }
        let x = timeline_time_to_pointer_x(marker.offset, plot.left(), plot.width(), view_range);
        let color = match marker.kind {
            TimelineMarkerKind::HalfStart => ui.visuals().selection.bg_fill,
            TimelineMarkerKind::Clear => semantic_success(dark_mode),
            TimelineMarkerKind::Exit => semantic_danger(dark_mode),
        };
        plot_painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.5_f32, color),
        );
        plot_painter.text(
            egui::pos2(x + 4.0, plot.top() + 10.0),
            egui::Align2::LEFT_CENTER,
            t(&marker.label),
            egui::FontId::proportional(10.0),
            color,
        );
    }

    let user_marker_color = ui.visuals().selection.bg_fill.gamma_multiply(0.82);
    for (index, marker) in view_state.user_markers.iter().copied().enumerate() {
        if !timeline_time_is_visible(marker, view_range) {
            continue;
        }
        let x = timeline_time_to_pointer_x(marker, plot.left(), plot.width(), view_range);
        plot_painter.line_segment(
            [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
            Stroke::new(1.0_f32, user_marker_color),
        );
        plot_painter.circle_filled(egui::pos2(x, plot.top() + 4.0), 3.0, user_marker_color);
        plot_painter.text(
            egui::pos2(x + 4.0, plot.bottom() - 8.0),
            egui::Align2::LEFT_CENTER,
            tf(
                "Marker {} · {}s",
                &[&(index + 1).to_string(), &format_timeline_seconds(marker)],
            ),
            egui::FontId::proportional(9.5),
            user_marker_color,
        );
    }

    match dps_view_mode {
        TimelineDpsViewMode::Team => {
            let dps_points = series
                .buckets
                .iter()
                .filter(|bucket| {
                    timeline_time_is_visible(
                        (bucket.start_offset + bucket.end_offset) * 0.5,
                        view_range,
                    )
                })
                .map(|bucket| {
                    let x = timeline_time_to_pointer_x(
                        (bucket.start_offset + bucket.end_offset) * 0.5,
                        plot.left(),
                        plot.width(),
                        view_range,
                    );
                    let y = plot.bottom() - (bucket.dps / max_dps) as f32 * plot.height();
                    egui::pos2(x, y)
                })
                .collect::<Vec<_>>();
            if dps_points.len() >= 2 {
                plot_painter.line(
                    dps_points,
                    Stroke::new(2.0_f32, ui.visuals().selection.bg_fill),
                );
            }

            let cumulative_points = series
                .buckets
                .iter()
                .filter(|bucket| {
                    timeline_time_is_visible(
                        (bucket.start_offset + bucket.end_offset) * 0.5,
                        view_range,
                    )
                })
                .map(|bucket| {
                    let x = timeline_time_to_pointer_x(
                        (bucket.start_offset + bucket.end_offset) * 0.5,
                        plot.left(),
                        plot.width(),
                        view_range,
                    );
                    let y = plot.bottom()
                        - (bucket.cumulative_damage / max_damage) as f32 * plot.height();
                    egui::pos2(x, y)
                })
                .collect::<Vec<_>>();
            if cumulative_points.len() >= 2 {
                plot_painter.line(
                    cumulative_points,
                    Stroke::new(1.5_f32, ui.visuals().weak_text_color()),
                );
            }
        }
        TimelineDpsViewMode::Characters => {
            for (rank, (char_id, _, _)) in role_totals.iter().enumerate() {
                let color = readable_accent(
                    character_color(*char_id, characters, rank, dark_mode),
                    dark_mode,
                );
                let selected = selected_char.is_some_and(|selected| selected == *char_id);
                let dimmed = selected_char.is_some() && !selected;
                let points = series
                    .buckets
                    .iter()
                    .filter(|bucket| {
                        timeline_time_is_visible(
                            (bucket.start_offset + bucket.end_offset) * 0.5,
                            view_range,
                        )
                    })
                    .map(|bucket| {
                        let x = timeline_time_to_pointer_x(
                            (bucket.start_offset + bucket.end_offset) * 0.5,
                            plot.left(),
                            plot.width(),
                            view_range,
                        );
                        let dps = bucket
                            .role_damage
                            .iter()
                            .find(|role| role.char_id == *char_id)
                            .map_or(0.0, |role| role.dps);
                        let y = plot.bottom() - (dps / max_dps) as f32 * plot.height();
                        egui::pos2(x, y)
                    })
                    .collect::<Vec<_>>();
                if points.len() >= 2 {
                    plot_painter.line(
                        points,
                        Stroke::new(
                            if selected { 3.0_f32 } else { 1.5_f32 },
                            color.gamma_multiply(if dimmed { 0.25 } else { 0.95 }),
                        ),
                    );
                }
            }
            if let Some(selected) = *selected_char
                && let Some((rank, (char_id, _, _))) = role_totals
                    .iter()
                    .enumerate()
                    .find(|(_, (char_id, _, _))| *char_id == selected)
            {
                let color = readable_accent(
                    character_color(*char_id, characters, rank, dark_mode),
                    dark_mode,
                );
                let points = series
                    .buckets
                    .iter()
                    .filter(|bucket| {
                        timeline_time_is_visible(
                            (bucket.start_offset + bucket.end_offset) * 0.5,
                            view_range,
                        )
                    })
                    .map(|bucket| {
                        let x = timeline_time_to_pointer_x(
                            (bucket.start_offset + bucket.end_offset) * 0.5,
                            plot.left(),
                            plot.width(),
                            view_range,
                        );
                        let dps = bucket
                            .role_damage
                            .iter()
                            .find(|role| role.char_id == *char_id)
                            .map_or(0.0, |role| role.dps);
                        let y = plot.bottom() - (dps / max_dps) as f32 * plot.height();
                        egui::pos2(x, y)
                    })
                    .collect::<Vec<_>>();
                if points.len() >= 2 {
                    plot_painter.line(points, Stroke::new(3.4_f32, color));
                }
            }
        }
    }

    if let Some((start, end)) = view_state.drag_selection
        && let Some((visible_start, visible_end)) = intersect_timeline_range(start, end, view_range)
    {
        let left = timeline_time_to_pointer_x(visible_start, plot.left(), plot.width(), view_range);
        let right = timeline_time_to_pointer_x(visible_end, plot.left(), plot.width(), view_range);
        let selection_rect = egui::Rect::from_min_max(
            egui::pos2(left, plot.top()),
            egui::pos2(right, plot.bottom()),
        );
        let selection_color = ui.visuals().selection.bg_fill;
        plot_painter.rect_filled(selection_rect, 0.0, selection_color.gamma_multiply(0.12));
        plot_painter.rect_stroke(
            selection_rect,
            0.0,
            Stroke::new(1.0_f32, selection_color.gamma_multiply(0.75)),
            egui::StrokeKind::Inside,
        );
        plot_painter.text(
            selection_rect.center_top() + egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            tf(
                "Selection · {}s - {}s",
                &[
                    &format_timeline_seconds(start),
                    &format_timeline_seconds(end),
                ],
            ),
            egui::FontId::proportional(9.5),
            selection_color,
        );
    }

    painter.text(
        rect.left_top() + egui::vec2(12.0, 12.0),
        egui::Align2::LEFT_CENTER,
        format!(
            "{} {}",
            match dps_view_mode {
                TimelineDpsViewMode::Team => t("Peak DPS"),
                TimelineDpsViewMode::Characters => t("Peak Character DPS"),
            },
            format_number(max_dps)
        ),
        egui::FontId::monospace(11.0),
        ui.visuals().selection.bg_fill,
    );
    painter.text(
        rect.right_top() + egui::vec2(-12.0, 12.0),
        egui::Align2::RIGHT_CENTER,
        tf("Total {}", &[&format_number(series.total_damage)]),
        egui::FontId::monospace(11.0),
        ui.visuals().weak_text_color(),
    );
    if matches!(dps_view_mode, TimelineDpsViewMode::Characters) {
        let mut x = plot.left();
        let mut y = rect.top() + 39.0;
        let mut row = 0;
        for (rank, (char_id, name, _)) in role_totals.iter().enumerate() {
            let color = readable_accent(
                character_color(*char_id, characters, rank, dark_mode),
                dark_mode,
            );
            let display_name = character_display_name(characters, *char_id, name);
            let label = display_name.as_str();
            let label_width = (label.chars().count() as f32 * 11.0 + 34.0).clamp(76.0, 164.0);
            if x + label_width > rect.right() - 12.0 {
                row += 1;
                if row >= 2 {
                    break;
                }
                x = plot.left();
                y += 30.0;
            }
            let item_rect = egui::Rect::from_min_size(
                egui::pos2(x - 6.0, y - 13.0),
                egui::vec2(label_width.min(rect.right() - x - 8.0), 26.0),
            );
            let response = ui.interact(
                item_rect,
                ui.make_persistent_id(("timeline_role_legend", *char_id)),
                egui::Sense::click(),
            );
            let selected = selected_char.is_some_and(|selected| selected == *char_id);
            if response.clicked() {
                *selected_char = if selected { None } else { Some(*char_id) };
            }
            let fill = if selected {
                color.gamma_multiply(0.18)
            } else if response.hovered() {
                shadcn_card_hover(dark_mode)
            } else {
                Color32::TRANSPARENT
            };
            if fill != Color32::TRANSPARENT {
                painter.rect_filled(item_rect, 6.0, fill);
            }
            if selected {
                painter.rect_stroke(
                    item_rect,
                    6.0,
                    Stroke::new(1.0_f32, color.gamma_multiply(0.8)),
                    egui::StrokeKind::Inside,
                );
            }
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(x, y - 5.0), egui::vec2(10.0, 10.0)),
                3.0,
                color,
            );
            painter.text(
                egui::pos2(x + 16.0, y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(13.0),
                shadcn_foreground(dark_mode),
            );
            response.on_hover_text(t(
                "Click to highlight this character's line; click again to clear",
            ));
            x += label_width;
        }
    }

    if let Some(pointer) = ui.ctx().pointer_hover_pos()
        && response.hovered()
        && plot.contains(pointer)
    {
        let hover_time =
            pointer_x_to_timeline_time(pointer.x, plot.left(), plot.width(), view_range);
        let bucket_index = ((hover_time / series.bucket_seconds.max(0.001)).floor() as usize)
            .min(series.buckets.len().saturating_sub(1));
        if let Some(bucket) = series.buckets.get(bucket_index) {
            let bucket_left = timeline_time_to_pointer_x(
                bucket.start_offset,
                plot.left(),
                plot.width(),
                view_range,
            );
            let bucket_right = timeline_time_to_pointer_x(
                bucket.end_offset,
                plot.left(),
                plot.width(),
                view_range,
            );
            let bucket_rect = egui::Rect::from_min_max(
                egui::pos2(bucket_left, plot.top()),
                egui::pos2(bucket_right, plot.bottom()),
            );
            plot_painter.rect_filled(
                bucket_rect,
                0.0,
                ui.visuals().selection.bg_fill.gamma_multiply(0.08),
            );
            let x = timeline_time_to_pointer_x(
                (bucket.start_offset + bucket.end_offset) * 0.5,
                plot.left(),
                plot.width(),
                view_range,
            );
            let hovered_dps = match dps_view_mode {
                TimelineDpsViewMode::Team => bucket.dps,
                TimelineDpsViewMode::Characters => bucket
                    .role_damage
                    .iter()
                    .map(|role| role.dps)
                    .fold(0.0, f64::max),
            };
            let y = plot.bottom() - (hovered_dps / max_dps) as f32 * plot.height();
            plot_painter.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0_f32, ui.visuals().selection.bg_fill.gamma_multiply(0.8)),
            );
            if hovered_dps > 0.0 {
                plot_painter.line_segment(
                    [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
                    Stroke::new(1.0_f32, ui.visuals().selection.bg_fill.gamma_multiply(0.45)),
                );
            }
            match dps_view_mode {
                TimelineDpsViewMode::Team => {
                    plot_painter.circle_filled(
                        egui::pos2(x, y),
                        4.0,
                        ui.visuals().selection.bg_fill,
                    );
                }
                TimelineDpsViewMode::Characters => {
                    for (rank, (char_id, _, _)) in role_totals.iter().enumerate() {
                        let Some(role) = bucket
                            .role_damage
                            .iter()
                            .find(|role| role.char_id == *char_id && role.dps > 0.0)
                        else {
                            continue;
                        };
                        let role_y = plot.bottom() - (role.dps / max_dps) as f32 * plot.height();
                        plot_painter.circle_filled(
                            egui::pos2(x, role_y),
                            3.0,
                            readable_accent(
                                character_color(*char_id, characters, rank, dark_mode),
                                dark_mode,
                            ),
                        );
                    }
                }
            }

            let label_pos = if x + 132.0 <= plot.right() {
                egui::pos2(x + 8.0, y - 8.0)
            } else {
                egui::pos2(x - 8.0, y - 8.0)
            };
            let align = if x + 132.0 <= plot.right() {
                egui::Align2::LEFT_BOTTOM
            } else {
                egui::Align2::RIGHT_BOTTOM
            };
            plot_painter.text(
                label_pos,
                align,
                format!(
                    "{}s · {} {}",
                    format_timeline_seconds(bucket.start_offset),
                    match dps_view_mode {
                        TimelineDpsViewMode::Team => t("DPS"),
                        TimelineDpsViewMode::Characters => t("Top Character DPS"),
                    },
                    format_number(hovered_dps)
                ),
                egui::FontId::monospace(10.0),
                shadcn_foreground(dark_mode),
            );

            let hovered_time_stop = series.time_stop_intervals.iter().copied().find(|interval| {
                hover_time >= interval.start_offset && hover_time <= interval.end_offset
            });
            if let Some(interval) = hovered_time_stop {
                let left = timeline_time_to_pointer_x(
                    interval.start_offset,
                    plot.left(),
                    plot.width(),
                    view_range,
                );
                let right = timeline_time_to_pointer_x(
                    interval.end_offset,
                    plot.left(),
                    plot.width(),
                    view_range,
                );
                let interval_rect = egui::Rect::from_min_max(
                    egui::pos2(left, plot.top()),
                    egui::pos2(right, plot.bottom()),
                );
                plot_painter.rect_filled(
                    interval_rect,
                    0.0,
                    semantic_warning(dark_mode).gamma_multiply(0.28),
                );
                plot_painter.rect_stroke(
                    interval_rect,
                    0.0,
                    Stroke::new(1.0_f32, semantic_warning(dark_mode).gamma_multiply(0.8)),
                    egui::StrokeKind::Inside,
                );
                plot_painter.text(
                    egui::pos2(interval_rect.center().x, plot.top() + 12.0),
                    egui::Align2::CENTER_CENTER,
                    tf(
                        "Time stop {}s",
                        &[&format_timeline_seconds(
                            interval.end_offset - interval.start_offset,
                        )],
                    ),
                    egui::FontId::monospace(10.0),
                    semantic_warning(dark_mode),
                );
                response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(RichText::new(t("Time-stop Interval")).strong());
                    egui::Grid::new("timeline_time_stop_hover")
                        .num_columns(2)
                        .spacing([12.0, 3.0])
                        .show(ui, |ui| {
                            ui.label(t("Range"));
                            ui.monospace(format!(
                                "{}s - {}s",
                                format_timeline_seconds(interval.start_offset),
                                format_timeline_seconds(interval.end_offset)
                            ));
                            ui.end_row();
                            ui.label(t("Duration"));
                            ui.monospace(format!(
                                "{}s",
                                format_timeline_seconds(
                                    interval.end_offset - interval.start_offset
                                )
                            ));
                            ui.end_row();
                            ui.label(t("Current Bucket"));
                            ui.monospace(format!(
                                "{}s - {}s",
                                format_timeline_seconds(bucket.start_offset),
                                format_timeline_seconds(bucket.end_offset)
                            ));
                            ui.end_row();
                            ui.label(match dps_view_mode {
                                TimelineDpsViewMode::Team => t("Bucket DPS"),
                                TimelineDpsViewMode::Characters => t("Top Character DPS"),
                            });
                            ui.monospace(format_number(hovered_dps));
                            ui.end_row();
                            ui.label(t("Bucket Damage"));
                            ui.monospace(format_number(bucket.damage));
                            ui.end_row();
                        });
                });
            } else {
                response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.label(
                        RichText::new(format!(
                            "{}s - {}s",
                            format_timeline_seconds(bucket.start_offset),
                            format_timeline_seconds(bucket.end_offset)
                        ))
                        .strong(),
                    );
                    egui::Grid::new("timeline_bucket_hover")
                        .num_columns(2)
                        .spacing([12.0, 3.0])
                        .show(ui, |ui| {
                            ui.label(match dps_view_mode {
                                TimelineDpsViewMode::Team => t("DPS"),
                                TimelineDpsViewMode::Characters => t("Top Character DPS"),
                            });
                            ui.monospace(format_number(hovered_dps));
                            ui.end_row();
                            ui.label(t("Damage"));
                            ui.monospace(format_number(bucket.damage));
                            ui.end_row();
                            ui.label(t("Hits"));
                            ui.monospace(bucket.hits.to_string());
                            ui.end_row();
                            ui.label(t("Cumulative"));
                            ui.monospace(format_number(bucket.cumulative_damage));
                            ui.end_row();
                        });
                    let mut roles = bucket.role_damage.iter().collect::<Vec<_>>();
                    roles.sort_by(|left, right| {
                        right
                            .damage
                            .total_cmp(&left.damage)
                            .then_with(|| left.char_name.cmp(&right.char_name))
                            .then_with(|| left.char_id.cmp(&right.char_id))
                    });
                    if !roles.is_empty() {
                        ui.separator();
                        for role in roles.iter().take(4) {
                            ui.horizontal(|ui| {
                                ui.label(character_display_name(
                                    characters,
                                    role.char_id,
                                    &role.char_name,
                                ));
                                ui.monospace(format!(
                                    "{} · DPS {}",
                                    format_number(role.damage),
                                    format_number(role.dps)
                                ));
                            });
                        }
                    }
                });
            }
        }
    }

    let context_time = view_state.context_time;
    let normalized_selection = view_state
        .drag_selection
        .and_then(|(start, end)| normalize_timeline_selection(start, end, duration));
    let zoom_selection = clamp_timeline_zoom(normalized_selection, duration);
    response.context_menu(|ui| {
        if ui
            .add_enabled(
                context_time.is_some(),
                egui::Button::new(t("Add marker here")),
            )
            .clicked()
        {
            view_state.add_marker(context_time.expect("enabled marker action has a chart time"));
            ui.close();
        }
        if ui
            .add_enabled(
                zoom_selection.is_some(),
                egui::Button::new(t("Zoom to selection")),
            )
            .clicked()
        {
            view_state.zoom = zoom_selection;
            view_state.drag_selection = None;
            view_state.drag_anchor = None;
            ui.close();
        }
        if ui
            .add_enabled(
                view_state.zoom.is_some(),
                egui::Button::new(t("Reset zoom")),
            )
            .clicked()
        {
            view_state.zoom = None;
            ui.close();
        }
        if ui
            .add_enabled(
                context_time.is_some() && !view_state.user_markers.is_empty(),
                egui::Button::new(t("Remove nearest marker")),
            )
            .clicked()
        {
            view_state.remove_nearest_marker(
                context_time.expect("enabled marker removal has a chart time"),
            );
            ui.close();
        }
    });
}

pub(crate) fn timeline_bucket_millis(seconds: f32) -> u32 {
    (config::sanitize_timeline_bucket_seconds(seconds) * 1000.0).round() as u32
}

pub(crate) fn format_timeline_seconds(seconds: f64) -> String {
    if seconds.abs() >= 10.0 {
        format!("{seconds:.0}")
    } else {
        format!("{seconds:.1}")
    }
}

pub(crate) fn timeline_top_roles(series: &TimelineSeries, limit: usize) -> Vec<(u32, String, f64)> {
    let mut totals = HashMap::<u32, (String, f64)>::new();
    for bucket in &series.buckets {
        for role in &bucket.role_damage {
            let entry = totals
                .entry(role.char_id)
                .or_insert_with(|| (role.char_name.clone(), 0.0));
            entry.0.clone_from(&role.char_name);
            entry.1 += role.damage;
        }
    }
    let mut roles = totals
        .into_iter()
        .map(|(char_id, (name, damage))| (char_id, name, damage))
        .collect::<Vec<_>>();
    roles.sort_by(|left, right| {
        right
            .2
            .total_cmp(&left.2)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    roles.truncate(limit);
    roles
}

pub(crate) fn draw_skill_breakdown_rows(
    ui: &mut egui::Ui,
    rows: &[&SkillBreakdownRow],
    total_damage: f64,
    max_height: f32,
    dark_mode: bool,
    characters: &HashMap<u32, CharacterInfo>,
) {
    if rows.is_empty() {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 72.0),
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.label(
                    RichText::new(t("No skill attribution for this character yet"))
                        .color(ui.visuals().weak_text_color()),
                );
            },
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("skill_breakdown_rows")
        .max_height(max_height.max(120.0))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            for (index, row) in rows.iter().enumerate() {
                let share = if total_damage > 0.0 {
                    row.damage / total_damage * 100.0
                } else {
                    0.0
                };
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 36.0),
                    egui::Sense::hover(),
                );
                let color = readable_accent(
                    character_color(row.char_id, characters, index, dark_mode),
                    dark_mode,
                );
                let fill = if response.hovered() {
                    shadcn_card_hover(dark_mode)
                } else {
                    shadcn_card(dark_mode)
                };
                ui.painter().rect_filled(rect, 6.0, fill);
                let progress = egui::Rect::from_min_max(
                    rect.min,
                    egui::pos2(
                        rect.left() + rect.width() * (share as f32 / 100.0).clamp(0.0, 1.0),
                        rect.bottom(),
                    ),
                );
                ui.painter()
                    .rect_filled(progress, 6.0, color.gamma_multiply(0.16));
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        rect.left_top(),
                        egui::pos2(rect.left() + 3.0, rect.bottom()),
                    ),
                    6.0,
                    color,
                );
                let label = if row.is_follow_up {
                    format!("{} · {}", row.name, t("follow-up"))
                } else {
                    row.name.clone()
                };
                let left_clip = egui::Rect::from_min_max(
                    rect.min + egui::vec2(10.0, 0.0),
                    egui::pos2(rect.right() - 248.0, rect.bottom()),
                );
                ui.painter().with_clip_rect(left_clip).text(
                    rect.left_center() + egui::vec2(10.0, -6.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(12.0),
                    shadcn_foreground(dark_mode),
                );
                ui.painter().with_clip_rect(left_clip).text(
                    rect.left_center() + egui::vec2(10.0, 9.0),
                    egui::Align2::LEFT_CENTER,
                    format!(
                        "{} · {}",
                        character_display_name(characters, row.char_id, &row.char_name),
                        row.category
                    ),
                    egui::FontId::proportional(10.0),
                    ui.visuals().weak_text_color(),
                );
                ui.painter().text(
                    rect.right_center() - egui::vec2(10.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    format!(
                        "{share:.1}% · {} · {}",
                        format_number(row.damage),
                        tf("{} hits", &[&row.hits.to_string()])
                    ),
                    egui::FontId::monospace(11.0),
                    shadcn_foreground(dark_mode),
                );
                response.on_hover_text(skill_breakdown_hover_text(row));
            }
        });
}

pub(crate) fn skill_breakdown_hover_text(row: &SkillBreakdownRow) -> String {
    let mut lines = vec![
        tf("Character: {}", &[&row.char_name]),
        tf("Category: {}", &[&row.category]),
        tf("Damage: {}", &[&format_number(row.damage)]),
        tf("Hits: {}", &[&row.hits.to_string()]),
    ];
    if let Some(name) = row.ability_name.as_deref() {
        lines.push(format!("GA：{name}"));
    }
    if let Some(name) = row.gameplay_effect_name.as_deref() {
        lines.push(format!("GE：{name}"));
    }
    if let Some(index) = row.gameplay_effect_index {
        lines.push(format!("GE Index：{index}"));
    }
    lines.join("\n")
}

pub(crate) fn has_unknown_attribution(breakdown: &SkillBreakdown) -> bool {
    breakdown.unknown.unknown_character_count > 0
        || breakdown.unknown.unknown_direction_hits > 0
        || breakdown.unknown.unmapped_skill_hits > 0
        || !breakdown.unknown.unmapped_gameplay_effects.is_empty()
}

pub(crate) fn draw_unknown_attribution(
    ui: &mut egui::Ui,
    breakdown: &SkillBreakdown,
    dark_mode: bool,
) {
    egui::CollapsingHeader::new(
        RichText::new(t("Pending Mapping Diagnostics"))
            .strong()
            .color(shadcn_foreground(dark_mode)),
    )
    .default_open(false)
    .show(ui, |ui| {
        egui::Grid::new("unknown_attribution_summary")
            .num_columns(2)
            .spacing([16.0, 5.0])
            .show(ui, |ui| {
                ui.label(t("Unknown Characters"));
                ui.monospace(tf(
                    "{} / {} hits",
                    &[
                        &breakdown.unknown.unknown_character_count.to_string(),
                        &breakdown.unknown.unknown_character_hits.to_string(),
                    ],
                ));
                ui.end_row();
                ui.label(t("Candidate Direction"));
                ui.monospace(tf(
                    "{} / {}",
                    &[
                        &breakdown.unknown.unknown_direction_hits.to_string(),
                        &format_number(breakdown.unknown.unknown_direction_damage),
                    ],
                ));
                ui.end_row();
                ui.label(t("Pending Skills"));
                ui.monospace(tf(
                    "{} kinds / {} hits",
                    &[
                        &breakdown.unknown.unmapped_skill_rows.to_string(),
                        &breakdown.unknown.unmapped_skill_hits.to_string(),
                    ],
                ));
                ui.end_row();
            });
        if !breakdown.unknown.unmapped_gameplay_effects.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(t("Unmapped GE")).color(ui.visuals().weak_text_color()));
            for effect in breakdown.unknown.unmapped_gameplay_effects.iter().take(24) {
                ui.horizontal(|ui| {
                    ui.monospace(tf(
                        "{} · {} hits · {}",
                        &[
                            &effect.index.to_string(),
                            &effect.hits.to_string(),
                            &format_number(effect.damage),
                        ],
                    ));
                    if ui.small_button(t("Copy")).clicked() {
                        ui.ctx().copy_text(effect.index.to_string());
                    }
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1.0e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn pointer_and_timeline_time_round_trip_with_zoom() {
        let view = (10.0, 30.0);
        let x = timeline_time_to_pointer_x(20.0, 100.0, 400.0, view);
        assert_close(x as f64, 300.0);
        assert_close(pointer_x_to_timeline_time(x, 100.0, 400.0, view), 20.0);
        assert_close(pointer_x_to_timeline_time(0.0, 100.0, 400.0, view), 10.0);
        assert_close(pointer_x_to_timeline_time(600.0, 100.0, 400.0, view), 30.0);
        assert_close(
            timeline_time_to_pointer_x(5.0, 100.0, 400.0, view) as f64,
            100.0,
        );
        assert_close(
            timeline_time_to_pointer_x(40.0, 100.0, 400.0, view) as f64,
            500.0,
        );
    }

    #[test]
    fn selection_normalization_orders_and_clamps_endpoints() {
        assert_eq!(
            normalize_timeline_selection(8.0, 2.0, 10.0),
            Some((2.0, 8.0))
        );
        assert_eq!(
            normalize_timeline_selection(-4.0, 14.0, 10.0),
            Some((0.0, 10.0))
        );
        assert_eq!(normalize_timeline_selection(3.0, 3.0, 10.0), None);
        assert_eq!(normalize_timeline_selection(f64::NAN, 3.0, 10.0), None);
        assert_eq!(normalize_timeline_selection(1.0, 3.0, 0.0), None);
    }

    #[test]
    fn zoom_clamp_rejects_empty_and_full_ranges() {
        assert_eq!(
            clamp_timeline_zoom(Some((-2.0, 4.0)), 10.0),
            Some((0.0, 4.0))
        );
        assert_eq!(
            clamp_timeline_zoom(Some((12.0, 8.0)), 10.0),
            Some((8.0, 10.0))
        );
        assert_eq!(clamp_timeline_zoom(Some((0.0, 10.0)), 10.0), None);
        assert_eq!(clamp_timeline_zoom(Some((12.0, 14.0)), 10.0), None);
        assert_eq!(clamp_timeline_zoom(None, 10.0), None);
    }

    #[test]
    fn user_markers_are_sorted_and_removed_nearest_to_context() {
        let mut state = TimelineViewState::default();
        state.add_marker(8.0);
        state.add_marker(2.0);
        state.add_marker(5.0);
        assert_eq!(state.user_markers, [2.0, 5.0, 8.0]);

        state.remove_nearest_marker(6.0);
        assert_eq!(state.user_markers, [2.0, 8.0]);
    }
}
