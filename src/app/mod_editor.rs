use super::*;

#[derive(Clone, Debug)]
struct ModEditorTarget {
    region: ModsPluginGameRegion,
    directory: PathBuf,
    installed: bool,
    workspace: ModScriptWorkspace,
}

#[derive(Clone, Debug)]
enum ModEditorTaskError {
    Deployment(ModsPluginDeploymentError),
    Script(ModScriptError),
    WorkerDisconnected,
}

#[derive(Clone, Debug)]
enum ModEditorTaskAction {
    Load,
    Save {
        region: ModsPluginGameRegion,
        directory: PathBuf,
        id: String,
        source: String,
    },
    SetEnabled {
        region: ModsPluginGameRegion,
        directory: PathBuf,
        id: String,
        enabled: bool,
    },
}

struct PendingModEditorTask {
    action: ModEditorTaskAction,
    receiver: Receiver<Result<Vec<ModEditorTarget>, ModEditorTaskError>>,
}

#[derive(Default)]
pub(crate) struct ModEditorState {
    loaded: bool,
    targets: Vec<ModEditorTarget>,
    selected_region: Option<ModsPluginGameRegion>,
    selected_mod_id: Option<String>,
    source: String,
    saved_source: String,
    new_mod_id: String,
    is_new: bool,
    message: String,
    completion: ModCompletionState,
    bridge_events: VecDeque<crate::engine::model::ModScriptEvent>,
    pending: Option<PendingModEditorTask>,
}

impl ModEditorState {
    fn push_bridge_event(&mut self, event: crate::engine::model::ModScriptEvent) {
        const MAX_BRIDGE_EVENTS: usize = 256;
        if self.bridge_events.len() == MAX_BRIDGE_EVENTS {
            self.bridge_events.pop_front();
        }
        self.bridge_events.push_back(event);
    }

    fn dirty(&self) -> bool {
        self.source != self.saved_source
    }

    fn selected_target(&self) -> Option<&ModEditorTarget> {
        let region = self.selected_region?;
        self.targets.iter().find(|target| target.region == region)
    }

    fn selected_document(&self) -> Option<&ModScriptDocument> {
        let id = self.selected_mod_id.as_deref()?;
        self.selected_target()?
            .workspace
            .scripts
            .iter()
            .find(|script| script.id == id)
    }

    fn select_document(&mut self, id: Option<String>) {
        self.selected_mod_id = id;
        let document = self.selected_document().cloned();
        if let Some(document) = document {
            self.source = document.source.clone();
            self.saved_source = document.source;
        } else {
            self.source.clear();
            self.saved_source.clear();
        }
        self.is_new = false;
        self.completion = ModCompletionState::default();
    }

    fn replace_targets(
        &mut self,
        targets: Vec<ModEditorTarget>,
        preferred_region: Option<ModsPluginGameRegion>,
        preferred_mod_id: Option<String>,
    ) {
        let previous_region = self.selected_region;
        let previous_mod_id = self.selected_mod_id.clone();
        self.targets = targets;
        self.loaded = true;
        self.selected_region = preferred_region
            .filter(|region| self.targets.iter().any(|target| target.region == *region))
            .or_else(|| {
                previous_region
                    .filter(|region| self.targets.iter().any(|target| target.region == *region))
            })
            .or_else(|| {
                self.targets
                    .iter()
                    .find(|target| target.installed)
                    .map(|target| target.region)
            })
            .or_else(|| self.targets.first().map(|target| target.region));
        let preferred_mod_id = preferred_mod_id.or(previous_mod_id);
        let selected_mod_id = preferred_mod_id.filter(|id| {
            self.selected_target().is_some_and(|target| {
                target
                    .workspace
                    .scripts
                    .iter()
                    .any(|script| script.id == *id)
            })
        });
        let selected_mod_id = selected_mod_id.or_else(|| {
            self.selected_target()
                .and_then(|target| target.workspace.scripts.first())
                .map(|script| script.id.clone())
        });
        self.select_document(selected_mod_id);
    }
}

#[derive(Clone, Copy)]
struct NteCompletion {
    label: &'static str,
    insert: &'static str,
}

#[derive(Default)]
struct ModCompletionState {
    open: bool,
    selected: usize,
    cursor_char: usize,
    query: String,
}

#[derive(Clone, Copy)]
struct ModEditorPalette {
    chrome: Color32,
    sidebar: Color32,
    editor: Color32,
    border: Color32,
    hover: Color32,
    selected: Color32,
    selected_border: Color32,
    status: Color32,
    text: Color32,
    muted: Color32,
    line_number: Color32,
}

struct NteEditorResponse {
    changed: bool,
    line: usize,
    column: usize,
}

