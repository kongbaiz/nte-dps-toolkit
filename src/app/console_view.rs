use super::*;

impl DpsApp {
    pub(crate) fn console_panel(&mut self, ctx: &egui::Context) {
        let viewport_id = console_viewport_id();
        let recording_hotkey = self.recording_hotkey.is_some();
        let close_requested = ctx.show_viewport_immediate(
            viewport_id,
            secondary_viewport_builder(
                t("NTE Console"),
                self.console_window_size,
                config::CONSOLE_WINDOW_MIN_SIZE,
                self.console_geometry,
                self.console_corner_applied,
            ),
            |ctx, _class| {
                if self.recording_hotkey.is_some()
                    && ctx.input(|input| input.viewport().focused == Some(false))
                {
                    self.set_recording_hotkey(None);
                    self.status = t("Shortcut recording canceled when Console lost focus");
                }
                self.handle_local_hotkeys(ctx.ctx());
                let opening = !self.console_corner_applied;
                if opening {
                    apply_rounding_to_process_windows();
                    self.console_corner_applied = true;
                }
                let close_clicked = secondary_title_panel(ctx, &t("NTE Console"));
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::new()
                            .fill(self.theme().bg)
                            .inner_margin(egui::Margin::same(10)),
                    )
                    .show_inside(ctx, |ui| {
                        motion::apply_viewport_entrance(ui, "console", opening, self.reduce_motion);
                        self.console_contents(ui);
                    });
                track_secondary_viewport_geometry(
                    ctx,
                    &mut self.console_window_size,
                    &mut self.console_geometry,
                );
                window_resize_grips(ctx);
                self.show_status_toast(ctx.ctx());
                self.show_command_palette(ctx.ctx());
                self.show_viewport_dialogs(ctx);
                let native_close = ctx.input(|input| input.viewport().close_requested());
                let reserved_close = recording_hotkey
                    && ctx.input(|input| input.modifiers.alt && input.key_pressed(egui::Key::F4));
                if native_close && reserved_close {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    false
                } else {
                    close_clicked || native_close
                }
            },
        );
        if close_requested {
            self.console_open = false;
            self.console_corner_applied = false;
            self.set_recording_hotkey(None);
            self.retarget_dialogs(viewport_id, egui::ViewportId::ROOT);
        }
    }

    pub(crate) fn console_contents(&mut self, ui: &mut egui::Ui) {
        if !self.console_sidebar_migration_seen {
            let theme = self.theme();
            egui::Frame::new()
                .fill(theme.card)
                .stroke(Stroke::new(1.0_f32, theme.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(t(
                            "Console pages are now grouped in the left sidebar for faster access.",
                        ));
                        if ui.button(t("Got it")).clicked() {
                            self.console_sidebar_migration_seen = true;
                        }
                    });
                });
            ui.add_space(6.0);
        }
        if !ui.ctx().egui_wants_keyboard_input() {
            let (previous, next) = ui.input(|input| {
                let modifiers = input.modifiers;
                let ctrl_only =
                    modifiers.ctrl && !modifiers.alt && !modifiers.shift && !modifiers.mac_cmd;
                (
                    ctrl_only && input.key_pressed(egui::Key::PageUp),
                    ctrl_only && input.key_pressed(egui::Key::PageDown),
                )
            });
            if previous {
                self.console_tab = self.console_tab.adjacent(-1);
                self.set_recording_hotkey(None);
            } else if next {
                self.console_tab = self.console_tab.adjacent(1);
                self.set_recording_hotkey(None);
            }
        }

        let auto_collapsed = console_sidebar_collapsed(ui.available_width());
        let collapsed = auto_collapsed || self.console_sidebar_manually_collapsed;
        let collapse_progress = motion::animate_bool(
            ui.ctx(),
            "console_sidebar_collapse",
            collapsed,
            motion::dur::BASE,
            self.reduce_motion,
            motion::ease::standard,
        );
        ui.horizontal_top(|ui| {
            self.console_sidebar(ui, collapsed, !auto_collapsed, collapse_progress);
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                motion::content_swap_entrance(
                    ui,
                    "console_tab_content",
                    self.console_tab as u64,
                    self.reduce_motion,
                );
                match self.console_tab {
                    ConsoleTab::Settings => self.settings_contents(ui),
                    ConsoleTab::Timeline => self.timeline_contents(ui),
                    ConsoleTab::Skills => self.skills_contents(ui),
                    ConsoleTab::EmptyCurtain => self.empty_curtain_contents(ui),
                    ConsoleTab::History => self.history_contents(ui),
                    ConsoleTab::Characters => self.debug_characters_contents(ui),
                    ConsoleTab::EncryptedIni => self.debug_encrypted_ini_contents(ui),
                    ConsoleTab::Packets => self.debug_packets_contents(ui),
                    ConsoleTab::Resources => self.resource_audit_contents(ui),
                    ConsoleTab::Diagnostics => self.diagnostics_contents(ui),
                }
            });
        });
    }

    /// The sidebar is a dedicated column drawn inside the console's
    /// `horizontal_top` row, so it claims its own top-down child region first;
    /// drawing straight into the shared row `Ui` would lay the tabs out
    /// left-to-right across the window.
    fn console_sidebar(
        &mut self,
        ui: &mut egui::Ui,
        collapsed: bool,
        allow_toggle: bool,
        collapse_progress: f32,
    ) {
        let width = console_sidebar_width(collapse_progress);
        let theme = self.theme();
        ui.allocate_ui_with_layout(
            egui::vec2(width, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                if allow_toggle {
                    let tooltip = if collapsed {
                        t("Expand sidebar")
                    } else {
                        t("Collapse sidebar")
                    };
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        let hover = motion::animate_bool(
                            ui.ctx(),
                            "console_sidebar_toggle_hover",
                            response.hovered(),
                            motion::dur::FAST,
                            self.reduce_motion,
                            motion::ease::standard,
                        );
                        let button_rect = rect.shrink2(egui::vec2(4.0, 1.0));
                        ui.painter().rect_filled(
                            button_rect,
                            6.0,
                            mix_color(theme.card_hover, theme.muted, hover),
                        );
                        ui.painter().rect_stroke(
                            button_rect,
                            6.0,
                            Stroke::new(
                                1.0_f32,
                                mix_color(theme.border, theme.border_strong, hover),
                            ),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            button_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("‹  {}", t("Collapse")),
                            density_proportional_font(ui, 12.0),
                            theme
                                .fg_muted
                                .gamma_multiply((1.0 - collapse_progress).clamp(0.0, 1.0)),
                        );
                        ui.painter().text(
                            button_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "›",
                            density_proportional_font(ui, 18.0),
                            theme
                                .fg_muted
                                .gamma_multiply(collapse_progress.clamp(0.0, 1.0)),
                        );
                    }
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tooltip.clone())
                    });
                    if response.on_hover_text(tooltip).clicked() {
                        self.console_sidebar_manually_collapsed =
                            !self.console_sidebar_manually_collapsed;
                    }
                    ui.add_space(4.0);
                }
                let mut previous_group = None;
                for tab in ConsoleTab::visible_tabs() {
                    let group = tab.group();
                    if previous_group != Some(group) {
                        if previous_group.is_some() {
                            ui.add_space(8.0);
                        }
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(width, 15.0), egui::Sense::hover());
                        if ui.is_rect_visible(rect) {
                            ui.painter().text(
                                egui::pos2(rect.left() + 6.0, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                t(group.label_key()),
                                density_proportional_font(ui, 10.0),
                                theme
                                    .fg_faint
                                    .gamma_multiply((1.0 - collapse_progress).clamp(0.0, 1.0)),
                            );
                            let separator_y = rect.center().y;
                            ui.painter().line_segment(
                                [
                                    egui::pos2(rect.left() + 8.0, separator_y),
                                    egui::pos2(rect.right() - 8.0, separator_y),
                                ],
                                Stroke::new(
                                    1.0_f32,
                                    theme
                                        .border
                                        .gamma_multiply(collapse_progress.clamp(0.0, 1.0)),
                                ),
                            );
                        }
                        previous_group = Some(group);
                    }
                    self.console_sidebar_item(ui, *tab, collapsed, collapse_progress, width, theme);
                }
            },
        );
    }

    /// One sidebar row, painted by hand so rest, hover and selected states
    /// cross-fade instead of snapping: hover fades a muted fill in, selection
    /// fades the accent pill in over it. Expanded mode shows icon + label;
    /// collapsed mode shows the icon only, with the full label as a tooltip.
    /// Icon glyphs are guarded by the Material Icons font-coverage test so
    /// they never render as tofu boxes.
    fn console_sidebar_item(
        &mut self,
        ui: &mut egui::Ui,
        tab: ConsoleTab,
        collapsed: bool,
        collapse_progress: f32,
        width: f32,
        theme: ThemeTokens,
    ) {
        let selected = self.console_tab == tab;
        let label = t(tab.label_key());
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let hover = motion::animate_bool(
                ui.ctx(),
                ("console_sidebar_hover", tab as u64),
                response.hovered() && !selected,
                motion::dur::FAST,
                self.reduce_motion,
                motion::ease::standard,
            );
            let select_t = motion::animate_bool(
                ui.ctx(),
                ("console_sidebar_selected", tab as u64),
                selected,
                motion::dur::BASE,
                self.reduce_motion,
                motion::ease::standard,
            );
            let rest = theme.muted.gamma_multiply(hover * 0.9);
            let fill = mix_color(rest, theme.accent, select_t);
            ui.painter().rect_filled(rect, 6.0, fill);
            let text_color = mix_color(
                mix_color(theme.fg_muted, theme.fg, hover),
                theme.accent_fg,
                select_t,
            );
            let icon_x = egui::lerp(
                (rect.left() + 16.0)..=rect.center().x,
                collapse_progress.clamp(0.0, 1.0),
            );
            let icon = tab.icon();
            ui.painter().text(
                egui::pos2(icon_x, rect.center().y),
                egui::Align2::CENTER_CENTER,
                icon.codepoint,
                egui::FontId::new(
                    (17.0 + collapse_progress) * ui_density_scale(ui),
                    icon.font_family(),
                ),
                text_color,
            );
            ui.painter().text(
                egui::pos2(rect.left() + 32.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                label.as_str(),
                density_proportional_font(ui, 13.0),
                text_color.gamma_multiply((1.0 - collapse_progress).clamp(0.0, 1.0)),
            );
        }
        let response = if collapsed {
            response.on_hover_text(label)
        } else {
            response
        };
        if response.clicked() && self.console_tab != tab {
            self.console_tab = tab;
            self.set_recording_hotkey(None);
        }
    }

    pub(crate) fn timeline_contents(&mut self, ui: &mut egui::Ui) {
        self.abyss_selector(ui);
        inline_controls(ui, |ui| {
            ui.label(inline_text(
                t("Bucket Interval"),
                ui.visuals().weak_text_color(),
            ));
            let mut bucket_seconds =
                config::sanitize_timeline_bucket_seconds(self.timeline_bucket_seconds);
            let changed = ui
                .add_sized(
                    egui::vec2(220.0, INLINE_CONTROL_HEIGHT),
                    egui::Slider::new(
                        &mut bucket_seconds,
                        TIMELINE_BUCKET_SECONDS_MIN..=TIMELINE_BUCKET_SECONDS_MAX,
                    )
                    .step_by(0.1)
                    .suffix("s")
                    .show_value(true),
                )
                .on_hover_text(t(
                    "Seconds each bucket covers; smaller is finer, larger is smoother",
                ))
                .changed();
            if changed {
                self.timeline_bucket_seconds =
                    config::sanitize_timeline_bucket_seconds(bucket_seconds);
                self.timeline_cache = TimelineCache::default();
            }
            ui.separator();
            ui.label(inline_text(t("Curve"), ui.visuals().weak_text_color()));
            for mode in TimelineDpsViewMode::all() {
                stable_selectable_value(
                    ui,
                    &mut self.timeline_dps_view_mode,
                    *mode,
                    t(mode.label()),
                );
            }
        });
        ui.add_space(6.0);
        let timeline = self.cached_timeline_series();
        if timeline.buckets.is_empty() {
            self.capture_data_empty_state(
                ui,
                t("Waiting for damage data"),
                t("Start capture or import a replay to build the combat timeline."),
            );
            return;
        }

        let peak_dps = timeline
            .buckets
            .iter()
            .map(|bucket| bucket.dps)
            .fold(0.0, f64::max);
        let duration = timeline
            .buckets
            .last()
            .map_or(0.0, |bucket| bucket.end_offset);
        if !matches!(self.timeline_dps_view_mode, TimelineDpsViewMode::Characters)
            || self.selected_timeline_char.is_some_and(|char_id| {
                !timeline.buckets.iter().any(|bucket| {
                    bucket
                        .role_damage
                        .iter()
                        .any(|role| role.char_id == char_id)
                })
            })
        {
            self.selected_timeline_char = None;
        }
        ui.columns(4, |columns| {
            compact_metric(
                &mut columns[0],
                &t("Total Damage"),
                format_number(timeline.total_damage),
                self.theme().accent,
                true,
            );
            compact_metric(
                &mut columns[1],
                &t("Peak DPS"),
                format_number(peak_dps),
                self.theme().accent,
                true,
            );
            let bucket_color = columns[2].visuals().text_color();
            compact_metric(
                &mut columns[2],
                &t("Combat Time"),
                format!("{duration:.1}s"),
                bucket_color,
                false,
            );
            let interval_color = columns[3].visuals().text_color();
            compact_metric(
                &mut columns[3],
                &t("Time-stop Intervals"),
                timeline.time_stop_intervals.len().to_string(),
                interval_color,
                false,
            );
        });
        ui.add_space(8.0);
        // Read-only combat segmentation: the capture's outgoing damage split into
        // separate fights wherever an idle gap exceeds the threshold. Derived from
        // the same timeline buckets the chart uses, so it never touches live state.
        let segments = summarize_combat_segments(&timeline, COMBAT_SEGMENT_GAP_SECONDS);
        if segments.len() > 1 {
            let dark_mode = self.dark_mode;
            ui.horizontal_wrapped(|ui| {
                ui.label(inline_text(
                    tf(
                        "Combat segments · {} (auto-split at gaps >{}s)",
                        &[
                            &segments.len().to_string(),
                            &format!("{COMBAT_SEGMENT_GAP_SECONDS:.0}"),
                        ],
                    ),
                    ui.visuals().weak_text_color(),
                ));
                for (index, segment) in segments.iter().enumerate() {
                    draw_combat_segment_chip(ui, index + 1, segment, dark_mode);
                }
            });
            ui.add_space(6.0);
        }
        ui.label(
            RichText::new(t(
                "Click a legend to highlight; drag across the chart to select a range; right-click for markers and zoom",
            ))
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);
        let chart_height = (ui.available_height() - 30.0).max(260.0);
        draw_timeline_chart(
            ui,
            &timeline,
            self.timeline_dps_view_mode,
            chart_height,
            &mut self.selected_timeline_char,
            self.dark_mode,
            &self.characters,
            &mut self.timeline_view,
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(tf(
                "Retained window · {}s · {}s bucket · {} samples · {} event markers",
                &[
                    &format!("{duration:.1}"),
                    &format_timeline_seconds(timeline.bucket_seconds),
                    &timeline.buckets.len().to_string(),
                    &timeline.markers.len().to_string(),
                ],
            ))
            .size(11.0)
            .color(ui.visuals().weak_text_color()),
        );
    }

    pub(crate) fn skills_contents(&mut self, ui: &mut egui::Ui) {
        self.abyss_selector(ui);
        let breakdown = self.cached_skill_breakdown(None);
        if breakdown.rows.is_empty() {
            self.capture_data_empty_state(
                ui,
                t("Waiting for skill attribution data"),
                t("Start capture or import a replay to attribute damage to skills."),
            );
            return;
        }

        let mut characters = aggregate_skill_characters(&breakdown.rows);
        if let Some(selected) = self.selected_skill_breakdown_char
            && !characters.iter().any(|row| row.char_id == selected)
        {
            self.selected_skill_breakdown_char = None;
        }
        let content_height = ui.available_height().max(420.0);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), content_height),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(220.0, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(
                            RichText::new(t("Character"))
                                .strong()
                                .color(self.theme().fg),
                        );
                        ui.add_space(4.0);
                        if ui
                            .selectable_label(
                                self.selected_skill_breakdown_char.is_none(),
                                t("Whole Team"),
                            )
                            .clicked()
                        {
                            self.selected_skill_breakdown_char = None;
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("skill_character_list")
                            .max_height((content_height - 64.0).max(160.0))
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for (index, row) in characters.iter_mut().enumerate() {
                                    let selected =
                                        self.selected_skill_breakdown_char == Some(row.char_id);
                                    let label = format!(
                                        "{}  {} · {:.1}%",
                                        character_display_name(
                                            &self.characters,
                                            row.char_id,
                                            &row.name,
                                        ),
                                        format_number(row.damage),
                                        if breakdown.total_damage > 0.0 {
                                            row.damage / breakdown.total_damage * 100.0
                                        } else {
                                            0.0
                                        }
                                    );
                                    if ui.selectable_label(selected, label).clicked() {
                                        self.selected_skill_breakdown_char = Some(row.char_id);
                                    }
                                    row.color = character_color(
                                        row.char_id,
                                        &self.characters,
                                        index,
                                        self.dark_mode,
                                    );
                                }
                            });
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let selected_char = self.selected_skill_breakdown_char;
                        let visible_rows = breakdown
                            .rows
                            .iter()
                            .filter(|row| {
                                selected_char.is_none_or(|char_id| row.char_id == char_id)
                            })
                            .collect::<Vec<_>>();
                        let visible_total = visible_rows.iter().map(|row| row.damage).sum::<f64>();
                        ui.columns(4, |columns| {
                            compact_metric(
                                &mut columns[0],
                                &t("Attributed Damage"),
                                format_number(visible_total),
                                self.theme().accent,
                                true,
                            );
                            let skill_count_color = columns[1].visuals().text_color();
                            compact_metric(
                                &mut columns[1],
                                &t("Skill Entries"),
                                visible_rows.len().to_string(),
                                skill_count_color,
                                false,
                            );
                            let unmapped_color = if breakdown.unknown.unmapped_skill_hits > 0 {
                                semantic_warning(self.dark_mode)
                            } else {
                                columns[2].visuals().text_color()
                            };
                            compact_metric(
                                &mut columns[2],
                                &t("Pending Mapping"),
                                breakdown.unknown.unmapped_skill_hits.to_string(),
                                unmapped_color,
                                false,
                            );
                            let candidate_color = if breakdown.unknown.unknown_direction_hits > 0 {
                                semantic_warning(self.dark_mode)
                            } else {
                                columns[3].visuals().text_color()
                            };
                            compact_metric(
                                &mut columns[3],
                                &t("Candidate Direction"),
                                breakdown.unknown.unknown_direction_hits.to_string(),
                                candidate_color,
                                false,
                            );
                        });
                        ui.add_space(8.0);
                        let show_diagnostics = has_unknown_attribution(&breakdown);
                        let diagnostics_budget = if show_diagnostics { 130.0 } else { 0.0 };
                        let row_list_height =
                            (ui.available_height() - diagnostics_budget).max(220.0);
                        draw_skill_breakdown_rows(
                            ui,
                            &visible_rows,
                            visible_total,
                            row_list_height,
                            self.dark_mode,
                            &self.characters,
                        );
                        if show_diagnostics {
                            ui.add_space(8.0);
                            draw_unknown_attribution(ui, &breakdown, self.dark_mode);
                        }
                    },
                );
            },
        );
    }

    pub(crate) fn history_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(t("Save This Summary"))
                .on_hover_text(t("Save a de-identified stats summary; no packets, payload, IP, port or local paths"))
                .clicked()
            {
                self.save_current_history_summary(ui.ctx());
            }
            if ui.button(t("Reload")).clicked() {
                self.history.reload();
                self.history.message = t("History list refreshed");
            }
            ui.label(
                RichText::new(tf("{} records", &[&self.history.records.len().to_string()]))
                    .color(ui.visuals().weak_text_color()),
            );
            if self.history.skipped_files > 0 {
                ui.label(
                    RichText::new(tf(
                        "Skipped {} corrupt files",
                        &[&self.history.skipped_files.to_string()],
                    ))
                    .color(semantic_warning(self.dark_mode)),
                );
            }
            if !self.history.message.is_empty() {
                ui.label(
                    RichText::new(&self.history.message).color(ui.visuals().weak_text_color()),
                );
            }
        });
        ui.add_space(6.0);

        if self.history.records.is_empty() {
            self.capture_data_empty_state(
                ui,
                t("No history summaries yet"),
                t("Capture or import a combat, then save its summary for later comparison."),
            );
            return;
        }

        ui.label(
            RichText::new(t(
                "Click a record for details; right-click to compare, export or delete",
            ))
            .size(10.5)
            .color(ui.visuals().weak_text_color()),
        );
        ui.add_space(4.0);

        let mut keyboard_scroll_offset = None;
        let mut open_selected_details = false;
        if !ui.ctx().egui_wants_keyboard_input() {
            let keyboard_action = ui.input(|input| {
                if input.modifiers == egui::Modifiers::NONE && input.key_pressed(egui::Key::ArrowUp)
                {
                    Some(-1_isize)
                } else if input.modifiers == egui::Modifiers::NONE
                    && input.key_pressed(egui::Key::ArrowDown)
                {
                    Some(1_isize)
                } else if input.modifiers == egui::Modifiers::NONE
                    && input.key_pressed(egui::Key::Enter)
                {
                    Some(0_isize)
                } else {
                    None
                }
            });
            if let Some(direction) = keyboard_action {
                if direction == 0 {
                    open_selected_details = true;
                    self.history.message = t("Selected record opened in the detail pane");
                } else {
                    let current = self.history.selected_id.as_deref().map_or(0, |id| {
                        self.history
                            .records
                            .iter()
                            .position(|record| record.id == id)
                            .expect("selected history id always belongs to the loaded records")
                    });
                    let next = (current as isize + direction)
                        .clamp(0, self.history.records.len() as isize - 1)
                        as usize;
                    self.history.selected_id = Some(self.history.records[next].id.clone());
                    let list_height = (ui.available_height().max(420.0) - 24.0).max(160.0);
                    keyboard_scroll_offset =
                        Some((next as f32 * 64.0 - (list_height - 64.0) * 0.5).max(0.0));
                }
            }
        }

        let content_height = ui.available_height().max(420.0);
        let record_rows = self
            .history
            .records
            .iter()
            .map(|record| {
                (
                    record.id.clone(),
                    record.display_time(),
                    self.localized_party_label(record, 2),
                    record.summary.total_dps,
                    record.summary.total_damage,
                )
            })
            .collect::<Vec<_>>();
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), content_height),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(300.0, content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.label(RichText::new(t("Records")).strong().color(self.theme().fg));
                        ui.add_space(4.0);
                        let records_scroll = egui::ScrollArea::vertical()
                            .id_salt("history_record_list")
                            .max_height((content_height - 24.0).max(160.0))
                            .auto_shrink([false, false]);
                        let records_scroll = if let Some(offset) = keyboard_scroll_offset {
                            records_scroll.vertical_scroll_offset(offset)
                        } else {
                            records_scroll
                        };
                        records_scroll.show_rows(ui, 64.0, record_rows.len(), |ui, row_range| {
                            for row_index in row_range {
                                let (id, time, party, dps, damage) = &record_rows[row_index];
                                let selected = self.history.selected_id.as_deref() == Some(id);
                                let label = format!(
                                    "{time}\n{party}\n{} DPS · {}",
                                    format_number(*dps),
                                    format_number(*damage)
                                );
                                let response = ui.selectable_label(selected, label);
                                if response.clicked() {
                                    self.history.selected_id = Some(id.clone());
                                }
                                response.context_menu(|ui| {
                                    if ui.button(t("View details")).clicked() {
                                        self.history.selected_id = Some(id.clone());
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            record_rows.len() > 1,
                                            egui::Button::new(t("Compare with adjacent record")),
                                        )
                                        .clicked()
                                    {
                                        let adjacent = if row_index + 1 < record_rows.len() {
                                            row_index + 1
                                        } else {
                                            row_index - 1
                                        };
                                        self.history.selected_id = Some(id.clone());
                                        self.history.compare_left_id = Some(id.clone());
                                        self.history.compare_right_id =
                                            Some(record_rows[adjacent].0.clone());
                                        self.history.message = t("Comparison pair selected");
                                        ui.close();
                                    }
                                    if ui.button(t("Export record JSON")).clicked() {
                                        self.export_history_record(ui.ctx(), id);
                                        ui.close();
                                    }
                                    if ui
                                        .button(
                                            RichText::new(t("Delete"))
                                                .color(semantic_danger(self.dark_mode)),
                                        )
                                        .clicked()
                                    {
                                        self.delete_history_record_for(
                                            id.clone(),
                                            ui.ctx().viewport_id(),
                                        );
                                        ui.close();
                                    }
                                });
                            }
                        });
                    },
                );
                ui.separator();
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let detail_scroll = egui::ScrollArea::vertical()
                            .id_salt("history_detail_compare")
                            .auto_shrink([false, false]);
                        let detail_scroll = if open_selected_details {
                            detail_scroll.vertical_scroll_offset(0.0)
                        } else {
                            detail_scroll
                        };
                        detail_scroll.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let selected = self.history.selected_record().cloned();
                            if let Some(record) = selected {
                                self.history_detail_contents(ui, &record);
                                ui.add_space(8.0);
                                ui.separator();
                                ui.add_space(8.0);
                                self.history_compare_contents(ui);
                            }
                        });
                    },
                );
            },
        );
    }

    /// [`empty_state_card`] with the standard start/import actions, shared by
    /// the timeline, skills and loadout pages.
    pub(crate) fn capture_data_empty_state(
        &mut self,
        ui: &mut egui::Ui,
        title: String,
        body: String,
    ) {
        let theme = self.theme();
        let capture_idle = self.capture.is_none() && self.replay_thread.is_none();
        empty_state_card(ui, theme, title, body, |ui| {
            if capture_idle && ui.add(primary_button(t("Start"), theme.accent)).clicked() {
                self.request_start_live(ui.ctx());
            }
            if ui.button(t("Import Capture JSON")).clicked() {
                self.request_debug_import(ui.ctx(), DebugImportKind::CaptureJson);
            }
        });
    }

    fn export_history_record(&mut self, ctx: &egui::Context, record_id: &str) {
        let Some(record) = self
            .history
            .records
            .iter()
            .find(|record| record.id == record_id)
        else {
            self.set_last_error_in(ctx, t("No history summary found to export"), None);
            return;
        };
        let json = match serde_json::to_string_pretty(record) {
            Ok(json) => format!("{json}\n"),
            Err(error) => {
                self.set_last_error_in(
                    ctx,
                    tf(
                        "Failed to serialize history summary: {}",
                        &[&error.to_string()],
                    ),
                    None,
                );
                return;
            }
        };
        let default_name = format!("nte_history_{}.json", record.id);
        let filter = t("NTE history summary");
        self.spawn_file_dialog(
            ctx,
            FileDialogPurpose::HistoryExport { json },
            move |owner| {
                with_owner(
                    rfd::FileDialog::new()
                        .add_filter(filter, &["json"])
                        .set_file_name(default_name),
                    owner,
                )
                .save_file()
            },
        );
    }

    pub(crate) fn finish_history_record_export(
        &mut self,
        viewport: egui::ViewportId,
        path: &Path,
        json: &str,
    ) {
        match atomic_write_text(path, json) {
            Ok(()) => {
                self.status = t("History summary exported");
                self.clear_last_error();
            }
            Err(error) => self.set_last_error_for(
                viewport,
                tf(
                    "Failed to export history summary: {}",
                    &[&error.to_string()],
                ),
                None,
            ),
        }
    }

    /// Localized compact party preview for a history record — mirrors
    /// [`HistoryRecord::short_party_label`] but resolves member names to the active
    /// language and localizes the abyss up/down prefixes. `limit` caps names per side.
    pub(crate) fn localized_party_label(&self, record: &HistoryRecord, limit: usize) -> String {
        let names = |chars: &[CombatSessionCharacterSummary]| -> String {
            let list: Vec<String> = chars
                .iter()
                .take(limit)
                .map(|row| character_display_name(&self.characters, row.char_id, &row.name))
                .collect();
            if list.is_empty() {
                t("No recorded characters")
            } else {
                list.join(" / ")
            }
        };
        let abyss = &record.summary.abyss;
        let first = abyss
            .first_half
            .as_ref()
            .filter(|half| !half.characters.is_empty())
            .map(|half| format!("{}: {}", t("Upper"), names(&half.characters)));
        let second = abyss
            .second_half
            .as_ref()
            .filter(|half| !half.characters.is_empty())
            .map(|half| format!("{}: {}", t("Lower"), names(&half.characters)));
        let halves: Vec<String> = [first, second].into_iter().flatten().collect();
        if !halves.is_empty() {
            return halves.join(" | ");
        }
        names(&record.summary.characters)
    }

    pub(crate) fn history_detail_contents(&mut self, ui: &mut egui::Ui, record: &HistoryRecord) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(record.display_time())
                    .strong()
                    .color(self.theme().fg),
            );
            ui.label(
                RichText::new(format!(
                    "· {}",
                    localized_dps_time_mode(&record.summary.dps_time_mode)
                ))
                .color(ui.visuals().weak_text_color()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(t("Delete"))
                    .on_hover_text(t("Delete this local history summary"))
                    .clicked()
                {
                    self.delete_history_record_for(record.id.clone(), ui.ctx().viewport_id());
                }
                if let Some(team) = record.lower_team_dps()
                    && ui.button(t("Set as Lower Prediction")).clicked()
                {
                    self.abyss_overview.lower_team = Some(team);
                    self.history.message = t("Set as the lower-line prediction team");
                }
                if let Some(team) = record.upper_team_dps()
                    && ui.button(t("Set as Upper Prediction")).clicked()
                {
                    self.abyss_overview.upper_team = Some(team);
                    self.history.message = t("Set as the upper-line prediction team");
                }
            });
        });
        ui.add_space(6.0);
        ui.columns(4, |columns| {
            let damage_color = columns[1].visuals().text_color();
            let duration_color = columns[2].visuals().text_color();
            let quality_color = columns[3].visuals().text_color();
            compact_metric(
                &mut columns[0],
                &t("Total DPS"),
                format_number(record.summary.total_dps),
                self.theme().accent,
                true,
            );
            compact_metric(
                &mut columns[1],
                &t("Total Damage"),
                format_number(record.summary.total_damage),
                damage_color,
                false,
            );
            compact_metric(
                &mut columns[2],
                &t("Combat Time"),
                format_clear_seconds(record.summary.duration_seconds),
                duration_color,
                false,
            );
            compact_metric(
                &mut columns[3],
                &t("Parse Quality"),
                tf(
                    "{} hits / {} pending",
                    &[
                        &record.summary.quality.hit_count.to_string(),
                        &record.summary.quality.unmapped_skill_hits.to_string(),
                    ],
                ),
                quality_color,
                false,
            );
        });
        ui.add_space(8.0);
        if record.summary.abyss.first_half.is_some() || record.summary.abyss.second_half.is_some() {
            if let Some(half) = &record.summary.abyss.first_half {
                let visual = HistoryVisualContext {
                    dark_mode: self.dark_mode,
                    characters: &self.characters,
                    avatar_textures: &self.avatar_textures,
                };
                draw_history_abyss_half(ui, half, visual);
            }
            if record.summary.abyss.first_half.is_some()
                && record.summary.abyss.second_half.is_some()
            {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
            }
            if let Some(half) = &record.summary.abyss.second_half {
                let visual = HistoryVisualContext {
                    dark_mode: self.dark_mode,
                    characters: &self.characters,
                    avatar_textures: &self.avatar_textures,
                };
                draw_history_abyss_half(ui, half, visual);
            }
        } else {
            draw_history_summary_rows(
                ui,
                &t("Character"),
                &record.summary.characters,
                &t("Skill"),
                &record.summary.skills,
                HistoryVisualContext {
                    dark_mode: self.dark_mode,
                    characters: &self.characters,
                    avatar_textures: &self.avatar_textures,
                },
            );
        }
    }

    pub(crate) fn history_compare_contents(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("Compare")).strong().color(self.theme().fg));
        let choices = self
            .history
            .records
            .iter()
            .map(|record| {
                (
                    record.id.clone(),
                    format!(
                        "{} · {}",
                        record.display_time(),
                        self.localized_party_label(record, 4)
                    ),
                )
            })
            .collect::<Vec<_>>();
        // Stack the two selectors so they never overflow the panel horizontally; each combo's width
        // tracks the available width (clamped) and truncates long labels.
        let combo_width = (ui.available_width() - 56.0).clamp(180.0, 460.0);
        egui::Grid::new("history_compare_selectors")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(RichText::new(t("Baseline")).color(ui.visuals().weak_text_color()));
                history_record_combo(
                    ui,
                    "history_compare_left",
                    &mut self.history.compare_left_id,
                    &choices,
                    combo_width,
                );
                ui.end_row();
                ui.label(RichText::new(t("Compare")).color(ui.visuals().weak_text_color()));
                history_record_combo(
                    ui,
                    "history_compare_right",
                    &mut self.history.compare_right_id,
                    &choices,
                    combo_width,
                );
                ui.end_row();
            });
        let Some((left, right, comparison)) = self.history.compare_records() else {
            ui.label(
                RichText::new(t("Select two different records"))
                    .color(ui.visuals().weak_text_color()),
            );
            return;
        };
        if left.summary.dps_time_mode != right.summary.dps_time_mode {
            ui.label(
                RichText::new(t(
                    "The two records use different DPS time bases; compare with care",
                ))
                .color(semantic_warning(self.dark_mode)),
            );
        }
        ui.columns(3, |columns| {
            delta_metric(
                &mut columns[0],
                &t("Total DPS Δ"),
                comparison.total_dps_delta,
                self.dark_mode,
            );
            delta_metric(
                &mut columns[1],
                &t("Total Damage Δ"),
                comparison.total_damage_delta,
                self.dark_mode,
            );
            delta_metric(
                &mut columns[2],
                &t("Time Δ"),
                comparison.duration_delta,
                self.dark_mode,
            );
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(t("Character Δ")).color(ui.visuals().weak_text_color()));
                for row in &comparison.character_deltas {
                    ui.horizontal(|ui| {
                        ui.add_sized([120.0, 20.0], egui::Label::new(&row.name).truncate());
                        ui.monospace(format_signed_number(row.delta_dps));
                    });
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label(RichText::new(t("Skill Δ")).color(ui.visuals().weak_text_color()));
                for row in &comparison.skill_deltas {
                    ui.horizontal(|ui| {
                        ui.add_sized([190.0, 20.0], egui::Label::new(&row.name).truncate());
                        ui.monospace(format_signed_number(row.delta_damage));
                    });
                }
            });
        });
    }

    pub(crate) fn debug_encrypted_ini_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(t("Open INI")).clicked() {
                self.request_debug_import(ui.ctx(), DebugImportKind::EncryptedIni);
            }
            let can_save = self.encrypted_ini_editor.path.is_some();
            if ui
                .add_enabled(can_save, egui::Button::new(t("Save as Encrypted INI")))
                .clicked()
            {
                self.save_encrypted_ini(ui.ctx());
            }
            if ui
                .add_enabled(can_save, egui::Button::new(t("Reload")))
                .clicked()
                && let Some(path) = self.encrypted_ini_editor.path.clone()
            {
                if self.encrypted_ini_editor.dirty {
                    self.request_confirmation_for(
                        ui.ctx().viewport_id(),
                        ConfirmationAction::ReloadEncryptedIni(path),
                    );
                } else {
                    self.load_encrypted_ini_in(ui.ctx(), path);
                }
            }
            if ui.button(t("Clear")).clicked() {
                if self.encrypted_ini_editor.dirty {
                    self.request_confirmation_for(
                        ui.ctx().viewport_id(),
                        ConfirmationAction::ClearEncryptedIni,
                    );
                } else {
                    self.run_confirmation_action_for(
                        ConfirmationAction::ClearEncryptedIni,
                        ui.ctx().viewport_id(),
                    );
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_sized([92.0, 28.0], egui::Label::new(t("File")).truncate());
            ui.monospace(self.encrypted_ini_editor.display_path());
        });
        ui.horizontal(|ui| {
            ui.add_sized([92.0, 28.0], egui::Label::new(t("Save Key")).truncate());
            egui::ComboBox::from_id_salt("encrypted_ini_key")
                .width(200.0)
                .selected_text(t(self.encrypted_ini_editor.key.label()))
                .show_ui(ui, |ui| {
                    for key in EncryptedIniKey::all() {
                        stable_popup_selectable_value(
                            ui,
                            &mut self.encrypted_ini_editor.key,
                            key,
                            t(key.label()),
                        );
                    }
                });
        });
        let editor_id = ui.make_persistent_id("encrypted_ini_plaintext_editor");
        let mut jump_to_match = false;
        ui.horizontal(|ui| {
            ui.add_sized([92.0, 28.0], egui::Label::new(t("Search")).truncate());
            let search_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.encrypted_ini_editor.search)
                        .desired_width(360.0)
                        .vertical_align(egui::Align::Center)
                        .hint_text(t("Enter a config name or value")),
                )
                .changed();
            if search_changed {
                self.encrypted_ini_editor.search_match = None;
                self.encrypted_ini_editor.search_matches_dirty = true;
            }
            self.encrypted_ini_editor.refresh_search_matches();
            let matches = &self.encrypted_ini_editor.search_matches;
            let can_search = !matches.is_empty();
            if ui
                .add_enabled(can_search, egui::Button::new(t("Previous")))
                .clicked()
            {
                self.encrypted_ini_editor.search_match =
                    previous_search_match(self.encrypted_ini_editor.search_match, matches.len());
                jump_to_match = true;
            }
            if ui
                .add_enabled(can_search, egui::Button::new(t("Next")))
                .clicked()
            {
                self.encrypted_ini_editor.search_match =
                    next_search_match(self.encrypted_ini_editor.search_match, matches.len());
                jump_to_match = true;
            }
            if self.encrypted_ini_editor.search.is_empty() {
                ui.label(t("No search"));
            } else if let Some(current) = self.encrypted_ini_editor.search_match {
                if let Some(&byte_index) = matches.get(current) {
                    let (line, column) =
                        line_column_for_byte(&self.encrypted_ini_editor.plaintext, byte_index);
                    ui.monospace(tf(
                        "{}/{}  line {} col {}",
                        &[
                            &(current + 1).to_string(),
                            &matches.len().to_string(),
                            &line.to_string(),
                            &column.to_string(),
                        ],
                    ));
                }
            } else {
                ui.monospace(tf("{} matches", &[&matches.len().to_string()]));
            }
        });
        if !self.encrypted_ini_editor.message.is_empty() {
            ui.label(
                RichText::new(&self.encrypted_ini_editor.message)
                    .color(semantic_warning(self.dark_mode)),
            );
        }
        ui.separator();
        let editor_height = (ui.available_height() - 28.0).max(180.0);
        let editor_width = ui.available_width();
        let editor = &mut self.encrypted_ini_editor;
        let matches = &editor.search_matches;
        let current_match_byte = editor
            .search_match
            .and_then(|index| matches.get(index).copied());
        let current_cursor_range = current_match_byte.and_then(|byte_index| {
            encrypted_ini_match_cursor_range(&editor.plaintext, &editor.search, byte_index)
        });
        let dark_mode = self.dark_mode;
        let accent = self.accent;
        let search = &editor.search;
        let layout_cache = &mut editor.layout_cache;
        let plaintext = &mut editor.plaintext;
        let mut editor_changed = false;
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            encrypted_ini_layout_galley(
                ui,
                EncryptedIniLayoutRequest {
                    text: buffer.as_str(),
                    query: search,
                    matches,
                    current_match_byte,
                    wrap_width,
                    dark_mode,
                    accent,
                },
                layout_cache,
            )
        };
        egui::ScrollArea::both()
            .id_salt("encrypted_ini_editor_scroll")
            .auto_shrink([false, false])
            .max_height(editor_height)
            .show(ui, |ui| {
                let mut editor_output = egui::TextEdit::multiline(plaintext)
                    .id(editor_id)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(editor_width)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .hint_text(t(
                        "After opening an encrypted INI, the decrypted plaintext appears here.",
                    ))
                    .show(ui);
                if editor_output.response.changed() {
                    editor_changed = true;
                }
                if jump_to_match && let Some(cursor_range) = current_cursor_range {
                    editor_output
                        .state
                        .cursor
                        .set_char_range(Some(cursor_range));
                    editor_output
                        .state
                        .store(ui.ctx(), editor_output.response.id);
                    editor_output.response.request_focus();
                    let cursor_rect = editor_output
                        .galley
                        .pos_from_cursor(cursor_range.primary)
                        .translate(editor_output.galley_pos.to_vec2());
                    ui.scroll_to_rect(
                        cursor_rect.expand2(egui::vec2(80.0, 32.0)),
                        Some(egui::Align::Center),
                    );
                    ui.ctx().request_repaint();
                }
            });
        if editor_changed {
            editor.dirty = true;
            editor.search_matches_dirty = true;
            editor.layout_cache.clear();
        }
        ui.horizontal(|ui| {
            if self.encrypted_ini_editor.dirty {
                ui.label(t("Unsaved changes"));
            } else if self.encrypted_ini_editor.path.is_some() {
                ui.label(t("Current content is saved or unchanged"));
            }
        });
    }

    pub(crate) fn load_encrypted_ini_in(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.load_encrypted_ini_for(path, ctx.viewport_id());
    }

    pub(crate) fn load_encrypted_ini_for(&mut self, path: PathBuf, viewport: egui::ViewportId) {
        match EncryptedIniEditorState::load(path) {
            Ok(editor) => {
                self.encrypted_ini_editor = editor;
                self.clear_last_error();
            }
            Err(error) => {
                self.encrypted_ini_editor.message = error.clone();
                self.set_last_error_for(viewport, error, Some(ErrorAction::OpenEncryptedIni));
            }
        }
    }

    pub(crate) fn save_encrypted_ini(&mut self, ctx: &egui::Context) {
        let Some(path) = self.encrypted_ini_editor.path.clone() else {
            self.encrypted_ini_editor.message = t("Open an INI file first");
            return;
        };
        if self.encrypted_ini_editor.plaintext == self.encrypted_ini_editor.original_plaintext
            && self.encrypted_ini_editor.key == self.encrypted_ini_editor.original_key
        {
            self.encrypted_ini_editor.dirty = false;
            self.encrypted_ini_editor.message =
                t("Content unchanged; the original ciphertext file was kept");
            return;
        }
        let encrypted = match encrypt_encrypted_ini_records(
            &self.encrypted_ini_editor.plaintext,
            self.encrypted_ini_editor.key,
            self.encrypted_ini_editor.original_key,
            &self.encrypted_ini_editor.records,
            &self.encrypted_ini_editor.line_ending,
            self.encrypted_ini_editor.final_newline,
        ) {
            Ok(encrypted) => encrypted,
            Err(error) => {
                self.encrypted_ini_editor.message =
                    tf("Failed to generate ciphertext: {}", &[&error]);
                self.set_last_error_in(ctx, self.encrypted_ini_editor.message.clone(), None);
                return;
            }
        };
        if let Err(error) = atomic_write_text(&path, &encrypted) {
            self.encrypted_ini_editor.message = tf(
                "Failed to save {}: {}",
                &[&path.display().to_string(), &error],
            );
            self.set_last_error_in(ctx, self.encrypted_ini_editor.message.clone(), None);
            return;
        }
        self.encrypted_ini_editor.original_key = self.encrypted_ini_editor.key;
        self.encrypted_ini_editor.original_plaintext = self.encrypted_ini_editor.plaintext.clone();
        self.encrypted_ini_editor.dirty = false;
        self.encrypted_ini_editor.message = tf(
            "Saved to {} using the {} key",
            &[
                &path.display().to_string(),
                &t(self.encrypted_ini_editor.key.label()),
            ],
        );
        self.status = t("Encrypted INI saved");
        self.clear_last_error();
    }

    /// Manual capture-NIC override UI (Settings tab). Automatic detection is the default; checking
    /// the box pins capture to a chosen interface as a VPN fallback. The choice persists via
    /// `UiConfig` and re-applies the game network through `refresh_game_network` so it takes effect
    /// on the next capture.
    pub(crate) fn capture_device_selector(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            if self.devices.is_empty() {
                let mut unchecked = false;
                ui.add_enabled(
                    false,
                    egui::Checkbox::new(&mut unchecked, t("Pin capture NIC (VPN fallback)")),
                );
                ui.colored_label(
                    semantic_warning(self.dark_mode),
                    t("No usable NIC found; confirm Npcap is installed, then click refresh"),
                );
                if ui.button(t("Refresh NIC List")).clicked() {
                    let _ = self.refresh_game_network();
                }
                return;
            }

            let mut manual = self.manual_capture_device.is_some();
            if ui
                .checkbox(&mut manual, t("Pin capture NIC"))
                .on_hover_text(t(
                    "Auto-detection may pick the wrong NIC under a VPN; checking this pins the chosen NIC, effective on the next capture",
                ))
                .changed()
            {
                // A non-empty device list guarantees a default, so manual mode is never left
                // checked-but-empty.
                self.manual_capture_device = manual
                    .then(|| {
                        self.devices
                            .get(self.selected_device)
                            .or_else(|| self.devices.first())
                            .map(|device| device.name.clone())
                    })
                    .flatten();
                let _ = self.refresh_game_network();
            }

            if self.manual_capture_device.is_none() {
                return;
            }

            let mut chosen = self.manual_capture_device.clone();
            let selected_text = chosen
                .as_deref()
                .and_then(|name| self.devices.iter().find(|device| device.name == name))
                .map_or_else(|| t("Select a NIC"), capture_device_label);
            egui::ComboBox::from_id_salt("manual_capture_device")
                .width(300.0)
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    ui.set_min_width(300.0);
                    for device in &self.devices {
                        ui.selectable_value(
                            &mut chosen,
                            Some(device.name.clone()),
                            capture_device_label(device),
                        );
                    }
                });
            if chosen != self.manual_capture_device {
                self.manual_capture_device = chosen;
                let _ = self.refresh_game_network();
            }

            if ui
                .button(t("Refresh NIC List"))
                .on_hover_text(t("Re-enumerate NICs"))
                .clicked()
            {
                let _ = self.refresh_game_network();
            }

            // Self-contained status hint, independent of the shared diagnostic field.
            let resolved = self
                .manual_capture_device
                .as_deref()
                .is_some_and(|name| self.devices.iter().any(|device| device.name == name));
            if !resolved {
                ui.colored_label(
                    semantic_warning(self.dark_mode),
                    t("The selected NIC is currently unavailable; reselect or click refresh"),
                );
            } else if self.game_network.is_none() {
                ui.weak(t("No game connection detected; parsing by public/private direction heuristics"));
            }
        });
    }

    /// First-class settings promoted out of the old debug "环境" tab: parse
    /// options, team DPS import/export, and an entry to the abyss value tables.
    /// Always available (not gated behind the debug feature).
    /// Below this content width the settings page drops from two columns to one.
    /// Two ~300px columns can't hold a verbose-language row (e.g. English "Calibrate
    /// with server-side HP deltas"), so a narrow or high-DPI console reflows to a
    /// single full-width column instead of clipping.
    const SETTINGS_TWO_COLUMN_MIN_WIDTH: f32 = 900.0;

    pub(crate) fn settings_contents(&mut self, ui: &mut egui::Ui) {
        let previous_hud_config = self.hud_config.clone();
        // Two balanced columns when wide (interface/parse/hotkeys left, the
        // lighter HUD/team/capture/abyss cards right), single column when the
        // console is too narrow to split without clipping a localized row.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .scroll_source(egui::containers::scroll_area::ScrollSource {
                scroll_bar: true,
                drag: false,
                mouse_wheel: true,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if settings_uses_two_columns(ui.available_width()) {
                    ui.columns(2, |columns| {
                        self.settings_interface_section(&mut columns[0]);
                        self.settings_parse_section(&mut columns[0]);
                        self.settings_hotkeys_section(&mut columns[0]);
                        self.settings_hud_section(&mut columns[1]);
                        self.settings_layout_profiles_section(&mut columns[1]);
                        self.settings_team_section(&mut columns[1]);
                        self.settings_capture_logs_section(&mut columns[1]);
                        self.settings_abyss_section(&mut columns[1]);
                    });
                } else {
                    self.settings_interface_section(ui);
                    self.settings_parse_section(ui);
                    self.settings_hotkeys_section(ui);
                    self.settings_hud_section(ui);
                    self.settings_layout_profiles_section(ui);
                    self.settings_team_section(ui);
                    self.settings_capture_logs_section(ui);
                    self.settings_abyss_section(ui);
                }
            });
        self.hud_config = self.hud_config.clone().sanitized();
        if self.hud_config != previous_hud_config {
            self.hud_size_key = None;
        }
    }

    /// Interface preferences shown at the top of settings. Currently the UI
    /// language picker; the dropdown lists each language written in its own script
    /// and persists the choice to the config file.
    fn settings_interface_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Interface", |ui| {
            egui::Grid::new("settings_interface")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label(t("Language"));
                    let mut language = self.language;
                    egui::ComboBox::from_id_salt("ui_language")
                        .width(settings_value_width(ui))
                        .selected_text(language.native_name())
                        .show_ui(ui, |ui| {
                            ui.set_min_width(150.0);
                            for option in Language::all() {
                                stable_popup_selectable_value(
                                    ui,
                                    &mut language,
                                    *option,
                                    option.native_name(),
                                );
                            }
                        });
                    if language != self.language {
                        self.set_language(ui.ctx(), language);
                    }
                    ui.end_row();

                    ui.label(t("Theme Preset"));
                    let mut theme_preset = self.theme_preset;
                    egui::ComboBox::from_id_salt("ui_theme_preset")
                        .width(settings_value_width(ui))
                        .selected_text(t(theme_preset.label()))
                        .show_ui(ui, |ui| {
                            ui.set_min_width(220.0);
                            for option in ThemePreset::all() {
                                ui.selectable_value(
                                    &mut theme_preset,
                                    *option,
                                    format!("{} · {}", t(option.label()), t(option.description())),
                                );
                            }
                        });
                    if theme_preset != self.theme_preset {
                        self.set_theme_preset(ui.ctx(), theme_preset);
                    }
                    ui.end_row();

                    ui.label(t("Accent"));
                    let mut accent = self.accent;
                    egui::ComboBox::from_id_salt("ui_accent")
                        .width(settings_value_width(ui))
                        .selected_text(t(accent.label()))
                        .show_ui(ui, |ui| {
                            ui.set_min_width(150.0);
                            for option in AccentColor::all() {
                                let color = theme_tokens_for_preset(
                                    self.theme_preset,
                                    self.dark_mode,
                                    *option,
                                )
                                .accent;
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::vec2(10.0, 10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().circle_filled(rect.center(), 4.0, color);
                                    stable_popup_selectable_value(
                                        ui,
                                        &mut accent,
                                        *option,
                                        t(option.label()),
                                    );
                                });
                            }
                        });
                    self.accent = accent;
                    ui.end_row();

                    ui.label(t("Density"));
                    let mut density = self.density;
                    egui::ComboBox::from_id_salt("ui_density")
                        .width(settings_value_width(ui))
                        .selected_text(t(density.label()))
                        .show_ui(ui, |ui| {
                            ui.set_min_width(150.0);
                            for option in UiDensity::all() {
                                stable_popup_selectable_value(
                                    ui,
                                    &mut density,
                                    *option,
                                    t(option.label()),
                                );
                            }
                        });
                    self.density = density;
                    ui.end_row();

                    ui.label(t("Motion"));
                    ui.checkbox(&mut self.reduce_motion, t("Reduce motion"))
                        .on_hover_text(t(
                            "Complete interface transitions instantly and reduce idle redraws",
                        ));
                    ui.end_row();
                });
        });
    }

    fn settings_parse_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Parse Settings", |ui| {
            egui::Grid::new("settings_parse")
                    .num_columns(2)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(t("BPF Filter"));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.filter)
                                .desired_width(settings_value_width(ui)),
                        )
                            .on_hover_text(t("Capture filter expression; takes effect on the next capture"));
                        ui.end_row();
                        ui.label(t("Capture NIC"));
                        self.capture_device_selector(ui);
                        ui.end_row();
                        ui.label(t("Damage Source"));
                        ui.checkbox(
                            &mut self.server_damage_calibration,
                            t("Calibrate with server-side HP deltas"),
                        )
                        .on_hover_text(t(
                            "Takes effect after re-capturing or re-importing; only overrides damage when a server HP sync can be unambiguously paired to a single hit",
                        ));
                        ui.end_row();
                        ui.label(t("DPS Time"));
                        let mut dps_time_mode = self.dps_time_mode;
                        egui::ComboBox::from_id_salt("dps_time_mode")
                            .width(settings_value_width(ui))
                            .selected_text(t(dps_time_mode.label()))
                            .show_ui(ui, |ui| {
                                ui.set_min_width(150.0);
                                for option in DpsTimeMode::all() {
                                    stable_popup_selectable_value(
                                        ui,
                                        &mut dps_time_mode,
                                        *option,
                                        t(option.label()),
                                    );
                                }
                            })
                            .response
                            .on_hover_text(t(dps_time_mode.description()));
                        if dps_time_mode != self.dps_time_mode {
                            self.dps_time_mode = dps_time_mode;
                            self.character_hit_cache = HitDetailCache::default();
                            self.team_hit_cache = HitDetailCache::default();
                        }
                        ui.end_row();
                        ui.label(t("Passthrough Hotkey"));
                        let mut hotkey = self.passthrough_hotkey;
                        egui::ComboBox::from_id_salt("passthrough_hotkey")
                            .width(settings_value_width(ui))
                            .selected_text(hotkey.label())
                            .show_ui(ui, |ui| {
                                ui.set_min_width(PASSTHROUGH_HOTKEY_COMBO_WIDTH);
                                for option in PassthroughHotkey::all() {
                                    stable_popup_selectable_value(
                                        ui,
                                        &mut hotkey,
                                        *option,
                                        option.label(),
                                    );
                                }
                            });
                        if hotkey != self.passthrough_hotkey {
                            self.set_passthrough_hotkey(hotkey);
                        }
                        ui.end_row();
                    });
        });
    }

    fn settings_hud_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "HUD", |ui| {
            // Wrapped rows rather than a fixed grid: a verbose language's checkbox
            // labels reflow onto the next line instead of clipping when the column
            // is narrow. The leading label acts as the row heading.
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Top"));
                ui.checkbox(&mut self.hud_config.show_title, t("Title"));
                ui.checkbox(&mut self.hud_config.show_team_dps, t("DPS"));
                ui.checkbox(&mut self.hud_config.show_duration, t("Time"));
                ui.checkbox(&mut self.hud_config.show_total_damage, t("Total Damage"));
                ui.checkbox(&mut self.hud_config.show_damage_taken, t("Damage Taken"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Modules"));
                ui.checkbox(
                    &mut self.hud_config.show_character_rows,
                    t("Character Ranking"),
                );
                ui.checkbox(&mut self.hud_config.show_abyss_half, t("Abyss"));
                ui.checkbox(
                    &mut self.hud_config.show_passthrough_state,
                    t("Passthrough"),
                );
                ui.checkbox(&mut self.hud_config.show_mini_timeline, t("Curve"));
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Presets"));
                if ui.button(t("Minimal")).clicked() {
                    let mut preset = HudConfig::minimal();
                    preset.width = self.hud_config.width;
                    preset.module_order = self.hud_config.module_order.clone();
                    self.hud_config = preset;
                }
                if ui.button(t("Standard")).clicked() {
                    self.hud_config = HudConfig {
                        width: self.hud_config.width,
                        module_order: self.hud_config.module_order.clone(),
                        ..HudConfig::default()
                    };
                }
                if ui.button(t("Detailed")).clicked() {
                    let mut preset = HudConfig::detailed();
                    preset.width = self.hud_config.width;
                    preset.module_order = self.hud_config.module_order.clone();
                    self.hud_config = preset;
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.label(t("HUD Width"));
                ui.add(
                    egui::DragValue::new(&mut self.hud_config.width)
                        .range(HUD_WIDTH_MIN..=HUD_WIDTH_MAX)
                        .speed(4.0)
                        .suffix(" px"),
                );
                if ui.button(t("Open HUD Editor")).clicked() {
                    self.console_open = false;
                    self.console_corner_applied = false;
                    ui.ctx()
                        .send_viewport_cmd_to(console_viewport_id(), egui::ViewportCommand::Close);
                    self.set_hud_mode(ui.ctx(), true);
                    self.set_mouse_passthrough(ui.ctx(), false);
                    ui.ctx()
                        .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                }
            });
            ui.label(
                RichText::new(t("Drag HUD modules to reorder; right-click to hide"))
                    .size(11.0)
                    .color(ui.visuals().weak_text_color()),
            );
            let order = self.hud_config.module_order.clone();
            let mut reorder = None;
            let mut move_request = None;
            let mut hide_request = None;
            let dragged_module =
                egui::DragAndDrop::payload::<HudModule>(ui.ctx()).map(|item| *item);
            let theme = self.theme();
            for (index, module) in order.iter().copied().enumerate() {
                let mut visible = self.hud_config.module_visible(module);
                let target_slot = index as f32;
                let animated_slot = motion::animate_value(
                    ui.ctx(),
                    ("settings_hud_module_slot", module),
                    target_slot,
                    motion::dur::BASE,
                    self.reduce_motion,
                );
                let row_stride = ui.spacing().interact_size.y + 8.0 + ui.spacing().item_spacing.y;
                let transformed = ui.with_visual_transform(
                    egui::emath::TSTransform::from_translation(egui::vec2(
                        0.0,
                        settings_hud_module_animation_offset(index, animated_slot, row_stride),
                    )),
                    |ui| {
                        ui.dnd_drop_zone::<HudModule, _>(
                            egui::Frame::new()
                                .fill(if dragged_module == Some(module) {
                                    theme.muted
                                } else {
                                    theme.bg_elevated
                                })
                                .stroke(Stroke::new(1.0_f32, theme.border))
                                .corner_radius(6)
                                .inner_margin(egui::Margin::symmetric(8, 4)),
                            |ui| {
                                if dragged_module == Some(module) {
                                    ui.set_opacity(0.18);
                                }
                                ui.horizontal(|ui| {
                                    let drag_handle = ui
                                        .add(
                                            egui::Label::new(
                                                RichText::new("≡").strong().color(theme.fg_faint),
                                            )
                                            .sense(egui::Sense::drag()),
                                        )
                                        .on_hover_cursor(egui::CursorIcon::Grab);
                                    drag_handle.dnd_set_drag_payload(module);
                                    ui.checkbox(&mut visible, t(module.label()));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if ui
                                                .add_enabled(
                                                    index + 1 < order.len(),
                                                    egui::Button::new("↓"),
                                                )
                                                .on_hover_text(t("Move down"))
                                                .clicked()
                                            {
                                                move_request = Some((index, index + 1));
                                            }
                                            if ui
                                                .add_enabled(index > 0, egui::Button::new("↑"))
                                                .on_hover_text(t("Move up"))
                                                .clicked()
                                            {
                                                move_request = Some((index, index - 1));
                                            }
                                        },
                                    );
                                })
                            },
                        )
                    },
                );
                let (drop_zone, dropped) = transformed.inner;
                if visible != self.hud_config.module_visible(module) {
                    self.hud_config.set_module_visible(module, visible);
                }
                drop_zone.inner.response.context_menu(|ui| {
                    if ui.button(t("Hide module")).clicked() {
                        hide_request = Some(module);
                        ui.close();
                    }
                });
                if let Some(dragged) = drop_zone.response.dnd_hover_payload::<HudModule>()
                    && *dragged != module
                {
                    ui.painter().line_segment(
                        [
                            drop_zone.response.rect.left_bottom(),
                            drop_zone.response.rect.right_bottom(),
                        ],
                        Stroke::new(3.0_f32, theme.accent),
                    );
                }
                if let Some(dropped) = dropped
                    && *dropped != module
                {
                    reorder = Some((*dropped, module));
                }
            }
            if let (Some(module), Some(pointer)) = (dragged_module, ui.ctx().pointer_interact_pos())
            {
                paint_settings_hud_drag_ghost(
                    ui.ctx(),
                    pointer,
                    module,
                    self.hud_config.module_visible(module),
                    theme,
                );
            }
            if let Some((from, to)) = move_request {
                self.hud_config.module_order.swap(from, to);
            } else if let Some((dragged, target)) = reorder {
                let from = self
                    .hud_config
                    .module_order
                    .iter()
                    .position(|module| *module == dragged)
                    .expect("dragged HUD module belongs to module_order");
                let target = self
                    .hud_config
                    .module_order
                    .iter()
                    .position(|module| *module == target)
                    .expect("drop target belongs to module_order");
                self.hud_config.module_order.swap(from, target);
            }
            if let Some(module) = hide_request {
                self.hud_config.set_module_visible(module, false);
            }
        });
    }

    fn settings_layout_profiles_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Layout Profiles", |ui| {
            for profile in [
                LayoutProfile::Combat,
                LayoutProfile::Review,
                LayoutProfile::Research,
            ] {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t(profile.label())).clicked() {
                        self.apply_layout_profile(ui.ctx(), profile);
                    }
                    ui.label(
                        RichText::new(t(profile.description()))
                            .size(11.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                });
            }
        });
    }

    fn settings_team_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Team Data", |ui| {
            ui.horizontal(|ui| {
                    if ui
                        .button(t("Import DPS Data"))
                        .on_hover_text(t("Import team DPS data (json) for abyss clear prediction"))
                        .clicked()
                    {
                        self.import_team_dps(ui.ctx());
                    }
                    if ui
                        .button(t("Export Team Data"))
                        .on_hover_text(t("Export the current team and the abyss upper/lower teams' DPS (json, no packets)"))
                        .clicked()
                    {
                        self.export_team_dps(ui.ctx());
                    }
                });
            ui.small(t(
                "Import/export is scene-independent; works in both open world and abyss",
            ));
        });
    }

    fn settings_capture_logs_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Capture Files", |ui| {
            if self.capture_log_stats.is_none() {
                self.refresh_capture_log_stats();
            }
            let stats = self.capture_log_stats.unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(tf(
                    "Raw captures: {} · {}",
                    &[
                        &stats.count.to_string(),
                        &capture_logs::format_bytes(stats.total_bytes),
                    ],
                ));
                if ui.button(t("Refresh")).clicked() {
                    self.refresh_capture_log_stats();
                }
                if ui
                    .add_enabled(stats.count > 0, egui::Button::new(t("Clear")))
                    .clicked()
                {
                    self.request_confirmation_for(
                        ui.ctx().viewport_id(),
                        ConfirmationAction::ClearCaptureLogs,
                    );
                }
            });
            ui.small(t("Live capture writes raw frames to logs/nte_raw_*.pcapng; clearing does not affect stats or history."));
        });
    }

    fn settings_abyss_section(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Abyss Values", |ui| {
            if ui
                .button(t("Open Abyss Value Tables"))
                .on_hover_text(t(
                    "Opens in a separate window so you can view it side by side with live DPS",
                ))
                .clicked()
            {
                self.abyss_overview_open = true;
                self.abyss_overview.ensure_selection();
            }
        });
    }

    /// Runtime resource coverage for maintainers. This only checks distributable
    /// `res/` files and never touches client export paths or resource keys.
    pub(crate) fn resource_audit_contents(&mut self, ui: &mut egui::Ui) {
        if self.resource_audit.summary.is_none() && !self.resource_audit.loading {
            self.request_resource_audit();
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.resource_audit.loading,
                    egui::Button::new(t("Refresh Check")),
                )
                .clicked()
            {
                self.request_resource_audit();
            }
            if self.resource_audit.loading {
                ui.add(egui::Spinner::new().size(16.0));
                ui.label(t("Checking runtime resources"));
            } else if !self.resource_audit.message.is_empty() {
                ui.label(
                    RichText::new(&self.resource_audit.message)
                        .color(ui.visuals().weak_text_color()),
                );
            }
            if let Some(summary) = &self.resource_audit.summary
                && ui.button(t("Copy Redacted Report")).clicked()
            {
                ui.ctx().copy_text(summary.redacted_text());
            }
        });
        ui.add_space(6.0);
        let Some(summary) = self.resource_audit.summary.as_ref() else {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 160.0),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.label(
                        RichText::new(t("Waiting for resource check results"))
                            .color(ui.visuals().weak_text_color()),
                    );
                },
            );
            return;
        };
        ui.columns(4, |columns| {
            compact_metric(
                &mut columns[0],
                &t("Errors"),
                summary.error_count().to_string(),
                semantic_danger(self.dark_mode),
                true,
            );
            compact_metric(
                &mut columns[1],
                &t("Warnings"),
                summary.warning_count().to_string(),
                semantic_warning(self.dark_mode),
                true,
            );
            compact_metric(
                &mut columns[2],
                &t("Characters/Skills"),
                format!(
                    "{} / {}",
                    summary.counts.characters, summary.counts.skill_damage
                ),
                self.theme().accent,
                false,
            );
            let abyss_reaction_color = columns[3].visuals().text_color();
            compact_metric(
                &mut columns[3],
                &t("Abyss/Reactions"),
                format!(
                    "{} / {}",
                    summary.counts.abyss_monsters, summary.counts.reactions
                ),
                abyss_reaction_color,
                false,
            );
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(t("Level"));
            egui::ComboBox::from_id_salt("resource_audit_severity_filter")
                .width(120.0)
                .selected_text(t(self.resource_audit.severity_filter.label()))
                .show_ui(ui, |ui| {
                    stable_popup_selectable_value(
                        ui,
                        &mut self.resource_audit.severity_filter,
                        ResourceAuditSeverityFilter::All,
                        t(ResourceAuditSeverityFilter::All.label()),
                    );
                    stable_popup_selectable_value(
                        ui,
                        &mut self.resource_audit.severity_filter,
                        ResourceAuditSeverityFilter::Error,
                        t(ResourceAuditSeverityFilter::Error.label()),
                    );
                    stable_popup_selectable_value(
                        ui,
                        &mut self.resource_audit.severity_filter,
                        ResourceAuditSeverityFilter::Warning,
                        t(ResourceAuditSeverityFilter::Warning.label()),
                    );
                });
            ui.label(t("Category"));
            egui::ComboBox::from_id_salt("resource_audit_category_filter")
                .width(120.0)
                .selected_text(t(self.resource_audit.category_filter.label()))
                .show_ui(ui, |ui| {
                    stable_popup_selectable_value(
                        ui,
                        &mut self.resource_audit.category_filter,
                        ResourceAuditCategoryFilter::All,
                        t(ResourceAuditCategoryFilter::All.label()),
                    );
                    for category in ResourceAuditCategory::all() {
                        stable_popup_selectable_value(
                            ui,
                            &mut self.resource_audit.category_filter,
                            ResourceAuditCategoryFilter::Category(*category),
                            t(category.label()),
                        );
                    }
                });
        });
        let filtered = summary
            .items
            .iter()
            .filter(|item| self.resource_audit.severity_filter.matches(item.severity))
            .filter(|item| self.resource_audit.category_filter.matches(item.category))
            .collect::<Vec<_>>();
        ui.add_space(6.0);
        if filtered.is_empty() {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 120.0),
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.label(
                        RichText::new(t("No resource gaps under the current filter"))
                            .color(ui.visuals().weak_text_color()),
                    );
                },
            );
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("resource_audit_rows")
            .max_height((ui.available_height() - 12.0).max(180.0))
            .auto_shrink([false, false])
            .show_rows(ui, 44.0, filtered.len(), |ui, visible_rows| {
                for item in &filtered[visible_rows] {
                    draw_resource_audit_row(ui, item, self.dark_mode);
                }
            });
    }

    /// Read-only capture diagnostics plus raw-capture import/export. Genuine
    /// debugging — only reachable via the debug-gated "诊断" tab.
    pub(crate) fn diagnostics_contents(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("diagnostics_contents_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                self.diagnostics_contents_inner(ui);
            });
    }

    pub(crate) fn diagnostics_contents_inner(&mut self, ui: &mut egui::Ui) {
        settings_section(ui, self.theme(), "Capture Environment", |ui| {
            egui::Grid::new("diagnostics_environment")
                .num_columns(2)
                .spacing([14.0, 5.0])
                .show(ui, |ui| {
                    ui.label(t("NIC"));
                    let device_label = self
                        .devices
                        .get(self.selected_device)
                        .map(|device| {
                            if device.description.is_empty() {
                                device.name.clone()
                            } else {
                                device.description.clone()
                            }
                        })
                        .unwrap_or_else(|| t("Not detected"));
                    let mode_suffix = if self.manual_capture_device.is_some() {
                        t("(manual)")
                    } else {
                        t("(auto)")
                    };
                    ui.monospace(format!("{device_label}{mode_suffix}"));
                    ui.end_row();
                    ui.label(t("Local IP"));
                    ui.monospace(if self.local_ip.is_empty() {
                        t("Not detected")
                    } else {
                        self.local_ip.clone()
                    });
                    ui.end_row();
                    ui.label(t("Game Connection"));
                    if let Some(network) = &self.game_network {
                        ui.monospace(format!(
                            "PID {}  {} -> {}:{}",
                            network.pid, network.local_ip, network.remote_ip, network.remote_port
                        ));
                    } else {
                        ui.monospace(t("Not detected"));
                    }
                    ui.end_row();
                    ui.label(t("Diagnostics"));
                    ui.monospace(self.diagnostic.clone().unwrap_or_else(|| t("Normal")));
                    ui.end_row();
                    ui.label(t("Actual BPF"));
                    ui.monospace(self.active_capture_filter.clone().unwrap_or_else(|| {
                        if self.capture.is_some() {
                            t("Determining")
                        } else {
                            t("Not started")
                        }
                    }));
                    ui.end_row();
                    ui.label(t("Raw Capture"));
                    let raw_capture_label = self.raw_capture.as_ref().map_or_else(
                        || t("No raw capture"),
                        |capture| {
                            let file = capture.path().map_or_else(
                                || t("Write unavailable"),
                                |path| {
                                    path.file_name()
                                        .and_then(|name| name.to_str())
                                        .map(|name| name.to_owned())
                                        .unwrap_or_else(|| t("Raw capture file"))
                                },
                            );
                            tf(
                                "{} packets · {}",
                                &[&capture.packet_count().to_string(), &file],
                            )
                        },
                    );
                    ui.monospace(raw_capture_label);
                    ui.end_row();
                });
            ui.horizontal(|ui| {
                if ui.button(t("Re-detect")).clicked()
                    && let Err(error) = self.refresh_game_network()
                {
                    self.set_last_error_in(ui.ctx(), error, Some(ErrorAction::RefreshNetwork));
                }
                ui.label(t("Damage-taken logging enabled"));
                let can_export_json = self.capture.is_none()
                    && self.replay_thread.is_none()
                    && (!self.state.hits.is_empty()
                        || !self.state.packets.is_empty()
                        || !self.state.empty_curtain.is_empty());
                if ui
                    .add_enabled(can_export_json, egui::Button::new(t("Export Parsed JSON")))
                    .clicked()
                {
                    self.export_capture_info(ui.ctx());
                }
                let can_export_raw = self.capture.is_none()
                    && self
                        .raw_capture
                        .as_ref()
                        .is_some_and(|capture| capture.packet_count() > 0);
                if ui
                    .add_enabled(can_export_raw, egui::Button::new(t("Save Full PCAPNG As")))
                    .clicked()
                {
                    self.export_raw_capture(ui.ctx());
                }
            });
            ui.horizontal(|ui| {
                    if ui.button(t("Import pcapng")).clicked() {
                        self.request_debug_import(ui.ctx(), DebugImportKind::Pcapng);
                    }
                    if ui.button(t("Import Capture JSON")).clicked() {
                        self.request_debug_import(ui.ctx(), DebugImportKind::CaptureJson);
                    }
                    ui.small(t("Importing clears the current stats and uses the same parse pipeline as live capture"));
                });
        });
        ui.add_space(8.0);
        settings_section(ui, self.theme(), "Auto-diagnostics Wizard", |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !self.diagnostics_running,
                        egui::Button::new(t("Run Diagnostics")),
                    )
                    .clicked()
                {
                    self.request_capture_diagnostics();
                }
                if self.diagnostics_running {
                    ui.add(egui::Spinner::new().size(16.0));
                    ui.label(t(
                        "Checking Npcap, the game connection and the current capture state",
                    ));
                }
                if let Some(report) = &self.diagnostics_report
                    && ui.button(t("Copy Redacted Report")).clicked()
                {
                    ui.ctx().copy_text(report.redacted_text());
                }
            });
            ui.add_space(4.0);
            if let Some(report) = &self.diagnostics_report {
                draw_diagnostic_report(ui, report, self.dark_mode);
            } else {
                ui.label(
                        RichText::new(t("After you click Run Diagnostics, it checks the capture environment step by step and suggests next steps"))
                            .color(ui.visuals().weak_text_color()),
                    );
            }
        });
        ui.add_space(8.0);
        let quality = self.current_quality_summary();
        draw_capture_quality_summary(ui, &quality, self.theme());
    }

    pub(crate) fn debug_packets_contents(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.debug_only_hits, t("Hit packets only"));
            ui.label(t("Search"));
            ui.add(
                egui::TextEdit::singleline(&mut self.debug_search)
                    .desired_width(260.0)
                    .hint_text(t("IP / ID / protocol name")),
            );
            ui.separator();
            ui.monospace(format!(
                "events={} packets={} queued={}",
                self.state.hits.len(),
                self.state.packets.len(),
                self.receiver.len()
            ));
        });
        ui.separator();
        let scroll_width = ui.available_width();
        let debug_query = self.debug_search.to_lowercase();
        egui::ScrollArea::vertical()
            .max_width(scroll_width)
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.set_max_width(ui.available_width());
                for (packet_index, packet) in
                    self.state.packets.iter().rev().take(500).rev().enumerate()
                {
                    if self.debug_only_hits && packet.parsed_hits == 0 {
                        continue;
                    }
                    if !debug_query.is_empty() {
                        let searchable = format!(
                            "{} {} {} {:?} {}",
                            packet.source,
                            packet.destination,
                            packet.direction,
                            packet.declared_ids,
                            packet.decoded_text
                        )
                        .to_lowercase();
                        if !searchable.contains(&debug_query) {
                            continue;
                        }
                    }
                    let title = format!(
                        "{}  {}  {} -> {}  {} B  ids={:?}  hits={}",
                        format_time(packet.timestamp),
                        packet.direction,
                        packet.source,
                        packet.destination,
                        packet.payload_len,
                        packet.declared_ids,
                        packet.parsed_hits
                    );
                    let id = ui.make_persistent_id((
                        "debug_packet",
                        packet_index,
                        packet.timestamp.to_bits(),
                        &packet.source,
                        &packet.destination,
                    ));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        id,
                        false,
                    )
                    .show_header(ui, |ui| {
                        ui.add(
                            egui::Label::new(title)
                                .truncate()
                                .sense(egui::Sense::click()),
                        );
                    })
                    .body(|ui| {
                        if !packet.note.is_empty() {
                            ui.label(
                                RichText::new(&packet.note).color(semantic_warning(self.dark_mode)),
                            );
                        }
                        ui.label(
                            RichText::new(t("Auto Parse"))
                                .strong()
                                .color(self.theme().accent),
                        );
                        ui.add(
                            egui::TextEdit::multiline(&mut packet.decoded_text.clone())
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(packet.decoded_text.lines().count().clamp(2, 14))
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
                }
            });
    }

    pub(crate) fn debug_characters_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(t("New ID"));
            ui.add(
                egui::TextEdit::singleline(&mut self.character_editor.new_id)
                    .desired_width(100.0)
                    .hint_text(t("e.g. 1080")),
            );
            if ui.button(t("Add")).clicked()
                && let Err(error) = self.character_editor.start_new()
            {
                self.character_editor.message = error;
            }
            if ui.button(t("Reload")).clicked() {
                let path = data_root().join(CHARACTER_DATA_PATH);
                match CharacterEditorState::load(&path) {
                    Ok(editor) => {
                        self.character_editor = editor;
                        self.status = t("Reloaded characters.json");
                    }
                    Err(error) => self.character_editor.message = error,
                }
            }
            ui.separator();
            ui.label(tf(
                "Total {} entries",
                &[&self.character_editor.character_ids().len().to_string()],
            ));
        });
        if !self.character_editor.message.is_empty() {
            ui.label(
                RichText::new(&self.character_editor.message)
                    .color(semantic_warning(self.dark_mode)),
            );
        }
        ui.separator();

        let ids = self.character_editor.character_ids();
        let search = self.character_editor.search.to_lowercase();
        ui.columns(2, |columns| {
            columns[0].set_min_width(240.0);
            columns[0].set_max_width(320.0);
            columns[0].horizontal(|ui| {
                ui.label(t("Search"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.character_editor.search)
                        .desired_width(180.0)
                        .hint_text(t("ID / name / attribute")),
                );
            });
            columns[0].separator();
            egui::ScrollArea::vertical()
                .id_salt("character_editor_list")
                .auto_shrink([false, false])
                .show(&mut columns[0], |ui| {
                    ui.spacing_mut().item_spacing.y = 7.0;
                    for id in ids {
                        let row = self
                            .character_editor
                            .document
                            .get("characters")
                            .and_then(serde_json::Value::as_object)
                            .and_then(|characters| characters.get(&id))
                            .and_then(serde_json::Value::as_object);
                        let name_zh =
                            row.map_or_else(String::new, |row| json_string_field(row, "name_zh"));
                        let name_en =
                            row.map_or_else(String::new, |row| json_string_field(row, "name_en"));
                        let attribute =
                            row.map_or_else(String::new, |row| json_string_field(row, "attribute"));
                        let avatar =
                            row.map_or_else(String::new, |row| json_string_field(row, "avatar"));
                        let color =
                            row.map_or_else(String::new, |row| json_string_field(row, "color"));
                        let searchable =
                            format!("{id} {name_zh} {name_en} {attribute}").to_lowercase();
                        if !search.is_empty() && !searchable.contains(&search) {
                            continue;
                        }
                        let selected =
                            self.character_editor.selected_id.as_deref() == Some(id.as_str());
                        let fallback_color = parse_hex_color(color.trim()).unwrap_or_else(|| {
                            character_color(
                                id.parse::<u32>().unwrap_or_default(),
                                self.characters.as_ref(),
                                0,
                                self.dark_mode,
                            )
                        });
                        let dark_mode = self.dark_mode;
                        let fallback_color = readable_accent(fallback_color, dark_mode);
                        let clicked = {
                            let avatar_texture =
                                self.character_editor_avatar_texture(ui.ctx(), &avatar);
                            draw_character_editor_card(
                                ui,
                                CharacterEditorCard {
                                    id: &id,
                                    name_zh: &name_zh,
                                    name_en: &name_en,
                                    attribute: &attribute,
                                    avatar_texture,
                                    selected,
                                    fallback_color,
                                    dark_mode,
                                },
                            )
                            .clicked()
                        };
                        if clicked {
                            if self.character_editor.dirty {
                                self.character_editor.message =
                                    t("Save the current changes before switching characters");
                            } else {
                                self.character_editor.select(&id);
                            }
                        }
                    }
                });

            columns[1].heading(if self.character_editor.selected_id.is_some() {
                t("Edit Character")
            } else if self.character_editor.form.id.is_empty() {
                t("Select or add a character")
            } else {
                t("Add Character")
            });
            columns[1].separator();
            if self.character_editor.form.id.is_empty() {
                columns[1].label(t(
                    "Select a record on the left, or enter a new ID and click Add.",
                ));
                return;
            }
            egui::Grid::new("character_editor_form")
                .num_columns(2)
                .spacing([12.0, 7.0])
                .show(&mut columns[1], |ui| {
                    ui.label(t("Character ID"));
                    ui.add_enabled(
                        self.character_editor.selected_id.is_none(),
                        egui::TextEdit::singleline(&mut self.character_editor.form.id),
                    );
                    ui.end_row();
                    character_text_field(
                        ui,
                        &t("Chinese Name"),
                        &mut self.character_editor.form.name_zh,
                        &mut self.character_editor.dirty,
                    );
                    character_text_field(
                        ui,
                        &t("English Name"),
                        &mut self.character_editor.form.name_en,
                        &mut self.character_editor.dirty,
                    );
                    character_text_field(
                        ui,
                        "Codename",
                        &mut self.character_editor.form.codename,
                        &mut self.character_editor.dirty,
                    );
                    ui.label(t("Attribute"));
                    let previous_attribute = self.character_editor.form.attribute.clone();
                    egui::ComboBox::from_id_salt("character_attribute")
                        .width(CHARACTER_ATTRIBUTE_COMBO_WIDTH)
                        .selected_text(if self.character_editor.form.attribute.is_empty() {
                            t("Not set")
                        } else {
                            self.character_editor.form.attribute.clone()
                        })
                        .show_ui(ui, |ui| {
                            ui.set_min_width(CHARACTER_ATTRIBUTE_COMBO_WIDTH);
                            ui.set_max_width(CHARACTER_ATTRIBUTE_COMBO_WIDTH);
                            stable_popup_selectable_value(
                                ui,
                                &mut self.character_editor.form.attribute,
                                String::new(),
                                t("Not set"),
                            );
                            for attribute in CHARACTER_ATTRIBUTES {
                                stable_popup_selectable_value(
                                    ui,
                                    &mut self.character_editor.form.attribute,
                                    attribute.to_owned(),
                                    attribute,
                                );
                            }
                        });
                    if self.character_editor.form.attribute != previous_attribute {
                        self.character_editor.dirty = true;
                    }
                    ui.end_row();
                    ui.label(t("Verified"));
                    if ui
                        .checkbox(&mut self.character_editor.form.verified, "")
                        .changed()
                    {
                        self.character_editor.dirty = true;
                    }
                    ui.end_row();
                    character_text_field(
                        ui,
                        &t("Color"),
                        &mut self.character_editor.form.color,
                        &mut self.character_editor.dirty,
                    );
                    character_text_field(
                        ui,
                        &t("Avatar Path"),
                        &mut self.character_editor.form.avatar,
                        &mut self.character_editor.dirty,
                    );
                });
            columns[1].add_space(8.0);
            columns[1].horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.character_editor.dirty,
                        egui::Button::new(t("Save to characters.json")),
                    )
                    .clicked()
                {
                    self.save_character_editor(ui.ctx());
                }
                if ui
                    .add_enabled(
                        self.character_editor.dirty,
                        egui::Button::new(t("Cancel Changes")),
                    )
                    .clicked()
                {
                    self.character_editor.cancel_edit();
                }
                if self.character_editor.dirty {
                    ui.label(t("Unsaved changes"));
                }
            });
        });
    }

    pub(crate) fn character_editor_avatar_texture(
        &mut self,
        ctx: &egui::Context,
        avatar: &str,
    ) -> Option<&egui::TextureHandle> {
        let avatar = avatar.trim();
        if avatar.is_empty() {
            return None;
        }
        if !self.avatar_textures.contains_key(avatar)
            && let Some(texture) = load_image_texture(ctx, &data_root(), avatar, "character-avatar")
        {
            self.avatar_textures.insert(avatar.to_owned(), texture);
        }
        self.avatar_textures.get(avatar)
    }

    pub(crate) fn save_character_editor(&mut self, ctx: &egui::Context) {
        let id = match self.character_editor.apply_form() {
            Ok(id) => id,
            Err(error) => {
                self.character_editor.message = error;
                return;
            }
        };
        let path = data_root().join(CHARACTER_DATA_PATH);
        let text = match serde_json::to_string_pretty(&self.character_editor.document) {
            Ok(text) => format!("{text}\n"),
            Err(error) => {
                self.character_editor.message = tf(
                    "Character table serialization failed: {}",
                    &[&error.to_string()],
                );
                self.character_editor.dirty = true;
                return;
            }
        };
        if let Err(error) = atomic_write_text(&path, &text) {
            self.character_editor.message = tf(
                "Failed to save {}: {}",
                &[&path.display().to_string(), &error],
            );
            self.character_editor.dirty = true;
            return;
        }
        match load_characters(&path) {
            Ok(characters) => {
                self.avatar_textures = load_character_avatars(ctx, &data_root(), &characters);
                self.characters = Arc::new(characters);
                self.character_editor.message = tf(
                    "ID {} saved and reloaded; the live-capture mapping updates on next startup",
                    &[&id],
                );
                self.status = t("characters.json saved");
                self.clear_last_error();
            }
            Err(error) => {
                self.character_editor.message =
                    tf("File written, but reload failed: {}", &[&error.to_string()]);
                self.character_editor.dirty = true;
            }
        }
    }

    pub(crate) fn show_viewport_dialogs(&mut self, ctx: &egui::Context) {
        self.show_confirmation_dialog(ctx);
        self.show_error_window(ctx);
    }

    pub(crate) fn show_confirmation_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_confirmation.as_ref() else {
            return;
        };
        if self.pending_confirmation_viewport != ctx.viewport_id() {
            return;
        }
        let (title, message, confirm_label) = confirmation_content(action);
        let mut confirmed =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut cancelled =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        egui::Window::new(t(title))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(message);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t(confirm_label)).clicked() {
                        confirmed = true;
                    }
                    if ui.button(t("Cancel")).clicked() {
                        cancelled = true;
                    }
                });
            });
        if confirmed {
            if let Some(action) = self.pending_confirmation.take() {
                self.run_confirmation_action_for(action, ctx.viewport_id());
            }
        } else if cancelled {
            self.pending_confirmation = None;
        }
    }

    pub(crate) fn show_error_window(&mut self, ctx: &egui::Context) {
        let Some(error) = self.last_error.clone() else {
            return;
        };
        if self.last_error_viewport != ctx.viewport_id() {
            return;
        }
        let action = self.last_error_action;
        let mut run_action = None;
        let mut close = false;
        egui::Window::new(t("Error"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(error);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(action) = action
                        && ui.button(t(error_action_label(action))).clicked()
                    {
                        run_action = Some(action);
                    }
                    if ui.button(t("Close")).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(action) = run_action {
            self.clear_last_error();
            self.run_error_action(ctx, action);
        } else if close {
            self.clear_last_error();
        }
    }

    pub(crate) fn run_error_action(&mut self, ctx: &egui::Context, action: ErrorAction) {
        match action {
            ErrorAction::RefreshNetwork => {
                if let Err(error) = self.refresh_game_network() {
                    self.set_last_error_in(ctx, error, Some(ErrorAction::RefreshNetwork));
                }
            }
            ErrorAction::OpenPcapng => self.request_debug_import(ctx, DebugImportKind::Pcapng),
            ErrorAction::OpenCaptureJson => {
                self.request_debug_import(ctx, DebugImportKind::CaptureJson);
            }
            ErrorAction::OpenEncryptedIni => {
                self.request_debug_import(ctx, DebugImportKind::EncryptedIni);
            }
            ErrorAction::OpenTeamDpsImport => self.import_team_dps(ctx),
            ErrorAction::OpenConsole => {
                self.console_open = true;
                self.console_corner_applied = false;
            }
        }
    }

    pub(crate) fn retarget_dialogs(&mut self, from: egui::ViewportId, to: egui::ViewportId) {
        if self.last_error.is_some() && self.last_error_viewport == from {
            self.last_error_viewport = to;
        }
        if self.pending_confirmation.is_some() && self.pending_confirmation_viewport == from {
            self.pending_confirmation_viewport = to;
        }
        if let Some(pending) = &mut self.pending_file_dialog
            && pending.viewport == from
        {
            pending.viewport = to;
        }
        if let Some(active_import) = &mut self.active_import
            && active_import.viewport == from
        {
            active_import.viewport = to;
        }
        if self.engine_task_viewport == Some(from) {
            self.engine_task_viewport = Some(to);
        }
        self.close_command_palette_for(from);
        for toast in &mut self.status_toasts {
            if toast.viewport == from {
                toast.viewport = to;
            }
        }
    }
}

fn console_sidebar_collapsed(width: f32) -> bool {
    width < 720.0
}

fn console_sidebar_width(collapse_progress: f32) -> f32 {
    egui::lerp(164.0..=44.0, collapse_progress.clamp(0.0, 1.0))
}

fn settings_uses_two_columns(width: f32) -> bool {
    width >= DpsApp::SETTINGS_TWO_COLUMN_MIN_WIDTH
}

/// Width for a value widget inside a settings grid cell: fill the card's
/// remaining width so the form stretches with the window, with a floor so a
/// narrow column stays usable.
fn settings_value_width(ui: &egui::Ui) -> f32 {
    ui.available_width().max(150.0)
}

fn paint_settings_hud_drag_ghost(
    ctx: &egui::Context,
    pointer: egui::Pos2,
    module: HudModule,
    visible: bool,
    theme: ThemeTokens,
) {
    let rect = settings_hud_drag_ghost_rect(ctx.content_rect(), pointer);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("settings_hud_module_drag_ghost"),
    ));
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 6.0)),
        8.0,
        Color32::from_black_alpha(72),
    );
    painter.rect_filled(rect, 8.0, theme.floating);
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(2.0_f32, theme.accent),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.left_center() + egui::vec2(16.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "≡",
        egui::FontId::proportional(14.0),
        theme.fg_faint,
    );
    let check_center = rect.left_center() + egui::vec2(48.0, 0.0);
    painter.circle_filled(check_center, 10.0, theme.card);
    painter.circle_stroke(
        check_center,
        10.0,
        Stroke::new(1.0_f32, theme.border_strong),
    );
    if visible {
        painter.line_segment(
            [
                check_center + egui::vec2(-4.0, 0.0),
                check_center + egui::vec2(-1.0, 4.0),
            ],
            Stroke::new(1.8_f32, theme.fg),
        );
        painter.line_segment(
            [
                check_center + egui::vec2(-1.0, 4.0),
                check_center + egui::vec2(5.0, -5.0),
            ],
            Stroke::new(1.8_f32, theme.fg),
        );
    }
    painter.text(
        rect.left_center() + egui::vec2(68.0, 0.0),
        egui::Align2::LEFT_CENTER,
        t(module.label()),
        egui::FontId::proportional(13.0),
        theme.fg,
    );
    ctx.request_repaint();
}

