use super::*;

#[derive(Clone, Copy)]
pub(crate) struct HudThemeTokens {
    pub(crate) accent: Color32,
    pub(crate) text: Color32,
    pub(crate) muted: Color32,
    pub(crate) track: Color32,
    pub(crate) halo: Color32,
    pub(crate) edit_bg: Color32,
    pub(crate) edit_border: Color32,
    pub(crate) edit_text: Color32,
}

#[derive(Clone, Copy)]
pub(crate) struct ThemeTokens {
    pub(crate) bg: Color32,
    pub(crate) bg_elevated: Color32,
    pub(crate) card: Color32,
    pub(crate) card_hover: Color32,
    pub(crate) muted: Color32,
    pub(crate) border: Color32,
    pub(crate) border_strong: Color32,
    pub(crate) fg: Color32,
    pub(crate) fg_muted: Color32,
    pub(crate) fg_faint: Color32,
    pub(crate) accent: Color32,
    pub(crate) accent_fg: Color32,
    pub(crate) success: Color32,
    pub(crate) warning: Color32,
    pub(crate) danger: Color32,
    pub(crate) info: Color32,
    pub(crate) dataviz: [Color32; 8],
    pub(crate) detail_row: Color32,
    pub(crate) detail_separator: Color32,
    pub(crate) floating: Color32,
    pub(crate) modal_backdrop: Color32,
    pub(crate) notice_bg: Color32,
    pub(crate) hud: HudThemeTokens,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DensityTokens {
    pub(crate) font_scale: f32,
    pub(crate) item_spacing: egui::Vec2,
    pub(crate) interact_height: f32,
    pub(crate) button_padding: egui::Vec2,
    pub(crate) detail_row_height: f32,
}

pub(crate) fn density_tokens(density: UiDensity) -> DensityTokens {
    let scale = match density {
        UiDensity::Compact => 0.9,
        UiDensity::Cozy => 1.0,
        UiDensity::Comfortable => 1.15,
    };
    DensityTokens {
        font_scale: scale,
        item_spacing: egui::vec2(8.0 * scale, 5.0 * scale),
        interact_height: INLINE_CONTROL_HEIGHT * scale,
        button_padding: egui::vec2(11.0 * scale, 4.0 * scale),
        detail_row_height: DETAIL_HIT_ROW_HEIGHT * scale,
    }
}

pub(crate) fn ui_density_scale(ui: &egui::Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&egui::TextStyle::Body)
        .map_or(1.0, |font| font.size / 14.0)
}

pub(crate) fn density_proportional_font(ui: &egui::Ui, base_size: f32) -> egui::FontId {
    egui::FontId::proportional(base_size * ui_density_scale(ui))
}

pub(crate) fn density_monospace_font(ui: &egui::Ui, base_size: f32) -> egui::FontId {
    egui::FontId::monospace(base_size * ui_density_scale(ui))
}