const NTE_COMPLETIONS: &[NteCompletion] = &[
    NteCompletion {
        label: "nte_mod(4)",
        insert: "nte_mod(4)",
    },
    NteCompletion {
        label: "mod(\"id\")",
        insert: "mod(\"mod-id\")",
    },
    NteCompletion {
        label: "requires(\"viewport.tick\")",
        insert: "requires(\"viewport.tick\")",
    },
    NteCompletion {
        label: "requires(\"game.session\")",
        insert: "requires(\"game.session\")",
    },
    NteCompletion {
        label: "requires(\"memory.read\")",
        insert: "requires(\"memory.read\")",
    },
    NteCompletion {
        label: "requires(\"sdk.read\")",
        insert: "requires(\"sdk.read\")",
    },
    NteCompletion {
        label: "requires(\"ipc\")",
        insert: "requires(\"ipc\")",
    },
    NteCompletion {
        label: "requires(\"equipment\")",
        insert: "requires(\"equipment\")",
    },
    NteCompletion {
        label: "requires(\"combat-clock\")",
        insert: "requires(\"combat-clock\")",
    },
    NteCompletion {
        label: "requires(\"log\")",
        insert: "requires(\"log\")",
    },
    NteCompletion {
        label: "def on_viewport_tick(event):",
        insert: "def on_viewport_tick(event):",
    },
    NteCompletion {
        label: "event.viewport",
        insert: "event.viewport",
    },
    NteCompletion {
        label: "game.viewport",
        insert: "game.viewport",
    },
    NteCompletion {
        label: "game.instance",
        insert: "game.instance",
    },
    NteCompletion {
        label: "game.local_player",
        insert: "game.local_player",
    },
    NteCompletion {
        label: "game.player_controller",
        insert: "game.player_controller",
    },
    NteCompletion {
        label: "game.player_state",
        insert: "game.player_state",
    },
    NteCompletion {
        label: "game.player_character",
        insert: "game.player_character",
    },
    NteCompletion {
        label: "memory.read_ptr(base, offset)",
        insert: "memory.read_ptr(",
    },
    NteCompletion {
        label: "memory.read_u8(base, offset)",
        insert: "memory.read_u8(",
    },
    NteCompletion {
        label: "memory.read_u16(base, offset)",
        insert: "memory.read_u16(",
    },
    NteCompletion {
        label: "memory.read_u32(base, offset)",
        insert: "memory.read_u32(",
    },
    NteCompletion {
        label: "memory.read_u64(base, offset)",
        insert: "memory.read_u64(",
    },
    NteCompletion {
        label: "memory.read_i32(base, offset)",
        insert: "memory.read_i32(",
    },
    NteCompletion {
        label: "memory.tarray_first(base, offset)",
        insert: "memory.tarray_first(",
    },
    NteCompletion {
        label: "memory.tarray_count(base, offset)",
        insert: "memory.tarray_count(",
    },
    NteCompletion {
        label: "memory.is_readable(pointer, size)",
        insert: "memory.is_readable(",
    },
    NteCompletion {
        label: "sdk.player_character(controller)",
        insert: "sdk.player_character(",
    },
    NteCompletion {
        label: "sdk.player_state(controller)",
        insert: "sdk.player_state(",
    },
    NteCompletion {
        label: "sdk.game_paused(controller)",
        insert: "sdk.game_paused(",
    },
    NteCompletion {
        label: "sdk.attack_target(character)",
        insert: "sdk.attack_target(",
    },
    NteCompletion {
        label: "sdk.current_weapon(character)",
        insert: "sdk.current_weapon(",
    },
    NteCompletion {
        label: "sdk.character_level(character)",
        insert: "sdk.character_level(",
    },
    NteCompletion {
        label: "sdk.character_hp_milli(character)",
        insert: "sdk.character_hp_milli(",
    },
    NteCompletion {
        label: "sdk.character_hp_max_milli(character, fixed)",
        insert: "sdk.character_hp_max_milli(",
    },
    NteCompletion {
        label: "sdk.character_is_alive(character)",
        insert: "sdk.character_is_alive(",
    },
    NteCompletion {
        label: "sdk.character_is_dead(character)",
        insert: "sdk.character_is_dead(",
    },
    NteCompletion {
        label: "sdk.character_is_controlled(character)",
        insert: "sdk.character_is_controlled(",
    },
    NteCompletion {
        label: "sdk.character_slomo_milli(character)",
        insert: "sdk.character_slomo_milli(",
    },
    NteCompletion {
        label: "equipment.cache_missing()",
        insert: "equipment.cache_missing()",
    },
    NteCompletion {
        label: "equipment.cache_ready(player_state)",
        insert: "equipment.cache_ready(",
    },
    NteCompletion {
        label: "equipment.prepare(player_state)",
        insert: "equipment.prepare(",
    },
    NteCompletion {
        label: "combat_clock.pause_mask(controller)",
        insert: "combat_clock.pause_mask(",
    },
    NteCompletion {
        label: "combat_clock.state_flags(controller)",
        insert: "combat_clock.state_flags(",
    },
    NteCompletion {
        label: "combat_clock.forward(pause_mask, state_flags)",
        insert: "combat_clock.forward(",
    },
    NteCompletion {
        label: "ipc.bind(player_state, controller)",
        insert: "ipc.bind(",
    },
    NteCompletion {
        label: "ipc.emit(\"event\", value...)",
        insert: "ipc.emit(\"event.name\", ",
    },
    NteCompletion {
        label: "ipc.emit(\"pre.event\", value...)",
        insert: "ipc.emit(\"pre.event.name\", ",
    },
    NteCompletion {
        label: "ipc.emit(\"post.event\", value...)",
        insert: "ipc.emit(\"post.event.name\", ",
    },
    NteCompletion {
        label: "time.now_ms()",
        insert: "time.now_ms()",
    },
    NteCompletion {
        label: "log.info(\"message\")",
        insert: "log.info(\"message\")",
    },
    NteCompletion {
        label: "for name in range(COUNT):",
        insert: "for index in range(1):",
    },
];

impl DpsApp {
    pub(crate) fn push_mod_script_event(&mut self, event: crate::engine::model::ModScriptEvent) {
        self.mod_editor.push_bridge_event(event);
    }

