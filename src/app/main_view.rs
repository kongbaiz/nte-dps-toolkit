use super::*;

const TOOLBAR_FLEX_GAP: f32 = 24.0;
const HUD_PREVIEW_ROW_COUNT: usize = 4;

struct HudPreviewData {
    rows: Vec<CharacterStats>,
    total_damage: f64,
    team_dps: f64,
    duration: f64,
    damage_taken: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainToolbarLayout {
    SingleRow,
    TwoRows,
    ThreeRows,
}

impl MainToolbarLayout {
    fn height(self) -> f32 {
        match self {
            Self::SingleRow => MAIN_CONTROLS_SINGLE_ROW_HEIGHT,
            Self::TwoRows => MAIN_CONTROLS_SINGLE_ROW_HEIGHT * 2.0,
            Self::ThreeRows => MAIN_CONTROLS_SINGLE_ROW_HEIGHT * 3.0,
        }
    }
}

fn main_toolbar_layout(
    available_width: f32,
    lifecycle_width: f32,
    primary_width: f32,
    context_width: f32,
    overlay_width: f32,
) -> MainToolbarLayout {
    if lifecycle_width + overlay_width + TOOLBAR_FLEX_GAP <= available_width {
        MainToolbarLayout::SingleRow
    } else if primary_width <= available_width
        && context_width + overlay_width + TOOLBAR_FLEX_GAP <= available_width
    {
        MainToolbarLayout::TwoRows
    } else {
        MainToolbarLayout::ThreeRows
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MainWidthClass {
    Compact,
    Standard,
    Wide,
}

fn main_width_class(width: f32) -> MainWidthClass {
    if width < 420.0 {
        MainWidthClass::Compact
    } else if width <= 560.0 {
        MainWidthClass::Standard
    } else {
        MainWidthClass::Wide
    }
}

impl DpsApp {
    pub(crate) fn abyss_selector(&mut self, ui: &mut egui::Ui) {
        if !self.state.abyss.is_active() {
            return;
        }
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().interact_size.y = 28.0;
                let floor = self.state.abyss.floor.map_or_else(
                    || t("Abyss"),
                    |floor| tf("Abyss Floor {}", &[&floor.to_string()]),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(floor)
                            .size(13.0)
                            .strong()
                            .color(self.theme().fg),
                    )
                    .selectable(false),
                );
                ui.separator();
                stable_selectable_value(
                    ui,
                    &mut self.selected_abyss_half,
                    AbyssHalf::First,
                    RichText::new(t(AbyssHalf::First.label())).size(13.0),
                );
                stable_selectable_value(
                    ui,
                    &mut self.selected_abyss_half,
                    AbyssHalf::Second,
                    RichText::new(t(AbyssHalf::Second.label())).size(13.0),
                );
                if self.state.abyss.success_at.is_some() {
                    ui.separator();
                    ui.label(
                        RichText::new(t("Challenge Cleared"))
                            .color(semantic_success(self.dark_mode)),
                    );
                }
                if self.abyss_compact_mode {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("Expand")).clicked() {
                            self.abyss_compact_mode = false;
                        }
                    });
                }
            },
        );
        ui.add_space(3.0);
    }

    pub(crate) fn summary_bar(&mut self, ui: &mut egui::Ui) {
        let (duration, dps, total_damage, total_damage_taken) =
            if let Some(party) = self.selected_party_state() {
                (
                    self.party_duration_for_current_mode(party),
                    self.party_dps_for_current_mode(party),
                    party.total_damage,
                    party.total_damage_taken,
                )
            } else {
                (
                    self.state_duration_for_current_mode(),
                    self.state_dps_for_current_mode(),
                    self.state.total_damage,
                    self.state.total_damage_taken,
                )
            };
        let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
        let abyss_active = self.state.abyss.is_active();
        let dps_trend = motion::trend_indicator(
            ui.ctx(),
            (
                "main_summary_dps_trend",
                self.session_epoch,
                abyss_active,
                second_half,
            ),
            dps,
            self.reduce_motion,
        );
        let dps = motion::rolling_value(
            ui.ctx(),
            (
                "main_summary_dps",
                self.session_epoch,
                abyss_active,
                second_half,
            ),
            dps,
            motion::dur::BASE,
            self.reduce_motion,
        );
        let total_damage = motion::rolling_value(
            ui.ctx(),
            (
                "main_summary_total_damage",
                self.session_epoch,
                abyss_active,
                second_half,
            ),
            total_damage,
            motion::dur::BASE,
            self.reduce_motion,
        );
        let start_pulse = motion::bounce_envelope(motion::animate_generation(
            ui.ctx(),
            "combat_start_pulse",
            self.combat_start_generation,
            motion::dur::SLOW,
            self.reduce_motion,
        ));
        let end_bounce = motion::bounce_envelope(motion::animate_generation(
            ui.ctx(),
            "combat_end_bounce",
            self.combat_end_generation,
            motion::dur::SLOW,
            self.reduce_motion,
        ));
        let width_class = main_width_class(ui.available_width());
        let summary_height = 38.0 * density_tokens(self.density).font_scale;
        let summary_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), summary_height),
        );
        if start_pulse > 0.0 {
            ui.painter().rect_filled(
                summary_rect,
                6.0,
                self.theme().accent.gamma_multiply(start_pulse * 0.16),
            );
        }
        ui.spacing_mut().item_spacing.x = 6.0;
        let accent = self.theme().accent;
        let paint_primary_metrics = |columns: &mut [egui::Ui]| {
            let dps_metric_bounds = columns[0].available_rect_before_wrap();
            compact_metric(
                &mut columns[0],
                &t("Team DPS"),
                format_number(dps),
                accent,
                true,
            );
            paint_metric_trend(&columns[0], dps_metric_bounds, dps_trend, accent);
            let total_color = columns[1].visuals().text_color();
            compact_metric(
                &mut columns[1],
                &t("Total Damage"),
                format_number(total_damage),
                total_color,
                true,
            );
        };
        match width_class {
            MainWidthClass::Compact => ui.columns(2, paint_primary_metrics),
            MainWidthClass::Standard | MainWidthClass::Wide => ui.columns(4, |columns| {
                paint_primary_metrics(&mut columns[..2]);
                compact_metric(
                    &mut columns[2],
                    &t("Total Damage Taken"),
                    format_number(total_damage_taken),
                    semantic_danger(self.dark_mode),
                    false,
                );
                let time_color = columns[3].visuals().text_color();
                compact_metric_scaled(
                    &mut columns[3],
                    &t("Time"),
                    tf("{}s", &[&format!("{duration:.1}")]),
                    time_color,
                    false,
                    1.0 + end_bounce * 0.06,
                );
            }),
        }
        if end_bounce > 0.0 {
            ui.painter().rect_stroke(
                summary_rect.expand(end_bounce * 2.0),
                7.0,
                Stroke::new(
                    1.0_f32 + end_bounce,
                    self.theme().accent.gamma_multiply(end_bounce * 0.8),
                ),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// Live-combat toolbar: capture lifecycle plus the overlay-shrink toggle.
    /// Everything else (settings, team data, abyss tables, debug) moved into the
    /// console window — see [`Self::console_panel`] — to keep this bar uncrowded.
    fn controls(&mut self, ui: &mut egui::Ui, layout: MainToolbarLayout) {
        let density = density_tokens(self.density);
        ui.spacing_mut().item_spacing.x = density.item_spacing.x;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.spacing_mut().button_padding = density.button_padding;
        match layout {
            MainToolbarLayout::SingleRow => {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| self.control_buttons(ui),
                );
            }
            MainToolbarLayout::TwoRows => {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        self.capture_lifecycle_button(ui);
                        self.reset_session_button(ui);
                        self.processing_button(ui);
                        self.console_button(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        self.context_buttons(ui);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.overlay_toggle_buttons(ui)
                        });
                    },
                );
            }
            MainToolbarLayout::ThreeRows => {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        self.capture_lifecycle_button(ui);
                        self.reset_session_button(ui);
                        self.processing_button(ui);
                        self.console_button(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| self.context_buttons(ui),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), MAIN_CONTROLS_SINGLE_ROW_HEIGHT),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| self.overlay_toggle_buttons(ui),
                );
            }
        }
    }

    fn control_buttons(&mut self, ui: &mut egui::Ui) {
        self.lifecycle_buttons(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.overlay_toggle_buttons(ui)
        });
    }

    /// Capture-lifecycle buttons on the left of the toolbar: start/stop, reset,
    /// pause/resume, the abyss collapse toggle, HUD and console.
    fn lifecycle_buttons(&mut self, ui: &mut egui::Ui) {
        self.capture_lifecycle_button(ui);
        self.reset_session_button(ui);
        self.processing_button(ui);
        self.context_buttons(ui);
        self.console_button(ui);
    }

    fn context_buttons(&mut self, ui: &mut egui::Ui) {
        if self.state.abyss.is_active()
            && ui
                .button(t("Collapse"))
                .on_hover_text(t("Collapse the abyss line selector and toolbar"))
                .clicked()
        {
            self.abyss_compact_mode = true;
        }
        if ui
                .button(t("HUD"))
            .on_hover_text(t("Switch to the backing-less combat HUD (overlays the game · exit from the appearance menu)"))
            .clicked()
        {
            self.set_hud_mode(ui.ctx(), true);
        }
    }

    fn toolbar_layout(&self, ui: &egui::Ui) -> MainToolbarLayout {
        let density = density_tokens(self.density);
        let primary_labels = [
            if self.capture.is_none() && self.replay_thread.is_none() {
                t("Start")
            } else {
                t("Stop")
            },
            t("Reset"),
            if self.paused { t("Resume") } else { t("Pause") },
            t("Console"),
        ];
        let collapse_label = self.state.abyss.is_active().then(|| t("Collapse"));
        let hud_label = t("HUD");
        let overlay_labels = [
            t("Appearance"),
            if self.mouse_passthrough {
                t("Passthrough on")
            } else {
                t("Passthrough")
            },
            t("Pin"),
        ];
        let primary_width = toolbar_button_group_width(
            ui,
            primary_labels.iter().map(String::as_str),
            density.button_padding.x,
            density.item_spacing.x,
        );
        let context_width = toolbar_button_group_width(
            ui,
            collapse_label
                .as_deref()
                .into_iter()
                .chain(std::iter::once(hud_label.as_str())),
            density.button_padding.x,
            density.item_spacing.x,
        );
        let lifecycle_width = primary_width + density.item_spacing.x + context_width;
        let overlay_width = toolbar_button_group_width(
            ui,
            overlay_labels.iter().map(String::as_str),
            density.button_padding.x,
            density.item_spacing.x,
        );
        main_toolbar_layout(
            ui.available_width(),
            lifecycle_width,
            primary_width,
            context_width,
            overlay_width,
        )
    }

    fn capture_lifecycle_button(&mut self, ui: &mut egui::Ui) {
        if self.capture.is_none() && self.replay_thread.is_none() {
            if ui
                .add(primary_button(t("Start"), self.theme().accent))
                .on_hover_text(t(
                    "Auto-detect the HTGame.exe connection and NIC, then start live capture",
                ))
                .clicked()
            {
                self.request_start_live(ui.ctx());
            }
        } else if ui
            .add(
                egui::Button::new(
                    RichText::new(t("Stop"))
                        .strong()
                        .color(semantic_danger(self.dark_mode)),
                )
                .stroke(Stroke::new(1.0_f32, semantic_danger(self.dark_mode))),
            )
            .on_hover_text(t("Stop the current live capture or import replay"))
            .clicked()
        {
            self.stop_engine();
            self.drain_pending_events();
        }
    }

    fn reset_session_button(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(t("Reset"))
            .on_hover_text(t(
                "Clear the current stats; undo remains available for 5 seconds",
            ))
            .clicked()
        {
            self.request_reset_combat_session(ui.ctx());
        }
    }

    fn processing_button(&mut self, ui: &mut egui::Ui) -> bool {
        let clicked = ui
            .add(
                egui::Button::selectable(
                    self.paused,
                    if self.paused { t("Resume") } else { t("Pause") },
                )
                .frame_when_inactive(true),
            )
            .on_hover_text(t(
                "Pause UI processing; resuming catches up on buffered hit events",
            ))
            .clicked();
        if clicked {
            self.paused = !self.paused;
        }
        clicked
    }

    fn console_button(&mut self, ui: &mut egui::Ui) {
        if ui
            .button(t("Console"))
            .on_hover_text(t(
                "Settings · team data · abyss values · character/INI · debug (F12)",
            ))
            .clicked()
        {
            self.console_open = true;
            self.console_corner_applied = false;
        }
    }

    /// Overlay toggles on the right of the toolbar: appearance menu, passthrough, pin.
    /// Added appearance→passthrough→pin so a right-to-left layout renders them
    /// pin · passthrough · appearance (a left-to-right sizing pass measures the same
    /// total width regardless of order).
    pub(crate) fn overlay_toggle_buttons(&mut self, ui: &mut egui::Ui) {
        self.appearance_menu(ui);
        self.passthrough_button(ui);
        self.pin_button(ui);
    }

    fn passthrough_button(&mut self, ui: &mut egui::Ui) -> bool {
        let passthrough_label = if self.mouse_passthrough {
            t("Passthrough on")
        } else {
            t("Passthrough")
        };
        let clicked = ui
            .add(
                egui::Button::selectable(self.mouse_passthrough, passthrough_label)
                    .frame_when_inactive(true),
            )
            .on_hover_text(tf(
                "{} toggles mouse passthrough anytime",
                &[self.passthrough_hotkey.label()],
            ))
            .clicked();
        if clicked {
            self.toggle_mouse_passthrough(ui.ctx());
        }
        clicked
    }

    fn pin_button(&mut self, ui: &mut egui::Ui) -> bool {
        let clicked = ui
            .add(egui::Button::selectable(self.always_on_top, t("Pin")).frame_when_inactive(true))
            .on_hover_text(t("Keep the main window above the game"))
            .clicked();
        if clicked {
            self.toggle_always_on_top(ui.ctx());
        }
        clicked
    }

    /// Appearance dropdown (opacity · theme · Combat HUD), shared by the live
    /// toolbar. Moved off the title bar; see [`Self::control_buttons`].
    pub(crate) fn appearance_menu(&mut self, ui: &mut egui::Ui) {
        let (appearance_response, _) = egui::containers::menu::MenuButton::from_button(
            egui::Button::new(t("Appearance")),
        )
        .ui(ui, |ui| {
            ui.set_min_width(190.0);
            ui.horizontal(|ui| {
                ui.label(t("Opacity"));
                ui.add(
                    egui::Slider::new(&mut self.opacity, 0.35..=1.0)
                        .show_value(true)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)),
                );
            });
            if ui
                .button(if self.dark_mode {
                    t("Switch to light")
                } else {
                    t("Switch to dark")
                })
                .clicked()
            {
                self.toggle_theme(ui.ctx());
                ui.close();
            }
            ui.menu_button(t("Theme Preset"), |ui| {
                for preset in ThemePreset::all() {
                    if ui
                        .selectable_label(self.theme_preset == *preset, t(preset.label()))
                        .on_hover_text(t(preset.description()))
                        .clicked()
                    {
                        self.set_theme_preset(ui.ctx(), *preset);
                        ui.close();
                    }
                }
            });
            ui.separator();
            if self.processing_button(ui) {
                ui.close();
            }
            if self.state.abyss.is_active()
                && ui
                    .button(t("Collapse"))
                    .on_hover_text(t("Collapse the abyss line selector and toolbar"))
                    .clicked()
            {
                self.abyss_compact_mode = true;
                ui.close();
            }
            if ui
                .add(
                    egui::Button::selectable(self.hud_mode, t("Combat HUD"))
                        .frame_when_inactive(true),
                )
                .on_hover_text(t("Backing-less HUD, overlaid directly on the game"))
                .clicked()
            {
                self.set_hud_mode(ui.ctx(), !self.hud_mode);
                ui.close();
            }
            if self.passthrough_button(ui) {
                ui.close();
            }
            if self.pin_button(ui) {
                ui.close();
            }
            if !self.hidden_character_ids.is_empty()
                && ui
                    .button(tf(
                        "Show hidden characters ({})",
                        &[&self.hidden_character_ids.len().to_string()],
                    ))
                    .clicked()
            {
                self.hidden_character_ids.clear();
                ui.close();
            }
        });
        appearance_response.on_hover_text(t("Adjust opacity, theme and HUD mode"));
    }

    pub(crate) fn animated_controls(&mut self, ui: &mut egui::Ui) {
        let expanded = !self.abyss_compact_mode || !self.state.abyss.is_active();
        let progress = motion::animate_bool(
            ui.ctx(),
            "main_controls_expanded",
            expanded,
            motion::dur::BASE,
            self.reduce_motion,
            motion::ease::entrance,
        );
        if progress <= 0.001 {
            return;
        }

        let toolbar_layout = self.toolbar_layout(ui);
        let full_height = toolbar_layout.height();
        let content_top_offset = 2.5;
        let visible_height = full_height * progress;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), visible_height),
            egui::Sense::hover(),
        );
        let full_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0, content_top_offset),
            egui::vec2((rect.width() - 4.0).max(0.0), full_height),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("animated_controls")
                .max_rect(full_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(rect);
        child.set_opacity(progress);
        self.controls(&mut child, toolbar_layout);
    }

    pub(crate) fn animated_party_content(&mut self, ui: &mut egui::Ui) {
        let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
        let phase = motion::animate_value(
            ui.ctx(),
            "abyss_half_transition",
            if second_half { 1.0 } else { 0.0 },
            motion::dur::SLOW,
            self.reduce_motion,
        );
        let visibility = if second_half { phase } else { 1.0 - phase };
        let direction = if second_half { 1.0 } else { -1.0 };
        let offset_x = direction * (1.0 - visibility) * 14.0;
        let available = ui.available_rect_before_wrap();
        let content_rect = available.translate(egui::vec2(offset_x, 0.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("animated_party_content")
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(available);
        child.set_opacity(0.25 + visibility * 0.75);

        self.summary_bar(&mut child);
        child.add_space(2.0);
        child.horizontal(|ui| {
            ui.label(
                RichText::new(t("Team"))
                    .size(12.0)
                    .strong()
                    .color(self.theme().fg),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self
                            .selected_party_state()
                            .map_or(self.state.hits.is_empty(), |party| party.hits.is_empty()),
                        egui::Button::new(t("Team Combat Details")),
                    )
                    .clicked()
                {
                    self.team_hit_detail_open = true;
                    self.team_hit_detail_filter = HitDetailFilter::All;
                    self.team_hit_detail_corner_applied = false;
                }
            });
        });
        child.label(
            RichText::new(t(
                "Click a character for details; right-click for copy or hide",
            ))
            .size(10.5)
            .color(child.visuals().weak_text_color()),
        );
        self.party_panel(&mut child);
        ui.allocate_rect(available, egui::Sense::hover());
    }

    pub(crate) fn import_loading_content(&mut self, ui: &mut egui::Ui) {
        let available_rect = ui.available_rect_before_wrap();
        ui.allocate_rect(available_rect, egui::Sense::hover());
        let card_size = egui::vec2(
            360.0_f32.min(available_rect.width()),
            190.0_f32.min(available_rect.height()),
        );
        let card_rect = egui::Rect::from_center_size(available_rect.center(), card_size);
        ui.painter().rect(
            card_rect,
            egui::CornerRadius::same(10),
            self.theme().card,
            Stroke::new(1.0_f32, self.theme().border),
            egui::StrokeKind::Inside,
        );
        let content_rect = card_rect.shrink(18.0);
        let mut content = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("import_loading_content")
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        content.add_space((content_rect.height() - 126.0).max(0.0) * 0.5);
        content.add(egui::Spinner::new().size(28.0).color(self.theme().accent));
        content.add_space(8.0);
        content.label(
            RichText::new(t("Importing and parsing capture"))
                .size(15.0)
                .strong()
                .color(self.theme().fg),
        );
        content.add_space(2.0);
        if let Some(task) = &self.active_import {
            let elapsed = task.started_at.elapsed().as_secs();
            content.label(
                RichText::new(format!(
                    "{} · {} · {}s",
                    t(task.kind.label()),
                    file_display_name(&task.path),
                    elapsed
                ))
                .size(11.0)
                .color(content.visuals().weak_text_color()),
            );
        }
        content.label(
            RichText::new(tf(
                "Parsed {} damage records · {} packets",
                &[
                    &self.state.hits.len().to_string(),
                    &self.state.packets.len().to_string(),
                ],
            ))
            .size(11.0)
            .color(content.visuals().weak_text_color()),
        );
        content.add_space(8.0);
        if content.button(t("Cancel Import")).clicked() {
            self.stop_engine();
            self.status = t("Import canceled");
        }
    }

    pub(crate) fn paint_theme_transition(&mut self, ctx: &egui::Context) {
        let Some(color) = self.theme_transition_from else {
            return;
        };
        let progress = motion::animate_bool(
            ctx,
            "theme_transition_overlay",
            true,
            motion::dur::BASE,
            self.reduce_motion,
            motion::ease::entrance,
        );
        if progress >= 1.0 {
            self.theme_transition_from = None;
            return;
        }

        let alpha = ((1.0 - progress) * 96.0).round() as u8;
        let overlay = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha);
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("theme_transition_overlay"),
        ))
        .rect_filled(ctx.content_rect(), 0.0, overlay);
    }

    pub(crate) fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme_transition_from = Some(self.theme().bg);
        motion::seed_bool_for_viewport(
            ctx,
            egui::ViewportId::ROOT,
            "theme_transition_overlay",
            false,
        );
        self.dark_mode = !self.dark_mode;
    }

    pub(crate) fn set_theme_preset(&mut self, ctx: &egui::Context, preset: ThemePreset) {
        if self.theme_preset == preset {
            return;
        }
        self.theme_transition_from = Some(self.theme().bg);
        motion::seed_bool_for_viewport(
            ctx,
            egui::ViewportId::ROOT,
            "theme_transition_overlay",
            false,
        );
        self.theme_preset = preset;
    }

    /// Party-member rows (damage desc) plus the team totals shared by the party
    /// panel and the HUD. Returns owned rows so callers can paint without holding
    /// a borrow on `state`. Tuple: `(rows, total_damage, team_dps, duration)`.
    /// Character display name for the active UI language, falling back to `fallback`
    /// (the name baked at parse time) when the character table has no usable entry.
    /// Thin wrapper over [`character_display_name`] for `DpsApp` call sites.
    pub(crate) fn localized_character_name(&self, char_id: u32, fallback: &str) -> String {
        character_display_name(&self.characters, char_id, fallback)
    }

    pub(crate) fn party_readout(&self) -> (Vec<CharacterStats>, f64, f64, f64) {
        let prep = |stats: &HashMap<u32, CharacterStats>,
                    hits: &VecDeque<crate::engine::model::Hit>,
                    total: f64,
                    dps: f64,
                    duration: f64| {
            let mut rows: Vec<CharacterStats> = stats
                .values()
                .filter(|row| {
                    is_party_member_row(row, hits)
                        && !self.hidden_character_ids.contains(&row.char_id)
                })
                .cloned()
                .collect();
            rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
            (rows, total, dps, duration)
        };
        let (mut rows, total, dps, duration) = if let Some(party) = self.selected_party_state() {
            prep(
                &party.stats,
                &party.hits,
                party.total_damage,
                self.party_dps_for_current_mode(party),
                self.party_duration_for_current_mode(party),
            )
        } else {
            prep(
                &self.state.stats,
                &self.state.hits,
                self.state.total_damage,
                self.state_dps_for_current_mode(),
                self.state_duration_for_current_mode(),
            )
        };
        // Resolve each row's display name to the active language; the baked name
        // (Chinese, from parse time) is the fallback for unknown characters.
        for row in &mut rows {
            row.name = self.localized_character_name(row.char_id, &row.name);
        }
        (rows, total, dps, duration)
    }

    /// Cheap count of party-member rows (no clone), for HUD window sizing.
    pub(crate) fn party_member_count(&self) -> usize {
        if let Some(party) = self.selected_party_state() {
            party
                .stats
                .values()
                .filter(|row| {
                    is_party_member_row(row, &party.hits)
                        && !self.hidden_character_ids.contains(&row.char_id)
                })
                .count()
        } else {
            self.state
                .stats
                .values()
                .filter(|row| {
                    is_party_member_row(row, &self.state.hits)
                        && !self.hidden_character_ids.contains(&row.char_id)
                })
                .count()
        }
    }

    pub(crate) fn hud_visible_row_count(&self) -> usize {
        if self.hud_config.show_character_rows {
            let count = self.party_member_count().min(TEAM_DPS_MAX_MEMBERS);
            if !self.mouse_passthrough && count == 0 {
                let (_, total_damage, team_dps, _) = self.party_readout();
                if total_damage <= 0.0 && team_dps <= 0.0 {
                    return HUD_PREVIEW_ROW_COUNT;
                }
            }
            count
        } else {
            0
        }
    }

    pub(crate) fn hud_status_row_visible(&self) -> bool {
        (self.hud_config.show_abyss_half && self.state.abyss.is_active())
            || self.hud_config.show_passthrough_state
    }

    pub(crate) fn party_panel(&mut self, ui: &mut egui::Ui) {
        let (rows, total_damage, _, _) = self.party_readout();
        let density_scale = density_tokens(self.density).font_scale;
        let available_height = ui.available_height();
        let row_height = (party_row_height(available_height, rows.len()) * density_scale)
            .min(available_height.max(38.0 * density_scale));
        if rows.is_empty() {
            if self.hidden_character_ids.is_empty() {
                self.main_empty_state(ui);
            } else {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(t("All ranking rows are hidden"))
                            .color(ui.visuals().weak_text_color()),
                    );
                    if ui.button(t("Show all characters")).clicked() {
                        self.hidden_character_ids.clear();
                    }
                });
            }
            return;
        }

        let row_spacing = 5.0 * density_scale;
        let total_height =
            row_height * rows.len() as f32 + row_spacing * rows.len().saturating_sub(1) as f32;
        let ranking_rect = ui.available_rect_before_wrap();
        let ranking_scrolling = ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|position| ranking_rect.contains(position))
                && (input.is_scrolling()
                    || input.smooth_scroll_delta() != egui::Vec2::ZERO
                    || input.pointer.primary_down())
        });
        egui::ScrollArea::vertical()
            .id_salt("party_ranking")
            .max_height(available_height.max(38.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                let (container, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), total_height),
                    egui::Sense::hover(),
                );
                let stride = row_height + row_spacing;
                let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
                let abyss_active = self.state.abyss.is_active();
                let visible_rect = ui.clip_rect().intersect(container).expand(stride);
                for (index, row) in rows.iter().enumerate().rev() {
                    let target_y = index as f32 * stride;
                    let animation_id = (
                        "party_rank_y",
                        self.session_epoch,
                        abyss_active,
                        second_half,
                        row.char_id,
                    );
                    let target_rect = egui::Rect::from_min_size(
                        egui::pos2(container.left(), container.top() + target_y),
                        egui::vec2(container.width(), row_height),
                    );
                    if !visible_rect.intersects(target_rect) || ranking_scrolling {
                        motion::snap_value(ui.ctx(), animation_id, target_y);
                    }
                    if !visible_rect.intersects(target_rect) {
                        continue;
                    }
                    let animated_y = if ranking_scrolling {
                        target_y
                    } else {
                        motion::animate_value(
                            ui.ctx(),
                            animation_id,
                            target_y,
                            motion::dur::BASE,
                            self.reduce_motion,
                        )
                    };
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(container.left(), container.top() + animated_y),
                        egui::vec2(container.width(), row_height),
                    );
                    let mut row_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("party_row", row.char_id))
                            .max_rect(row_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    row_ui.set_clip_rect(ui.clip_rect().intersect(container));
                    self.draw_party_row(&mut row_ui, row, index, total_damage, row_height);
                }
            });
    }

    fn main_empty_state(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme();
        let game_detected = self.game_process_detected;
        let game_process_error = self.game_process_monitor_error.clone();
        let capture_active = self.capture.is_some() || self.replay_thread.is_some();
        let has_damage = !self.state.hits.is_empty();
        let mut action = None;
        // Centered card, sized by hand: the card gets its own fixed-width
        // top-down child region because `centered_and_justified` only centers
        // a single widget, and a `Frame` child inherits the parent layout —
        // multi-widget content inside it would flow left-to-right.
        let card_height = ui.available_height().clamp(150.0, 220.0);
        let card_width = (ui.available_width() - 12.0).clamp(0.0, 420.0);
        ui.add_space(((ui.available_height() - card_height) / 2.0).max(0.0));
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - card_width) / 2.0).max(0.0));
            ui.allocate_ui_with_layout(
                egui::vec2(card_width, card_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::Frame::new()
                        .fill(theme.card)
                        .stroke(Stroke::new(1.0_f32, theme.border))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(18, 14))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(t("Ready for your next combat"))
                                    .size(14.0)
                                    .strong()
                                    .color(theme.fg),
                            );
                            ui.add_space(8.0);
                            main_empty_step(ui, 1, t("Start HTGame.exe"), game_detected, theme);
                            main_empty_step(ui, 2, t("Start live capture"), capture_active, theme);
                            main_empty_step(
                                ui,
                                3,
                                t("Enter combat and deal damage"),
                                has_damage,
                                theme,
                            );
                            if let Some(error) = &game_process_error {
                                ui.small(
                                    RichText::new(tf("Game process check failed: {}", &[error]))
                                        .color(theme.danger),
                                );
                            }
                            ui.add_space(10.0);
                            ui.horizontal_wrapped(|ui| {
                                if !game_detected
                                    && !self.awaiting_device_detection
                                    && ui.button(t("Re-detect")).clicked()
                                {
                                    action = Some(MainEmptyAction::Detect);
                                }
                                if !capture_active
                                    && ui
                                        .add(primary_button(t("Start Capture"), theme.accent))
                                        .clicked()
                                {
                                    action = Some(MainEmptyAction::Capture);
                                }
                                if !capture_active && ui.button(t("Import Replay")).clicked() {
                                    action = Some(MainEmptyAction::Import);
                                }
                                if capture_active {
                                    ui.weak(t("Waiting for the first damage event..."));
                                }
                            });
                        });
                },
            );
        });

        match action {
            Some(MainEmptyAction::Detect) => {
                if let Err(error) = self.refresh_game_network() {
                    self.set_last_error_in(ui.ctx(), error, Some(ErrorAction::RefreshNetwork));
                }
            }
            Some(MainEmptyAction::Capture) => self.request_start_live(ui.ctx()),
            Some(MainEmptyAction::Import) => {
                self.request_debug_import(ui.ctx(), DebugImportKind::Pcapng)
            }
            None => {}
        }
    }

    /// Combat HUD optimized for quick scanning: one team DPS header and a compact
    /// horizontal damage-share board. Details stay in the normal window.
    pub(crate) fn hud_panel(&mut self, ui: &mut egui::Ui) {
        let hud_theme = self.theme().hud;
        let editor = !self.mouse_passthrough;
        let display_text = if editor {
            hud_theme.edit_text
        } else {
            hud_theme.text
        };
        let display_muted = if editor {
            mix_color(hud_theme.edit_text, hud_theme.edit_bg, 0.45)
        } else {
            hud_theme.muted
        };
        let display_halo = if editor && !self.dark_mode {
            Color32::TRANSPARENT
        } else {
            hud_theme.halo
        };
        let display_accent = if editor {
            hud_theme.edit_border
        } else {
            hud_theme.accent
        };
        let (mut rows, mut total_damage, mut team_dps, mut duration) = self.party_readout();
        let preview =
            !self.mouse_passthrough && rows.is_empty() && total_damage <= 0.0 && team_dps <= 0.0;
        let mut damage_taken = self.current_damage_taken_for_hud();
        if preview {
            let preview_data = hud_preview_party_readout();
            rows = preview_data.rows;
            total_damage = preview_data.total_damage;
            team_dps = preview_data.team_dps;
            duration = preview_data.duration;
            damage_taken = preview_data.damage_taken;
        }
        if self.hud_config.show_character_rows {
            rows.truncate(TEAM_DPS_MAX_MEMBERS);
        } else {
            rows.clear();
        }
        let area = ui.available_rect_before_wrap();
        let horizontal_inset = 8.0;
        let left = area.left() + horizontal_inset;
        let width = (area.width() - horizontal_inset * 2.0)
            .min(self.hud_config.width as f32 - horizontal_inset * 2.0);
        let right = left + width;
        let painter = ui.painter().clone();
        let colors = HudPaintColors {
            accent: display_accent,
            text: display_text,
            muted: display_muted,
            halo: display_halo,
        };

        if rows.is_empty() && total_damage <= 0.0 && team_dps <= 0.0 {
            let empty = egui::Rect::from_min_size(
                egui::pos2(left, area.top() + 8.0),
                egui::vec2(width.min(210.0), 38.0),
            );
            paint_haloed_with_halo(
                &painter,
                egui::pos2(empty.left(), empty.center().y),
                egui::Align2::LEFT_CENTER,
                t("Waiting for damage data"),
                egui::FontId::proportional(13.0),
                display_muted,
                display_halo,
            );
            ui.allocate_rect(empty, egui::Sense::hover());
            return;
        }

        let mut top = area.top() + 8.0;
        let mut reorder = None;
        let mut hide = None;
        let mut restore = None;
        let hidden_modules = self
            .hud_config
            .module_order
            .iter()
            .copied()
            .filter(|module| !self.hud_config.module_visible(*module))
            .collect::<Vec<_>>();
        if editor && !hidden_modules.is_empty() {
            let tray_height = hud_editor_hidden_tray_height(hidden_modules.len());
            let tray_rect = egui::Rect::from_min_size(
                egui::pos2(left - 4.0, top),
                egui::vec2(width + 8.0, tray_height),
            );
            ui.allocate_rect(tray_rect, egui::Sense::hover());
            painter.rect_filled(tray_rect, 4.0, hud_theme.edit_bg);
            painter.rect_stroke(
                tray_rect,
                4.0,
                Stroke::new(1.0_f32, hud_theme.edit_border),
                egui::StrokeKind::Inside,
            );
            painter.text(
                tray_rect.left_top() + egui::vec2(7.0, HUD_EDITOR_HIDDEN_TRAY_HEADER_HEIGHT * 0.5),
                egui::Align2::LEFT_CENTER,
                t("Hidden modules"),
                egui::FontId::proportional(10.0),
                hud_theme.edit_text,
            );
            for (index, module) in hidden_modules.iter().copied().enumerate() {
                let row_top = tray_rect.top()
                    + HUD_EDITOR_HIDDEN_TRAY_HEADER_HEIGHT
                    + index as f32 * HUD_EDITOR_HIDDEN_MODULE_ROW_HEIGHT;
                let row_rect = egui::Rect::from_min_max(
                    egui::pos2(tray_rect.left() + 4.0, row_top),
                    egui::pos2(
                        tray_rect.right() - 4.0,
                        row_top + HUD_EDITOR_HIDDEN_MODULE_ROW_HEIGHT - 4.0,
                    ),
                );
                let response = ui
                    .interact(
                        row_rect,
                        ui.make_persistent_id(("hud_hidden_module", module)),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(t("Restore module"));
                painter.rect_filled(
                    row_rect,
                    3.0,
                    if response.hovered() {
                        hud_theme.edit_border.gamma_multiply(0.22)
                    } else {
                        hud_theme.edit_border.gamma_multiply(0.1)
                    },
                );
                painter.text(
                    row_rect.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("＋ {}", t(module.label())),
                    egui::FontId::proportional(10.0),
                    hud_theme.edit_text,
                );
                if response.clicked() {
                    restore = Some(module);
                }
            }
            top += tray_height;
        }
        let dragged_module = egui::DragAndDrop::payload::<HudModule>(ui.ctx()).map(|item| *item);
        let mut dragged_size = None;
        for module in self.hud_config.module_order.clone() {
            let target_module_top = top;
            let module_top = if editor {
                motion::animate_value(
                    ui.ctx(),
                    ("hud_module_order_y", module),
                    target_module_top,
                    motion::dur::BASE,
                    self.reduce_motion,
                )
            } else {
                target_module_top
            };
            let target_content_top = target_module_top
                + if editor {
                    HUD_EDITOR_MODULE_HEADER_HEIGHT
                } else {
                    0.0
                };
            let content_top = module_top
                + if editor {
                    HUD_EDITOR_MODULE_HEADER_HEIGHT
                } else {
                    0.0
                };
            let rendered_height = match module {
                HudModule::Title if self.hud_config.show_title => {
                    self.hud_title_readout_row(&painter, left, content_top, width, colors)
                }
                HudModule::Summary if self.hud_config.has_summary_row() => self.hud_summary_row(
                    &painter,
                    egui::Rect::from_min_size(
                        egui::pos2(left, content_top),
                        egui::vec2(width, 50.0),
                    ),
                    HudSummaryValues {
                        total_damage,
                        team_dps,
                        duration,
                        damage_taken,
                    },
                    &rows,
                    colors,
                ),
                HudModule::Status if self.hud_status_row_visible() => {
                    self.hud_status_row(&painter, left, content_top, width, colors)
                }
                HudModule::Characters if !rows.is_empty() => self.hud_character_rows(
                    ui,
                    &painter,
                    HudRowsLayout {
                        left,
                        right,
                        top: content_top,
                        width,
                    },
                    &rows,
                    total_damage,
                    colors,
                ),
                HudModule::Timeline if self.hud_config.show_mini_timeline => {
                    self.hud_mini_timeline(&painter, left, content_top, width, preview, colors)
                }
                HudModule::Title
                | HudModule::Summary
                | HudModule::Status
                | HudModule::Characters
                | HudModule::Timeline => 0.0,
            };
            if rendered_height > 0.0 {
                top = target_content_top + rendered_height;
            }
            if editor && rendered_height > 0.0 {
                let module_rect = egui::Rect::from_min_max(
                    egui::pos2(left - 4.0, module_top - 1.0),
                    egui::pos2(right + 4.0, content_top + rendered_height - 1.0),
                );
                if dragged_module == Some(module) {
                    dragged_size = Some(module_rect.size());
                    painter.rect_filled(module_rect, 4.0, hud_theme.edit_bg);
                }
                paint_hud_module_editor_outline(
                    &painter,
                    module_rect,
                    t(module.label()),
                    hud_theme.edit_border,
                    display_halo,
                );
                let response = ui.interact(
                    module_rect,
                    ui.make_persistent_id(("hud_module_editor", module)),
                    egui::Sense::click_and_drag(),
                );
                response.dnd_set_drag_payload(module);
                let pointer_y = ui
                    .ctx()
                    .pointer_interact_pos()
                    .map_or(module_rect.center().y, |pointer| pointer.y);
                let insert_after = pointer_y >= module_rect.center().y;
                if let Some(dragged) = response.dnd_hover_payload::<HudModule>()
                    && *dragged != module
                {
                    let indicator_y = if insert_after {
                        module_rect.bottom()
                    } else {
                        module_rect.top()
                    };
                    painter.line_segment(
                        [
                            egui::pos2(module_rect.left(), indicator_y),
                            egui::pos2(module_rect.right(), indicator_y),
                        ],
                        Stroke::new(3.0_f32, hud_theme.accent),
                    );
                }
                if let Some(dropped) = response.dnd_release_payload::<HudModule>()
                    && *dropped != module
                {
                    reorder = Some((*dropped, module, insert_after));
                }
                response.context_menu(|ui| {
                    if ui.button(t("Hide module")).clicked() {
                        hide = Some(module);
                        ui.close();
                    }
                });
            }
        }

        if let (Some(module), Some(size), Some(pointer)) = (
            dragged_module,
            dragged_size,
            ui.ctx().pointer_interact_pos(),
        ) {
            paint_hud_drag_ghost(ui.ctx(), pointer, size, module, hud_theme);
        }

        if let Some((dragged, target, insert_after)) = reorder {
            self.hud_config.move_module(dragged, target, insert_after);
        }
        if let Some(module) = hide {
            self.hud_config.set_module_visible(module, false);
        }
        if let Some(module) = restore {
            self.hud_config.set_module_visible(module, true);
        }

        ui.allocate_rect(
            egui::Rect::from_min_max(area.min, egui::pos2(right, top)),
            egui::Sense::hover(),
        );
    }

    pub(crate) fn hud_title_readout_row(
        &self,
        painter: &egui::Painter,
        left: f32,
        top: f32,
        width: f32,
        colors: HudPaintColors,
    ) -> f32 {
        let rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, 20.0));
        paint_haloed(
            painter,
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            "NTE DPS",
            egui::FontId::proportional(12.0),
            colors.text,
            colors.halo,
        );
        paint_haloed(
            painter,
            egui::pos2(rect.right(), rect.center().y),
            egui::Align2::RIGHT_CENTER,
            t(self.dps_time_mode.label()),
            egui::FontId::proportional(10.5),
            colors.muted,
            colors.halo,
        );
        22.0
    }

    pub(crate) fn hud_summary_row(
        &self,
        painter: &egui::Painter,
        header: egui::Rect,
        values: HudSummaryValues,
        rows: &[CharacterStats],
        colors: HudPaintColors,
    ) -> f32 {
        let track_color = self.theme().hud.track;
        let start_pulse = motion::bounce_envelope(motion::animate_generation(
            painter.ctx(),
            "combat_start_pulse",
            self.combat_start_generation,
            motion::dur::SLOW,
            self.reduce_motion,
        ));
        let end_bounce = motion::bounce_envelope(motion::animate_generation(
            painter.ctx(),
            "combat_end_bounce",
            self.combat_end_generation,
            motion::dur::SLOW,
            self.reduce_motion,
        ));
        if start_pulse > 0.0 || end_bounce > 0.0 {
            painter.rect_stroke(
                header.expand(2.0 * end_bounce),
                6.0,
                Stroke::new(
                    1.0 + end_bounce,
                    colors
                        .accent
                        .gamma_multiply((start_pulse * 0.8 + end_bounce * 0.65).min(1.0)),
                ),
                egui::StrokeKind::Outside,
            );
        }
        let duration_scale = 1.0 + end_bounce * 0.06;
        let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
        let abyss_active = self.state.abyss.is_active();
        if self.hud_config.show_team_dps {
            let dps_trend = motion::trend_indicator(
                painter.ctx(),
                (
                    "hud_summary_dps_trend",
                    self.session_epoch,
                    abyss_active,
                    second_half,
                ),
                values.team_dps,
                self.reduce_motion,
            );
            let animated_dps = motion::rolling_value(
                painter.ctx(),
                (
                    "hud_summary_dps",
                    self.session_epoch,
                    abyss_active,
                    second_half,
                ),
                values.team_dps,
                motion::dur::BASE,
                self.reduce_motion,
            );
            let label = if self.hud_config.show_duration {
                format!("{} · {:.1}s", t("Team DPS"), values.duration)
            } else {
                t("Team DPS")
            };
            paint_haloed(
                painter,
                egui::pos2(header.left(), header.top() + 12.0),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(10.5 * duration_scale),
                colors.muted,
                colors.halo,
            );
            let dps_text = format_number(animated_dps);
            let dps_font = egui::FontId::proportional(26.0);
            let dps_width = painter
                .layout_no_wrap(dps_text.clone(), dps_font.clone(), colors.accent)
                .size()
                .x;
            let dps_pos = egui::pos2(header.left(), header.bottom() - 17.0);
            paint_haloed(
                painter,
                dps_pos,
                egui::Align2::LEFT_CENTER,
                dps_text,
                dps_font,
                colors.accent,
                colors.halo,
            );
            paint_hud_trend(
                painter,
                egui::pos2(dps_pos.x + dps_width + 5.0, dps_pos.y),
                dps_trend,
                colors.accent,
            );
        } else if self.hud_config.show_duration {
            paint_haloed(
                painter,
                egui::pos2(header.left(), header.center().y),
                egui::Align2::LEFT_CENTER,
                format!("{} {:.1}s", t("Time"), values.duration),
                egui::FontId::monospace(14.0 * duration_scale),
                colors.text,
                colors.halo,
            );
        }

        let animated_total_damage = if self.hud_config.show_total_damage {
            motion::rolling_value(
                painter.ctx(),
                (
                    "hud_summary_total_damage",
                    self.session_epoch,
                    abyss_active,
                    second_half,
                ),
                values.total_damage,
                motion::dur::BASE,
                self.reduce_motion,
            )
        } else {
            values.total_damage
        };
        let right_label = match (
            self.hud_config.show_total_damage,
            self.hud_config.show_damage_taken,
        ) {
            (true, true) => Some((
                t("Total Damage / Taken"),
                format!(
                    "{} / {}",
                    format_number(animated_total_damage),
                    format_number(values.damage_taken)
                ),
            )),
            (true, false) => Some((t("Total Damage"), format_number(animated_total_damage))),
            (false, true) => Some((t("Total Damage Taken"), format_number(values.damage_taken))),
            (false, false) => None,
        };
        if let Some((label, value)) = right_label {
            paint_haloed(
                painter,
                egui::pos2(header.right(), header.top() + 12.0),
                egui::Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(10.5),
                colors.muted,
                colors.halo,
            );
            paint_haloed(
                painter,
                egui::pos2(header.right(), header.bottom() - 17.0),
                egui::Align2::RIGHT_CENTER,
                value,
                egui::FontId::monospace(13.0),
                colors.text,
                colors.halo,
            );
        }

        let share_strip = egui::Rect::from_min_size(
            egui::pos2(header.left(), header.bottom() - 3.0),
            egui::vec2(header.width(), 2.0),
        );
        painter.rect_filled(share_strip, 1.0, track_color);
        if self.hud_config.show_character_rows && values.total_damage > 0.0 {
            let mut seg_left = share_strip.left();
            for (index, row) in rows.iter().enumerate() {
                let target = (row.damage / values.total_damage) as f32;
                let share = motion::animate_share(
                    painter.ctx(),
                    (
                        "hud_summary_share",
                        self.session_epoch,
                        abyss_active,
                        second_half,
                        row.char_id,
                    ),
                    target.clamp(0.0, 1.0),
                    self.reduce_motion,
                );
                let seg_width = (share_strip.width() * share.value)
                    .min((share_strip.right() - seg_left).max(0.0));
                if seg_width <= 0.5 {
                    seg_left += seg_width;
                    continue;
                }
                let color = character_color(row.char_id, &self.characters, index, self.dark_mode);
                let seg = egui::Rect::from_min_size(
                    egui::pos2(seg_left, share_strip.top()),
                    egui::vec2(seg_width, share_strip.height()),
                );
                painter.rect_filled(seg, 1.0, color);
                paint_share_tail(
                    painter,
                    share_strip,
                    seg.right(),
                    color,
                    share.highlight_opacity,
                );
                seg_left += seg_width;
            }
        }
        56.0
    }

    pub(crate) fn hud_status_row(
        &self,
        painter: &egui::Painter,
        left: f32,
        top: f32,
        width: f32,
        colors: HudPaintColors,
    ) -> f32 {
        let rect = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(width, 18.0));
        if self.hud_config.show_abyss_half && self.state.abyss.is_active() {
            paint_haloed(
                painter,
                egui::pos2(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                t(self.selected_abyss_half.label()),
                egui::FontId::proportional(11.0),
                colors.text,
                colors.halo,
            );
        }
        if self.hud_config.show_passthrough_state {
            let label = if self.mouse_passthrough {
                t("Passthrough")
            } else {
                t("Edit")
            };
            paint_haloed(
                painter,
                egui::pos2(rect.right(), rect.center().y),
                egui::Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(11.0),
                colors.muted,
                colors.halo,
            );
        }
        22.0
    }

    pub(crate) fn hud_character_rows(
        &self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        layout: HudRowsLayout,
        rows: &[CharacterStats],
        total_damage: f64,
        colors: HudPaintColors,
    ) -> f32 {
        let hud_theme = self.theme().hud;
        let track_color = hud_theme.track;
        let row_h = 24.0;
        let row_gap = 4.0;
        let stride = row_h + row_gap;
        let total_height = stride * rows.len() as f32;
        let container = egui::Rect::from_min_size(
            egui::pos2(layout.left, layout.top),
            egui::vec2(layout.width, total_height),
        );
        let previous_clip = ui.clip_rect();
        ui.set_clip_rect(previous_clip.intersect(container));
        let clipped_painter = painter.with_clip_rect(container);
        let painter = &clipped_painter;
        let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
        let abyss_active = self.state.abyss.is_active();
        for (index, row) in rows.iter().enumerate().rev() {
            let color = character_color(row.char_id, &self.characters, index, self.dark_mode);
            let row_dps = self.character_dps_for_current_source(row);
            let target_y = index as f32 * stride;
            let animated_y = motion::animate_value(
                ui.ctx(),
                (
                    "hud_rank_y",
                    self.session_epoch,
                    abyss_active,
                    second_half,
                    row.char_id,
                ),
                target_y,
                motion::dur::BASE,
                self.reduce_motion,
            );
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(layout.left, layout.top + animated_y),
                egui::vec2(layout.width, row_h),
            );
            let center_y = row_rect.center().y;
            painter.rect_filled(
                egui::Rect::from_min_size(row_rect.min, egui::vec2(3.0, row_h)),
                1.5,
                color,
            );

            let avatar = egui::Rect::from_min_size(
                egui::pos2(row_rect.left() + 8.0, center_y - 9.0),
                egui::vec2(18.0, 18.0),
            );
            self.draw_hud_avatar(ui, avatar, row.char_id, color, &row.name);

            let name_x = avatar.right() + 7.0;
            let bar_left = row_rect.left() + 104.0;
            let bar_right = row_rect.right() - 96.0;
            let clipped = painter.with_clip_rect(egui::Rect::from_min_max(
                egui::pos2(name_x, row_rect.top()),
                egui::pos2(bar_left - 7.0, row_rect.bottom()),
            ));
            paint_haloed(
                &clipped,
                egui::pos2(name_x, center_y),
                egui::Align2::LEFT_CENTER,
                &row.name,
                egui::FontId::proportional(12.0),
                colors.text,
                colors.halo,
            );

            let share = if total_damage > 0.0 {
                row.damage / total_damage * 100.0
            } else {
                0.0
            };
            let track = egui::Rect::from_min_max(
                egui::pos2(bar_left, center_y - 3.5),
                egui::pos2(bar_right.max(bar_left + 8.0), center_y + 3.5),
            );
            painter.rect_filled(track, 3.5, track_color);
            painter.rect_stroke(
                track,
                3.5,
                Stroke::new(1.0_f32, hud_theme.halo),
                egui::StrokeKind::Inside,
            );
            let share_animation = motion::animate_share(
                ui.ctx(),
                (
                    "hud_character_share",
                    self.session_epoch,
                    abyss_active,
                    second_half,
                    row.char_id,
                ),
                (share as f32 / 100.0).clamp(0.0, 1.0),
                self.reduce_motion,
            );
            let fill_width = track.width() * share_animation.value;
            if fill_width > 0.5 {
                painter.rect_filled(
                    egui::Rect::from_min_size(track.min, egui::vec2(fill_width, track.height())),
                    3.5,
                    color,
                );
                paint_share_tail(
                    painter,
                    track,
                    track.left() + fill_width,
                    color,
                    share_animation.highlight_opacity,
                );
            }
            paint_haloed(
                painter,
                egui::pos2(row_rect.right() - 8.0, center_y),
                egui::Align2::RIGHT_CENTER,
                format!("{share:.1}%"),
                egui::FontId::proportional(10.5),
                color,
                colors.halo,
            );
            paint_haloed(
                painter,
                egui::pos2(layout.right - 52.0, center_y),
                egui::Align2::RIGHT_CENTER,
                format_number(row_dps),
                egui::FontId::monospace(12.0),
                colors.text,
                colors.halo,
            );
        }
        ui.set_clip_rect(previous_clip);
        total_height
    }

    pub(crate) fn hud_mini_timeline(
        &mut self,
        painter: &egui::Painter,
        left: f32,
        top: f32,
        width: f32,
        preview: bool,
        colors: HudPaintColors,
    ) -> f32 {
        let accent = colors.accent;
        let rect = egui::Rect::from_min_size(egui::pos2(left, top + 4.0), egui::vec2(width, 34.0));
        let baseline_y = rect.bottom() - 5.0;
        painter.hline(
            rect.x_range(),
            baseline_y,
            Stroke::new(1.0_f32, colors.muted.gamma_multiply(0.6)),
        );
        if preview {
            let points = [0.16, 0.42, 0.28, 0.7, 0.48, 0.82, 0.36, 0.58, 0.24];
            let mut previous = None;
            for (index, value) in points.into_iter().enumerate() {
                let x = rect.left() + rect.width() * index as f32 / (points.len() - 1) as f32;
                let y = baseline_y - (rect.height() - 8.0) * value;
                let point = egui::pos2(x, y);
                if let Some(previous) = previous {
                    painter.line_segment([previous, point], Stroke::new(1.4_f32, accent));
                }
                previous = Some(point);
            }
        } else {
            let timeline = self.cached_timeline_series();
            let peak = timeline
                .buckets
                .iter()
                .map(|bucket| bucket.dps)
                .fold(0.0, f64::max);
            if peak > 0.0 && timeline.buckets.len() > 1 {
                let duration = timeline
                    .buckets
                    .last()
                    .map_or(1.0, |bucket| bucket.end_offset)
                    .max(0.001);
                let mut previous = None;
                for bucket in &timeline.buckets {
                    let x = rect.left() + rect.width() * (bucket.end_offset / duration) as f32;
                    let y = baseline_y - (rect.height() - 8.0) * (bucket.dps / peak) as f32;
                    let point = egui::pos2(x, y);
                    if let Some(previous) = previous {
                        painter.line_segment([previous, point], Stroke::new(1.4_f32, accent));
                    }
                    previous = Some(point);
                }
            }
        }
        paint_haloed(
            painter,
            egui::pos2(rect.left(), rect.top() + 6.0),
            egui::Align2::LEFT_CENTER,
            t("DPS"),
            egui::FontId::proportional(9.5),
            colors.muted,
            colors.halo,
        );
        42.0
    }

    pub(crate) fn current_damage_taken_for_hud(&self) -> f64 {
        self.selected_party_state()
            .map_or(self.state.total_damage_taken, |party| {
                party.total_damage_taken
            })
    }

    /// Square avatar for the HUD (image, else a colored tile with the initial),
    /// with a dark edge so it separates from a bright game scene.
    pub(crate) fn draw_hud_avatar(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        char_id: u32,
        color: Color32,
        name: &str,
    ) {
        let radius_f = rect.width() * 0.24;
        let texture = self
            .characters
            .get(&char_id)
            .and_then(|character| character.avatar.as_deref())
            .and_then(|avatar| self.avatar_textures.get(avatar));
        if let Some(texture) = texture {
            ui.put(
                rect,
                egui::Image::new((texture.id(), rect.size())).corner_radius(radius_f as u8),
            );
        } else {
            ui.painter()
                .rect_filled(rect, radius_f, color.gamma_multiply(0.85));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                name.chars().next().unwrap_or('?').to_string(),
                egui::FontId::proportional(rect.height() * 0.52),
                contrast_text(color),
            );
        }
        ui.painter().rect_stroke(
            rect,
            radius_f,
            Stroke::new(1.0_f32, self.theme().hud.halo),
            egui::StrokeKind::Inside,
        );
    }

    pub(crate) fn draw_party_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &CharacterStats,
        index: usize,
        total_damage: f64,
        row_height: f32,
    ) {
        let density_scale = density_tokens(self.density).font_scale;
        let color = readable_accent(
            character_color(row.char_id, &self.characters, index, self.dark_mode),
            self.dark_mode,
        );
        let avatar_texture = self
            .characters
            .get(&row.char_id)
            .and_then(|character| character.avatar.as_deref())
            .and_then(|avatar| self.avatar_textures.get(avatar));
        let attribute_texture = self
            .characters
            .get(&row.char_id)
            .and_then(|character| character.attribute.as_deref())
            .and_then(|attribute| self.attribute_textures.get(attribute));
        let share = if total_damage > 0.0 {
            row.damage / total_damage * 100.0
        } else {
            0.0
        };
        let duration = self.character_duration_for_current_source(row);
        let dps = self.character_dps_for_current_source(row);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row_height),
            egui::Sense::click(),
        );
        let hover = motion::animate_bool(
            ui.ctx(),
            ("party_row_hover", response.id),
            response.hovered(),
            motion::dur::FAST,
            self.reduce_motion,
            motion::ease::standard,
        );
        let card_fill = mix_color(self.theme().card, shadcn_card_hover(self.dark_mode), hover);
        ui.painter().rect_filled(rect, 6.0, card_fill);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(
                1.0_f32,
                mix_color(self.theme().border, color.gamma_multiply(0.72), hover),
            ),
            egui::StrokeKind::Inside,
        );
        let contribution_track = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 7.0, rect.bottom() - 4.0),
            egui::pos2(rect.right() - 7.0, rect.bottom() - 2.0),
        );
        let second_half = matches!(self.selected_abyss_half, AbyssHalf::Second);
        let abyss_active = self.state.abyss.is_active();
        let animated_share = motion::animate_share(
            ui.ctx(),
            (
                "party_row_share",
                self.session_epoch,
                abyss_active,
                second_half,
                row.char_id,
            ),
            (share as f32 / 100.0).clamp(0.0, 1.0),
            self.reduce_motion,
        );
        ui.painter()
            .rect_filled(contribution_track, 1.0, self.theme().muted);
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                contribution_track.min,
                egui::vec2(
                    contribution_track.width() * animated_share.value,
                    contribution_track.height(),
                ),
            ),
            1.0,
            color,
        );
        paint_share_tail(
            ui.painter(),
            contribution_track,
            contribution_track.left() + contribution_track.width() * animated_share.value,
            color,
            animated_share.highlight_opacity,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + 3.0 + hover, rect.bottom()),
            ),
            6.0,
            color,
        );
        if let Some(texture) = attribute_texture {
            let attribute_rect = pixel_aligned_rect(
                egui::pos2(rect.left(), rect.center().y - 12.0),
                24.0,
                ui.ctx().pixels_per_point(),
            );
            ui.put(
                attribute_rect,
                egui::Image::new((texture.id(), attribute_rect.size())),
            );
        } else {
            ui.painter().text(
                egui::pos2(rect.left() + 10.0, rect.center().y),
                egui::Align2::CENTER_CENTER,
                format!("#{}", index + 1),
                egui::FontId::monospace(9.5 * density_scale),
                color,
            );
        }
        // Grows with the (now height-adaptive) row so tall cards don't leave the
        // avatar marooned in empty space; normal-height rows keep the 40px avatar.
        let avatar_size = (row_height - 8.0).clamp(32.0, 56.0);
        let avatar_rect = pixel_aligned_rect(
            egui::pos2(rect.left() + 24.0, rect.center().y - avatar_size * 0.5),
            avatar_size,
            ui.ctx().pixels_per_point(),
        );
        let avatar_border = self.theme().border_strong;
        ui.painter().rect_filled(avatar_rect, 8.0, avatar_border);
        if let Some(texture) = avatar_texture {
            ui.put(
                avatar_rect,
                egui::Image::new((texture.id(), avatar_rect.size())).corner_radius(8),
            );
            ui.painter().rect_stroke(
                avatar_rect,
                8.0,
                Stroke::new(1.0_f32, avatar_border),
                egui::StrokeKind::Inside,
            );
        } else {
            ui.painter()
                .rect_filled(avatar_rect, 8.0, color.gamma_multiply(0.82));
            ui.painter().text(
                avatar_rect.center(),
                egui::Align2::CENTER_CENTER,
                row.name.chars().next().unwrap_or('?').to_string(),
                egui::FontId::proportional(14.0 * density_scale),
                contrast_text(color),
            );
        }
        let compact = main_width_class(rect.width()) == MainWidthClass::Compact;
        let primary_text_y = if compact {
            rect.center().y
        } else {
            rect.center().y - 8.0
        };
        let text_left = avatar_rect.right() + 8.0;
        ui.painter().text(
            egui::pos2(text_left, primary_text_y),
            egui::Align2::LEFT_CENTER,
            &row.name,
            egui::FontId::proportional(14.0 * density_scale),
            ui.visuals().text_color(),
        );
        ui.painter().text(
            egui::pos2(rect.right() - 10.0, primary_text_y),
            egui::Align2::RIGHT_CENTER,
            format!("{} DPS", format_number(dps)),
            egui::FontId::monospace(12.0 * density_scale),
            self.theme().accent,
        );
        if !compact {
            ui.painter().text(
                egui::pos2(text_left, rect.center().y + 9.0),
                egui::Align2::LEFT_CENTER,
                tf(
                    "{} hits · {}",
                    &[
                        &row.hits.to_string(),
                        &tf("{}s", &[&format!("{duration:.1}")]),
                    ],
                ),
                egui::FontId::monospace(10.5 * density_scale),
                ui.visuals().weak_text_color(),
            );
            ui.painter().text(
                egui::pos2(rect.right() - 10.0, rect.center().y + 9.0),
                egui::Align2::RIGHT_CENTER,
                tf(
                    "Damage {} · Share {}% · Taken {}",
                    &[
                        &format_number(row.damage),
                        &format!("{share:.1}"),
                        &format_number(row.damage_taken),
                    ],
                ),
                egui::FontId::monospace(10.5 * density_scale),
                ui.visuals().weak_text_color(),
            );
        }
        let response = response.on_hover_text(t("View combat details in a separate window"));
        let mut open_details = false;
        response.context_menu(|ui| {
            if ui.button(t("View combat details")).clicked() {
                open_details = true;
                ui.close();
            }
            if ui.button(t("Copy values")).clicked() {
                ui.ctx().copy_text(tf(
                    "Character: {}\nDPS: {}\nDamage: {}\nShare: {}%\nTaken: {}",
                    &[
                        &row.name,
                        &format_number(dps),
                        &format_number(row.damage),
                        &format!("{share:.1}"),
                        &format_number(row.damage_taken),
                    ],
                ));
                ui.close();
            }
            if ui.button(t("Hide from ranking")).clicked() {
                self.hidden_character_ids.insert(row.char_id);
                ui.close();
            }
        });
        if response.clicked() || open_details {
            self.hit_detail_char_id = Some(row.char_id);
            self.hit_detail_filter = HitDetailFilter::All;
            self.hit_detail_skill_filter.clear();
            self.hit_detail_corner_applied = false;
        }
    }
}

