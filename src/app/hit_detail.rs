use super::*;

pub(crate) fn is_qte_follow_up_damage_type(attack_type: &str) -> bool {
    matches!(
        attack_type,
        "创生花" | "覆纹" | "延滞" | "黯星" | "浊燃" | "浸染" | "盈蓄" | "失谐"
    )
}

pub(crate) fn is_qte_follow_up_damage_hit(hit: &crate::engine::model::Hit) -> bool {
    hit.follow_up_attack_type
        .as_deref()
        .is_some_and(is_qte_follow_up_damage_type)
        || (!hit.char_known
            && hit
                .attack_type
                .as_deref()
                .is_some_and(is_qte_follow_up_damage_type))
}

pub(crate) fn reaction_text_key_for_hit(hit: &crate::engine::model::Hit) -> Option<u8> {
    hit.attack_type
        .as_deref()
        .and_then(reaction_text_key_from_trigger_attack_type)
}

pub(crate) fn reaction_text_key_from_trigger_attack_type(attack_type: &str) -> Option<u8> {
    let reaction = attack_type.strip_prefix("环合·")?;
    match reaction {
        "创生" | "创生花" => Some(1),
        "覆纹" => Some(2),
        "黯星" => Some(3),
        "浊燃" | "灼燃" => Some(4),
        "浸染" => Some(5),
        "延滞" => Some(6),
        "盈蓄" => Some(7),
        "失谐" => Some(8),
        _ => None,
    }
}

pub(crate) fn hit_detail_hover_text(
    hit: &crate::engine::model::Hit,
    include_character: bool,
) -> String {
    let mut lines = Vec::new();
    let type_text = match hit.direction {
        HitDirection::Incoming | HitDirection::Unknown => t(hit_type_label(hit)),
        HitDirection::Outgoing => hit_type_display_text(hit),
    };
    if include_character {
        lines.push(format!("{} · {type_text}", hit.char_name));
    } else {
        lines.push(type_text);
    }
    if hit.follow_up_damage > 0.0 {
        lines.push(tf(
            "Damage: {} + {}",
            &[
                &format_number(hit.damage),
                &format_number(hit.follow_up_damage),
            ],
        ));
    } else {
        lines.push(tf("Damage: {}", &[&format_number(hit.damage)]));
    }
    if hit.target_max_hp > 0.0 {
        lines.push(tf(
            "Target HP: {} / {}  {}%",
            &[
                &format_number(hit.target_hp_after),
                &format_number(hit.target_max_hp),
                &format!("{:.1}", hit.target_hp_percent),
            ],
        ));
    }
    if hit.direction.is_unknown() {
        lines.push(t("Direction not yet confirmed"));
    } else if let Some(ability_name) = hit.ability_name.as_deref() {
        lines.push(format!("GA：{ability_name}"));
    }
    lines.join("\n")
}

pub(crate) fn aggregate_character_skill_damage(
    hits: &std::collections::VecDeque<crate::engine::model::Hit>,
    char_id: u32,
) -> Vec<SkillDamageSummary> {
    let mut summaries = HashMap::<String, SkillDamageSummary>::new();
    for hit in hits
        .iter()
        .filter(|hit| hit.char_id == char_id && !hit.direction.is_incoming())
    {
        let name = hit_specific_type(hit).to_owned();
        let row = summaries
            .entry(name.clone())
            .or_insert_with(|| SkillDamageSummary {
                name,
                category: hit.attack_type.clone().unwrap_or_else(|| "未知".to_owned()),
                hits: 0,
                damage: 0.0,
            });
        row.hits += 1;
        row.damage += hit.total_damage();
    }
    let mut rows: Vec<_> = summaries.into_values().collect();
    rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

pub(crate) fn skill_summary_display_text(summary: &SkillDamageSummary) -> String {
    skill_name_display_text(
        summary
            .name
            .starts_with("GA_")
            .then_some(summary.name.as_str()),
        summary
            .name
            .starts_with("GE_")
            .then_some(summary.name.as_str()),
        None,
        &summary.name,
    )
}

/// Attack types shown as their own filterable chip in the "Reaction Damage"
/// summary strip instead of being folded into whoever triggered them: the
/// QTE-chain reactions plus "倾陷伤害" (Unbalance/Tenacity burst, issue #15 —
/// also excluded from personal ranking in [`is_unbalance_damage_hit`]).
fn is_shared_attribution_attack_type(attack_type: &str) -> bool {
    is_qte_follow_up_damage_type(attack_type) || attack_type == UNBALANCE_ATTACK_TYPE
}

pub(crate) fn summarize_qte_type_filters(
    hits: &VecDeque<crate::engine::model::Hit>,
    char_id: Option<u32>,
) -> Vec<QteTypeFilterSummary> {
    let mut summaries = HashMap::<String, QteTypeFilterSummary>::new();
    for hit in hits.iter().filter(|hit| {
        !hit.direction.is_incoming() && char_id.is_none_or(|char_id| hit.char_id == char_id)
    }) {
        if let Some(attack_type) = hit.attack_type.as_deref()
            && is_shared_attribution_attack_type(attack_type)
        {
            let row =
                summaries
                    .entry(attack_type.to_owned())
                    .or_insert_with(|| QteTypeFilterSummary {
                        attack_type: attack_type.to_owned(),
                        hits: 0,
                        damage: 0.0,
                    });
            row.hits += 1;
            row.damage += hit.damage;
        }
        if hit.follow_up_damage > 0.0
            && let Some(attack_type) = hit.follow_up_attack_type.as_deref()
            && is_shared_attribution_attack_type(attack_type)
        {
            let row =
                summaries
                    .entry(attack_type.to_owned())
                    .or_insert_with(|| QteTypeFilterSummary {
                        attack_type: attack_type.to_owned(),
                        hits: 0,
                        damage: 0.0,
                    });
            row.hits += 1;
            row.damage += hit.follow_up_damage;
        }
    }
    let mut rows = summaries.into_values().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.attack_type.cmp(&right.attack_type))
    });
    rows
}

pub(crate) fn hit_detail_filter_available(
    filter: &HitDetailFilter,
    qte_summaries: &[QteTypeFilterSummary],
) -> bool {
    match filter {
        HitDetailFilter::QteType(attack_type) => qte_summaries
            .iter()
            .any(|summary| summary.attack_type == *attack_type),
        _ => true,
    }
}

#[cfg(test)]
pub(crate) fn qte_type_filter_label(summary: &QteTypeFilterSummary, total_damage: f64) -> String {
    let share = if total_damage > 0.0 {
        summary.damage / total_damage * 100.0
    } else {
        0.0
    };
    format!(
        "{} {} · {share:.1}%",
        summary.attack_type,
        format_number(summary.damage)
    )
}