    pub(crate) fn mod_editor_contents(&mut self, ui: &mut egui::Ui) {
        self.drain_mod_editor_task(ui.ctx());
        if !self.mod_editor.loaded && self.mod_editor.pending.is_none() {
            self.start_mod_editor_task(ui.ctx(), ModEditorTaskAction::Load);
        }

        let mut pending = self.mod_editor.pending.is_some();
        let dirty = self.mod_editor.dirty();
        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        let regions: Vec<_> = self
            .mod_editor
            .targets
            .iter()
            .map(|target| target.region)
            .collect();
        let mut selected_region = self.mod_editor.selected_region;
        let deployment_changed = egui::Frame::new()
            .fill(palette.chrome)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Mod Studio")).size(18.0).strong());
                    ui.separator();
                    ui.label(
                        RichText::new(t(
                            "Manage installed NTE Script v4 Mods and edit them with syntax assistance.",
                        ))
                        .color(palette.muted),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Game client")).color(palette.muted));
                    ui.add_enabled_ui(!pending && !dirty, |ui| {
                        egui::ComboBox::from_id_salt("mod_editor_region")
                            .selected_text(
                                selected_region
                                    .map(plugin_game_region_label)
                                    .map(t)
                                    .unwrap_or_else(|| t("Not detected")),
                            )
                            .show_ui(ui, |ui| {
                                for region in &regions {
                                    ui.selectable_value(
                                        &mut selected_region,
                                        Some(*region),
                                        t(plugin_game_region_label(*region)),
                                    );
                                }
                            });
                    });
                    if ui
                        .add_enabled(
                            !pending && !dirty,
                            egui::Button::new(t("Refresh")).small(),
                        )
                        .on_hover_text(t("Refresh Mod workspace"))
                        .clicked()
                    {
                        self.start_mod_editor_task(ui.ctx(), ModEditorTaskAction::Load);
                    }
                    let mod_directory = self
                        .mod_editor
                        .selected_target()
                        .map(|target| target.directory.join("nte-mods"));
                    if ui
                        .add_enabled(
                            mod_directory.is_some(),
                            egui::Button::new(t("Open Mod folder")).small(),
                        )
                        .clicked()
                        && let Some(directory) = mod_directory
                    {
                        match open_directory(&directory) {
                            Ok(()) => self.notifications.status = t("Mod folder opened"),
                            Err(error) => self.set_last_error_in(
                                ui.ctx(),
                                tf("Failed to open the Mod folder: {}", &[&error]),
                                None,
                            ),
                        }
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(t(
                            "The running game applies saved Mod changes automatically.",
                        ))
                        .small()
                        .color(palette.muted),
                    );
                });
                ui.add_space(4.0);
                self.mod_loader_management_contents(
                    ui,
                    self.mod_editor.selected_region,
                    !pending && !dirty,
                )
            })
            .inner;
        if selected_region != self.mod_editor.selected_region {
            self.mod_editor.selected_region = selected_region;
            let selected = self
                .mod_editor
                .selected_target()
                .and_then(|target| target.workspace.scripts.first())
                .map(|script| script.id.clone());
            self.mod_editor.select_document(selected);
        }
        if deployment_changed && !dirty && self.mod_editor.pending.is_none() {
            self.start_mod_editor_task(ui.ctx(), ModEditorTaskAction::Load);
            pending = true;
        }

        if !self.mod_editor.message.is_empty() {
            ui.label(
                RichText::new(&self.mod_editor.message)
                    .color(semantic_warning(self.preferences.dark_mode)),
            );
        }
        if pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(t("Updating the Mod workspace..."));
            });
        }
        ui.add_space(6.0);

        if self.mod_editor.selected_target().is_none() {
            ui.centered_and_justified(|ui| {
                ui.label(t("No supported game client was detected."));
            });
            return;
        }
        egui::Panel::left("mod_editor_script_list")
            .resizable(true)
            .default_size(230.0)
            .size_range(190.0..=320.0)
            .frame(
                egui::Frame::new()
                    .fill(palette.sidebar)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .inner_margin(egui::Margin::same(0)),
            )
            .show_inside(ui, |ui| {
                self.mod_editor_sidebar(ui, pending);
            });
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            self.mod_editor_source_panel(ui, pending);
        });
    }

    fn mod_editor_sidebar(&mut self, ui: &mut egui::Ui, pending: bool) {
        use egui_material_icons::icons::{ICON_CODE, ICON_EXPAND_MORE, ICON_FOLDER_OPEN};

        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        egui::Frame::new()
            .fill(palette.chrome)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(t("Explorer").to_uppercase())
                        .size(11.0)
                        .strong()
                        .color(palette.text),
                );
            });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            mod_editor_icon(ui, ICON_EXPAND_MORE, 16.0, palette.muted);
            mod_editor_icon(ui, ICON_FOLDER_OPEN, 16.0, palette.muted);
            ui.label(
                RichText::new(t("NTE Mods").to_uppercase())
                    .size(12.0)
                    .strong()
                    .color(palette.text),
            );
        });
        ui.add_space(2.0);
        let dirty = self.mod_editor.dirty();
        let scripts = self
            .mod_editor
            .selected_target()
            .map(|target| target.workspace.scripts.clone())
            .unwrap_or_default();
        let mut select_id = None;
        let mut toggle = None;
        egui::ScrollArea::vertical()
            .id_salt("mod_editor_script_list_scroll")
            .max_height((ui.available_height() - 132.0).max(120.0))
            .show(ui, |ui| {
                for script in &scripts {
                    let selected =
                        self.mod_editor.selected_mod_id.as_deref() == Some(script.id.as_str());
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, 30.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let rect = ui.max_rect();
                            if selected {
                                ui.painter().rect_filled(rect, 0.0, palette.selected);
                                ui.painter().line_segment(
                                    [rect.left_top(), rect.left_bottom()],
                                    Stroke::new(2.0_f32, palette.selected_border),
                                );
                            } else if ui.rect_contains_pointer(rect) {
                                ui.painter().rect_filled(rect, 0.0, palette.hover);
                            }
                            ui.add_space(8.0);
                            let mut enabled = script.enabled;
                            if ui
                                .add_enabled(
                                    !pending && !dirty,
                                    egui::Checkbox::without_text(&mut enabled),
                                )
                                .on_hover_text(if enabled {
                                    t("Disable this Mod")
                                } else {
                                    t("Enable this Mod")
                                })
                                .changed()
                            {
                                toggle = Some((script.id.clone(), enabled));
                            }
                            mod_editor_icon(
                                ui,
                                ICON_CODE,
                                15.0,
                                if selected {
                                    palette.selected_border
                                } else {
                                    palette.muted
                                },
                            );
                            if ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("{}.nte", script.id))
                                            .monospace()
                                            .color(palette.text),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .clicked()
                            {
                                select_id = Some(script.id.clone());
                            }
                        },
                    );
                }
            });
        if let Some(id) = select_id {
            if self.mod_editor.dirty() {
                self.mod_editor.message =
                    t("Save or revert the current changes before switching Mods.");
            } else {
                self.mod_editor.select_document(Some(id));
                self.mod_editor.message.clear();
            }
        }
        if let Some((id, enabled)) = toggle {
            let target = self
                .mod_editor
                .selected_target()
                .expect("Mod workspace target remains selected");
            self.start_mod_editor_task(
                ui.ctx(),
                ModEditorTaskAction::SetEnabled {
                    region: target.region,
                    directory: target.directory.clone(),
                    id,
                    enabled,
                },
            );
        }

        egui::Frame::new()
            .fill(palette.chrome)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(8, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new(t("New Mod ID").to_uppercase())
                        .size(10.0)
                        .strong()
                        .color(palette.muted),
                );
                ui.add_enabled(
                    !pending && !self.mod_editor.dirty(),
                    egui::TextEdit::singleline(&mut self.mod_editor.new_mod_id)
                        .hint_text("character-telemetry")
                        .desired_width(f32::INFINITY),
                );
                let create_enabled = !pending
                    && !self.mod_editor.dirty()
                    && !self.mod_editor.new_mod_id.trim().is_empty();
                if ui
                    .add_enabled(
                        create_enabled,
                        egui::Button::new(format!("+ {}", t("Create Mod"))).small(),
                    )
                    .clicked()
                {
                    let id = self.mod_editor.new_mod_id.trim().to_owned();
                    if scripts.iter().any(|script| script.id == id) {
                        self.mod_editor.select_document(Some(id));
                        self.mod_editor.message = t("A Mod with this ID already exists.");
                    } else {
                        match new_mod_script_template(&id) {
                            Ok(source) => {
                                self.mod_editor.selected_mod_id = Some(id);
                                self.mod_editor.source = source;
                                self.mod_editor.saved_source.clear();
                                self.mod_editor.is_new = true;
                                self.mod_editor.new_mod_id.clear();
                                self.mod_editor.message.clear();
                                self.mod_editor.completion = ModCompletionState::default();
                            }
                            Err(error) => {
                                self.mod_editor.message = mod_script_error_text(&error);
                            }
                        }
                    }
                }
            });
    }

    fn mod_editor_source_panel(&mut self, ui: &mut egui::Ui, pending: bool) {
        use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_CODE, ICON_ERROR};

        let Some(id) = self.mod_editor.selected_mod_id.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label(t("Select a Mod or create a new one."));
            });
            return;
        };
        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        let validation = validate_mod_source(&id, &self.mod_editor.source);
        egui::Frame::new()
            .fill(palette.chrome)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    mod_editor_icon(ui, ICON_CODE, 16.0, palette.selected_border);
                    ui.label(
                        RichText::new(format!("{id}.nte"))
                            .monospace()
                            .strong()
                            .color(palette.text),
                    );
                    if self.mod_editor.dirty() {
                        ui.label(RichText::new("●").size(8.0).color(palette.muted))
                            .on_hover_text(t("Unsaved changes"));
                    }
                    if self.mod_editor.is_new {
                        ui.label(RichText::new(t("New")).small().color(palette.status));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !pending && self.mod_editor.dirty(),
                                egui::Button::new(t("Revert changes")).small(),
                            )
                            .clicked()
                        {
                            if self.mod_editor.is_new {
                                let selected = self
                                    .mod_editor
                                    .selected_target()
                                    .and_then(|target| target.workspace.scripts.first())
                                    .map(|script| script.id.clone());
                                self.mod_editor.select_document(selected);
                            } else {
                                self.mod_editor
                                    .source
                                    .clone_from(&self.mod_editor.saved_source);
                                self.mod_editor.completion = ModCompletionState::default();
                            }
                            self.mod_editor.message.clear();
                        }
                        let save_enabled =
                            !pending && self.mod_editor.dirty() && validation.is_ok();
                        if ui
                            .add_enabled(save_enabled, egui::Button::new(t("Save Mod")).small())
                            .clicked()
                        {
                            let target = self
                                .mod_editor
                                .selected_target()
                                .expect("Mod workspace target remains selected");
                            self.start_mod_editor_task(
                                ui.ctx(),
                                ModEditorTaskAction::Save {
                                    region: target.region,
                                    directory: target.directory.clone(),
                                    id: id.clone(),
                                    source: self.mod_editor.source.clone(),
                                },
                            );
                        }
                    });
                });
            });
        egui::Frame::new()
            .fill(palette.editor)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("nte-mods").monospace().color(palette.muted));
                    ui.label(RichText::new("›").color(palette.muted));
                    ui.label(
                        RichText::new(format!("{id}.nte"))
                            .monospace()
                            .color(palette.text),
                    );
                    for capability in declared_capabilities(&self.mod_editor.source) {
                        ui.label(
                            RichText::new(capability)
                                .monospace()
                                .small()
                                .color(palette.selected_border),
                        );
                    }
                });
            });

        let editor_response = ui
            .add_enabled_ui(!pending, |ui| {
                let editor_height = (ui.available_height() - 28.0).max(220.0);
                let source = &mut self.mod_editor.source;
                let completion = &mut self.mod_editor.completion;
                nte_script_editor(
                    ui,
                    source,
                    completion,
                    editor_height,
                    self.preferences.dark_mode,
                    self.preferences.accent,
                )
            })
            .inner;
        if editor_response.changed {
            self.mod_editor.message.clear();
        }
        let validation = validate_mod_source(&id, &self.mod_editor.source);
        egui::Frame::new()
            .fill(palette.status)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if validation.is_ok() {
                        mod_editor_icon(ui, ICON_CHECK_CIRCLE, 14.0, Color32::WHITE);
                        ui.label(
                            RichText::new(t("NTE Script v4 structure is valid"))
                                .small()
                                .color(Color32::WHITE),
                        );
                    } else {
                        mod_editor_icon(ui, ICON_ERROR, 14.0, Color32::WHITE);
                        ui.add(
                            egui::Label::new(
                                RichText::new(mod_script_error_text(
                                    validation
                                        .as_ref()
                                        .expect_err("invalid source has a validation error"),
                                ))
                                .small()
                                .color(Color32::WHITE),
                            )
                            .truncate(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("NTE Script v4")
                                .monospace()
                                .small()
                                .color(Color32::WHITE),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(tf(
                                "{} / {} bytes",
                                &[
                                    &self.mod_editor.source.len().to_string(),
                                    &MAX_MOD_SOURCE_BYTES.to_string(),
                                ],
                            ))
                            .monospace()
                            .small()
                            .color(Color32::WHITE),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(tf(
                                "Ln {}, Col {}",
                                &[
                                    &editor_response.line.to_string(),
                                    &editor_response.column.to_string(),
                                ],
                            ))
                            .monospace()
                            .small()
                            .color(Color32::WHITE),
                        );
                    });
                });
            });
    }

    fn start_mod_editor_task(&mut self, ctx: &egui::Context, action: ModEditorTaskAction) {
        if self.mod_editor.pending.is_some() {
            return;
        }
        let (sender, receiver) = bounded(1);
        let repaint = ctx.clone();
        let worker_action = action.clone();
        thread::spawn(move || {
            let operation = match &worker_action {
                ModEditorTaskAction::Load => Ok(()),
                ModEditorTaskAction::Save {
                    directory,
                    id,
                    source,
                    ..
                } => save_mod_script(directory, id, source).map_err(ModEditorTaskError::Script),
                ModEditorTaskAction::SetEnabled {
                    directory,
                    id,
                    enabled,
                    ..
                } => set_mod_enabled(directory, id, *enabled).map_err(ModEditorTaskError::Script),
            };
            let result = operation.and_then(|()| load_mod_editor_targets());
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.mod_editor.pending = Some(PendingModEditorTask { action, receiver });
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    fn drain_mod_editor_task(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.mod_editor.pending.as_ref() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(TryRecvError::Disconnected) => Err(ModEditorTaskError::WorkerDisconnected),
        };
        let pending = self
            .mod_editor
            .pending
            .take()
            .expect("completed Mod editor task is still pending");
        let preferred = match &pending.action {
            ModEditorTaskAction::Load => (None, None),
            ModEditorTaskAction::Save { region, id, .. }
            | ModEditorTaskAction::SetEnabled { region, id, .. } => {
                (Some(*region), Some(id.clone()))
            }
        };
        match result {
            Ok(targets) => {
                self.mod_editor
                    .replace_targets(targets, preferred.0, preferred.1);
                self.mod_editor.message.clear();
                self.clear_last_error();
                self.notifications.status = match pending.action {
                    ModEditorTaskAction::Load => t("Mod workspace refreshed"),
                    ModEditorTaskAction::Save { .. } => t("Mod saved"),
                    ModEditorTaskAction::SetEnabled { enabled: true, .. } => t("Mod enabled"),
                    ModEditorTaskAction::SetEnabled { enabled: false, .. } => t("Mod disabled"),
                };
            }
            Err(error) => {
                let text = mod_editor_task_error_text(&error);
                self.mod_editor.message = text.clone();
                self.set_last_error_in(ctx, text, None);
            }
        }
    }
}

fn load_mod_editor_targets() -> Result<Vec<ModEditorTarget>, ModEditorTaskError> {
    let targets = crate::platform::mods_plugin::mods_plugin_mod_targets()
        .map_err(ModEditorTaskError::Deployment)?;
    targets
        .into_iter()
        .map(
            |ModsPluginModTarget {
                 region,
                 directory,
                 installed,
             }| {
                let workspace = if installed {
                    load_mod_script_workspace(&directory).map_err(ModEditorTaskError::Script)?
                } else {
                    ModScriptWorkspace::default()
                };
                Ok(ModEditorTarget {
                    region,
                    directory,
                    installed,
                    workspace,
                })
            },
        )
        .collect()
}

fn mod_editor_task_error_text(error: &ModEditorTaskError) -> String {
    match error {
        ModEditorTaskError::Deployment(error) => plugin_deployment_error_text(error),
        ModEditorTaskError::Script(error) => mod_script_error_text(error),
        ModEditorTaskError::WorkerDisconnected => t("Mod workspace worker disconnected."),
    }
}

fn mod_script_error_text(error: &ModScriptError) -> String {
    match error {
        ModScriptError::FileSystem(error) => tf("Failed to update Mod files: {}", &[error]),
        ModScriptError::InvalidModSet => t("nte-mods.enabled has an invalid format."),
        ModScriptError::DuplicateModId(id) => {
            tf("nte-mods.enabled contains duplicate Mod ID: {}", &[id])
        }
        ModScriptError::InvalidModId(id) => tf(
            "Invalid Mod ID: {}. Use lowercase letters, digits, dash, underscore, or dot.",
            &[id],
        ),
        ModScriptError::TooManyEnabledMods => t("At most 16 Mods can be enabled."),
        ModScriptError::SourceTooLarge => t("A Mod source file can contain at most 16 KiB."),
        ModScriptError::SourceContainsNul => t("A Mod source file cannot contain NUL bytes."),
        ModScriptError::SourceNotUtf8(id) => tf("Mod source is not valid UTF-8: {}", &[id]),
        ModScriptError::MissingVersionHeader => t("The first statement must be nte_mod(4)."),
        ModScriptError::MissingModDeclaration => {
            t("The second statement must declare mod(\"id\").")
        }
        ModScriptError::MismatchedModDeclaration => {
            t("The mod(\"id\") declaration must match the file name.")
        }
        ModScriptError::MissingViewportTickHandler => {
            t("The script must define on_viewport_tick(event).")
        }
        ModScriptError::ModSourceMissing(id) => tf("The Mod source file is missing: {}", &[id]),
    }
}

fn plugin_game_region_label(region: ModsPluginGameRegion) -> &'static str {
    match region {
        ModsPluginGameRegion::China => "China client",
        ModsPluginGameRegion::Global => "Global client",
    }
}