#[derive(Clone, Copy)]
enum MainEmptyAction {
    Detect,
    Capture,
    Import,
}

fn main_empty_step(
    ui: &mut egui::Ui,
    number: usize,
    label: String,
    complete: bool,
    theme: ThemeTokens,
) {
    ui.horizontal(|ui| {
        // U+2714 — U+2713 "✓" is missing from the app font stack and renders
        // as a tofu box (see `painted_glyphs_exist_in_font_stack`).
        let marker = if complete {
            "✔".to_owned()
        } else {
            number.to_string()
        };
        let color = if complete {
            theme.success
        } else {
            theme.fg_faint
        };
        ui.label(RichText::new(marker).strong().color(color));
        ui.label(RichText::new(label).color(if complete { theme.fg_muted } else { theme.fg }));
    });
}

fn trend_glyph(direction: motion::TrendDirection) -> &'static str {
    match direction {
        motion::TrendDirection::Up => "▲",
        motion::TrendDirection::Down => "▼",
    }
}

fn paint_metric_trend(
    ui: &egui::Ui,
    bounds: egui::Rect,
    trend: Option<motion::TrendIndicator>,
    color: Color32,
) {
    let Some(trend) = trend else {
        return;
    };
    ui.painter().text(
        egui::pos2(bounds.right() - 7.0, bounds.top() + 7.0),
        egui::Align2::RIGHT_TOP,
        trend_glyph(trend.direction),
        egui::FontId::proportional(9.0),
        color.gamma_multiply(trend.opacity),
    );
}