pub(crate) fn theme_tokens(dark_mode: bool, accent: AccentColor) -> ThemeTokens {
    let accent_choice = accent;
    let (
        bg,
        bg_elevated,
        card,
        card_hover,
        muted,
        border,
        border_strong,
        fg,
        fg_muted,
        fg_faint,
        success,
        warning,
        danger,
        info,
        dataviz,
    ) = if dark_mode {
        (
            Color32::from_rgb(9, 9, 11),
            Color32::from_rgb(17, 17, 20),
            Color32::from_rgb(24, 24, 27),
            Color32::from_rgb(31, 31, 35),
            Color32::from_rgb(39, 39, 42),
            Color32::from_rgb(39, 39, 42),
            Color32::from_rgb(63, 63, 70),
            Color32::from_rgb(250, 250, 250),
            Color32::from_rgb(161, 161, 170),
            Color32::from_rgb(113, 113, 122),
            Color32::from_rgb(74, 222, 128),
            Color32::from_rgb(250, 204, 21),
            Color32::from_rgb(248, 113, 113),
            Color32::from_rgb(96, 165, 250),
            [
                Color32::from_rgb(96, 165, 250),
                Color32::from_rgb(167, 139, 250),
                Color32::from_rgb(52, 211, 153),
                Color32::from_rgb(251, 146, 60),
                Color32::from_rgb(244, 114, 182),
                Color32::from_rgb(34, 211, 238),
                Color32::from_rgb(250, 204, 21),
                Color32::from_rgb(248, 113, 113),
            ],
        )
    } else {
        (
            Color32::from_rgb(250, 250, 250),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(248, 248, 249),
            Color32::from_rgb(228, 228, 231),
            Color32::from_rgb(228, 228, 231),
            Color32::from_rgb(212, 212, 216),
            Color32::from_rgb(9, 9, 11),
            Color32::from_rgb(82, 82, 91),
            Color32::from_rgb(113, 113, 122),
            Color32::from_rgb(22, 128, 76),
            Color32::from_rgb(161, 98, 7),
            Color32::from_rgb(190, 55, 65),
            Color32::from_rgb(37, 99, 235),
            [
                Color32::from_rgb(37, 99, 235),
                Color32::from_rgb(124, 58, 237),
                Color32::from_rgb(5, 150, 105),
                Color32::from_rgb(194, 65, 12),
                Color32::from_rgb(190, 24, 93),
                Color32::from_rgb(8, 145, 178),
                Color32::from_rgb(161, 98, 7),
                Color32::from_rgb(190, 55, 65),
            ],
        )
    };
    let accent = match (dark_mode, accent) {
        (true, AccentColor::Zinc) => Color32::from_rgb(250, 250, 250),
        (false, AccentColor::Zinc) => Color32::from_rgb(24, 24, 27),
        (true, AccentColor::Blue) => Color32::from_rgb(96, 165, 250),
        (false, AccentColor::Blue) => Color32::from_rgb(37, 99, 235),
        (true, AccentColor::Violet) => Color32::from_rgb(167, 139, 250),
        (false, AccentColor::Violet) => Color32::from_rgb(124, 58, 237),
        (true, AccentColor::Orange) => Color32::from_rgb(251, 146, 60),
        (false, AccentColor::Orange) => Color32::from_rgb(194, 65, 12),
        (true, AccentColor::Green) => Color32::from_rgb(74, 222, 128),
        (false, AccentColor::Green) => Color32::from_rgb(22, 128, 76),
    };
    let hud_accent = if accent_choice == AccentColor::Zinc {
        Color32::from_rgb(44, 214, 150)
    } else {
        accent
    };
    let detail_row = if dark_mode {
        Color32::from_white_alpha(8)
    } else {
        Color32::from_black_alpha(5)
    };
    let detail_separator = if dark_mode {
        Color32::from_rgba_unmultiplied(255, 255, 255, 92)
    } else {
        Color32::from_rgba_unmultiplied(70, 74, 82, 88)
    };
    let floating = Color32::from_rgba_unmultiplied(card.r(), card.g(), card.b(), 242);
    ThemeTokens {
        bg,
        bg_elevated,
        card,
        card_hover,
        muted,
        border,
        border_strong,
        fg,
        fg_muted,
        fg_faint,
        accent,
        accent_fg: contrast_text(accent),
        success,
        warning,
        danger,
        info,
        dataviz,
        detail_row,
        detail_separator,
        floating,
        modal_backdrop: Color32::from_black_alpha(150),
        notice_bg: Color32::from_black_alpha(210),
        hud: HudThemeTokens {
            accent: hud_accent,
            text: Color32::from_rgb(242, 246, 248),
            muted: Color32::from_rgb(176, 187, 194),
            track: Color32::from_black_alpha(96),
            halo: Color32::from_black_alpha(185),
            edit_bg: Color32::from_rgb(14, 16, 20),
            edit_border: Color32::from_rgb(39, 201, 146),
            edit_text: Color32::from_rgb(218, 224, 228),
        },
    }
}

impl DpsApp {
    pub(crate) fn theme(&self) -> ThemeTokens {
        theme_tokens(self.dark_mode, self.accent)
    }
}

/// Card-framed settings/diagnostics section — a bold title above the body —
/// replacing the bare collapsing headers so those pages read as grouped cards.
pub(crate) fn settings_section(
    ui: &mut egui::Ui,
    theme: ThemeTokens,
    title_key: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(theme.card)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(t(title_key))
                    .size(13.0)
                    .strong()
                    .color(theme.fg),
            );
            ui.add_space(8.0);
            add_contents(ui);
        });
    ui.add_space(10.0);
}

/// Centered empty-state card shared by the pages that wait for data: title,
/// one-line body and a wrapped row of `actions` inside one bordered card. The
/// card claims its own fixed-width top-down child region —
/// `centered_and_justified` only centers a single widget, and a horizontal row
/// inside it stays left-aligned.
pub(crate) fn empty_state_card(
    ui: &mut egui::Ui,
    theme: ThemeTokens,
    title: String,
    body: String,
    actions: impl FnOnce(&mut egui::Ui),
) {
    let card_height = ui.available_height().clamp(120.0, 180.0);
    let card_width = (ui.available_width() - 12.0).clamp(0.0, 380.0);
    ui.add_space(((ui.available_height() - card_height) / 2.0).max(0.0));
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - card_width) / 2.0).max(0.0));
        ui.allocate_ui_with_layout(
            egui::vec2(card_width, card_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::Frame::new()
                    .fill(theme.card)
                    .stroke(Stroke::new(1.0, theme.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(18, 14))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(title).strong().color(theme.fg));
                        ui.add_space(4.0);
                        ui.label(RichText::new(body).size(12.0).color(theme.fg_muted));
                        ui.add_space(10.0);
                        ui.horizontal_wrapped(actions);
                    });
            },
        );
    });
}