fn declared_capabilities(source: &str) -> Vec<&str> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix("requires(\"")
                .and_then(|line| line.strip_suffix("\")"))
        })
        .collect()
}

fn nte_script_editor(
    ui: &mut egui::Ui,
    source: &mut String,
    completion: &mut ModCompletionState,
    height: f32,
    dark_mode: bool,
    accent: AccentColor,
) -> NteEditorResponse {
    let editor_id = ui.make_persistent_id("nte_script_source_editor");
    let palette = mod_editor_palette(dark_mode, accent);
    let editor_width = ui.available_width();
    let desired_rows = (height / 19.0).max(12.0) as usize;
    let content_rows = editor_content_row_count(source, desired_rows);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let content_height = content_rows as f32 * row_height + 16.0;
    let mut cursor_char = None;
    let mut focused = false;
    let mut changed = false;
    let mut completion_anchor = None;
    let had_focus = ui.memory(|memory| memory.has_focus(editor_id));
    let prior_suggestions = completion_candidates(
        source,
        completion.cursor_char,
        completion.open && completion.query.is_empty(),
    );
    let mut accept_completion = None;
    let mut explicit_completion = false;
    let mut dismiss_completion = false;
    let mut scroll_completion = false;
    if had_focus {
        explicit_completion = ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                egui::Key::Space,
            )
        });
        if completion.open && !prior_suggestions.is_empty() {
            let (previous, next, accept, dismiss) = ui.input_mut(|input| {
                (
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
                )
            });
            completion.selected = completion_selection_index(
                completion.selected,
                prior_suggestions.len(),
                previous,
                next,
            );
            scroll_completion = previous || next;
            if accept {
                accept_completion = Some(prior_suggestions[completion.selected]);
            }
            dismiss_completion = dismiss;
        }
    }
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap_width: f32| {
        let job = nte_script_layout_job(ui, buffer.as_str(), dark_mode, accent);
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    egui::Frame::new()
        .fill(palette.editor)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::both()
                .id_salt("nte_script_source_scroll")
                .auto_shrink([false, false])
                .max_height(height)
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal_top(|ui| {
                        let (gutter_rect, _) = ui.allocate_exact_size(
                            egui::vec2(48.0, content_height),
                            egui::Sense::hover(),
                        );
                        let output = egui::TextEdit::multiline(source)
                            .id(editor_id)
                            .code_editor()
                            .desired_width((editor_width - 48.0).max(360.0))
                            .desired_rows(desired_rows)
                            .margin(egui::Margin {
                                left: 10,
                                right: 8,
                                top: 8,
                                bottom: 8,
                            })
                            .frame(egui::Frame::new().fill(palette.editor))
                            .layouter(&mut layouter)
                            .hint_text(t("Write NTE Script v4 code here."))
                            .show(ui);
                        changed = output.response.changed();
                        focused = output.response.has_focus();
                        cursor_char = output.cursor_range.map(|range| range.primary.index);
                        let active_row = output
                            .cursor_range
                            .map(|range| output.galley.layout_from_cursor(range.primary).row);
                        let painter = ui
                            .painter()
                            .with_clip_rect(gutter_rect.intersect(ui.clip_rect()));
                        painter.rect_filled(gutter_rect, 0.0, palette.sidebar);
                        painter.line_segment(
                            [gutter_rect.right_top(), gutter_rect.right_bottom()],
                            Stroke::new(1.0_f32, palette.border),
                        );
                        for (index, row) in output.galley.rows.iter().enumerate() {
                            let y = output.galley_pos.y + row.rect().center().y;
                            if gutter_rect.contains(egui::pos2(gutter_rect.center().x, y)) {
                                painter.text(
                                    egui::pos2(gutter_rect.right() - 8.0, y),
                                    egui::Align2::RIGHT_CENTER,
                                    (index + 1).to_string(),
                                    egui::TextStyle::Monospace.resolve(ui.style()),
                                    if active_row == Some(index) {
                                        palette.text
                                    } else {
                                        palette.line_number
                                    },
                                );
                            }
                        }
                        if let Some(range) = output.cursor_range {
                            let cursor_rect = output
                                .galley
                                .pos_from_cursor(range.primary)
                                .translate(output.galley_pos.to_vec2());
                            completion_anchor = Some(cursor_rect);
                        }
                    });
                });
        });

    let cursor_char = cursor_char.unwrap_or_else(|| source.chars().count());
    let query = completion_prefix(source, cursor_char)
        .map(|(_, prefix)| prefix.to_owned())
        .unwrap_or_default();
    let query_changed = completion.cursor_char != cursor_char || completion.query != query;
    let suggestions =
        completion_candidates(source, cursor_char, explicit_completion || query.is_empty());
    if query_changed {
        completion.selected = 0;
        completion.open = !suggestions.is_empty() && !query.is_empty();
        completion.cursor_char = cursor_char;
        completion.query.clone_from(&query);
    }
    if explicit_completion {
        completion.open = !suggestions.is_empty();
        completion.selected = completion.selected.min(suggestions.len().saturating_sub(1));
    }
    if dismiss_completion {
        completion.open = false;
    }
    let mut selected = accept_completion;
    let mut status_cursor_char = cursor_char;
    if completion.open
        && focused
        && !suggestions.is_empty()
        && let Some(anchor) = completion_anchor
        && let Some(clicked) = completion_popup(
            ui,
            anchor,
            &suggestions,
            &query,
            &mut completion.selected,
            scroll_completion,
            palette,
        )
    {
        selected = Some(clicked);
    }
    if let Some(selected) = selected
        && let Some(new_cursor) = apply_completion(source, cursor_char, selected.insert)
    {
        let cursor = egui::text::CCursor::new(new_cursor);
        let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        egui::TextEdit::store_state(ui.ctx(), editor_id, state);
        ui.memory_mut(|memory| memory.request_focus(editor_id));
        completion.cursor_char = new_cursor;
        completion.query = completion_prefix(source, new_cursor)
            .map(|(_, prefix)| prefix.to_owned())
            .unwrap_or_default();
        completion.open = false;
        status_cursor_char = new_cursor;
        changed = true;
    }
    let cursor_byte = char_to_byte_index(source, status_cursor_char).unwrap_or(source.len());
    let (line, column) = line_column_for_byte(source, cursor_byte);
    NteEditorResponse {
        changed,
        line,
        column,
    }
}