fn settings_hud_drag_ghost_rect(content_rect: egui::Rect, pointer: egui::Pos2) -> egui::Rect {
    let size = egui::vec2(280.0, 54.0);
    let bounds = content_rect.shrink(8.0);
    let desired = pointer - egui::vec2(20.0, 27.0);
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

fn settings_hud_module_animation_offset(
    target_index: usize,
    animated_slot: f32,
    row_stride: f32,
) -> f32 {
    (animated_slot - target_index as f32) * row_stride
}

/// Compact chip for one detected combat segment in the timeline page.
fn draw_combat_segment_chip(
    ui: &mut egui::Ui,
    index: usize,
    segment: &CombatSegment,
    dark_mode: bool,
) {
    egui::Frame::popup(ui.style())
        .fill(shadcn_card(dark_mode))
        .stroke(Stroke::new(1.0_f32, shadcn_border(dark_mode)))
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.label(
                RichText::new(tf(
                    "Seg {} · {}~{} · {} · {} DPS",
                    &[
                        &index.to_string(),
                        &format_timeline_seconds(segment.start_offset),
                        &format_timeline_seconds(segment.end_offset),
                        &format_number(segment.total_damage),
                        &format_number(segment.dps),
                    ],
                ))
                .size(11.0),
            );
        });
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn console_sidebar_breakpoint_is_stable() {
        assert!(console_sidebar_collapsed(719.9));
        assert!(!console_sidebar_collapsed(720.0));
    }

    #[test]
    fn console_sidebar_width_interpolates_between_layout_states() {
        assert_eq!(console_sidebar_width(0.0), 164.0);
        assert_eq!(console_sidebar_width(0.5), 104.0);
        assert_eq!(console_sidebar_width(1.0), 44.0);
        assert_eq!(console_sidebar_width(2.0), 44.0);
    }

    #[test]
    fn settings_columns_reflow_at_the_content_breakpoint() {
        assert!(!settings_uses_two_columns(899.9));
        assert!(settings_uses_two_columns(900.0));
    }

    #[test]
    fn settings_hud_drag_preview_keeps_pointer_inside_the_row() {
        let pointer = egui::pos2(500.0, 260.0);
        let rect = settings_hud_drag_ghost_rect(
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)),
            pointer,
        );

        assert!(rect.contains(pointer));
    }

    #[test]
    fn settings_hud_module_animation_is_relative_to_stable_slots() {
        assert_eq!(settings_hud_module_animation_offset(2, 2.0, 40.0), 0.0);
        assert_eq!(settings_hud_module_animation_offset(2, 1.5, 40.0), -20.0);
        assert_eq!(settings_hud_module_animation_offset(1, 1.5, 40.0), 20.0);
    }
}