pub(crate) fn draw_qte_damage_summary(
    ui: &mut egui::Ui,
    qte_summaries: &[QteTypeFilterSummary],
    total_damage: f64,
    selected: &mut HitDetailFilter,
) {
    if qte_summaries.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.add(
            egui::Label::new(
                RichText::new(t("Reaction Damage"))
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            )
            .selectable(false),
        );
        for summary in qte_summaries {
            qte_damage_summary_chip(ui, summary, total_damage, selected);
        }
    });
}

pub(crate) fn qte_damage_summary_chip(
    ui: &mut egui::Ui,
    summary: &QteTypeFilterSummary,
    total_damage: f64,
    selected: &mut HitDetailFilter,
) {
    let target_filter = HitDetailFilter::QteType(summary.attack_type.clone());
    let is_selected = selected == &target_filter;
    let share = if total_damage > 0.0 {
        summary.damage / total_damage * 100.0
    } else {
        0.0
    };
    let label = translate_reaction_label(&summary.attack_type);
    let width = 156.0_f32.max(96.0 + label.chars().count() as f32 * 12.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 42.0), egui::Sense::click());
    let dark_mode = ui.visuals().dark_mode;
    let accent = ui.visuals().selection.bg_fill;
    let bg = if is_selected {
        accent
    } else if response.hovered() {
        shadcn_card_hover(dark_mode)
    } else {
        shadcn_card(dark_mode)
    };
    let text_color = if is_selected {
        contrast_text(accent)
    } else {
        shadcn_foreground(dark_mode)
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(6),
        bg,
        Stroke::new(
            1.0_f32,
            if is_selected {
                accent
            } else {
                shadcn_border(dark_mode)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let progress_rect = egui::Rect::from_min_max(
        rect.left_bottom() - egui::vec2(0.0, 3.0),
        egui::pos2(
            rect.left() + rect.width() * (share as f32 / 100.0).clamp(0.0, 1.0),
            rect.bottom(),
        ),
    );
    ui.painter().rect_filled(
        progress_rect,
        1.0,
        if is_selected {
            contrast_text(accent).gamma_multiply(0.45)
        } else {
            accent.gamma_multiply(0.55)
        },
    );
    let text_rect = rect.shrink2(egui::vec2(10.0, 5.0));
    ui.painter().text(
        egui::pos2(text_rect.left(), text_rect.top() + 9.0),
        egui::Align2::LEFT_CENTER,
        label,
        density_proportional_font(ui, 12.5),
        text_color,
    );
    ui.painter().text(
        egui::pos2(text_rect.left(), text_rect.top() + 27.0),
        egui::Align2::LEFT_CENTER,
        format!("{} · {share:.1}%", format_number(summary.damage)),
        density_monospace_font(ui, 11.0),
        if is_selected {
            contrast_text(accent).gamma_multiply(0.82)
        } else {
            ui.visuals().weak_text_color()
        },
    );
    if response
        .on_hover_text(tf(
            "{} hits · total damage {} · {}% of total",
            &[
                &summary.hits.to_string(),
                &format_number(summary.damage),
                &format!("{share:.1}"),
            ],
        ))
        .clicked()
    {
        *selected = if is_selected {
            HitDetailFilter::All
        } else {
            target_filter
        };
    }
}

pub(crate) fn detail_hits_for_source(
    state: &CombatState,
    source: HitDetailSource,
) -> &VecDeque<crate::engine::model::Hit> {
    match source {
        HitDetailSource::Global => &state.hits,
        HitDetailSource::AbyssFirst => &state.abyss.first_half.hits,
        HitDetailSource::AbyssSecond => &state.abyss.second_half.hits,
    }
}

pub(crate) fn build_hit_detail_cache(
    hits: &VecDeque<crate::engine::model::Hit>,
    generation: u64,
    key: HitDetailCacheKey,
) -> HitDetailCache {
    let mut filtered_count = 0;
    let mut rows = Vec::with_capacity(key.limit.min(hits.len()));
    for (index, hit) in hits.iter().enumerate().rev().filter(|(_, hit)| {
        key.char_id.is_none_or(|char_id| hit.char_id == char_id)
            && key.filter.matches(hit)
            && (key.skill_filter.is_empty() || hit_specific_type(hit) == key.skill_filter.as_str())
    }) {
        filtered_count += 1;
        if rows.len() < key.limit {
            rows.push(cached_hit_row(index, hit));
        }
    }

    if key.char_id.is_some() {
        rows.sort_by(compare_cached_character_hits);
    } else {
        rows.sort_by(compare_cached_team_hits);
    }

    let max_damage = rows.iter().map(|row| row.damage).fold(1.0_f64, f64::max);

    HitDetailCache {
        key: Some(key),
        generation,
        source_len: hits.len(),
        rows,
        filtered_count,
        max_damage,
        dirty_since: None,
        last_scroll_offset: None,
    }
}

pub(crate) fn cached_hit_row(index: usize, hit: &crate::engine::model::Hit) -> CachedHitRow {
    let is_incoming = hit.direction.is_incoming();
    CachedHitRow {
        index,
        is_incoming,
        damage: hit.total_damage(),
        char_id: hit.char_id,
        hp_fraction: (hit.target_hp_percent / 100.0).clamp(0.0, 1.0) as f32,
        timestamp: hit.timestamp,
        byte_offset: hit.byte_offset,
        bit_shift: hit.bit_shift,
        target_hp_after: hit.target_hp_after,
        target_max_hp: hit.target_max_hp,
    }
}

pub(crate) fn resolve_cached_hit<'a>(
    hits: &'a VecDeque<crate::engine::model::Hit>,
    row: &CachedHitRow,
    source_len: usize,
    appended: u64,
) -> Option<&'a crate::engine::model::Hit> {
    let appended = usize::try_from(appended).unwrap_or(usize::MAX);
    adjusted_cached_index(row.index, source_len, hits.len(), appended)
        .and_then(|index| hits.get(index))
        .filter(|hit| cached_hit_matches(row, hit))
        .or_else(|| {
            hits.get(row.index)
                .filter(|hit| cached_hit_matches(row, hit))
        })
}

pub(crate) fn cached_hit_matches(row: &CachedHitRow, hit: &crate::engine::model::Hit) -> bool {
    row.char_id == hit.char_id
        && (row.timestamp - hit.timestamp).abs() <= 0.001
        && row.byte_offset == hit.byte_offset
        && row.bit_shift == hit.bit_shift
        && (row.target_max_hp - hit.target_max_hp).abs() <= 0.5
}

pub(crate) fn adjusted_cached_index(
    index: usize,
    source_len: usize,
    current_len: usize,
    appended: usize,
) -> Option<usize> {
    let popped = source_len
        .saturating_add(appended)
        .saturating_sub(current_len);
    index.checked_sub(popped)
}