fn completion_candidates(
    source: &str,
    cursor_char: usize,
    include_all_without_prefix: bool,
) -> Vec<NteCompletion> {
    let Some((_, prefix)) = completion_prefix(source, cursor_char) else {
        return if include_all_without_prefix {
            NTE_COMPLETIONS.to_vec()
        } else {
            Vec::new()
        };
    };
    NTE_COMPLETIONS
        .iter()
        .copied()
        .filter(|completion| {
            completion.insert.starts_with(prefix)
                || completion.label.starts_with(prefix)
                || completion.insert.contains(prefix)
        })
        .collect()
}

fn completion_selection_index(
    selected: usize,
    suggestion_count: usize,
    previous: bool,
    next: bool,
) -> usize {
    match (previous, next) {
        (true, false) => (selected + suggestion_count - 1) % suggestion_count,
        (false, true) => (selected + 1) % suggestion_count,
        _ => selected.min(suggestion_count - 1),
    }
}

fn editor_content_row_count(source: &str, minimum_rows: usize) -> usize {
    let source_rows = source.lines().count() + usize::from(source.ends_with('\n'));
    minimum_rows.max(source_rows.max(1))
}

fn completion_popup(
    ui: &mut egui::Ui,
    cursor_rect: egui::Rect,
    suggestions: &[NteCompletion],
    query: &str,
    selected: &mut usize,
    scroll_selected: bool,
    palette: ModEditorPalette,
) -> Option<NteCompletion> {
    let screen = ui.ctx().content_rect();
    let width = 430.0_f32.min((screen.width() - 8.0).max(280.0));
    let row_height = 29.0;
    let visible_rows = suggestions.len().min(8) as f32;
    let footer_height = if query.is_empty() { 29.0 } else { 46.0 };
    let height = visible_rows * row_height + footer_height;
    let x = cursor_rect.left().clamp(
        screen.left() + 4.0,
        (screen.right() - width - 4.0).max(screen.left()),
    );
    let y = if cursor_rect.bottom() + height + 4.0 <= screen.bottom() {
        cursor_rect.bottom() + 3.0
    } else {
        (cursor_rect.top() - height - 3.0).max(screen.top() + 4.0)
    };
    let mut clicked = None;
    egui::Area::new(ui.make_persistent_id("nte_script_completion_popup"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x, y))
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            egui::Frame::new()
                .fill(palette.chrome)
                .stroke(Stroke::new(1.0_f32, palette.selected_border))
                .inner_margin(egui::Margin::same(1))
                .show(ui, |ui| {
                    ui.set_width(width - 2.0);
                    egui::ScrollArea::vertical()
                        .id_salt("nte_script_completion_scroll")
                        .max_height(visible_rows * row_height)
                        .show(ui, |ui| {
                            for (index, suggestion) in suggestions.iter().enumerate() {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_height),
                                    egui::Sense::click(),
                                );
                                if response.hovered() {
                                    *selected = index;
                                }
                                let active = *selected == index;
                                if active {
                                    if scroll_selected {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
                                    ui.painter().rect_filled(rect, 0.0, palette.selected);
                                    ui.painter().line_segment(
                                        [rect.left_top(), rect.left_bottom()],
                                        Stroke::new(2.0_f32, palette.selected_border),
                                    );
                                } else if response.hovered() {
                                    ui.painter().rect_filled(rect, 0.0, palette.hover);
                                }
                                let marker = completion_kind_marker(*suggestion);
                                ui.painter().text(
                                    egui::pos2(rect.left() + 11.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    marker,
                                    egui::FontId::monospace(13.0),
                                    palette.selected_border,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.left() + 37.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    suggestion.label,
                                    egui::FontId::monospace(13.0),
                                    palette.text,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.right() - 10.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    t(completion_kind_label(*suggestion)),
                                    egui::FontId::proportional(11.0),
                                    palette.muted,
                                );
                                if response.clicked() {
                                    clicked = Some(*suggestion);
                                }
                            }
                        });
                    ui.separator();
                    ui.label(
                        RichText::new(t("Use ↑↓ to select · Enter/Tab to insert · Esc to close"))
                            .size(10.0)
                            .color(palette.muted),
                    );
                    if !query.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(query)
                                        .monospace()
                                        .size(10.0)
                                        .color(palette.selected_border),
                                )
                                .truncate(),
                            );
                        });
                    }
                });
        });
    clicked
}

