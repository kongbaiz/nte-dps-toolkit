use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppAction {
    ToggleCapture,
    ResetSession,
    ToggleHud,
    TogglePassthrough,
    TogglePause,
    TogglePin,
    ToggleTheme,
    ToggleReducedMotion,
    UndoLatest,
    OpenConsole(ConsoleTab),
    OpenAbyss,
    OpenTeamDetails,
    ImportPcapng,
    ImportCaptureJson,
    SaveHistory,
    ExportTeamDps,
    ExportCaptureInfo,
    ExportRawCapture,
    OpenCaptureLogs,
    SetAccent(AccentColor),
    SetDensity(UiDensity),
}

#[derive(Clone, Copy)]
pub(crate) struct CommandSpec {
    pub(crate) id: &'static str,
    pub(crate) title_key: &'static str,
    pub(crate) category_key: &'static str,
    pub(crate) keywords: &'static [&'static str],
    pub(crate) action: AppAction,
}

#[derive(Default)]
pub(crate) struct CommandPaletteState {
    pub(crate) open: bool,
    pub(crate) owner: Option<egui::ViewportId>,
    pub(crate) query: String,
    pub(crate) selected: usize,
    pub(crate) request_focus: bool,
    pub(crate) reset_scroll: bool,
}

impl DpsApp {
    pub(crate) fn command_specs(&self) -> Vec<CommandSpec> {
        let mut commands = vec![
            command(
                "capture.toggle",
                "Start / Stop Capture",
                "Combat",
                &["capture", "start", "stop"],
                AppAction::ToggleCapture,
            ),
            command(
                "session.reset",
                "Reset Session",
                "Combat",
                &["clear", "undo"],
                AppAction::ResetSession,
            ),
            command(
                "hud.toggle",
                "Toggle Combat HUD",
                "Windows",
                &["overlay", "game"],
                AppAction::ToggleHud,
            ),
            command(
                "passthrough.toggle",
                "Toggle Mouse Passthrough",
                "Windows",
                &["mouse", "click"],
                AppAction::TogglePassthrough,
            ),
            command(
                "capture.pause",
                "Pause / Resume Processing",
                "Combat",
                &["pause", "resume"],
                AppAction::TogglePause,
            ),
            command(
                "window.pin",
                "Toggle Always on Top",
                "Windows",
                &["pin", "top"],
                AppAction::TogglePin,
            ),
            command(
                "theme.toggle",
                "Toggle Light / Dark Theme",
                "Appearance",
                &["theme", "light", "dark"],
                AppAction::ToggleTheme,
            ),
            command(
                "motion.toggle",
                "Toggle Reduced Motion",
                "Appearance",
                &["animation", "accessibility"],
                AppAction::ToggleReducedMotion,
            ),
            command(
                "action.undo",
                "Undo Last Action",
                "General",
                &["restore", "reset", "delete"],
                AppAction::UndoLatest,
            ),
            command(
                "window.abyss",
                "Open Abyss Overview",
                "Windows",
                &["abyss", "prediction"],
                AppAction::OpenAbyss,
            ),
            command(
                "window.team_details",
                "Open Team Combat Details",
                "Windows",
                &["hits", "detail"],
                AppAction::OpenTeamDetails,
            ),
            command(
                "import.pcapng",
                "Import PCAPNG",
                "Data",
                &["replay", "wireshark"],
                AppAction::ImportPcapng,
            ),
            command(
                "import.capture_json",
                "Import Capture JSON",
                "Data",
                &["replay", "json"],
                AppAction::ImportCaptureJson,
            ),
            command(
                "history.save",
                "Save History Summary",
                "Data",
                &["history", "snapshot"],
                AppAction::SaveHistory,
            ),
            command(
                "team.export",
                "Export Team DPS Data",
                "Data",
                &["json", "abyss"],
                AppAction::ExportTeamDps,
            ),
            command(
                "capture.export_parsed",
                "Export Parsed JSON",
                "Data",
                &["capture", "json", "export"],
                AppAction::ExportCaptureInfo,
            ),
            command(
                "capture.export_raw",
                "Save Full PCAPNG As",
                "Data",
                &["capture", "pcapng", "raw"],
                AppAction::ExportRawCapture,
            ),
            command(
                "capture.open_logs",
                "Open Capture Logs Folder",
                "Data",
                &["capture", "logs", "folder"],
                AppAction::OpenCaptureLogs,
            ),
        ];
        for tab in ConsoleTab::visible_tabs() {
            let (id, title) = console_command_metadata(*tab);
            commands.push(command(
                id,
                title,
                "Console",
                &["console", "tab", "page"],
                AppAction::OpenConsole(*tab),
            ));
        }
        commands.extend([
            command(
                "accent.zinc",
                "Use Zinc Accent",
                "Appearance",
                &["color", "neutral"],
                AppAction::SetAccent(AccentColor::Zinc),
            ),
            command(
                "accent.blue",
                "Use Blue Accent",
                "Appearance",
                &["color"],
                AppAction::SetAccent(AccentColor::Blue),
            ),
            command(
                "accent.violet",
                "Use Violet Accent",
                "Appearance",
                &["color", "purple"],
                AppAction::SetAccent(AccentColor::Violet),
            ),
            command(
                "accent.orange",
                "Use Orange Accent",
                "Appearance",
                &["color"],
                AppAction::SetAccent(AccentColor::Orange),
            ),
            command(
                "accent.green",
                "Use Green Accent",
                "Appearance",
                &["color"],
                AppAction::SetAccent(AccentColor::Green),
            ),
            command(
                "density.compact",
                "Use Compact Density",
                "Appearance",
                &["spacing", "small"],
                AppAction::SetDensity(UiDensity::Compact),
            ),
            command(
                "density.cozy",
                "Use Cozy Density",
                "Appearance",
                &["spacing", "default"],
                AppAction::SetDensity(UiDensity::Cozy),
            ),
            command(
                "density.comfortable",
                "Use Comfortable Density",
                "Appearance",
                &["spacing", "large"],
                AppAction::SetDensity(UiDensity::Comfortable),
            ),
        ]);
        commands
    }