impl DpsApp {
    fn settings_hotkeys_section(&mut self, ui: &mut egui::Ui) {
        self.capture_recorded_hotkey(ui.ctx());
        settings_section(ui, self.theme(), "Hotkeys", |ui| {
            let mut hotkeys = self.global_hotkeys;
            if ui
                .checkbox(&mut hotkeys.enabled, t("Enable global hotkeys"))
                .changed()
            {
                self.set_global_hotkeys(hotkeys);
            }
            ui.add_space(4.0);
            egui::Grid::new("settings_global_hotkeys")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for action in GlobalHotkeyAction::all() {
                        ui.label(t(action.label()));
                        let recording = self.recording_hotkey == Some(*action);
                        let label = if recording {
                            t("Press a shortcut...")
                        } else {
                            self.global_hotkeys
                                .binding(*action)
                                .map(HotkeyBinding::label)
                                .unwrap_or_else(|| t("Disabled"))
                        };
                        if ui
                            .add_sized(
                                egui::vec2((ui.available_width() - 96.0).max(150.0), 28.0),
                                egui::Button::new(label),
                            )
                            .on_hover_text(t(
                                "Click, then press Ctrl/Alt/Shift plus an F1-F12 key; Esc cancels",
                            ))
                            .clicked()
                        {
                            self.set_recording_hotkey(Some(*action));
                        }
                        if ui
                            .add_enabled(
                                self.global_hotkeys.binding(*action).is_some(),
                                egui::Button::new(t("Disable")),
                            )
                            .clicked()
                        {
                            let mut hotkeys = self.global_hotkeys;
                            hotkeys.set_binding(*action, None);
                            self.set_global_hotkeys(hotkeys);
                            self.set_recording_hotkey(None);
                        }
                        ui.end_row();
                    }
                });
            ui.small(
                RichText::new(tf("Command palette: {}", &["Ctrl+K"]))
                    .color(ui.visuals().weak_text_color()),
            );
        });
    }

    fn capture_recorded_hotkey(&mut self, ctx: &egui::Context) {
        let Some(action) = self.recording_hotkey else {
            return;
        };
        let event = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        let Some((key, modifiers)) = event else {
            return;
        };
        if key == egui::Key::Escape {
            self.set_recording_hotkey(None);
            return;
        }
        if key == egui::Key::Backspace || key == egui::Key::Delete {
            let mut hotkeys = self.global_hotkeys;
            hotkeys.set_binding(action, None);
            self.set_global_hotkeys(hotkeys);
            self.set_recording_hotkey(None);
            return;
        }
        let Some(key) = hotkey_key_from_egui(key) else {
            self.status = t("Use an F1-F12 key for global shortcuts");
            return;
        };
        if !modifiers.ctrl && !modifiers.alt && !modifiers.shift {
            self.status = t("Global shortcuts require Ctrl, Alt, or Shift");
            return;
        }
        let binding = HotkeyBinding::new(modifiers.ctrl, modifiers.alt, modifiers.shift, key);
        if binding.is_reserved() {
            self.status = t("This shortcut is reserved by Windows");
            self.set_recording_hotkey(None);
            return;
        }
        if GlobalHotkeyAction::all()
            .iter()
            .copied()
            .any(|other| other != action && self.global_hotkeys.binding(other) == Some(binding))
        {
            self.status = t("This shortcut is already assigned");
            return;
        }
        let mut hotkeys = self.global_hotkeys;
        hotkeys.set_binding(action, Some(binding));
        self.set_global_hotkeys(hotkeys);
        self.set_recording_hotkey(None);
        self.status = tf(
            "{} shortcut switched to {}",
            &[&t(action.label()), &binding.label()],
        );
    }
}

fn hotkey_key_from_egui(key: egui::Key) -> Option<HotkeyKey> {
    HotkeyKey::all()
        .iter()
        .copied()
        .find(|candidate| hotkey_key_to_egui(*candidate) == key)
}

#[cfg(test)]
mod layout_navigation_tests {
    use super::*;

    #[test]
    fn console_tab_navigation_follows_sidebar_order() {
        assert_eq!(ConsoleTab::Settings.adjacent(1), ConsoleTab::History);
        assert_eq!(ConsoleTab::History.adjacent(1), ConsoleTab::Timeline);
        assert_eq!(
            ConsoleTab::Settings.adjacent(-1),
            *ConsoleTab::visible_tabs()
                .last()
                .expect("visible tabs are not empty")
        );
    }
}