fn paint_hud_trend(
    painter: &egui::Painter,
    position: egui::Pos2,
    trend: Option<motion::TrendIndicator>,
    color: Color32,
) {
    let Some(trend) = trend else {
        return;
    };
    painter.text(
        position,
        egui::Align2::LEFT_CENTER,
        trend_glyph(trend.direction),
        egui::FontId::proportional(10.0),
        color.gamma_multiply(trend.opacity),
    );
}

fn paint_share_tail(
    painter: &egui::Painter,
    track: egui::Rect,
    end_x: f32,
    color: Color32,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let left = (end_x - 1.0).clamp(track.left(), track.right());
    let right = (left + 2.0).min(track.right());
    if right <= left {
        return;
    }
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(left, track.top()),
            egui::pos2(right, track.bottom()),
        ),
        1.0,
        mix_color(color, contrast_text(color), 0.72).gamma_multiply(opacity),
    );
}

fn paint_hud_module_editor_outline(
    painter: &egui::Painter,
    rect: egui::Rect,
    label: String,
    color: Color32,
    halo: Color32,
) {
    painter.rect_filled(
        egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width(), HUD_EDITOR_MODULE_HEADER_HEIGHT),
        ),
        3.0,
        color.gamma_multiply(0.12),
    );
    let path = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &path,
        Stroke::new(1.0_f32, color.gamma_multiply(0.8)),
        5.0,
        3.0,
    ));
    paint_haloed_with_halo(
        painter,
        rect.left_top() + egui::vec2(6.0, HUD_EDITOR_MODULE_HEADER_HEIGHT * 0.5),
        egui::Align2::LEFT_CENTER,
        format!("≡  {label}"),
        egui::FontId::proportional(9.5),
        color,
        halo,
    );
}