    pub(crate) fn execute_action(&mut self, ctx: &egui::Context, action: AppAction) {
        match action {
            AppAction::ToggleCapture => {
                if self.capture.is_some() || self.replay_thread.is_some() {
                    self.stop_engine();
                    self.status = t("Stopped");
                } else {
                    self.request_start_live(ctx);
                }
            }
            AppAction::ResetSession => self.request_reset_combat_session(ctx),
            AppAction::ToggleHud => self.set_hud_mode(ctx, !self.hud_mode),
            AppAction::TogglePassthrough => self.toggle_mouse_passthrough(ctx),
            AppAction::TogglePause => {
                self.paused = !self.paused;
                self.status = if self.paused {
                    t("Processing paused")
                } else {
                    t("Processing resumed")
                };
            }
            AppAction::TogglePin => self.toggle_always_on_top(ctx),
            AppAction::ToggleTheme => self.toggle_theme(ctx),
            AppAction::ToggleReducedMotion => self.reduce_motion = !self.reduce_motion,
            AppAction::UndoLatest => self.undo_latest(ctx.viewport_id()),
            AppAction::OpenConsole(tab) => {
                self.console_tab = tab;
                self.console_open = true;
                self.console_corner_applied = false;
                ctx.send_viewport_cmd_to(console_viewport_id(), egui::ViewportCommand::Focus);
            }
            AppAction::OpenAbyss => {
                self.abyss_overview_open = true;
                self.abyss_overview_corner_applied = false;
                ctx.send_viewport_cmd_to(
                    abyss_overview_viewport_id(),
                    egui::ViewportCommand::Focus,
                );
            }
            AppAction::OpenTeamDetails => {
                self.team_hit_detail_open = true;
                self.team_hit_detail_corner_applied = false;
                ctx.send_viewport_cmd_to(
                    team_hit_detail_viewport_id(),
                    egui::ViewportCommand::Focus,
                );
            }
            AppAction::ImportPcapng => self.request_debug_import(ctx, DebugImportKind::Pcapng),
            AppAction::ImportCaptureJson => {
                self.request_debug_import(ctx, DebugImportKind::CaptureJson)
            }
            AppAction::SaveHistory => self.save_current_history_summary(ctx),
            AppAction::ExportTeamDps => self.export_team_dps(ctx),
            AppAction::ExportCaptureInfo => self.export_capture_info(ctx),
            AppAction::ExportRawCapture => self.export_raw_capture(ctx),
            AppAction::OpenCaptureLogs => {
                let path = Path::new(capture_logs::CAPTURE_LOG_DIR);
                match std::fs::create_dir_all(path)
                    .map_err(|error| error.to_string())
                    .and_then(|_| std::fs::canonicalize(path).map_err(|error| error.to_string()))
                    .and_then(|path| open_directory(&path))
                {
                    Ok(()) => self.status = t("Capture logs folder opened"),
                    Err(error) => self.set_last_error_in(
                        ctx,
                        tf("Failed to open capture logs folder: {}", &[&error]),
                        None,
                    ),
                }
            }
            AppAction::SetAccent(accent) => self.accent = accent,
            AppAction::SetDensity(density) => self.density = density,
        }
    }