/// Character display name for the active UI language (`name_en` for English and any
/// non-Chinese language, `name_zh` for Simplified Chinese), falling back to
/// `fallback` when the character table has no usable entry for `char_id`. A free
/// function so the timeline, skills and history views — which render from a
/// `&HashMap<u32, CharacterInfo>` without a `DpsApp` handle — resolve names the same
/// way. Reads the active language from the shared i18n store.
pub(crate) fn character_display_name(
    characters: &HashMap<u32, CharacterInfo>,
    char_id: u32,
    fallback: &str,
) -> String {
    if let Some(info) = characters.get(&char_id) {
        let candidate = if i18n::current_language() == Language::SimplifiedChinese {
            info.name_zh.trim()
        } else {
            info.name_en.trim()
        };
        if !candidate.is_empty() {
            return candidate.to_owned();
        }
    }
    fallback.to_owned()
}

pub(crate) fn compact_metric(
    ui: &mut egui::Ui,
    label: &str,
    value: String,
    color: Color32,
    prominent: bool,
) {
    compact_metric_scaled(ui, label, value, color, prominent, 1.0);
}

pub(crate) fn compact_metric_scaled(
    ui: &mut egui::Ui,
    label: &str,
    value: String,
    color: Color32,
    prominent: bool,
    value_scale: f32,
) {
    let density_scale = ui_density_scale(ui);
    let id = ui.make_persistent_id(("compact_metric", label));
    let hovered = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|pointer| ui.max_rect().contains(pointer));
    let hover = motion::animate_bool(
        ui.ctx(),
        id,
        hovered,
        motion::dur::FAST,
        ui.style().animation_time == 0.0,
        motion::ease::standard,
    );
    let fill = mix_color(
        shadcn_card(ui.visuals().dark_mode),
        shadcn_card_hover(ui.visuals().dark_mode),
        hover,
    );
    egui::Frame::new()
        .fill(fill)
        .corner_radius(6)
        .stroke(Stroke::new(
            1.0_f32,
            mix_color(
                shadcn_border(ui.visuals().dark_mode),
                ui.visuals().selection.bg_fill.gamma_multiply(0.55),
                hover,
            ),
        ))
        .inner_margin(egui::Margin::symmetric(4, 4))
        .show(ui, |ui| {
            ui.set_min_height(38.0 * density_scale);
            ui.vertical_centered(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(
                    RichText::new(value)
                        .size((if prominent { 17.0 } else { 15.0 }) * density_scale * value_scale)
                        .strong()
                        .color(color),
                );
                ui.label(
                    RichText::new(label)
                        .size(9.5 * density_scale)
                        .color(ui.visuals().weak_text_color()),
                );
            });
        });
}

/// Height of each party row so the list fills the available vertical space (the
/// window is freely resizable, so the rows grow with it rather than leaving a large
/// empty gap under the last member). Only a lower bound is enforced — 38px keeps a
/// full roster readable when the window is short; there is no upper cap, so a few
/// members in a tall window stretch to fill it.
pub(crate) fn party_row_height(available_height: f32, row_count: usize) -> f32 {
    if row_count == 0 {
        return 52.0;
    }

    let spacing = 5.0 * row_count.saturating_sub(1) as f32;
    ((available_height - spacing - 2.0) / row_count as f32).max(38.0)
}

pub(crate) fn primary_button(label: impl Into<String>, fill: Color32) -> egui::Button<'static> {
    egui::Button::new(
        RichText::new(label.into())
            .strong()
            .color(contrast_text(fill)),
    )
    .fill(fill)
    .stroke(Stroke::new(1.0_f32, fill))
}

/// Severity color for the live status text. The status string can be in either
/// language (app text is localized; engine-sent status stays Chinese), so both
/// languages' severity keywords are matched. ASCII is lowercased for
/// case-insensitive English matching; Chinese needles are unaffected.
pub(crate) fn status_color(status: &str, paused: bool, dark_mode: bool) -> Color32 {
    if paused {
        return semantic_warning(dark_mode);
    }
    let lower = status.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&[
        "失败",
        "不可用",
        "未检测到",
        "failed",
        "unavailable",
        "not detected",
        "no game connection",
        "error",
    ]) {
        semantic_danger(dark_mode)
    } else if has(&[
        "正在",
        "启动",
        "导入",
        "处理",
        "starting",
        "importing",
        "processing",
        "capturing",
        "detecting",
        "...",
    ]) {
        semantic_warning(dark_mode)
    } else {
        semantic_success(dark_mode)
    }
}