pub(crate) fn compare_cached_character_hits(
    left: &CachedHitRow,
    right: &CachedHitRow,
) -> std::cmp::Ordering {
    (left.timestamp.floor() as i64)
        .cmp(&(right.timestamp.floor() as i64))
        .then_with(|| u8::from(left.is_incoming).cmp(&u8::from(right.is_incoming)))
        .then_with(|| cached_health_pool_key(left).cmp(&cached_health_pool_key(right)))
        .then_with(|| right.target_hp_after.total_cmp(&left.target_hp_after))
        .then_with(|| left.timestamp.total_cmp(&right.timestamp))
        .then_with(|| left.byte_offset.cmp(&right.byte_offset))
        .then_with(|| left.bit_shift.cmp(&right.bit_shift))
        .then_with(|| left.damage.total_cmp(&right.damage))
}

pub(crate) fn compare_cached_team_hits(
    left: &CachedHitRow,
    right: &CachedHitRow,
) -> std::cmp::Ordering {
    (left.timestamp.floor() as i64)
        .cmp(&(right.timestamp.floor() as i64))
        .then_with(|| {
            u8::from(left.target_hp_after <= 0.0 || left.hp_fraction <= 0.0).cmp(&u8::from(
                right.target_hp_after <= 0.0 || right.hp_fraction <= 0.0,
            ))
        })
        .then_with(|| cached_health_pool_key(left).cmp(&cached_health_pool_key(right)))
        .then_with(|| right.target_hp_after.total_cmp(&left.target_hp_after))
        .then_with(|| left.timestamp.total_cmp(&right.timestamp))
        .then_with(|| left.byte_offset.cmp(&right.byte_offset))
        .then_with(|| left.bit_shift.cmp(&right.bit_shift))
        .then_with(|| left.char_id.cmp(&right.char_id))
        .then_with(|| right.is_incoming.cmp(&left.is_incoming))
        .then_with(|| left.damage.total_cmp(&right.damage))
}

pub(crate) fn cached_health_pool_key(row: &CachedHitRow) -> i64 {
    if row.target_max_hp.is_finite() && row.target_max_hp > 0.0 {
        row.target_max_hp.round() as i64
    } else {
        i64::MIN
    }
}

pub(crate) fn draw_skill_damage_summary(
    ui: &mut egui::Ui,
    summaries: &[SkillDamageSummary],
    total_damage: f64,
    selected_skill: &mut String,
    dark_mode: bool,
) {
    egui::CollapsingHeader::new(
        RichText::new(t("Move Output Composition"))
            .strong()
            .color(shadcn_foreground(dark_mode)),
    )
    .default_open(true)
    .show(ui, |ui| {
        let header_width = ui.available_width() - ui.style().spacing.scroll.allocated_width();
        let (header_rect, _) =
            ui.allocate_exact_size(egui::vec2(header_width, 24.0), egui::Sense::hover());
        let header_font = density_proportional_font(ui, 12.0);
        let header_color = ui.visuals().weak_text_color();
        ui.painter().text(
            header_rect.left_center() + egui::vec2(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            t("Specific Move"),
            header_font.clone(),
            header_color,
        );
        ui.painter().text(
            header_rect.right_center() - egui::vec2(10.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            t("Share / Total / Count"),
            header_font,
            header_color,
        );
        egui::ScrollArea::vertical()
            .id_salt("skill_damage_summary")
            .max_height(190.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                for (rank, summary) in summaries.iter().enumerate() {
                    let display_name = skill_summary_display_text(summary);
                    let share = if total_damage > 0.0 {
                        summary.damage / total_damage * 100.0
                    } else {
                        0.0
                    };
                    let selected = selected_skill == &summary.name;
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 34.0),
                        egui::Sense::click(),
                    );
                    let corner_radius = egui::CornerRadius::same(6);
                    let base_color = if selected {
                        shadcn_muted(dark_mode)
                    } else if response.hovered() {
                        shadcn_card_hover(dark_mode)
                    } else {
                        shadcn_card(dark_mode)
                    };
                    ui.painter().rect_filled(rect, corner_radius, base_color);
                    let progress_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(
                            rect.left() + rect.width() * (share as f32 / 100.0).clamp(0.0, 1.0),
                            rect.bottom(),
                        ),
                    );
                    ui.painter().rect_filled(
                        progress_rect,
                        corner_radius,
                        ui.visuals().selection.bg_fill.gamma_multiply(if selected {
                            0.28
                        } else {
                            0.16
                        }),
                    );
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            corner_radius,
                            Stroke::new(1.0_f32, ui.visuals().selection.bg_fill),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let foreground = shadcn_foreground(dark_mode);
                    let metrics_width = 230.0_f32.min(rect.width() * 0.48);
                    let left_clip = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right() - metrics_width - 8.0, rect.bottom()),
                    );
                    ui.painter().with_clip_rect(left_clip).text(
                        rect.left_center() + egui::vec2(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{}. {}  [{}]", rank + 1, display_name, summary.category),
                        density_proportional_font(ui, 12.0),
                        foreground,
                    );
                    let metrics_clip = egui::Rect::from_min_max(
                        egui::pos2(rect.right() - metrics_width, rect.top()),
                        rect.max,
                    );
                    ui.painter().with_clip_rect(metrics_clip).text(
                        rect.right_center() - egui::vec2(10.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        format!(
                            "{share:.1}%  ·  {}  ·  {}",
                            format_number(summary.damage),
                            tf("{} hits", &[&summary.hits.to_string()])
                        ),
                        density_monospace_font(ui, 11.5),
                        foreground,
                    );
                    if response.clicked() {
                        if selected {
                            selected_skill.clear();
                        } else {
                            selected_skill.clone_from(&summary.name);
                        }
                    }
                }
            });
    });
}

#[derive(Clone, Copy, Debug)]
struct HitColumnGeometry {
    left: f32,
    width: f32,
}

impl HitColumnGeometry {
    fn content_x(self) -> f32 {
        self.left + 10.0
    }