fn paint_hud_drag_ghost(
    ctx: &egui::Context,
    pointer: egui::Pos2,
    source_size: egui::Vec2,
    module: HudModule,
    theme: HudThemeTokens,
) {
    let rect = hud_drag_ghost_rect(ctx.content_rect(), pointer, source_size);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("hud_module_drag_ghost"),
    ));
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 7.0)),
        8.0,
        Color32::from_black_alpha(128),
    );
    painter.rect_filled(rect, 8.0, theme.edit_bg);
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(2.0_f32, theme.accent),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(4.0, rect.height())),
        2.0,
        theme.accent,
    );
    painter.text(
        rect.left_top() + egui::vec2(14.0, 15.0),
        egui::Align2::LEFT_CENTER,
        format!("≡  {}", t(module.label())),
        egui::FontId::proportional(12.0),
        theme.edit_text,
    );
    let detail = match module {
        HudModule::Title => "NTE DPS".to_owned(),
        HudModule::Summary => "64,301 DPS".to_owned(),
        HudModule::Status => t("Edit"),
        HudModule::Characters => format!("4 × {}", t("Character")),
        HudModule::Timeline => t("DPS"),
    };
    painter.text(
        rect.left_top() + egui::vec2(14.0, 36.0),
        egui::Align2::LEFT_CENTER,
        detail,
        egui::FontId::proportional(11.0),
        mix_color(theme.edit_text, theme.edit_bg, 0.45),
    );
    ctx.request_repaint();
}