fn completion_kind_label(completion: NteCompletion) -> &'static str {
    if completion.label.starts_with("nte_mod")
        || completion.label.starts_with("mod(")
        || completion.label.starts_with("requires(")
    {
        "Declaration"
    } else if completion.label.starts_with("def ") || completion.label.starts_with("for ") {
        "Snippet"
    } else if completion.label.contains('(') {
        "Function"
    } else {
        "Property"
    }
}

fn completion_kind_marker(completion: NteCompletion) -> &'static str {
    match completion_kind_label(completion) {
        "Declaration" => "D",
        "Snippet" => "{ }",
        "Function" => "ƒ",
        "Property" => "◇",
        _ => unreachable!("every completion kind has a marker"),
    }
}

fn mod_editor_palette(dark_mode: bool, accent: AccentColor) -> ModEditorPalette {
    let theme = theme_tokens(dark_mode, accent);
    if dark_mode {
        ModEditorPalette {
            chrome: Color32::from_rgb(37, 37, 38),
            sidebar: Color32::from_rgb(24, 24, 24),
            editor: Color32::from_rgb(30, 30, 30),
            border: Color32::from_rgb(51, 51, 51),
            hover: Color32::from_rgb(42, 45, 46),
            selected: Color32::from_rgb(55, 55, 61),
            selected_border: theme.accent,
            status: Color32::from_rgb(0, 122, 204),
            text: Color32::from_rgb(212, 212, 212),
            muted: Color32::from_rgb(150, 150, 150),
            line_number: Color32::from_rgb(133, 133, 133),
        }
    } else {
        ModEditorPalette {
            chrome: Color32::from_rgb(243, 243, 243),
            sidebar: Color32::from_rgb(248, 248, 248),
            editor: Color32::from_rgb(255, 255, 255),
            border: Color32::from_rgb(225, 225, 225),
            hover: Color32::from_rgb(232, 232, 232),
            selected: Color32::from_rgb(226, 226, 226),
            selected_border: theme.accent,
            status: Color32::from_rgb(0, 122, 204),
            text: Color32::from_rgb(51, 51, 51),
            muted: Color32::from_rgb(105, 105, 105),
            line_number: Color32::from_rgb(43, 145, 175),
        }
    }
}