    fn content_rect(self, row: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(
            egui::pos2(row.left() + self.left + 6.0, row.top()),
            egui::pos2(row.left() + self.left + self.width - 6.0, row.bottom()),
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct HitTableLayout {
    row_width: f32,
    columns: [Option<HitColumnGeometry>; 5],
    separators: [Option<f32>; 4],
}

pub(crate) type CharacterHitLayout = HitTableLayout;
pub(crate) type TeamHitLayout = HitTableLayout;

pub(crate) struct TeamHitRowAssets<'a> {
    /// Character display name in the active UI language (resolved by the caller).
    pub(crate) char_name: &'a str,
    pub(crate) avatar_texture: Option<&'a egui::TextureHandle>,
    pub(crate) damage_digits: Option<&'a [egui::TextureHandle]>,
    pub(crate) follow_up_damage_digits: Option<&'a [egui::TextureHandle]>,
    pub(crate) reaction_textures: &'a HashMap<u8, Vec<egui::TextureHandle>>,
}

fn hit_column_index(column: HitDetailColumn) -> usize {
    match column {
        HitDetailColumn::Time => 0,
        HitDetailColumn::Character => 1,
        HitDetailColumn::Type => 2,
        HitDetailColumn::Damage => 3,
        HitDetailColumn::TargetHp => 4,
    }
}

impl HitTableLayout {
    fn new(
        available_width: f32,
        config: HitDetailColumnsConfig,
        order: &[HitDetailColumn],
    ) -> Self {
        const LEFT_INSET: f32 = 4.0;
        const MIN_COLUMN_WIDTH: f32 = 64.0;

        let visible = order
            .iter()
            .copied()
            .filter(|column| config.visible(*column))
            .collect::<Vec<_>>();
        let mut columns = [None; 5];
        let mut separators = [None; 4];
        let mut left = LEFT_INSET;
        let mut remaining = (available_width - LEFT_INSET).max(0.0);
        for (index, column) in visible.iter().copied().enumerate() {
            let remaining_columns = visible.len() - index;
            let width = if remaining_columns == 1 {
                remaining
            } else {
                let reserved = MIN_COLUMN_WIDTH * (remaining_columns - 1) as f32;
                f32::from(config.width(column)).clamp(
                    MIN_COLUMN_WIDTH,
                    (remaining - reserved).max(MIN_COLUMN_WIDTH),
                )
            };
            if index > 0 {
                separators[index - 1] = Some(left);
            }
            columns[hit_column_index(column)] = Some(HitColumnGeometry { left, width });
            left += width;
            remaining = (remaining - width).max(0.0);
        }
        Self {
            row_width: available_width,
            columns,
            separators,
        }
    }

    fn column(self, column: HitDetailColumn) -> Option<HitColumnGeometry> {
        self.columns[hit_column_index(column)]
    }
}

pub(crate) fn character_hit_layout(
    available_width: f32,
    config: HitDetailColumnsConfig,
) -> CharacterHitLayout {
    HitTableLayout::new(
        available_width,
        config,
        &[
            HitDetailColumn::Time,
            HitDetailColumn::Type,
            HitDetailColumn::Damage,
            HitDetailColumn::TargetHp,
        ],
    )
}

pub(crate) fn team_hit_layout(
    available_width: f32,
    config: HitDetailColumnsConfig,
) -> TeamHitLayout {
    HitTableLayout::new(
        available_width,
        config,
        &[
            HitDetailColumn::Time,
            HitDetailColumn::Character,
            HitDetailColumn::Type,
            HitDetailColumn::Damage,
            HitDetailColumn::TargetHp,
        ],
    )
}

fn draw_hit_table_header(
    ui: &mut egui::Ui,
    layout: HitTableLayout,
    config: &mut HitDetailColumnsConfig,
    order: &[HitDetailColumn],
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::Label::new(
                RichText::new(t("Drag column dividers to resize"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            )
            .selectable(false),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(t("Column settings"), |ui| {
                draw_hit_column_menu(ui, config, order);
            });
        });
    });
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(layout.row_width, 24.0), egui::Sense::click());
    let painter = ui.painter().clone();
    let font = density_proportional_font(ui, 12.0);
    let color = ui.visuals().weak_text_color();
    draw_hit_column_separators(&painter, rect, layout);

    let visible = order
        .iter()
        .copied()
        .filter(|column| layout.column(*column).is_some())
        .collect::<Vec<_>>();
    for (index, column) in visible.iter().copied().enumerate() {
        let geometry = layout
            .column(column)
            .expect("visible hit-detail column has geometry");
        painter.text(
            egui::pos2(rect.left() + geometry.content_x(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            t(column.label()),
            font.clone(),
            color,
        );
        if index + 1 < visible.len() {
            let divider_x = rect.left() + geometry.left + geometry.width;
            let divider = egui::Rect::from_center_size(
                egui::pos2(divider_x, rect.center().y),
                egui::vec2(8.0, rect.height()),
            );
            let resize = ui
                .interact(
                    divider,
                    ui.make_persistent_id(("hit_detail_column_resize", column)),
                    egui::Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if resize.hovered() || resize.dragged() {
                let highlight = ui.visuals().selection.stroke.color;
                painter.line_segment(
                    [
                        egui::pos2(divider_x, rect.top()),
                        egui::pos2(divider_x, rect.bottom()),
                    ],
                    Stroke::new(2.0_f32, highlight),
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(
                        egui::pos2(divider_x, rect.center().y),
                        egui::vec2(4.0, 12.0),
                    ),
                    2.0,
                    highlight,
                );
            }
            if resize.dragged() {
                let width = (f32::from(config.width(column)) + resize.drag_motion().x)
                    .round()
                    .max(0.0) as u16;
                config.set_width(column, width);
            }
            if resize.double_clicked() {
                config.set_width(column, HitDetailColumnsConfig::default().width(column));
            }
        }
    }
    response
        .on_hover_text(t("Right-click to choose columns; drag dividers to resize"))
        .context_menu(|ui| {
            draw_hit_column_menu(ui, config, order);
        });
}

fn draw_hit_column_menu(
    ui: &mut egui::Ui,
    config: &mut HitDetailColumnsConfig,
    order: &[HitDetailColumn],
) {
    for column in order {
        let mut visible = config.visible(*column);
        if ui.checkbox(&mut visible, t(column.label())).changed() {
            config.set_visible(*column, visible);
        }
    }
    ui.separator();
    if ui.button(t("Reset column widths")).clicked() {
        let defaults = HitDetailColumnsConfig::default();
        for column in order {
            config.set_width(*column, defaults.width(*column));
        }
        ui.close();
    }
}

pub(crate) fn draw_character_hit_header(
    ui: &mut egui::Ui,
    layout: CharacterHitLayout,
    config: &mut HitDetailColumnsConfig,
) {
    draw_hit_table_header(
        ui,
        layout,
        config,
        &[
            HitDetailColumn::Time,
            HitDetailColumn::Type,
            HitDetailColumn::Damage,
            HitDetailColumn::TargetHp,
        ],
    );
}

pub(crate) fn damage_digit_textures_for_hit<'a>(
    hit: &crate::engine::model::Hit,
    characters: &HashMap<u32, CharacterInfo>,
    damage_digit_textures: &'a HashMap<String, Vec<egui::TextureHandle>>,
) -> Option<&'a [egui::TextureHandle]> {
    damage_digit_key_for_hit(hit, characters)
        .and_then(|key| damage_digit_textures.get(key))
        .map(Vec::as_slice)
}

pub(crate) fn follow_up_damage_digit_textures_for_hit<'a>(
    hit: &crate::engine::model::Hit,
    damage_digit_textures: &'a HashMap<String, Vec<egui::TextureHandle>>,
) -> Option<&'a [egui::TextureHandle]> {
    follow_up_damage_digit_key_for_hit(hit)
        .and_then(|key| damage_digit_textures.get(key))
        .map(Vec::as_slice)
}