fn hud_drag_ghost_rect(
    content_rect: egui::Rect,
    pointer: egui::Pos2,
    source_size: egui::Vec2,
) -> egui::Rect {
    let size = egui::vec2(
        source_size.x.clamp(240.0, 420.0),
        source_size.y.clamp(54.0, 140.0),
    );
    let bounds = content_rect.shrink(8.0);
    let desired = pointer - egui::vec2(22.0, 14.0);
    let min = egui::pos2(
        desired
            .x
            .clamp(bounds.left(), (bounds.right() - size.x).max(bounds.left())),
        desired
            .y
            .clamp(bounds.top(), (bounds.bottom() - size.y).max(bounds.top())),
    );
    egui::Rect::from_min_size(min, size)
}

fn hud_preview_party_readout() -> HudPreviewData {
    let rows = [
        (u32::MAX - 3, "A", 38, 1_227_500.0),
        (u32::MAX - 2, "B", 27, 579_000.0),
        (u32::MAX - 1, "C", 16, 260_000.0),
        (u32::MAX, "D", 11, 179_785.0),
    ]
    .into_iter()
    .map(|(char_id, suffix, hits, damage)| CharacterStats {
        char_id,
        name: format!("{} {suffix}", t("Character")),
        hits,
        damage,
        first_hit: 1.0,
        last_hit: 34.9,
        ..CharacterStats::default()
    })
    .collect::<Vec<_>>();
    let total_damage = rows.iter().map(|row| row.damage).sum();
    HudPreviewData {
        rows,
        total_damage,
        team_dps: 64_301.0,
        duration: 34.9,
        damage_taken: 2_834.0,
    }
}