fn mod_editor_icon(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    size: f32,
    color: Color32,
) -> egui::Response {
    ui.label(
        RichText::new(icon.codepoint)
            .font(egui::FontId::new(size, icon.font_family()))
            .color(color),
    )
}

fn completion_prefix(source: &str, cursor_char: usize) -> Option<(std::ops::Range<usize>, &str)> {
    let cursor_byte = char_to_byte_index(source, cursor_char)?;
    let start = source[..cursor_byte]
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!character.is_ascii_alphanumeric() && !matches!(character, '_' | '.' | '"'))
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let prefix = &source[start..cursor_byte];
    (!prefix.is_empty()).then_some((start..cursor_byte, prefix))
}

fn apply_completion(source: &mut String, cursor_char: usize, insert: &str) -> Option<usize> {
    let (range, _) = completion_prefix(source, cursor_char)?;
    source.replace_range(range.clone(), insert);
    Some(source[..range.start + insert.len()].chars().count())
}

fn char_to_byte_index(text: &str, char_index: usize) -> Option<usize> {
    if char_index == text.chars().count() {
        Some(text.len())
    } else {
        text.char_indices().nth(char_index).map(|(index, _)| index)
    }
}

fn nte_script_layout_job(
    ui: &egui::Ui,
    source: &str,
    dark_mode: bool,
    accent: AccentColor,
) -> egui::text::LayoutJob {
    let palette = mod_editor_palette(dark_mode, accent);
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let format = |color| egui::text::TextFormat {
        font_id: font_id.clone(),
        color,
        ..Default::default()
    };
    let base = format(palette.text);
    let (keyword, function, namespace, variable, string, number, comment) = if dark_mode {
        (
            format(Color32::from_rgb(197, 134, 192)),
            format(Color32::from_rgb(220, 220, 170)),
            format(Color32::from_rgb(78, 201, 176)),
            format(Color32::from_rgb(156, 220, 254)),
            format(Color32::from_rgb(206, 145, 120)),
            format(Color32::from_rgb(181, 206, 168)),
            format(Color32::from_rgb(106, 153, 85)),
        )
    } else {
        (
            format(Color32::from_rgb(175, 0, 219)),
            format(Color32::from_rgb(121, 94, 38)),
            format(Color32::from_rgb(38, 127, 153)),
            format(Color32::from_rgb(0, 16, 128)),
            format(Color32::from_rgb(163, 21, 21)),
            format(Color32::from_rgb(9, 134, 88)),
            format(Color32::from_rgb(0, 128, 0)),
        )
    };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;

    let mut index = 0;
    let mut expect_function_name = false;
    while index < source.len() {
        let character = source[index..]
            .chars()
            .next()
            .expect("index remains on a character boundary");
        let end = if character == '#' {
            source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset)
        } else if character == '"' {
            let tail = index + character.len_utf8();
            source[tail..]
                .find('"')
                .map_or(source.len(), |offset| tail + offset + 1)
        } else if character.is_ascii_digit() {
            consume_ascii_token(source, index, |value| {
                value.is_ascii_hexdigit() || matches!(value, 'x' | 'X')
            })
        } else if character.is_ascii_alphabetic() || character == '_' {
            consume_ascii_token(source, index, |value| {
                value.is_ascii_alphanumeric() || value == '_'
            })
        } else {
            index + character.len_utf8()
        };
        let token = &source[index..end];
        let token_format = if character == '#' {
            comment.clone()
        } else if character == '"' {
            string.clone()
        } else if character.is_ascii_digit() {
            number.clone()
        } else if character.is_ascii_alphabetic() || character == '_' {
            let followed_by_call =
                source[end..].chars().find(|value| !value.is_whitespace()) == Some('(');
            let kind = classify_nte_identifier(token, expect_function_name, followed_by_call);
            expect_function_name = token == "def";
            match kind {
                NteIdentifierKind::Keyword => keyword.clone(),
                NteIdentifierKind::Function => function.clone(),
                NteIdentifierKind::Namespace => namespace.clone(),
                NteIdentifierKind::Variable => variable.clone(),
            }
        } else {
            base.clone()
        };
        job.append(token, 0.0, token_format);
        index = end;
    }
    if source.is_empty() {
        job.append("", 0.0, base);
    }
    job
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteIdentifierKind {
    Keyword,
    Function,
    Namespace,
    Variable,
}