pub(crate) fn damage_digit_key_for_hit<'a>(
    hit: &'a crate::engine::model::Hit,
    characters: &'a HashMap<u32, CharacterInfo>,
) -> Option<&'a str> {
    if hit.direction.is_incoming() {
        return Some("HP");
    }
    let source_attribute = hit.damage_attribute.as_deref().or_else(|| {
        characters
            .get(&hit.char_id)
            .and_then(|character| character.attribute.as_deref())
    });
    let attack_type = hit.attack_type.as_deref();

    if attack_type.is_some_and(|attack_type| attack_type == "倾陷伤害")
        || hit
            .damage_name
            .as_deref()
            .is_some_and(|damage_name| damage_name.contains("倾陷"))
    {
        return Some("真实");
    }

    attack_type
        .and_then(|attack_type| mixed_damage_digit_key(attack_type, source_attribute))
        .or(source_attribute)
}

pub(crate) fn follow_up_damage_digit_key_for_hit(hit: &crate::engine::model::Hit) -> Option<&str> {
    let source_attribute = hit.follow_up_damage_attribute.as_deref()?;
    hit.follow_up_attack_type
        .as_deref()
        .and_then(|attack_type| mixed_damage_digit_key(attack_type, Some(source_attribute)))
        .or(Some(source_attribute))
}

pub(crate) fn mixed_damage_digit_key(
    attack_type: &str,
    source_attribute: Option<&str>,
) -> Option<&'static str> {
    // 触发环合的那一下伤害（attack_type 形如 "环合·创生"）是造成伤害角色自己打出的
    // 攻击，跳字应使用该角色的属性字系，而不是环合反应字系。这里返回 None，让调用方
    // 回退到 source_attribute（造成伤害角色的属性）。只有环合之后产生的反应伤害本体
    // （attack_type 为不带 "环合·" 前缀的反应名）才使用下面固定的反应字系。
    if attack_type.starts_with("环合·") {
        return None;
    }
    match attack_type {
        // 环合反应伤害本体的跳字固定为触发侧属性的字系，与该伤害最终记给环合双方
        // 哪一名角色无关。每个反应都只用属性对里的一侧：
        //   创生 (光灵) -> 恒为 Guangling_G（光），不再出现 Guangling_L
        //   覆纹 (灵咒) -> 恒为 lingzhou_L（灵），不再出现 lingzhou_Z
        //   黯星 (暗魂) -> 恒为 Anhun_A（暗），不再出现 Anhun_H
        //   浊燃 (咒暗) -> 恒为 Zhouan_A（暗），不再出现 Zhouan_Z
        "创生" | "创生花" => Some("Guangling_G"),
        "覆纹" => Some("lingzhou_L"),
        "黯星" => Some("Anhun_A"),
        "浊燃" => Some("Zhouan_A"),
        "延滞" => match source_attribute? {
            "光" => Some("Guangxiang_G"),
            "相" => Some("Guangxiang_X"),
            _ => None,
        },
        "浸染" | "魂相" => match source_attribute? {
            "魂" => Some("Hunxiang_H"),
            "相" => Some("Hunxiang_X"),
            _ => None,
        },
        // 盈蓄 / 失谐 only keep the reaction series whose digit PNGs still exist
        // (the trigger-side ones). The removed `_L`/`_H`/`_Z` sides fall through
        // to `None`, so the caller uses the credited character's plain element
        // digits instead of a missing texture.
        "盈蓄" => match source_attribute? {
            "光" => Some("Guangling_G"),
            "相" => Some("Guangxiang_X"),
            _ => None,
        },
        "失谐" => match source_attribute? {
            "暗" => Some("Anhun_A"),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn draw_damage_number(
    ui: &egui::Ui,
    rect: egui::Rect,
    value: f64,
    damage_digits: Option<&[egui::TextureHandle]>,
    fallback_color: Color32,
) -> egui::Rect {
    let text = damage_number_digits_text(value);
    let Some(damage_digits) = damage_digits.filter(|digits| digits.len() == 10) else {
        return draw_damage_number_fallback(ui, rect, &text, fallback_color);
    };
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return draw_damage_number_fallback(ui, rect, &text, fallback_color);
    }

    let base_height = (rect.height() - 10.0).clamp(12.0, 22.0);
    let Some(base_width) = damage_number_image_width(&text, damage_digits, base_height) else {
        return draw_damage_number_fallback(ui, rect, &text, fallback_color);
    };
    if base_width <= 0.0 {
        return draw_damage_number_fallback(ui, rect, &text, fallback_color);
    }

    let height = if base_width > rect.width() {
        (base_height * rect.width() / base_width).max(10.0)
    } else {
        base_height
    };
    let Some(total_width) = damage_number_image_width(&text, damage_digits, height) else {
        return draw_damage_number_fallback(ui, rect, &text, fallback_color);
    };

    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let mut cursor = rect.left();
    let top = rect.center().y - height * 0.5;
    let drawn_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left(), top),
        egui::vec2(total_width, height),
    );
    for digit in text.bytes().map(|byte| (byte - b'0') as usize) {
        let texture = &damage_digits[digit];
        let size = texture.size_vec2();
        if size.y <= 0.0 {
            return draw_damage_number_fallback(ui, rect, &text, fallback_color);
        }
        let width = size.x / size.y * height;
        let digit_rect =
            egui::Rect::from_min_size(egui::pos2(cursor, top), egui::vec2(width, height));
        painter.image(
            texture.id(),
            digit_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        cursor += width;
        if cursor - rect.left() >= total_width {
            break;
        }
    }
    drawn_rect
}

pub(crate) fn damage_number_image_width(
    text: &str,
    damage_digits: &[egui::TextureHandle],
    height: f32,
) -> Option<f32> {
    let mut width = 0.0;
    for digit in text.bytes().map(|byte| (byte - b'0') as usize) {
        let size = damage_digits.get(digit)?.size_vec2();
        if size.y <= 0.0 {
            return None;
        }
        width += size.x / size.y * height;
    }
    Some(width)
}

pub(crate) fn damage_number_digits_text(value: f64) -> String {
    let rounded = value.round() as i64;
    if rounded < 0 {
        format!("-{}", rounded.unsigned_abs())
    } else {
        rounded.to_string()
    }
}

pub(crate) fn draw_damage_number_fallback(
    ui: &egui::Ui,
    rect: egui::Rect,
    text: &str,
    color: Color32,
) -> egui::Rect {
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        density_monospace_font(ui, 15.0),
        color,
    );
    let width = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), density_monospace_font(ui, 15.0), color)
            .size()
            .x
    });
    egui::Rect::from_center_size(
        egui::pos2(rect.left() + width * 0.5, rect.center().y),
        egui::vec2(width, 18.0),
    )
}

