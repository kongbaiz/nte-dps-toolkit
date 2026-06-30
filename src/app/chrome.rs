use super::*;

pub(crate) enum UiConfigSavePlan {
    NoChange,
    SetPending((UiConfig, Instant)),
    KeepPending((UiConfig, Instant)),
    Save(UiConfig),
}

pub(crate) fn passthrough_egui_key(hotkey: PassthroughHotkey) -> egui::Key {
    match hotkey {
        PassthroughHotkey::Home => egui::Key::Home,
        PassthroughHotkey::Insert => egui::Key::Insert,
        PassthroughHotkey::F8 => egui::Key::F8,
        PassthroughHotkey::F9 => egui::Key::F9,
    }
}

pub(crate) fn stable_selectable_value<'a, Value: PartialEq>(
    ui: &mut egui::Ui,
    current_value: &mut Value,
    selected_value: Value,
    text: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    let mut response = ui.add(
        egui::Button::selectable(*current_value == selected_value, text).frame_when_inactive(true),
    );
    if response.clicked() && *current_value != selected_value {
        *current_value = selected_value;
        response.mark_changed();
    }
    response
}

pub(crate) fn stable_popup_selectable_value<'a, Value: PartialEq>(
    ui: &mut egui::Ui,
    current_value: &mut Value,
    selected_value: Value,
    text: impl egui::IntoAtoms<'a>,
) -> egui::Response {
    let mut style = (**ui.style()).clone();
    style.visuals.widgets.inactive.bg_stroke =
        Stroke::new(0.0, style.visuals.widgets.inactive.bg_stroke.color);
    ui.scope(|ui| {
        ui.set_style(style);
        ui.selectable_value(current_value, selected_value, text)
    })
    .inner
}

pub(crate) fn inline_controls<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope(|ui| {
        let mut style = (**ui.style()).clone();
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(INLINE_CONTROL_TEXT_SIZE),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(INLINE_CONTROL_TEXT_SIZE),
        );
        ui.set_style(style);
        ui.spacing_mut().interact_size.y = INLINE_CONTROL_HEIGHT;
        ui.spacing_mut().button_padding = egui::vec2(10.0, 3.0);
        ui.horizontal(|ui| {
            ui.set_min_height(INLINE_CONTROL_HEIGHT);
            add_contents(ui)
        })
        .inner
    })
    .inner
}

pub(crate) fn inline_text(text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text)
        .size(INLINE_CONTROL_TEXT_SIZE)
        .color(color)
}

pub(crate) fn file_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("所选文件")
        .to_owned()
}

pub(crate) fn confirmation_content(
    action: &ConfirmationAction,
) -> (&'static str, String, &'static str) {
    match action {
        ConfirmationAction::StartLive => (
            "确认开始",
            "开始实时抓包会清空当前统计并重新检测游戏连接。".to_owned(),
            "开始",
        ),
        ConfirmationAction::ResetSession => (
            "确认重置",
            "将停止当前任务并清空本次统计、深渊状态和明细缓存。".to_owned(),
            "重置",
        ),
        ConfirmationAction::ImportPcapng(path) => (
            "确认导入",
            format!(
                "导入 {} 会停止当前任务并清空现有统计。",
                file_display_name(path)
            ),
            "导入",
        ),
        ConfirmationAction::ImportCaptureJson(path) => (
            "确认导入",
            format!(
                "导入 {} 会停止当前任务并清空现有统计。",
                file_display_name(path)
            ),
            "导入",
        ),
        ConfirmationAction::ClearEncryptedIni => (
            "确认清空",
            "当前 INI 有未保存修改，清空后这些修改会丢失。".to_owned(),
            "清空",
        ),
        ConfirmationAction::ReloadEncryptedIni(path) => (
            "确认重新载入",
            format!(
                "当前 INI 有未保存修改，重新载入 {} 后这些修改会丢失。",
                file_display_name(path)
            ),
            "重新载入",
        ),
        ConfirmationAction::DeleteHistory(record_id) => (
            "确认删除",
            format!("删除历史摘要 {record_id} 后不可撤销。"),
            "删除",
        ),
    }
}

pub(crate) fn error_action_label(action: ErrorAction) -> &'static str {
    match action {
        ErrorAction::RefreshNetwork => "重新检测",
        ErrorAction::OpenPcapng => "重新选择 PCAPNG",
        ErrorAction::OpenCaptureJson => "重新选择 JSON",
        ErrorAction::OpenEncryptedIni => "重新选择 INI",
        ErrorAction::OpenTeamDpsImport => "重新选择 DPS 数据",
        ErrorAction::OpenConsole => "打开控制台",
    }
}

pub(crate) fn import_error_action(error: &str) -> Option<ErrorAction> {
    let lower = error.to_ascii_lowercase();
    if lower.contains("pcapng") {
        Some(ErrorAction::OpenPcapng)
    } else if lower.contains("json") {
        Some(ErrorAction::OpenCaptureJson)
    } else {
        None
    }
}

pub(crate) fn humanize_engine_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    const PCAPNG_PREFIX: &str = "pcapng import failed:";
    const JSON_PREFIX: &str = "json import failed:";
    if lower.starts_with(PCAPNG_PREFIX) {
        let reason = error[PCAPNG_PREFIX.len()..].trim();
        return format!("PCAPNG 导入失败：{}", reason.trim());
    }
    if lower.starts_with(JSON_PREFIX) {
        let reason = error[JSON_PREFIX.len()..].trim();
        return format!("抓包 JSON 导入失败：{}", reason.trim());
    }
    error.to_owned()
}

