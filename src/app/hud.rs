use super::*;

pub(crate) const HUD_EDITOR_MODULE_HEADER_HEIGHT: f32 = 18.0;

/// Paint text with a light dark halo so HUD text stays readable without the
/// heavy caption-like outline that would compete with the game scene.
pub(crate) fn paint_haloed(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
    halo: Color32,
) {
    paint_haloed_with_halo(painter, pos, anchor, text, font, color, halo);
}

pub(crate) fn paint_haloed_with_halo(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: egui::Align2,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
    halo: Color32,
) {
    let text = text.into();
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

/// Window size the HUD shrinks to: configured width, height sized to hug `rows`,
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
    let editor_headers = if show_title_strip {
        let visible_modules = usize::from(config.show_title)
            + usize::from(config.has_summary_row())
            + usize::from(show_status_row)
            + usize::from(config.show_character_rows)
            + usize::from(config.show_mini_timeline);
        visible_modules as f32 * HUD_EDITOR_MODULE_HEADER_HEIGHT
    } else {
        0.0
    };
    let content = 16.0 + editor_headers + readout_title + summary + status + rows + timeline + 4.0;
    egui::vec2(config.width as f32, (title_strip + content).round())
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

/// English key for an attack-type/category label (the ones
/// [`crate::engine::parser::classify_attack_type`] and the fuwen follow-up
/// tracker produce as literal Chinese text, e.g. "创生花", "普攻"). These
/// aren't sourced from a character's per-ability kit, so unlike move names
/// they have no locale-aware resource of their own — wrap with
/// [`crate::storage::i18n::t`] at the display site. Names for the 8 reaction
/// conditions, `Esper Cycle`/`Break Damage`/`Parry Attack`/`Basic Attack`/
/// `Ultimate`/`Block` match the official `ST_ReactionDes`/glossary
/// localization in `NTE_Assets`; the rest (`Skill`, `Vehicle Damage`, `Abyss
/// Field Buff`, `HP Sync Damage`) are this tool's own labels with no official
/// game term, so they're translated by hand instead.
fn attack_type_translation_key(label: &str) -> Option<&'static str> {
    match label {
        "创生" => Some("Blossom"),
        "创生花" => Some("Blossom Damage"),
        "覆纹" => Some("Hexed"),
        "覆纹追加攻击" => Some("Hexed Follow-up Attack"),
        "延滞" => Some("Remora"),
        "黯星" => Some("Nova"),
        "浊燃" => Some("Scorch"),
        "浸染" => Some("Stain"),
        "盈蓄" => Some("Charge"),
        "失谐" => Some("Discord"),
        "环合" => Some("Esper Cycle"),
        "环合伤害" => Some("Reaction Damage"),
        "倾陷伤害" => Some("Break Damage"),
        "普攻" => Some("Basic Attack"),
        "E技能" => Some("Skill"),
        "Q技能" => Some("Ultimate"),
        "闪避反击" => Some("Parry Attack"),
        "格挡反击" => Some("Block Counter"),
        "载具伤害" => Some("Vehicle Damage"),
        "深渊场地Buff" => Some("Abyss Field Buff"),
        "HP同步伤害" => Some("HP Sync Damage"),
        _ => None,
    }
}

/// Translates an attack-type/reaction label for display, including the
/// "环合·X" QTE-trigger form (e.g. "环合·创生" -> "Esper Cycle · Blossom").
/// Leaves anything else (move names) unchanged.
pub(crate) fn translate_reaction_label(label: &str) -> String {
    if let Some(key) = attack_type_translation_key(label) {
        return t(key);
    }
    if let Some(reaction) = label.strip_prefix("环合·")
        && let Some(key) = attack_type_translation_key(reaction)
    {
        return format!("{} · {}", t("Esper Cycle"), t(key));
    }
    label.to_owned()
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn hud_editor_size_reserves_module_headers_and_uses_configured_width() {
        let config = HudConfig::detailed();
        let passthrough = hud_window_size(4, false, true, &config);
        let editor = hud_window_size(4, true, true, &config);

        assert_eq!(editor.x, config.width as f32);
        assert_eq!(
            editor.y - passthrough.y,
            24.0 + HUD_EDITOR_MODULE_HEADER_HEIGHT * 5.0
        );
    }
}

/// "类型·名称": the broad attack-type category joined with the resolved skill
/// name, e.g. "普攻·酸甜口味的制裁". Since [`crate::engine::parser::load_ability_tip_names`]
/// resolves one name per ability rather than per combo hit, several hits under
/// the same skill can share an identical name; the leading attack type keeps
/// them distinguishable from other categories using the same name. Falls back
/// to whichever half is available, and drops the join when both halves match.
///
/// The skill name is re-resolved from `gameplay_effect_name` through
/// [`crate::storage::ability_names`] rather than read straight off
/// `hit.damage_name`, which was baked in at capture time in whatever language
/// was active then; falls back to that stored value when live resolution
/// misses (e.g. reaction-only effects have no ability-tip entry).
pub(crate) fn hit_type_display_text(hit: &crate::engine::model::Hit) -> String {
    let attack_type = hit.attack_type.as_deref().filter(|value| !value.is_empty());
    let live_name = hit
        .gameplay_effect_name
        .as_deref()
        .and_then(crate::storage::ability_names::resolve_damage_name);
    let name = live_name
        .as_deref()
        .or(hit.damage_name.as_deref())
        .filter(|value| !value.is_empty());
    match (attack_type, name) {
        (Some(attack_type), Some(name)) if attack_type != name => {
            format!("{}·{name}", translate_reaction_label(attack_type))
        }
        (Some(attack_type), _) => translate_reaction_label(attack_type),
        (None, Some(name)) => name.to_owned(),
        (None, None) => "未知招式".to_owned(),
    }
}