pub(crate) fn draw_follow_up_damage_badge(
    ui: &egui::Ui,
    damage_cell_rect: egui::Rect,
    base_damage_rect: egui::Rect,
    hit: &crate::engine::model::Hit,
    damage_digits: Option<&[egui::TextureHandle]>,
    fallback_color: Color32,
) {
    if hit.follow_up_damage <= 0.0 {
        return;
    }
    let badge_height = 15.0_f32.min((damage_cell_rect.height() - 8.0).max(12.0));
    let text = damage_number_digits_text(hit.follow_up_damage);
    let width = damage_digits
        .filter(|digits| digits.len() == 10)
        .and_then(|digits| damage_number_image_width(&text, digits, badge_height))
        .unwrap_or_else(|| (text.chars().count() as f32 * 8.0).max(16.0));
    let left = (base_damage_rect.right() - width * 0.18)
        .max(damage_cell_rect.left())
        .min((damage_cell_rect.right() - width).max(damage_cell_rect.left()));
    let top = (base_damage_rect.top() - badge_height * 0.35).max(damage_cell_rect.top() + 1.0);
    let badge_rect =
        egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, badge_height));
    draw_damage_number(
        ui,
        badge_rect,
        hit.follow_up_damage,
        damage_digits,
        fallback_color,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_character_hit_row(
    ui: &mut egui::Ui,
    layout: CharacterHitLayout,
    hit: &crate::engine::model::Hit,
    max_damage: f64,
    damage_digits: Option<&[egui::TextureHandle]>,
    follow_up_damage_digits: Option<&[egui::TextureHandle]>,
    reaction_textures: &HashMap<u8, Vec<egui::TextureHandle>>,
    row_height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(layout.row_width, row_height),
        egui::Sense::hover(),
    );
    let incoming = hit.direction.is_incoming();
    let type_color = match hit.direction {
        HitDirection::Incoming => semantic_danger(ui.visuals().dark_mode),
        HitDirection::Unknown => semantic_warning(ui.visuals().dark_mode),
        HitDirection::Outgoing => hit_output_badge_color(ui.visuals().dark_mode),
    };
    ui.painter().rect_filled(
        rect,
        5.0,
        theme_tokens(ui.visuals().dark_mode, AccentColor::Zinc).detail_row,
    );
    let damage_fraction = (hit.total_damage() / max_damage).clamp(0.0, 1.0) as f32;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width() * damage_fraction, rect.height()),
        ),
        5.0,
        type_color.gamma_multiply(if ui.visuals().dark_mode { 0.12 } else { 0.08 }),
    );
    let y = rect.center().y;
    let x = rect.left();
    let painter = ui.painter().clone();
    let text_color = ui.visuals().text_color();
    let damage_color = if incoming {
        semantic_danger(ui.visuals().dark_mode)
    } else {
        hit_output_text_color(ui.visuals().dark_mode)
    };
    let mono = density_monospace_font(ui, 13.0);
    draw_hit_column_separators(&painter, rect, layout);
    if let Some(column) = layout.column(HitDetailColumn::Time) {
        painter.text(
            egui::pos2(x + column.content_x(), y),
            egui::Align2::LEFT_CENTER,
            format_short_time(hit.timestamp),
            mono.clone(),
            text_color,
        );
    }
    if let Some(column) = layout.column(HitDetailColumn::Type) {
        let badge_rect = column.content_rect(rect).shrink2(egui::vec2(4.0, 2.0));
        painter.rect_filled(badge_rect, 10.0, type_color);
        draw_hit_type_badge_content(ui, badge_rect, hit, type_color, reaction_textures);
    }
    if let Some(column) = layout.column(HitDetailColumn::Damage) {
        let damage_cell_rect = column.content_rect(rect);
        let base_damage_rect = draw_damage_number(
            ui,
            damage_cell_rect,
            hit.damage,
            damage_digits,
            damage_color,
        );
        draw_follow_up_damage_badge(
            ui,
            damage_cell_rect,
            base_damage_rect,
            hit,
            follow_up_damage_digits,
            damage_color,
        );
    }
    if let Some(column) = layout.column(HitDetailColumn::TargetHp) {
        let hp_fraction = (hit.target_hp_percent / 100.0).clamp(0.0, 1.0) as f32;
        let hp_cell_rect = column.content_rect(rect).shrink2(egui::vec2(0.0, 2.0));
        painter.rect_filled(hp_cell_rect, 4.0, ui.visuals().faint_bg_color);
        painter.rect_filled(
            egui::Rect::from_min_size(
                hp_cell_rect.min,
                egui::vec2(hp_cell_rect.width() * hp_fraction, hp_cell_rect.height()),
            ),
            4.0,
            if hp_fraction > 0.5 {
                semantic_success(ui.visuals().dark_mode).gamma_multiply(0.16)
            } else if hp_fraction > 0.2 {
                semantic_warning(ui.visuals().dark_mode).gamma_multiply(0.16)
            } else {
                semantic_danger(ui.visuals().dark_mode).gamma_multiply(0.16)
            },
        );
        draw_target_hp_text(ui, hp_cell_rect, hit, text_color, mono.clone());
    }
    response.on_hover_text(hit_detail_hover_text(hit, false));
}

pub(crate) fn draw_team_hit_header(
    ui: &mut egui::Ui,
    layout: TeamHitLayout,
    config: &mut HitDetailColumnsConfig,
) {
    draw_hit_table_header(
        ui,
        layout,
        config,
        &[
            HitDetailColumn::Time,
            HitDetailColumn::Character,
            HitDetailColumn::Type,
            HitDetailColumn::Damage,
            HitDetailColumn::TargetHp,
        ],
    );
}

pub(crate) fn draw_hit_type_badge_content(
    ui: &mut egui::Ui,
    badge_rect: egui::Rect,
    hit: &crate::engine::model::Hit,
    type_color: Color32,
    reaction_textures: &HashMap<u8, Vec<egui::TextureHandle>>,
) {
    if hit.direction.is_outgoing()
        && let Some(textures) =
            reaction_text_key_for_hit(hit).and_then(|key| reaction_textures.get(&key))
        && !textures.is_empty()
    {
        draw_reaction_text_images(ui, badge_rect.shrink2(egui::vec2(8.0, 3.0)), textures);
        return;
    }
    let text = match hit.direction {
        HitDirection::Incoming | HitDirection::Unknown => t(hit_type_label(hit)),
        HitDirection::Outgoing => hit_type_display_text(hit),
    };
    draw_clipped_label(
        ui,
        badge_rect.shrink2(egui::vec2(8.0, 0.0)),
        &text,
        density_proportional_font(ui, 12.0),
        contrast_text(type_color),
        egui::Align::Center,
        None,
    );
}

