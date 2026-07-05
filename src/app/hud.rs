use super::*;

/// Paint text with a light dark halo so HUD text stays readable without the
/// heavy caption-like outline that would compete with the game scene.
pub(crate) fn paint_haloed(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
) {
    let text = text.into();
    let halo = Color32::from_black_alpha(185);
    for offset in [
        egui::vec2(-0.8, 0.0),
        egui::vec2(0.8, 0.0),
        egui::vec2(0.0, -0.8),
        egui::vec2(0.0, 0.8),
        egui::vec2(-0.65, -0.65),
        egui::vec2(0.65, 0.65),
    ] {
        painter.text(pos + offset, anchor, text.clone(), font.clone(), halo);
    }
    painter.text(pos, anchor, text, font, color);
}

/// Window size the HUD shrinks to: fixed width, height sized to hug `rows`,
/// the team header, and the optional positioning rail.
pub(crate) fn hud_window_size(
    rows: usize,
    show_title_strip: bool,
    show_status_row: bool,
    config: &HudConfig,
) -> egui::Vec2 {
    let title_strip = if show_title_strip { 24.0 } else { 0.0 };
    let readout_title = if config.show_title { 22.0 } else { 0.0 };
    let summary = if config.has_summary_row() { 56.0 } else { 0.0 };
    let status = if show_status_row { 22.0 } else { 0.0 };
    let rows = if config.show_character_rows {
        rows.max(1) as f32 * 28.0
    } else {
        0.0
    };
    let timeline = if config.show_mini_timeline { 42.0 } else { 0.0 };
    let content = 16.0 + readout_title + summary + status + rows + timeline + 4.0;
    egui::vec2(HUD_WINDOW_WIDTH, (title_strip + content).round())
}

pub(crate) fn is_party_member_row(
    row: &CharacterStats,
    hits: &VecDeque<crate::engine::model::Hit>,
) -> bool {
    hits.iter()
        .any(|hit| hit.char_id == row.char_id && !is_qte_follow_up_damage_hit(hit))
}

pub(crate) fn hit_specific_type(hit: &crate::engine::model::Hit) -> &str {
    hit.damage_name
        .as_deref()
        .or(hit.attack_type.as_deref())
        .unwrap_or("未知招式")
}

/// English key (or the raw move name for outgoing hits). Wrap with
/// [`crate::storage::i18n::t`] at the display site; the raw move-name branch is left
/// untranslated so it keeps its original value and stays comparable to skill filters.
pub(crate) fn hit_type_label(hit: &crate::engine::model::Hit) -> &str {
    match hit.direction.as_str() {
        "incoming" => "Incoming",
        "unknown" => "Candidate Output",
        _ => hit_specific_type(hit),
    }
}

/// "类型·名称": the broad attack-type category joined with the resolved skill
/// name, e.g. "普攻·酸甜口味的制裁". Since [`crate::engine::parser::load_ability_tip_names`]
/// resolves one name per ability rather than per combo hit, several hits under
/// the same skill can share an identical name; the leading attack type keeps
/// them distinguishable from other categories using the same name. Falls back
/// to whichever half is available, and drops the join when both halves match.
pub(crate) fn hit_type_display_text(hit: &crate::engine::model::Hit) -> String {
    let attack_type = hit.attack_type.as_deref().filter(|value| !value.is_empty());
    let name = hit.damage_name.as_deref().filter(|value| !value.is_empty());
    match (attack_type, name) {
        (Some(attack_type), Some(name)) if attack_type != name => format!("{attack_type}·{name}"),
        (Some(attack_type), _) => attack_type.to_owned(),
        (None, Some(name)) => name.to_owned(),
        (None, None) => "未知招式".to_owned(),
    }
}