/// Human-readable label for a capture NIC: its description (or raw name) plus any IPv4 addresses,
/// so users can disambiguate adapters — especially a VPN interface vs. the physical one.
pub(crate) fn capture_device_label(device: &CaptureDevice) -> String {
    let base = if device.description.is_empty() {
        device.name.as_str()
    } else {
        device.description.as_str()
    };
    if device.ipv4.is_empty() {
        base.to_owned()
    } else {
        let addresses = device
            .ipv4
            .iter()
            .map(|address| address.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("{base} · {addresses}")
    }
}

pub(crate) fn semantic_success(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).success
}

pub(crate) fn semantic_warning(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).warning
}

pub(crate) fn semantic_danger(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).danger
}

pub(crate) fn hit_output_badge_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(63, 63, 70)
    } else {
        Color32::from_rgb(24, 24, 27)
    }
}

pub(crate) fn hit_output_text_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_rgb(244, 244, 245)
    } else {
        Color32::from_rgb(24, 24, 27)
    }
}

pub(crate) fn readable_accent(color: Color32, dark_mode: bool) -> Color32 {
    let luminance = 0.2126 * f32::from(color.r())
        + 0.7152 * f32::from(color.g())
        + 0.0722 * f32::from(color.b());
    if !dark_mode && luminance > 210.0 {
        Color32::from_rgb(82, 82, 91)
    } else if dark_mode && luminance < 52.0 {
        Color32::from_rgb(161, 161, 170)
    } else {
        color
    }
}

pub(crate) fn contrast_text(background: Color32) -> Color32 {
    let luminance = 0.2126 * f32::from(background.r())
        + 0.7152 * f32::from(background.g())
        + 0.0722 * f32::from(background.b());
    if luminance > 150.0 {
        Color32::from_rgb(9, 9, 11)
    } else {
        Color32::from_rgb(250, 250, 250)
    }
}

pub(crate) fn shadcn_background(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).bg
}

pub(crate) fn shadcn_foreground(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).fg
}

pub(crate) fn shadcn_card(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).card
}

pub(crate) fn shadcn_card_hover(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).card_hover
}

pub(crate) fn shadcn_muted(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).muted
}

pub(crate) fn shadcn_border(dark_mode: bool) -> Color32 {
    theme_tokens(dark_mode, AccentColor::Zinc).border
}

pub(crate) fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let mix = |from: u8, to: u8| {
        (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8
    };
    Color32::from_rgba_unmultiplied(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
        mix(from.a(), to.a()),
    )
}

pub(crate) fn format_number(value: f64) -> String {
    let rounded = value.round() as i64;
    let source = rounded.abs().to_string();
    let grouped = source
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join(",");
    if rounded < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

pub(crate) fn format_time(timestamp: f64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + Duration::from_secs_f64(timestamp.max(0.0)))
        .format("%H:%M:%S%.3f")
        .to_string()
}

pub(crate) fn format_short_time(timestamp: f64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + Duration::from_secs_f64(timestamp.max(0.0)))
        .format("%H:%M:%S")
        .to_string()
}

pub(crate) fn show_detail_limit_notice(ui: &mut egui::Ui, filtered_count: usize) {
    if filtered_count > MAX_DETAIL_HITS {
        ui.label(
            RichText::new(tf(
                "Showing the latest {} of {} matching rows; stats within the full retained range are already counted in the summary above.",
                &[
                    &format_number(MAX_DETAIL_HITS as f64),
                    &format_number(filtered_count as f64),
                ],
            ))
            .size(11.0)
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);
    }
}

pub(crate) fn character_color(
    char_id: u32,
    characters: &HashMap<u32, CharacterInfo>,
    _fallback_index: usize,
    dark_mode: bool,
) -> Color32 {
    if let Some(value) = characters
        .get(&char_id)
        .and_then(|row| row.color.as_deref())
        && let Some(color) = parse_hex_color(value)
    {
        return color;
    }
    deterministic_character_fallback_color(&char_id.to_le_bytes(), dark_mode)
}

pub(crate) fn parse_hex_color(value: &str) -> Option<Color32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 {
        return None;
    }
    Some(Color32::from_rgb(
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ))
}

pub(crate) fn data_root() -> PathBuf {
    if PathBuf::from(CHARACTER_DATA_PATH).is_file() {
        return PathBuf::from(".");
    }
    std::env::current_exe()
        .ok()
        .into_iter()
        .flat_map(|path| path.ancestors().map(PathBuf::from).collect::<Vec<_>>())
        .find(|path| path.join(CHARACTER_DATA_PATH).is_file())
        .unwrap_or_else(|| PathBuf::from("."))
}