pub(crate) fn draw_reaction_text_images(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    textures: &[egui::TextureHandle],
) {
    let gap = 2.0;
    let mut height = rect.height().clamp(1.0, 19.0);
    let mut widths = textures
        .iter()
        .map(|texture| {
            let size = texture.size_vec2();
            if size.y > 0.0 {
                size.x / size.y * height
            } else {
                height
            }
        })
        .collect::<Vec<_>>();
    let total_width = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1) as f32);
    if total_width > rect.width() && total_width > 0.0 {
        let scale = rect.width() / total_width;
        height *= scale;
        for width in &mut widths {
            *width *= scale;
        }
    }
    let total_width = widths.iter().sum::<f32>() + gap * (widths.len().saturating_sub(1) as f32);
    let mut left = rect.center().x - total_width * 0.5;
    let top = rect.center().y - height * 0.5;
    let painter = ui.painter().with_clip_rect(rect);
    for (texture, width) in textures.iter().zip(widths) {
        let image_rect =
            egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, height));
        painter.image(
            texture.id(),
            image_rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        left += width + gap;
    }
}

pub(crate) fn draw_team_hit_row(
    ui: &mut egui::Ui,
    layout: TeamHitLayout,
    hit: &crate::engine::model::Hit,
    max_damage: f64,
    character_color: Color32,
    assets: TeamHitRowAssets<'_>,
    row_height: f32,
) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(layout.row_width, row_height),
        egui::Sense::hover(),
    );
    let incoming = hit.direction.is_incoming();
    let type_color = match hit.direction {
        HitDirection::Incoming => semantic_danger(ui.visuals().dark_mode),
        HitDirection::Unknown => semantic_warning(ui.visuals().dark_mode),
        HitDirection::Outgoing => hit_output_badge_color(ui.visuals().dark_mode),
    };
    ui.painter().rect_filled(
        rect,
        5.0,
        theme_tokens(ui.visuals().dark_mode, AccentColor::Zinc).detail_row,
    );
    let damage_fraction = (hit.total_damage() / max_damage).clamp(0.0, 1.0) as f32;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width() * damage_fraction, rect.height()),
        ),
        5.0,
        type_color.gamma_multiply(if ui.visuals().dark_mode { 0.12 } else { 0.08 }),
    );
    let y = rect.center().y;
    let x = rect.left();
    let painter = ui.painter().clone();
    let text_color = ui.visuals().text_color();
    let mono = density_monospace_font(ui, 13.0);
    draw_team_hit_column_separators(&painter, rect, layout);

    if let Some(column) = layout.column(HitDetailColumn::Time) {
        painter.text(
            egui::pos2(x + column.content_x(), y),
            egui::Align2::LEFT_CENTER,
            format_short_time(hit.timestamp),
            mono.clone(),
            text_color,
        );
    }
    if let Some(column) = layout.column(HitDetailColumn::Character) {
        let column_rect = column.content_rect(rect);
        let avatar_rect = pixel_aligned_rect(
            egui::pos2(column_rect.left(), y - 16.0),
            32.0,
            ui.ctx().pixels_per_point(),
        );
        painter.rect_filled(
            avatar_rect,
            7.0,
            theme_tokens(ui.visuals().dark_mode, AccentColor::Zinc).border_strong,
        );
        if let Some(texture) = assets.avatar_texture {
            ui.put(
                avatar_rect,
                egui::Image::new((texture.id(), avatar_rect.size())).corner_radius(7),
            );
        } else {
            painter.rect_filled(avatar_rect, 7.0, character_color.gamma_multiply(0.82));
            painter.text(
                avatar_rect.center(),
                egui::Align2::CENTER_CENTER,
                assets.char_name.chars().next().unwrap_or('?').to_string(),
                density_proportional_font(ui, 14.0),
                contrast_text(character_color),
            );
        }
        painter.rect_stroke(
            avatar_rect,
            7.0,
            Stroke::new(1.5_f32, character_color),
            egui::StrokeKind::Inside,
        );
        let name_clip = egui::Rect::from_min_max(
            egui::pos2(avatar_rect.right() + 7.0, column_rect.top()),
            column_rect.right_bottom(),
        );
        painter.with_clip_rect(name_clip).text(
            egui::pos2(name_clip.left(), y),
            egui::Align2::LEFT_CENTER,
            assets.char_name,
            density_proportional_font(ui, 12.0),
            text_color,
        );
    }
    if let Some(column) = layout.column(HitDetailColumn::Type) {
        let badge_rect = column.content_rect(rect).shrink2(egui::vec2(4.0, 2.0));
        painter.rect_filled(badge_rect, 10.0, type_color);
        draw_hit_type_badge_content(ui, badge_rect, hit, type_color, assets.reaction_textures);
    }
    let follow_up_color = if incoming {
        semantic_danger(ui.visuals().dark_mode)
    } else {
        hit_output_text_color(ui.visuals().dark_mode)
    };
    if let Some(column) = layout.column(HitDetailColumn::Damage) {
        let damage_cell_rect = column.content_rect(rect);
        let base_damage_rect = draw_damage_number(
            ui,
            damage_cell_rect,
            hit.damage,
            assets.damage_digits,
            follow_up_color,
        );
        draw_follow_up_damage_badge(
            ui,
            damage_cell_rect,
            base_damage_rect,
            hit,
            assets.follow_up_damage_digits,
            follow_up_color,
        );
    }

    if let Some(column) = layout.column(HitDetailColumn::TargetHp) {
        let hp_fraction = (hit.target_hp_percent / 100.0).clamp(0.0, 1.0) as f32;
        let hp_cell_rect = column.content_rect(rect).shrink2(egui::vec2(0.0, 2.0));
        painter.rect_filled(hp_cell_rect, 4.0, ui.visuals().faint_bg_color);
        painter.rect_filled(
            egui::Rect::from_min_size(
                hp_cell_rect.min,
                egui::vec2(hp_cell_rect.width() * hp_fraction, hp_cell_rect.height()),
            ),
            4.0,
            if hp_fraction > 0.5 {
                semantic_success(ui.visuals().dark_mode).gamma_multiply(0.16)
            } else if hp_fraction > 0.2 {
                semantic_warning(ui.visuals().dark_mode).gamma_multiply(0.16)
            } else {
                semantic_danger(ui.visuals().dark_mode).gamma_multiply(0.16)
            },
        );
        draw_target_hp_text(ui, hp_cell_rect, hit, text_color, mono);
    }
    response.on_hover_text(hit_detail_hover_text(hit, true));
}

pub(crate) fn draw_hit_metric_row(ui: &mut egui::Ui, metrics: [(&str, String, Color32); 5]) {
    const CARD_HEIGHT: f32 = 56.0;

    ui.columns(5, |columns| {
        for (column, (label, value, color)) in columns.iter_mut().zip(metrics) {
            hit_metric_card_sized(
                column,
                label,
                &value,
                color,
                egui::vec2(column.available_width(), CARD_HEIGHT),
            );
        }
    });
}