fn classify_nte_identifier(
    token: &str,
    function_definition_name: bool,
    followed_by_call: bool,
) -> NteIdentifierKind {
    if matches!(
        token,
        "def"
            | "if"
            | "elif"
            | "else"
            | "for"
            | "in"
            | "and"
            | "or"
            | "not"
            | "None"
            | "True"
            | "False"
    ) {
        NteIdentifierKind::Keyword
    } else if function_definition_name || followed_by_call {
        NteIdentifierKind::Function
    } else if matches!(
        token,
        "game"
            | "memory"
            | "sdk"
            | "ipc"
            | "equipment"
            | "combat_clock"
            | "time"
            | "log"
            | "event"
            | "state"
            | "range"
    ) {
        NteIdentifierKind::Namespace
    } else {
        NteIdentifierKind::Variable
    }
}

fn consume_ascii_token(source: &str, start: usize, accepted: impl Fn(char) -> bool) -> usize {
    source[start..]
        .char_indices()
        .find_map(|(offset, character)| (!accepted(character)).then_some(start + offset))
        .unwrap_or(source.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_replaces_only_the_token_before_the_cursor() {
        let mut source = "value = sdk.char".to_owned();
        let cursor = source.chars().count();

        let next = apply_completion(&mut source, cursor, "sdk.character_hp_milli(").unwrap();

        assert_eq!(source, "value = sdk.character_hp_milli(");
        assert_eq!(next, source.chars().count());
    }

    #[test]
    fn completion_filters_host_apis_by_dotted_prefix() {
        let source = "    hp = sdk.character_hp";
        let suggestions = completion_candidates(source, source.chars().count(), false);

        assert!(
            suggestions
                .iter()
                .any(|completion| { completion.insert == "sdk.character_hp_milli(" })
        );
        assert!(suggestions.iter().all(|completion| {
            completion.insert.starts_with("sdk.character_hp")
                || completion.label.starts_with("sdk.character_hp")
        }));
    }

    #[test]
    fn completion_exposes_stable_game_session_builtins() {
        let source = "    controller = game.player_";
        let suggestions = completion_candidates(source, source.chars().count(), false);

        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "game.player_controller")
        );
        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "game.player_state")
        );
        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "game.player_character")
        );
    }

    #[test]
    fn completion_navigation_wraps_in_both_directions() {
        assert_eq!(completion_selection_index(0, 4, true, false), 3);
        assert_eq!(completion_selection_index(3, 4, false, true), 0);
    }

    #[test]
    fn script_bridge_event_history_keeps_the_newest_256_records() {
        let mut editor = ModEditorState::default();
        for sequence in 1..=257 {
            editor.push_bridge_event(crate::engine::model::ModScriptEvent::from_bridge(
                sequence,
                sequence,
                "example".to_owned(),
                "post.value".to_owned(),
                vec![sequence],
            ));
        }

        assert_eq!(editor.bridge_events.len(), 256);
        assert_eq!(editor.bridge_events.front().unwrap().sequence, 2);
        assert_eq!(editor.bridge_events.back().unwrap().sequence, 257);
    }

    #[test]
    fn explicit_completion_lists_entries_without_a_prefix() {
        assert!(completion_candidates("", 0, false).is_empty());
        assert_eq!(
            completion_candidates("", 0, true).len(),
            NTE_COMPLETIONS.len()
        );
    }

    #[test]
    fn editor_gutter_reserves_rows_for_empty_and_trailing_lines() {
        assert_eq!(editor_content_row_count("", 12), 12);
        assert_eq!(editor_content_row_count("one\ntwo\n", 1), 3);
        assert_eq!(editor_content_row_count("one\ntwo", 1), 2);
    }

    #[test]
    fn syntax_classification_distinguishes_language_roles() {
        assert_eq!(
            classify_nte_identifier("if", false, false),
            NteIdentifierKind::Keyword
        );
        assert_eq!(
            classify_nte_identifier("on_viewport_tick", true, false),
            NteIdentifierKind::Function
        );
        assert_eq!(
            classify_nte_identifier("read_ptr", false, true),
            NteIdentifierKind::Function
        );
        assert_eq!(
            classify_nte_identifier("memory", false, false),
            NteIdentifierKind::Namespace
        );
        assert_eq!(
            classify_nte_identifier("game", false, false),
            NteIdentifierKind::Namespace
        );
        assert_eq!(
            classify_nte_identifier("player_controller", false, false),
            NteIdentifierKind::Variable
        );
    }

    #[test]
    fn declared_capabilities_follow_requires_lines() {
        assert_eq!(
            declared_capabilities(
                "nte_mod(4)\nrequires(\"viewport.tick\")\nrequires(\"sdk.read\")\n"
            ),
            vec!["viewport.tick", "sdk.read"]
        );
    }
}