    pub(crate) fn toggle_command_palette(&mut self, ctx: &egui::Context) {
        if self.command_palette.open {
            self.command_palette.open = false;
            self.command_palette.owner = None;
            return;
        }
        let owner = self.interactive_viewport_for(ctx);
        self.command_palette.open = true;
        self.command_palette.owner = Some(owner);
        self.command_palette.request_focus = true;
        self.command_palette.reset_scroll = true;
        self.command_palette.selected = 0;
        ctx.send_viewport_cmd_to(owner, egui::ViewportCommand::Focus);
    }

    pub(crate) fn close_command_palette_for(&mut self, viewport: egui::ViewportId) {
        if self.command_palette.owner == Some(viewport) {
            self.command_palette.open = false;
            self.command_palette.owner = None;
        }
    }

    pub(crate) fn show_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette.open || self.command_palette.owner != Some(ctx.viewport_id()) {
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.command_palette.open = false;
            self.command_palette.owner = None;
            return;
        }

        let query = self.command_palette.query.trim().to_lowercase();
        let filtered = self
            .command_specs()
            .into_iter()
            .filter(|command| command_matches(command, &query))
            .collect::<Vec<_>>();
        let mut keyboard_navigation = false;
        if filtered.is_empty() {
            self.command_palette.selected = 0;
        } else {
            self.command_palette.selected = self.command_palette.selected.min(filtered.len() - 1);
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown))
            {
                self.command_palette.selected =
                    (self.command_palette.selected + 1) % filtered.len();
                keyboard_navigation = true;
            }
            if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                self.command_palette.selected =
                    (self.command_palette.selected + filtered.len() - 1) % filtered.len();
                keyboard_navigation = true;
            }
        }

        let mut run = None;
        let mut close = false;
        let theme = self.theme();
        let available = ctx.content_rect();
        let width = (available.width() - 24.0).clamp(300.0, 560.0);
        let top_offset = 56.0_f32.min((available.height() * 0.12).max(16.0));
        let position = egui::pos2(
            available.center().x - width * 0.5,
            available.top() + top_offset,
        );
        let list_height = (available.bottom() - position.y - 72.0).clamp(96.0, 360.0);
        // Modal backdrop: dims the window, blocks the UI underneath and closes
        // the palette on an outside click. Same Foreground order as the
        // palette; the palette is shown after it so it stacks on top.
        egui::Area::new(egui::Id::new("command_palette_backdrop"))
            .order(egui::Order::Foreground)
            .fixed_pos(available.min)
            .show(ctx, |ui| {
                let response = ui.allocate_rect(available, egui::Sense::click());
                ui.painter()
                    .rect_filled(available, 0.0, theme.modal_backdrop);
                if response.clicked() {
                    close = true;
                }
            });
        egui::Area::new(egui::Id::new("command_palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(self.theme().bg_elevated)
                    .stroke(Stroke::new(1.0_f32, self.theme().border_strong))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.horizontal(|ui| {
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.command_palette.query)
                                    .hint_text(t("Search commands"))
                                    .desired_width(width - 76.0),
                            );
                            if self.command_palette.request_focus {
                                response.request_focus();
                                self.command_palette.request_focus = false;
                            }
                            if response.changed() {
                                self.command_palette.selected = 0;
                                self.command_palette.reset_scroll = true;
                            }
                            if ui.button(t("Close")).clicked() {
                                close = true;
                            }
                        });
                        ui.add_space(6.0);
                        let results = egui::ScrollArea::vertical()
                            .id_salt("command_palette_results")
                            .max_height(list_height)
                            .auto_shrink([false, true]);
                        let results = if self.command_palette.reset_scroll {
                            results.vertical_scroll_offset(0.0)
                        } else {
                            results
                        };
                        results.show(ui, |ui| {
                            if filtered.is_empty() {
                                ui.label(
                                    RichText::new(t("No matching commands"))
                                        .color(ui.visuals().weak_text_color()),
                                );
                            }
                            for (index, command) in filtered.iter().enumerate() {
                                let response = ui
                                    .push_id(command.id, |ui| {
                                        ui.add_sized(
                                            egui::vec2(ui.available_width(), 34.0),
                                            egui::Button::selectable(
                                                index == self.command_palette.selected,
                                                "",
                                            )
                                            .frame_when_inactive(true),
                                        )
                                    })
                                    .inner;
                                let selected = index == self.command_palette.selected;
                                let accessible_label = format!(
                                    "{} · {}",
                                    t(command.title_key),
                                    t(command.category_key)
                                );
                                let enabled = ui.is_enabled();
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Button,
                                        enabled,
                                        selected,
                                        accessible_label.clone(),
                                    )
                                });
                                // The selected row is filled with `selection.bg_fill`
                                // (the accent), so its text must switch to `accent_fg`
                                // — the theme foreground is invisible on the accent.
                                let (title_color, secondary_color) = if selected {
                                    (theme.accent_fg, theme.accent_fg.gamma_multiply(0.75))
                                } else {
                                    (theme.fg, theme.fg_muted)
                                };
                                let rect = response.rect.shrink2(egui::vec2(8.0, 3.0));
                                ui.painter().text(
                                    egui::pos2(rect.left(), rect.top() + 7.0),
                                    egui::Align2::LEFT_CENTER,
                                    t(command.title_key),
                                    egui::FontId::proportional(13.0),
                                    title_color,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.left(), rect.bottom() - 5.0),
                                    egui::Align2::LEFT_CENTER,
                                    t(command.category_key),
                                    egui::FontId::proportional(9.5),
                                    secondary_color,
                                );
                                if let Some(shortcut) = self.action_hotkey(command.action) {
                                    ui.painter().text(
                                        rect.right_center(),
                                        egui::Align2::RIGHT_CENTER,
                                        shortcut,
                                        egui::FontId::monospace(10.5),
                                        secondary_color,
                                    );
                                }
                                if response.clicked() {
                                    run = Some(command.action);
                                }
                                if keyboard_navigation && index == self.command_palette.selected {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                            }
                        });
                        self.command_palette.reset_scroll = false;
                    });
            });

        if !filtered.is_empty()
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        {
            run = Some(filtered[self.command_palette.selected].action);
        }
        if close {
            self.command_palette.open = false;
            self.command_palette.owner = None;
        } else if let Some(action) = run {
            self.command_palette.open = false;
            self.command_palette.owner = None;
            self.execute_action(ctx, action);
        }
    }

    fn action_hotkey(&self, action: AppAction) -> Option<String> {
        if !self.global_hotkeys.enabled
            && matches!(
                action,
                AppAction::ToggleCapture | AppAction::ResetSession | AppAction::ToggleHud
            )
        {
            return None;
        }
        match action {
            AppAction::ToggleCapture => self
                .global_hotkeys
                .binding(GlobalHotkeyAction::ToggleCapture)
                .map(HotkeyBinding::label),
            AppAction::ResetSession => self
                .global_hotkeys
                .binding(GlobalHotkeyAction::ResetSession)
                .map(HotkeyBinding::label),
            AppAction::ToggleHud => self
                .global_hotkeys
                .binding(GlobalHotkeyAction::ToggleHud)
                .map(HotkeyBinding::label),
            AppAction::TogglePassthrough => Some(self.passthrough_hotkey.label().to_owned()),
            AppAction::UndoLatest => Some("Ctrl+Z".to_owned()),
            _ => None,
        }
    }
}