fn toolbar_button_group_width<'a>(
    ui: &egui::Ui,
    labels: impl IntoIterator<Item = &'a str>,
    horizontal_padding: f32,
    item_spacing: f32,
) -> f32 {
    let font_id = egui::TextStyle::Button.resolve(ui.style());
    let mut width = 0.0;
    let mut count: usize = 0;
    for label in labels {
        let text_width = ui
            .painter()
            .layout_no_wrap(label.to_owned(), font_id.clone(), Color32::WHITE)
            .size()
            .x;
        width += (text_width + horizontal_padding * 2.0).max(ui.spacing().interact_size.x);
        count += 1;
    }
    width + item_spacing * count.saturating_sub(1) as f32
}

#[cfg(test)]
mod responsive_tests {
    use super::*;

    #[test]
    fn main_width_class_uses_exact_breakpoints() {
        assert_eq!(main_width_class(419.9), MainWidthClass::Compact);
        assert_eq!(main_width_class(420.0), MainWidthClass::Standard);
        assert_eq!(main_width_class(560.0), MainWidthClass::Standard);
        assert_eq!(main_width_class(560.1), MainWidthClass::Wide);
    }

    #[test]
    fn toolbar_reflows_instead_of_hiding_actions() {
        assert_eq!(
            main_toolbar_layout(540.0, 320.0, 220.0, 90.0, 196.0),
            MainToolbarLayout::SingleRow
        );
        assert_eq!(
            main_toolbar_layout(539.9, 320.0, 220.0, 90.0, 196.0),
            MainToolbarLayout::TwoRows
        );
        assert_eq!(
            main_toolbar_layout(300.0, 320.0, 220.0, 90.0, 196.0),
            MainToolbarLayout::ThreeRows
        );
    }

