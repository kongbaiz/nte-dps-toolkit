use super::*;

pub(crate) enum UiConfigSavePlan {
    NoChange,
    SetPending((UiConfig, Instant)),
    KeepPending((UiConfig, Instant)),
    Save(UiConfig),
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
        Stroke::new(0.0_f32, style.visuals.widgets.inactive.bg_stroke.color);
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
        // Wrapped, not a plain horizontal: localized labels/buttons vary in width per
        // language, so a row that fits in Chinese can overflow in English. Wrapping
        // pushes the overflow to a second line instead of letting it spill past the
        // card/window edge and get clipped.
        ui.horizontal_wrapped(|ui| {
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
        .map(|name| name.to_owned())
        .unwrap_or_else(|| t("the selected file"))
}

/// Returns `(title_key, localized_message, confirm_key)`. Title and confirm are
/// English keys (wrap with [`t`] at the display site); the message is already
/// localized here because it interpolates a runtime path/id.
pub(crate) fn confirmation_content(
    action: &ConfirmationAction,
) -> (&'static str, String, &'static str) {
    match action {
        ConfirmationAction::StartLive => (
            "Confirm Start",
            t("Starting live capture clears the current stats and re-detects the game connection."),
            "Start",
        ),
        ConfirmationAction::ResetSession => (
            "Confirm Reset",
            t(
                "This stops the current task and clears this session's stats, abyss state and detail caches.",
            ),
            "Reset",
        ),
        ConfirmationAction::ImportPcapng(path) => (
            "Confirm Import",
            tf(
                "Importing {} stops the current task and clears existing stats.",
                &[&file_display_name(path)],
            ),
            "Import",
        ),
        ConfirmationAction::ImportCaptureJson(path) => (
            "Confirm Import",
            tf(
                "Importing {} stops the current task and clears existing stats.",
                &[&file_display_name(path)],
            ),
            "Import",
        ),
        ConfirmationAction::ClearEncryptedIni => (
            "Confirm Clear",
            t("The current INI has unsaved changes; clearing loses them."),
            "Clear",
        ),
        ConfirmationAction::ReloadEncryptedIni(path) => (
            "Confirm Reload",
            tf(
                "The current INI has unsaved changes; reloading {} loses them.",
                &[&file_display_name(path)],
            ),
            "Reload",
        ),
        ConfirmationAction::ClearCaptureLogs => (
            "Confirm Clear",
            t(
                "This deletes every raw capture file (nte_raw_*.pcapng) under logs; a file in use during capture is kept. This cannot be undone.",
            ),
            "Clear",
        ),
    }
}

/// English key; wrap with [`t`] at the display site.
pub(crate) fn error_action_label(action: ErrorAction) -> &'static str {
    match action {
        ErrorAction::RefreshNetwork => "Re-detect",
        ErrorAction::OpenPcapng => "Reselect PCAPNG",
        ErrorAction::OpenCaptureJson => "Reselect JSON",
        ErrorAction::OpenEncryptedIni => "Reselect INI",
        ErrorAction::OpenTeamDpsImport => "Reselect DPS Data",
        ErrorAction::OpenConsole => "Open Console",
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
        return tf("PCAPNG import failed: {}", &[reason.trim()]);
    }
    if lower.starts_with(JSON_PREFIX) {
        let reason = error[JSON_PREFIX.len()..].trim();
        if reason.eq_ignore_ascii_case("invalid Console equipment snapshot") {
            return t("Capture JSON contains invalid Console equipment data");
        }
        if reason.eq_ignore_ascii_case("Console equipment data is unavailable") {
            return t("Console equipment data is required to restore this capture JSON");
        }
        return tf("Capture JSON import failed: {}", &[reason.trim()]);
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

/// The app's font stack: egui's defaults (which include its emoji/icon
/// fallback fonts) with the first available Windows CJK font prepended.
/// `None` when no candidate CJK font exists. Kept separate from
/// [`install_fonts`] so tests can verify glyph coverage against the exact
/// same stack the app renders with.
pub(crate) fn font_definitions() -> Option<egui::FontDefinitions> {
    let windows_dir = std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts_dir = windows_dir.join("Fonts");
    let bytes = CJK_FONT_CANDIDATES
        .iter()
        .find_map(|name| std::fs::read(fonts_dir.join(name)).ok())?;
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
    Some(fonts)
}

pub(crate) fn install_fonts(ctx: &egui::Context) {
    if let Some(fonts) = font_definitions() {
        ctx.set_fonts(fonts);
    }
    ctx.add_font(egui_material_icons::font_insert());
}

/// Invisible edge/corner drag handles that give a borderless window native OS resize.
///
/// The window has no OS chrome (`with_decorations(false)`), so there is no native resize border.
/// This paints eight thin, frameless [`egui::Area`]s (four edges + four corners) along the
/// viewport's edges; dragging one hands off to the OS resize loop via
/// [`egui::ViewportCommand::BeginResize`] (winit `drag_resize_window`). Unlike the retired
/// `−／＋` stepper this never streams `InnerSize`, but the handoff to the OS resize loop is
/// exactly what egui #4061 / #5460 crash on: a fast corner drag on a transparent borderless
/// window. Under the glow backend that killed the process via an NVIDIA OpenGL context loss
/// with no Rust panic, so the app renders with wgpu (see `main`) to dodge that driver path.
///
/// Each grip is its own `Area`, so its interaction rect is just the strip: the window center
/// still routes pointer events to the panels underneath. Works for the root viewport and any
/// child viewport because [`egui::Context::content_rect`] returns that viewport's inner rect.
pub(crate) fn window_resize_grips(ctx: &egui::Context) {
    use egui::CursorIcon;
    use egui::viewport::ResizeDirection as Dir;

    const EDGE: f32 = 6.0;
    const CORNER: f32 = 12.0;
    let r = ctx.content_rect();
    if r.width() <= 2.0 * CORNER || r.height() <= 2.0 * CORNER {
        return;
    }
    let rect = egui::Rect::from_min_size;
    let grips: [(&str, egui::Rect, Dir, CursorIcon); 8] = [
        // Corners first so they win the shared pixels with the edges.
        (
            "nw",
            rect(r.left_top(), egui::vec2(CORNER, CORNER)),
            Dir::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            "ne",
            rect(
                egui::pos2(r.right() - CORNER, r.top()),
                egui::vec2(CORNER, CORNER),
            ),
            Dir::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            "sw",
            rect(
                egui::pos2(r.left(), r.bottom() - CORNER),
                egui::vec2(CORNER, CORNER),
            ),
            Dir::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            "se",
            rect(
                egui::pos2(r.right() - CORNER, r.bottom() - CORNER),
                egui::vec2(CORNER, CORNER),
            ),
            Dir::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
        (
            "n",
            rect(
                egui::pos2(r.left() + CORNER, r.top()),
                egui::vec2(r.width() - 2.0 * CORNER, EDGE),
            ),
            Dir::North,
            CursorIcon::ResizeVertical,
        ),
        (
            "s",
            rect(
                egui::pos2(r.left() + CORNER, r.bottom() - EDGE),
                egui::vec2(r.width() - 2.0 * CORNER, EDGE),
            ),
            Dir::South,
            CursorIcon::ResizeVertical,
        ),
        (
            "w",
            rect(
                egui::pos2(r.left(), r.top() + CORNER),
                egui::vec2(EDGE, r.height() - 2.0 * CORNER),
            ),
            Dir::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            "e",
            rect(
                egui::pos2(r.right() - EDGE, r.top() + CORNER),
                egui::vec2(EDGE, r.height() - 2.0 * CORNER),
            ),
            Dir::East,
            CursorIcon::ResizeHorizontal,
        ),
    ];
    for (id, grip_rect, direction, cursor) in grips {
        let response = egui::Area::new(egui::Id::new(("window_resize_grip", id)))
            .order(egui::Order::Foreground)
            .fixed_pos(grip_rect.min)
            .movable(false)
            .constrain(false)
            .sense(egui::Sense::drag())
            .show(ctx, |ui| {
                ui.allocate_space(grip_rect.size());
            })
            .response
            .on_hover_and_drag_cursor(cursor);
        // Mirrors the title-bar `StartDrag`: fire on drag-start while the button is still down so
        // winit's native resize loop takes over immediately.
        if response.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

/// Horizontal-only native resize handles used by the HUD editor. The HUD owns
/// its height, so exposing the top/bottom handles would let a transient manual
/// height fight the next content-sized update.
pub(crate) fn window_width_resize_grips(ctx: &egui::Context) {
    use egui::CursorIcon;
    use egui::viewport::ResizeDirection as Dir;

    const EDGE: f32 = 8.0;
    let rect = ctx.content_rect();
    if rect.width() <= EDGE * 2.0 || rect.height() <= EDGE * 2.0 {
        return;
    }
    for (id, grip, direction) in [
        (
            "west",
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(EDGE, rect.height())),
            Dir::West,
        ),
        (
            "east",
            egui::Rect::from_min_size(
                egui::pos2(rect.right() - EDGE, rect.top()),
                egui::vec2(EDGE, rect.height()),
            ),
            Dir::East,
        ),
    ] {
        let response = egui::Area::new(egui::Id::new(("hud_width_resize", id)))
            .order(egui::Order::Foreground)
            .fixed_pos(grip.min)
            .movable(false)
            .constrain(false)
            .sense(egui::Sense::drag())
            .show(ctx, |ui| {
                ui.allocate_space(grip.size());
            })
            .response
            .on_hover_and_drag_cursor(CursorIcon::ResizeHorizontal);
        if response.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

/// Records a viewport's current normal inner size (logical points) into `target`, ignoring
/// minimized, maximized, degenerate, or sub-pixel changes. Callers persist `target` through the
/// normal debounced config path, so a window reopens at the size it was last dragged to.
pub(crate) fn track_window_size(ctx: &egui::Context, target: &mut egui::Vec2) {
    if ctx.input(|input| {
        let viewport = input.viewport();
        viewport.minimized == Some(true) || viewport.maximized == Some(true)
    }) {
        return;
    }
    let size = ctx.content_rect().size();
    if size.x.is_finite()
        && size.y.is_finite()
        && size.x >= 1.0
        && size.y >= 1.0
        && (size - *target).length() > 0.5
    {
        *target = size;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SecondaryViewportGeometry {
    normal_position: Option<egui::Pos2>,
    maximized: bool,
}

pub(crate) fn track_secondary_viewport_geometry(
    ctx: &egui::Context,
    normal_size: &mut egui::Vec2,
    geometry: &mut SecondaryViewportGeometry,
) {
    let (minimized, maximized, outer_position) = ctx.input(|input| {
        let viewport = input.viewport();
        (
            viewport.minimized,
            viewport.maximized.unwrap_or(false),
            viewport.outer_rect.map(|rect| rect.min),
        )
    });
    if minimized == Some(true) {
        return;
    }

    geometry.maximized = maximized;
    if maximized {
        return;
    }

    track_window_size(ctx, normal_size);
    if let Some(position) = outer_position.filter(|position| position.is_finite()) {
        geometry.normal_position = Some(position);
    }
}

/// Which native window control a [`window_control_button`] paints.
#[derive(Clone, Copy)]
pub(crate) enum WindowControlIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// A frameless, fixed-size window-control button with a hand-painted glyph.
///
/// The glyphs are painted (not font characters) so they render identically
/// regardless of which CJK fallback font [`install_fonts`] picks — the box /
/// restore shapes are not reliably present in every Windows system font.
/// Hover fades a native-caption-style backplate in (red for close, muted for
/// the rest) and the glyph color tracks the backplate so it stays readable in
/// both themes.
pub(crate) fn window_control_button(
    ui: &mut egui::Ui,
    icon: WindowControlIcon,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(TITLE_BAR_BUTTON_SIZE, egui::Sense::click());
    let tokens = theme_tokens(ui.visuals().dark_mode, AccentColor::Zinc);
    // `Context::animate_bool` uses the global `style.animation_time`, which
    // `configure_style` zeroes when reduce-motion is on.
    let hover = ui
        .ctx()
        .animate_bool(response.id.with("hover"), response.hovered());
    let (backplate, hover_glyph) = match icon {
        WindowControlIcon::Close => (tokens.danger, contrast_text(tokens.danger)),
        _ => (tokens.muted, tokens.fg),
    };
    if hover > 0.0 {
        ui.painter()
            .rect_filled(rect, 6.0, backplate.gamma_multiply(hover));
    }
    let color = mix_color(tokens.fg, hover_glyph, hover);
    let occlusion = mix_color(ui.visuals().panel_fill, backplate, hover);
    paint_window_control_icon(ui.painter(), rect, icon, color, occlusion);
    response.on_hover_text(tooltip.to_owned())
}

fn paint_window_control_icon(
    painter: &egui::Painter,
    rect: egui::Rect,
    icon: WindowControlIcon,
    color: Color32,
    bg: Color32,
) {
    let center = rect.center().round();
    let stroke = Stroke::new(1.2_f32, color);
    match icon {
        WindowControlIcon::Minimize => {
            painter.hline(center.x - 5.0..=center.x + 5.0, center.y, stroke);
        }
        WindowControlIcon::Maximize => {
            let square = egui::Rect::from_center_size(center, egui::vec2(9.0, 9.0));
            painter.rect_stroke(square, 0.0, stroke, egui::StrokeKind::Inside);
        }
        WindowControlIcon::Restore => {
            const SIDE: f32 = 8.0;
            const OFFSET: f32 = 2.0;
            // Back square peeks out to the top-right; the front square is filled
            // with the bar's own color first so it cleanly occludes the overlap,
            // giving the classic two-square "restore" look.
            let back = egui::Rect::from_min_size(
                egui::pos2(
                    center.x - SIDE * 0.5 + OFFSET,
                    center.y - SIDE * 0.5 - OFFSET,
                ),
                egui::vec2(SIDE, SIDE),
            );
            painter.rect_stroke(back, 0.0, stroke, egui::StrokeKind::Inside);
            let front = egui::Rect::from_min_size(
                egui::pos2(
                    center.x - SIDE * 0.5 - OFFSET,
                    center.y - SIDE * 0.5 + OFFSET,
                ),
                egui::vec2(SIDE, SIDE),
            );
            painter.rect_filled(front, 0.0, bg);
            painter.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
        }
        WindowControlIcon::Close => {
            let half = 5.0;
            painter.line_segment(
                [
                    egui::pos2(center.x - half, center.y - half),
                    egui::pos2(center.x + half, center.y + half),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - half, center.y + half),
                    egui::pos2(center.x + half, center.y - half),
                ],
                stroke,
            );
        }
    }
}

/// Shared borderless, always-on-top, resizable [`egui::ViewportBuilder`] for a
/// secondary window (console and the detail panels).
///
/// `already_open` must be the window's "first-frame setup done" flag (its
/// `*_corner_applied`): the saved normal size and runtime geometry are written into the
/// builder only on the opening frame. Re-passing them every frame would fight the user's
/// live move/resize — [`egui::ViewportBuilder::patch`] re-sends geometry commands whenever
/// their values change. After the first frame the builder omits them and the OS owns the
/// window geometry; the tracking helpers only record it for a later viewport recreation.
pub(crate) fn secondary_viewport_builder(
    title: impl Into<String>,
    size: egui::Vec2,
    min_size: [f32; 2],
    geometry: SecondaryViewportGeometry,
    already_open: bool,
) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_min_inner_size(egui::Vec2::from(min_size))
        .with_window_level(egui::WindowLevel::AlwaysOnTop)
        .with_decorations(false)
        // Borderless but freely resizable via the edge grips (window_resize_grips).
        .with_resizable(true);
    if !already_open {
        builder = builder
            .with_inner_size(size)
            .with_maximized(geometry.maximized);
        if let Some(position) = geometry.normal_position {
            builder = builder.with_position(position);
        }
    }
    builder
}

/// Renders the top title-bar strip shared by every secondary window, wrapping
/// [`secondary_title_bar`] in the same frame the main window uses: a subtle
/// panel-fill tone with no border stroke, so there is no hard divider line under the
/// title (the strokes here used to read as an abrupt full-width rule). Returns whether
/// the close button was clicked. `ui` is the viewport's root ui (the immediate-viewport
/// callback's first argument). Safe to reuse the same panel id across windows — each
/// secondary window is its own viewport with an independent ui tree.
pub(crate) fn secondary_title_panel(ui: &mut egui::Ui, title: &str) -> bool {
    let mut close_clicked = false;
    egui::Panel::top("secondary_title_bar")
        .exact_size(34.0)
        .frame(
            egui::Frame::new()
                .fill(ui.style().visuals.panel_fill)
                .inner_margin(egui::Margin::symmetric(10, 3)),
        )
        .show_inside(ui, |ui| {
            close_clicked = secondary_title_bar(ui, title);
        });
    close_clicked
}

pub(crate) fn secondary_title_bar(ui: &mut egui::Ui, title: &str) -> bool {
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
    // Controls first (their natural, localized width may vary), title truncates
    // into whatever's left — see the matching note in `title_bar`.
    let mut controls = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(full_rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    controls.set_clip_rect(full_rect);
    {
        let ui = &mut controls;
        ui.spacing_mut().item_spacing.x = 2.0;
        // Native window controls, matching the main title bar: minimize ·
        // maximize/restore · close (painted glyphs so they render regardless of font).
        if window_control_button(ui, WindowControlIcon::Close, &t("Close")).clicked() {
            close_clicked = true;
        }
        let maximized = ui
            .input(|input| input.viewport().maximized)
            .unwrap_or(false);
        let (icon, tooltip) = if maximized {
            (WindowControlIcon::Restore, t("Restore"))
        } else {
            (WindowControlIcon::Maximize, t("Maximize"))
        };
        if window_control_button(ui, icon, &tooltip).clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        if window_control_button(ui, WindowControlIcon::Minimize, &t("Minimize")).clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
    }
    let controls_left = controls.min_rect().left();

    let title_rect = egui::Rect::from_min_max(
        full_rect.min,
        egui::pos2((controls_left - 6.0).max(full_rect.left()), full_rect.max.y),
    );
    let mut left = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(title_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    left.set_clip_rect(title_rect);
    left.add_sized(
        title_rect.size(),
        egui::Label::new(
            RichText::new(title)
                .size(13.0)
                .strong()
                .color(left.visuals().text_color()),
        )
        .truncate(),
    );
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

#[cfg(test)]
mod viewport_tests {
    use super::*;

    fn track_geometry(
        viewport_size: egui::Vec2,
        position: egui::Pos2,
        minimized: bool,
        maximized: bool,
    ) -> (egui::Vec2, SecondaryViewportGeometry) {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport_size)),
            ..Default::default()
        };
        let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.minimized = Some(minimized);
        viewport.maximized = Some(maximized);
        viewport.outer_rect = Some(egui::Rect::from_min_size(position, viewport_size));
        let mut saved_size = egui::vec2(913.0, 587.0);
        let mut geometry = SecondaryViewportGeometry {
            normal_position: Some(egui::pos2(137.0, 89.0)),
            maximized: false,
        };

        let _ = ctx.run_ui(input, |ui| {
            track_secondary_viewport_geometry(ui.ctx(), &mut saved_size, &mut geometry);
        });

        (saved_size, geometry)
    }

    #[test]
    fn secondary_viewport_reinitialization_restores_saved_builder_state() {
        let saved_size = egui::vec2(913.0, 587.0);
        let saved_position = egui::pos2(321.0, 147.0);
        let builder = secondary_viewport_builder(
            "secondary",
            saved_size,
            config::CONSOLE_WINDOW_MIN_SIZE,
            SecondaryViewportGeometry {
                normal_position: Some(saved_position),
                maximized: true,
            },
            false,
        );

        assert_eq!(builder.inner_size, Some(saved_size));
        assert_eq!(builder.position, Some(saved_position));
        assert_eq!(builder.maximized, Some(true));
        assert_eq!(builder.decorations, Some(false));
        assert_eq!(builder.resizable, Some(true));
        assert_eq!(builder.window_level, Some(egui::WindowLevel::AlwaysOnTop));
    }

    #[test]
    fn minimized_viewport_does_not_replace_normal_geometry() {
        let (saved_size, geometry) =
            track_geometry(egui::vec2(240.0, 38.0), egui::pos2(0.0, 0.0), true, false);

        assert_eq!(saved_size, egui::vec2(913.0, 587.0));
        assert_eq!(
            geometry,
            SecondaryViewportGeometry {
                normal_position: Some(egui::pos2(137.0, 89.0)),
                maximized: false,
            }
        );
    }

    #[test]
    fn restored_viewport_updates_normal_geometry() {
        let restored_position = egui::pos2(420.0, 235.0);
        let (saved_size, geometry) =
            track_geometry(egui::vec2(840.0, 520.0), restored_position, false, false);

        assert_eq!(saved_size, egui::vec2(840.0, 520.0));
        assert_eq!(geometry.normal_position, Some(restored_position));
        assert!(!geometry.maximized);
    }

    #[test]
    fn maximized_viewport_preserves_normal_geometry() {
        let (saved_size, geometry) =
            track_geometry(egui::vec2(1920.0, 1040.0), egui::Pos2::ZERO, false, true);

        assert_eq!(saved_size, egui::vec2(913.0, 587.0));
        assert_eq!(geometry.normal_position, Some(egui::pos2(137.0, 89.0)));
        assert!(geometry.maximized);
    }
}

#[cfg(test)]
mod glyph_tests {
    use super::*;

    /// Guards hand-picked symbols painted with the regular UI font stack. A
    /// missing glyph renders as a tofu box (e.g. U+2713 "✓" is absent while
    /// U+2714 "✔" exists), so additions to this set must pass here first.
    #[test]
    fn painted_glyphs_exist_in_font_stack() {
        let definitions = font_definitions().unwrap_or_default();
        let mut fonts =
            egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), definitions);
        let font_id = egui::FontId::proportional(14.0);
        for c in "✔›‹▲▼".chars() {
            assert!(
                fonts.has_glyph(&font_id, c),
                "glyph {c} (U+{:04X}) is missing from the app font stack",
                c as u32
            );
        }
    }

    #[test]
    fn console_sidebar_icons_exist_in_material_font() {
        let insert = egui_material_icons::font_insert();
        let family = ConsoleTab::Settings.icon().font_family();
        assert!(
            insert.families.iter().any(|entry| entry.family == family),
            "Material Icons font insert must register the sidebar icon family"
        );

        let mut definitions = egui::FontDefinitions::empty();
        definitions
            .font_data
            .insert(insert.name.clone(), insert.data.into());
        definitions.families.insert(family, vec![insert.name]);
        let mut fonts =
            egui::epaint::text::Fonts::new(egui::epaint::text::TextOptions::default(), definitions);

        for tab in ConsoleTab::visible_tabs() {
            let icon = tab.icon();
            let font_id = egui::FontId::new(14.0, icon.font_family());
            for c in icon.codepoint.chars() {
                assert!(
                    fonts.has_glyph(&font_id, c),
                    "sidebar icon glyph {c} (U+{:04X}) is missing from Material Icons",
                    c as u32
                );
            }
        }
    }
}

#[cfg(test)]
mod confirmation_tests {
    use super::*;

    #[test]
    fn active_session_reset_warns_that_the_current_task_stops() {
        let (title, message, confirm) = confirmation_content(&ConfirmationAction::ResetSession);

        assert_eq!(title, "Confirm Reset");
        assert_eq!(
            message,
            t(
                "This stops the current task and clears this session's stats, abyss state and detail caches."
            )
        );
        assert_eq!(confirm, "Reset");
    }
}