pub(crate) fn hit_metric_card_sized(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    color: Color32,
    size: egui::Vec2,
) {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(
        rect,
        6.0,
        shadcn_card_hover(ui.visuals().dark_mode),
        Stroke::new(1.0_f32, shadcn_border(ui.visuals().dark_mode)),
        egui::StrokeKind::Inside,
    );
    let content_rect = rect.shrink2(egui::vec2(8.0, 5.0));
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
        |ui| {
            ui.set_clip_rect(content_rect);
            ui.add_sized(
                egui::vec2(content_rect.width(), 24.0),
                egui::Label::new(
                    RichText::new(value)
                        .monospace()
                        .size(15.0)
                        .strong()
                        .color(color),
                )
                .truncate()
                .halign(egui::Align::Center),
            );
            ui.add_sized(
                egui::vec2(content_rect.width(), 16.0),
                egui::Label::new(
                    RichText::new(label)
                        .size(10.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate()
                .halign(egui::Align::Center),
            );
        },
    );
    response.on_hover_text(format!("{label}：{value}"));
}

pub(crate) fn draw_clipped_label(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    text: &str,
    font: egui::FontId,
    color: Color32,
    align: egui::Align,
    hover_text: Option<&str>,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let display_text = truncate_text_to_width(ui, text, &font, color, rect.width());
    let (position, anchor) = match align {
        egui::Align::Min => (rect.left_center(), egui::Align2::LEFT_CENTER),
        egui::Align::Center => (rect.center(), egui::Align2::CENTER_CENTER),
        egui::Align::Max => (rect.right_center(), egui::Align2::RIGHT_CENTER),
    };
    ui.painter()
        .with_clip_rect(rect)
        .text(position, anchor, display_text, font, color);
    let id = ui.next_auto_id();
    let response = ui.interact(rect, id, egui::Sense::hover());
    if let Some(hover_text) = hover_text {
        response.on_hover_text(hover_text);
    }
}

pub(crate) fn truncate_text_to_width(
    ui: &egui::Ui,
    text: &str,
    font: &egui::FontId,
    color: Color32,
    max_width: f32,
) -> String {
    let text_width = |value: &str| {
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(value.to_owned(), font.clone(), color)
                .size()
                .x
        })
    };
    if text_width(text) <= max_width {
        return text.to_owned();
    }

    let chars = text.chars().collect::<Vec<_>>();
    let ellipsis = "…";
    if text_width(ellipsis) > max_width {
        return String::new();
    }
    let mut low = 0;
    let mut high = chars.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let mut candidate = chars[..middle].iter().collect::<String>();
        candidate.push('…');
        if text_width(&candidate) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    chars[..low].iter().collect::<String>() + ellipsis
}

pub(crate) fn draw_target_hp_text(
    ui: &mut egui::Ui,
    cell_rect: egui::Rect,
    hit: &crate::engine::model::Hit,
    target_color: Color32,
    hp_font: egui::FontId,
) {
    let text_rect = cell_rect.shrink2(egui::vec2(8.0, 0.0));
    let target_rect = egui::Rect::from_min_max(
        text_rect.min,
        egui::pos2(text_rect.right(), text_rect.center().y),
    );
    let hp_rect = egui::Rect::from_min_max(
        egui::pos2(text_rect.left(), text_rect.center().y),
        text_rect.max,
    );
    let target = "Target HP";
    let hp = format!(
        "{} / {}  {:.1}%",
        format_number(hit.target_hp_after),
        format_number(hit.target_max_hp),
        hit.target_hp_percent
    );
    draw_clipped_label(
        ui,
        target_rect,
        target,
        density_proportional_font(ui, 12.0),
        target_color,
        egui::Align::Min,
        None,
    );
    draw_clipped_label(
        ui,
        hp_rect,
        &hp,
        hp_font,
        ui.visuals().weak_text_color(),
        egui::Align::Min,
        None,
    );
}

pub(crate) fn draw_direction_summary(ui: &mut egui::Ui, summary: HitDirectionSummary) {
    ui.add_space(5.0);
    let text = tf(
        "Confirmed output {} ({} hits) · candidate output {} ({} hits, {}% of total output)",
        &[
            &format_number(summary.outgoing_damage),
            &summary.outgoing_hits.to_string(),
            &format_number(summary.unknown_damage),
            &summary.unknown_hits.to_string(),
            &format!("{:.1}", summary.unknown_share()),
        ],
    );
    ui.add(
        egui::Label::new(
            RichText::new(&text)
                .size(10.5)
                .color(ui.visuals().weak_text_color()),
        )
        .truncate(),
    )
    .on_hover_text(text);
}

pub(crate) fn draw_hit_column_separators(
    painter: &egui::Painter,
    rect: egui::Rect,
    layout: CharacterHitLayout,
) {
    let color = theme_tokens(
        painter.ctx().global_style().visuals.dark_mode,
        AccentColor::Zinc,
    )
    .detail_separator;
    for separator in layout.separators.into_iter().flatten() {
        let x = rect.left() + separator;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, color),
        );
    }
}

pub(crate) fn draw_team_hit_column_separators(
    painter: &egui::Painter,
    rect: egui::Rect,
    layout: TeamHitLayout,
) {
    let color = theme_tokens(
        painter.ctx().global_style().visuals.dark_mode,
        AccentColor::Zinc,
    )
    .detail_separator;
    for separator in layout.separators.into_iter().flatten() {
        let x = rect.left() + separator;
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0_f32, color),
        );
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn character_layout_omits_character_column_and_fills_available_width() {
        let layout = character_hit_layout(720.0, HitDetailColumnsConfig::default());

        assert!(layout.column(HitDetailColumn::Character).is_none());
        let target_hp = layout
            .column(HitDetailColumn::TargetHp)
            .expect("target HP column should be visible");
        assert!((target_hp.left + target_hp.width - 720.0).abs() < f32::EPSILON);
        assert_eq!(layout.separators.into_iter().flatten().count(), 3);
    }

    #[test]
    fn hidden_columns_are_removed_without_leaving_gaps() {
        let mut config = HitDetailColumnsConfig::default();
        config.set_visible(HitDetailColumn::Time, false);
        config.set_visible(HitDetailColumn::Type, false);
        let layout = team_hit_layout(640.0, config);

        assert!(layout.column(HitDetailColumn::Time).is_none());
        assert!(layout.column(HitDetailColumn::Type).is_none());
        assert_eq!(
            layout
                .column(HitDetailColumn::Character)
                .expect("character column should be visible")
                .left,
            4.0
        );
        assert_eq!(layout.separators.into_iter().flatten().count(), 2);
    }
}