/// System CJK fonts tried in order. The whole UI is Chinese, so without a CJK face every label
/// renders as tofu. Microsoft YaHei is the preferred match; the rest are fallbacks that ship on
/// common Windows editions (including ones where YaHei was removed or replaced).
pub(crate) const CJK_FONT_CANDIDATES: &[&str] = &[
    "msyh.ttc",   // Microsoft YaHei (Win 8+)
    "msyh.ttf",   // Microsoft YaHei (older layout)
    "msyhl.ttc",  // Microsoft YaHei Light
    "simsun.ttc", // SimSun / NSimSun
    "simhei.ttf", // SimHei
    "Deng.ttf",   // DengXian
];

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let windows_dir = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts_dir = windows_dir.join("Fonts");
    let Some(bytes) = CJK_FONT_CANDIDATES
        .iter()
        .find_map(|name| std::fs::read(fonts_dir.join(name)).ok())
    else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "system-cjk".to_owned(),
        egui::FontData::from_owned(bytes).into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Title-bar −／＋ stepper that resizes the current viewport proportionally.
///
/// Free drag-resize was removed: dragging a window edge on Windows can trigger a
/// native access violation in eframe (egui #4061 / #4091), and there is no OS
/// resize border anyway (`with_decorations(false)`). Instead the user steps the
/// window between [`WINDOW_SCALE_MIN`]..=[`WINDOW_SCALE_MAX`] of its default size.
///
/// `scale` is clamped to that range; on a click it sends exactly one
/// `InnerSize(base_size * scale)` — a discrete resize, never the per-pixel drag
/// or rapid `InnerSize` stream that the upstream bug chokes on. `base_size` keeps
/// the window's original aspect ratio. Draw inside a right-to-left layout.
pub(crate) fn window_scale_stepper(ui: &mut egui::Ui, scale: &mut f32, base_size: egui::Vec2) {
    let apply = |ui: &egui::Ui, scale: f32| {
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::InnerSize(scaled_window_size(
                base_size, scale,
            )));
    };
    // right-to-left: ＋ is added first (rightmost), then the readout, then －.
    if ui
        .add_enabled(
            *scale < WINDOW_SCALE_MAX - f32::EPSILON,
            egui::Button::new("＋")
                .frame(false)
                .min_size(TITLE_BAR_BUTTON_SIZE),
        )
        .on_hover_text("放大窗口")
        .clicked()
    {
        *scale = (*scale + WINDOW_SCALE_STEP).min(WINDOW_SCALE_MAX);
        apply(ui, *scale);
    }
    ui.add(
        egui::Label::new(
            RichText::new(format!("{:.0}%", *scale * 100.0))
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        )
        .selectable(false),
    )
    .on_hover_text("窗口缩放比例");
    if ui
        .add_enabled(
            *scale > WINDOW_SCALE_MIN + f32::EPSILON,
            egui::Button::new("－")
                .frame(false)
                .min_size(TITLE_BAR_BUTTON_SIZE),
        )
        .on_hover_text("缩小窗口")
        .clicked()
    {
        *scale = (*scale - WINDOW_SCALE_STEP).max(WINDOW_SCALE_MIN);
        apply(ui, *scale);
    }
}

pub(crate) fn scaled_window_size(base_size: egui::Vec2, scale: f32) -> egui::Vec2 {
    let scale = if scale.is_finite() {
        scale.clamp(WINDOW_SCALE_MIN, WINDOW_SCALE_MAX)
    } else {
        1.0
    };
    egui::vec2((base_size.x * scale).round(), (base_size.y * scale).round())
}

pub(crate) fn secondary_title_bar(
    ui: &mut egui::Ui,
    title: &str,
    scale: &mut f32,
    base_size: egui::Vec2,
) -> bool {
    let title_height = 28.0;
    let mut close_clicked = false;
    // Whole bar drags the window; buttons drawn on top win their own clicks. See
    // the matching note in `title_bar` — keeps the bar draggable on any empty
    // area regardless of how the controls pack in.
    let (full_rect, title_drag) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), title_height),
        egui::Sense::click_and_drag(),
    );
    if title_drag.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(full_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.label(
        RichText::new(title)
            .size(13.0)
            .strong()
            .color(bar.visuals().text_color()),
    );
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add_sized(TITLE_BAR_BUTTON_SIZE, egui::Button::new("×").frame(false))
            .on_hover_text("关闭")
            .clicked()
        {
            close_clicked = true;
        }
        if ui
            .add_sized(TITLE_BAR_BUTTON_SIZE, egui::Button::new("−").frame(false))
            .on_hover_text("最小化")
            .clicked()
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        window_scale_stepper(ui, scale, base_size);
    });
    close_clicked
}

pub(crate) fn console_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("nte_debug_viewport")
}

pub(crate) fn hit_detail_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("nte_hit_detail_viewport")
}

pub(crate) fn team_hit_detail_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("nte_team_hit_detail_viewport")
}

pub(crate) fn abyss_overview_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("nte_abyss_overview_viewport")
}