fn command(
    id: &'static str,
    title_key: &'static str,
    category_key: &'static str,
    keywords: &'static [&'static str],
    action: AppAction,
) -> CommandSpec {
    CommandSpec {
        id,
        title_key,
        category_key,
        keywords,
        action,
    }
}

fn command_matches(command: &CommandSpec, query: &str) -> bool {
    query.is_empty()
        || fuzzy_text_matches(command.title_key, query)
        || fuzzy_text_matches(&t(command.title_key), query)
        || fuzzy_text_matches(
            &i18n::t_for(Language::SimplifiedChinese, command.title_key),
            query,
        )
        || command
            .keywords
            .iter()
            .any(|keyword| fuzzy_text_matches(keyword, query))
}

fn fuzzy_text_matches(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    if text.contains(&query) {
        return true;
    }
    let mut text = text.chars();
    query
        .chars()
        .all(|needle| text.by_ref().any(|candidate| candidate == needle))
}

fn console_command_metadata(tab: ConsoleTab) -> (&'static str, &'static str) {
    match tab {
        ConsoleTab::Settings => ("console.settings", "Open Settings"),
        ConsoleTab::Timeline => ("console.timeline", "Open Timeline"),
        ConsoleTab::Skills => ("console.skills", "Open Skills"),
        ConsoleTab::EmptyCurtain => ("console.loadout", "Open Console Loadout"),
        ConsoleTab::History => ("console.history", "Open History"),
        ConsoleTab::Characters => ("console.characters", "Open Character Data"),
        ConsoleTab::EncryptedIni => ("console.ini", "Open Encrypted INI"),
        ConsoleTab::Packets => ("console.packets", "Open Packets"),
        ConsoleTab::Resources => ("console.resources", "Open Resources"),
        ConsoleTab::Diagnostics => ("console.diagnostics", "Open Diagnostics"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique() {
        let mut ids = std::collections::HashSet::new();
        for tab in ConsoleTab::visible_tabs() {
            assert!(ids.insert(console_command_metadata(*tab).0));
        }
        for id in [
            "capture.toggle",
            "session.reset",
            "hud.toggle",
            "passthrough.toggle",
            "capture.pause",
            "window.pin",
            "theme.toggle",
            "motion.toggle",
            "action.undo",
            "window.abyss",
            "window.team_details",
            "import.pcapng",
            "import.capture_json",
            "history.save",
            "team.export",
            "accent.zinc",
            "accent.blue",
            "accent.violet",
            "accent.orange",
            "accent.green",
            "density.compact",
            "density.cozy",
            "density.comfortable",
        ] {
            assert!(ids.insert(id));
        }
    }

    #[test]
    fn command_search_matches_english_text_and_keywords() {
        let spec = command(
            "test",
            "Open Timeline",
            "Console",
            &["review"],
            AppAction::OpenConsole(ConsoleTab::Timeline),
        );
        assert!(command_matches(&spec, "timeline"));
        assert!(command_matches(&spec, "opti"));
        assert!(command_matches(&spec, "review"));
        assert!(command_matches(&spec, "时间轴"));
        assert!(!command_matches(&spec, "unrelated"));
    }
}