    #[test]
    fn hud_preview_has_a_complete_four_member_snapshot() {
        let preview = hud_preview_party_readout();

        assert_eq!(preview.rows.len(), HUD_PREVIEW_ROW_COUNT);
        assert_eq!(
            preview.total_damage,
            preview.rows.iter().map(|row| row.damage).sum::<f64>()
        );
        assert!(preview.team_dps > 0.0);
        assert!(preview.duration > 0.0);
        assert!(preview.damage_taken > 0.0);
    }

    #[test]
    fn hud_drag_preview_anchors_the_pointer_inside_its_title_strip() {
        let pointer = egui::pos2(500.0, 260.0);
        let rect = hud_drag_ghost_rect(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)),
            pointer,
            egui::vec2(900.0, 90.0),
        );

        assert!(rect.contains(pointer));
        assert!(pointer.y <= rect.top() + 30.0);
    }

    #[test]
    fn hud_dragging_down_inserts_after_the_target() {
        let mut config = HudConfig::default();

        config.move_module(HudModule::Title, HudModule::Characters, true);

        assert_eq!(
            config.module_order,
            [
                HudModule::Summary,
                HudModule::Status,
                HudModule::Characters,
                HudModule::Title,
                HudModule::Timeline,
            ]
        );
    }

    #[test]
    fn hud_dragging_up_inserts_before_the_target() {
        let mut config = HudConfig::default();

        config.move_module(HudModule::Timeline, HudModule::Summary, false);

        assert_eq!(
            config.module_order,
            [
                HudModule::Title,
                HudModule::Timeline,
                HudModule::Summary,
                HudModule::Status,
                HudModule::Characters,
            ]
        );
    }
}
