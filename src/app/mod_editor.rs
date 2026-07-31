use super::*;

use crate::core::mod_sdk::{ModSdkSymbol, ModSdkSymbolKind, mod_sdk_symbols};

#[derive(Clone, Debug)]
struct ModEditorTarget {
    region: ModsPluginGameRegion,
    directory: PathBuf,
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

struct PendingModConsolePoll {
    receiver: Receiver<Result<ModConsolePollResult, String>>,
}

struct ModConsolePollResult {
    logs: Vec<ModLogSnapshot>,
    events: Vec<ModEventSnapshot>,
}

#[derive(Clone, Copy)]
enum ModConsoleEntryLevel {
    Editor,
    Event,
    Runtime(ModLogLevel),
}

struct ModConsoleEntry {
    time: String,
    mod_id: String,
    level: ModConsoleEntryLevel,
    message: String,
}

struct ModConsoleState {
    collapsed: bool,
    connected: bool,
    entries: VecDeque<ModConsoleEntry>,
    last_runtime_sequence: u64,
    last_event_sequence: u64,
    pending: Option<PendingModConsolePoll>,
    next_poll_at: Instant,
}

impl Default for ModConsoleState {
    fn default() -> Self {
        Self {
            collapsed: false,
            connected: false,
            entries: VecDeque::new(),
            last_runtime_sequence: 0,
            last_event_sequence: 0,
            pending: None,
            next_poll_at: Instant::now(),
        }
    }
}

impl ModConsoleState {
    fn push_editor(&mut self, mod_id: String, message: String) {
        self.push(ModConsoleEntry {
            time: Local::now().format("%H:%M:%S%.3f").to_string(),
            mod_id,
            level: ModConsoleEntryLevel::Editor,
            message,
        });
    }

    fn ingest_runtime(&mut self, result: ModConsolePollResult) {
        let newest_log_sequence = result
            .logs
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0);
        let newest_event_sequence = result
            .events
            .iter()
            .map(|entry| entry.sequence)
            .max()
            .unwrap_or(0);
        if (newest_log_sequence != 0 && newest_log_sequence < self.last_runtime_sequence)
            || (newest_event_sequence != 0 && newest_event_sequence < self.last_event_sequence)
        {
            self.entries.clear();
            self.last_runtime_sequence = 0;
            self.last_event_sequence = 0;
        }
        let mut entries = Vec::with_capacity(result.logs.len() + result.events.len());
        for log in result.logs {
            if log.sequence <= self.last_runtime_sequence {
                continue;
            }
            self.last_runtime_sequence = log.sequence;
            entries.push((
                log.timestamp_100ns,
                ModConsoleEntry {
                    time: mod_log_time(log.timestamp_100ns),
                    mod_id: log.mod_id,
                    level: ModConsoleEntryLevel::Runtime(log.level),
                    message: mod_runtime_log_text(&log.message),
                },
            ));
        }
        for event in result.events {
            if event.sequence <= self.last_event_sequence {
                continue;
            }
            self.last_event_sequence = event.sequence;
            entries.push((
                event.timestamp_100ns,
                ModConsoleEntry {
                    time: mod_log_time(event.timestamp_100ns),
                    mod_id: event.mod_id.clone(),
                    level: ModConsoleEntryLevel::Event,
                    message: mod_runtime_event_text(&event),
                },
            ));
        }
        entries.sort_by_key(|(timestamp, _)| *timestamp);
        for (_, entry) in entries {
            self.push(entry);
        }
    }

    fn push(&mut self, entry: ModConsoleEntry) {
        const MAX_CONSOLE_ENTRIES: usize = 256;
        if self.entries.len() == MAX_CONSOLE_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn plain_text(&self) -> String {
        self.entries
            .iter()
            .map(|entry| {
                format!(
                    "{} [{}] [{}] {}",
                    entry.time,
                    mod_console_level(entry.level),
                    entry.mod_id,
                    entry.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
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
    pending: Option<PendingModEditorTask>,
    console: ModConsoleState,
}

impl ModEditorState {
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
            .or_else(|| self.targets.first().map(|target| target.region));
        let preferred_mod_id = preferred_mod_id.or(previous_mod_id);
        let selected_mod_id = self.selected_target().and_then(|target| {
            preferred_mod_id
                .filter(|id| {
                    target
                        .workspace
                        .scripts
                        .iter()
                        .any(|script| script.id == *id)
                })
                .or_else(|| {
                    target
                        .workspace
                        .scripts
                        .first()
                        .map(|script| script.id.clone())
                })
        });
        self.select_document(selected_mod_id);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteCompletionKind {
    Declaration,
    Snippet,
    Function,
    Property,
    Variable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NteCompletionItem {
    label: String,
    insert: String,
    kind: NteCompletionKind,
    detail: String,
    documentation_key: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NteSignatureHelp {
    label: String,
    parameters: Vec<String>,
    active_parameter: usize,
    documentation_key: &'static str,
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

impl DpsApp {
    pub(crate) fn mod_editor_contents(&mut self, ui: &mut egui::Ui) {
        self.drain_mod_editor_task(ui.ctx());
        self.drain_mod_console_poll(ui.ctx());
        self.start_mod_console_poll_if_due(ui.ctx());
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
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Mod Code Editor")).size(18.0).strong());
                    ui.label(
                        RichText::new(t("Write, validate and manage NTE C++ Mods in one place."))
                            .small()
                            .color(palette.muted),
                    );
                });
                ui.add_space(2.0);
                let mut deployment_changed = false;
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
                        .add_enabled(!pending && !dirty, egui::Button::new(t("Refresh")).small())
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
                    deployment_changed = self.mod_loader_management_contents(
                        ui,
                        self.mod_editor.selected_region,
                        !pending && !dirty,
                    );
                });
                deployment_changed
            })
            .inner;
        if !self.mod_editor.loaded && self.mod_editor.pending.is_none() {
            self.start_mod_editor_task(ui.ctx(), ModEditorTaskAction::Load);
            pending = true;
        }
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
                ui.label(t("The Mod workspace is being prepared."));
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
        let console_panel = egui::Panel::bottom("mod_editor_runtime_console")
            .resizable(!self.mod_editor.console.collapsed)
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .stroke(Stroke::new(1.0_f32, palette.border))
                    .inner_margin(egui::Margin::same(0)),
            );
        let console_panel = if self.mod_editor.console.collapsed {
            console_panel.exact_size(31.0)
        } else {
            console_panel.default_size(164.0).size_range(96.0..=320.0)
        };
        console_panel.show_inside(ui, |ui| {
            self.mod_editor_console_contents(ui);
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
        let diagnostic_line = mod_source_error_line(&validation);
        let save_shortcut = ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                egui::Key::S,
            )
        });
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
                        let save_clicked = ui
                            .add_enabled(save_enabled, egui::Button::new(t("Save Mod")).small())
                            .on_hover_text(t("Save and hot-update the current Mod · Ctrl+S"))
                            .clicked();
                        if save_clicked || (save_shortcut && save_enabled) {
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
        egui::CollapsingHeader::new(t("Getting started"))
            .id_salt("mod_editor_getting_started")
            .default_open(self.mod_editor.is_new)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("1. Select or create a Mod."));
                    ui.label(t("2. Write NTE C++ and use Ctrl+Space for IntelliSense."));
                    ui.label(t("3. Save, then enable the Mod in Explorer."));
                });
                ui.label(
                    RichText::new(t(
                        "Saved code is hot-updated in the game; compile errors keep the previous working version.",
                    ))
                    .small()
                    .color(palette.muted),
                );
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

        let remaining_rect = ui.available_rect_before_wrap();
        let status_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
        let status_rect = egui::Rect::from_min_max(
            egui::pos2(
                remaining_rect.left(),
                remaining_rect.bottom() - status_height,
            ),
            remaining_rect.max,
        );
        let editor_rect = egui::Rect::from_min_max(
            remaining_rect.min,
            egui::pos2(
                remaining_rect.right(),
                status_rect.top() - ui.spacing().item_spacing.y,
            ),
        );
        ui.allocate_rect(remaining_rect, egui::Sense::hover());

        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(editor_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        editor_ui.set_clip_rect(editor_rect);
        let editor_response = editor_ui
            .add_enabled_ui(!pending, |ui| {
                let source = &mut self.mod_editor.source;
                let completion = &mut self.mod_editor.completion;
                nte_script_editor(
                    ui,
                    source,
                    completion,
                    editor_rect.height(),
                    self.preferences.dark_mode,
                    self.preferences.accent,
                    diagnostic_line,
                )
            })
            .inner;
        if editor_response.changed {
            self.mod_editor.message.clear();
        }
        let validation = validate_mod_source(&id, &self.mod_editor.source);
        let mut status_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(status_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        status_ui.set_clip_rect(status_rect);
        egui::Frame::new()
            .fill(palette.status)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(&mut status_ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if validation.is_ok() {
                        mod_editor_icon(ui, ICON_CHECK_CIRCLE, 14.0, Color32::WHITE);
                        ui.label(
                            RichText::new(t("Source is valid"))
                                .small()
                                .color(Color32::WHITE),
                        );
                    } else {
                        mod_editor_icon(ui, ICON_ERROR, 14.0, Color32::WHITE);
                        let error = mod_script_error_text(
                            validation
                                .as_ref()
                                .expect_err("invalid source has a validation error"),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(&error).small().color(Color32::WHITE))
                                .truncate(),
                        )
                        .on_hover_text(error);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(t("NTE C++"))
                                .monospace()
                                .small()
                                .color(Color32::WHITE),
                        );
                        ui.separator();
                        ui.label(
                            RichText::new(t("Ctrl+Space"))
                                .monospace()
                                .small()
                                .color(Color32::WHITE),
                        )
                        .on_hover_text(t("Open IntelliSense suggestions"));
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

    fn mod_editor_console_contents(&mut self, ui: &mut egui::Ui) {
        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        egui::Frame::new()
            .fill(palette.chrome)
            .inner_margin(egui::Margin::symmetric(9, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t("Runtime Console").to_uppercase())
                            .size(11.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(if self.mod_editor.console.connected {
                            "●"
                        } else {
                            "○"
                        })
                        .color(if self.mod_editor.console.connected {
                            semantic_success(self.preferences.dark_mode)
                        } else {
                            palette.muted
                        }),
                    )
                    .on_hover_text(if self.mod_editor.console.connected {
                        t("Hot reload connected")
                    } else {
                        t("Waiting for the game Mod loader")
                    });
                    ui.label(
                        RichText::new(if self.mod_editor.console.connected {
                            t("Hot reload connected")
                        } else {
                            t("Waiting for the game Mod loader")
                        })
                        .small()
                        .color(palette.muted),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let collapsed = self.mod_editor.console.collapsed;
                        if ui
                            .add(egui::Button::new(if collapsed { "□" } else { "—" }).small())
                            .on_hover_text(if collapsed {
                                t("Restore runtime console")
                            } else {
                                t("Minimize runtime console")
                            })
                            .clicked()
                        {
                            self.mod_editor.console.collapsed = !collapsed;
                        }
                        if ui
                            .add_enabled(
                                !self.mod_editor.console.entries.is_empty(),
                                egui::Button::new(t("Clear")).small(),
                            )
                            .on_hover_text(t("Clear runtime console"))
                            .clicked()
                        {
                            self.mod_editor.console.entries.clear();
                        }
                        if ui
                            .add_enabled(
                                !self.mod_editor.console.entries.is_empty(),
                                egui::Button::new(t("Copy")).small(),
                            )
                            .on_hover_text(t("Copy"))
                            .clicked()
                        {
                            ui.ctx().copy_text(self.mod_editor.console.plain_text());
                        }
                    });
                });
            });
        if self.mod_editor.console.collapsed {
            return;
        }
        ui.separator();
        egui::Frame::new()
            .fill(palette.editor)
            .inner_margin(egui::Margin::symmetric(9, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("mod_editor_runtime_console_scroll")
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.mod_editor.console.entries.is_empty() {
                            ui.label(
                                RichText::new(t("Script logs and emitted IPC events appear here."))
                                    .monospace()
                                    .small()
                                    .color(palette.muted),
                            );
                        }
                        for entry in &self.mod_editor.console.entries {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(&entry.time)
                                        .monospace()
                                        .small()
                                        .color(palette.muted),
                                );
                                let (level, level_color) = match entry.level {
                                    ModConsoleEntryLevel::Editor => {
                                        ("EDITOR", palette.selected_border)
                                    }
                                    ModConsoleEntryLevel::Event => ("EVENT", palette.status),
                                    ModConsoleEntryLevel::Runtime(ModLogLevel::Info) => {
                                        ("INFO", palette.text)
                                    }
                                    ModConsoleEntryLevel::Runtime(ModLogLevel::Warning) => {
                                        ("WARN", semantic_warning(self.preferences.dark_mode))
                                    }
                                    ModConsoleEntryLevel::Runtime(ModLogLevel::Error) => {
                                        ("ERROR", semantic_danger(self.preferences.dark_mode))
                                    }
                                };
                                ui.label(
                                    RichText::new(format!("[{level}]"))
                                        .monospace()
                                        .small()
                                        .color(level_color),
                                );
                                ui.label(
                                    RichText::new(format!("[{}]", entry.mod_id))
                                        .monospace()
                                        .small()
                                        .color(palette.selected_border),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&entry.message)
                                            .monospace()
                                            .small()
                                            .color(palette.text),
                                    )
                                    .selectable(true),
                                );
                            });
                        }
                    });
            });
    }

    fn start_mod_console_poll_if_due(&mut self, ctx: &egui::Context) {
        const MOD_CONSOLE_POLL_INTERVAL: Duration = Duration::from_millis(500);
        let now = Instant::now();
        if self.mod_editor.console.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        }
        if now < self.mod_editor.console.next_poll_at {
            ctx.request_repaint_after(
                self.mod_editor
                    .console
                    .next_poll_at
                    .saturating_duration_since(now),
            );
            return;
        }
        let (sender, receiver) = bounded(1);
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = query_mod_logs().and_then(|logs| {
                query_mod_events().map(|events| ModConsolePollResult { logs, events })
            });
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.mod_editor.console.pending = Some(PendingModConsolePoll { receiver });
        self.mod_editor.console.next_poll_at = now + MOD_CONSOLE_POLL_INTERVAL;
    }

    fn drain_mod_console_poll(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.mod_editor.console.pending.as_ref() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Err(TryRecvError::Disconnected) => Err("Mod console worker disconnected".to_owned()),
        };
        self.mod_editor.console.pending = None;
        match result {
            Ok(result) => {
                self.mod_editor.console.connected = true;
                self.mod_editor.console.ingest_runtime(result);
            }
            Err(_) => {
                self.mod_editor.console.connected = false;
            }
        }
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
                match &pending.action {
                    ModEditorTaskAction::Save { id, .. } => {
                        self.mod_editor.console.push_editor(
                            id.clone(),
                            t("Saved; waiting for the runtime hot update."),
                        );
                    }
                    ModEditorTaskAction::SetEnabled { id, enabled, .. } => {
                        self.mod_editor.console.push_editor(
                            id.clone(),
                            if *enabled {
                                t("Enabled; waiting for the runtime hot update.")
                            } else {
                                t("Disabled; waiting for the runtime hot update.")
                            },
                        );
                    }
                    ModEditorTaskAction::Load => {}
                }
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
                let mod_id = match &pending.action {
                    ModEditorTaskAction::Save { id, .. }
                    | ModEditorTaskAction::SetEnabled { id, .. } => id.clone(),
                    ModEditorTaskAction::Load => "editor".to_owned(),
                };
                self.mod_editor.console.push_editor(mod_id, text.clone());
                self.mod_editor.message = text.clone();
                self.set_last_error_in(ctx, text, None);
            }
        }
    }
}

fn mod_log_time(timestamp_100ns: u64) -> String {
    const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
    const HUNDRED_NS_PER_SECOND: u64 = 10_000_000;
    let unix_100ns = timestamp_100ns.saturating_sub(WINDOWS_TO_UNIX_EPOCH_100NS);
    let seconds = (unix_100ns / HUNDRED_NS_PER_SECOND) as i64;
    let nanoseconds = ((unix_100ns % HUNDRED_NS_PER_SECOND) * 100) as u32;
    DateTime::from_timestamp(seconds, nanoseconds)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|| "--:--:--.---".to_owned())
}

fn mod_runtime_log_text(message: &str) -> String {
    match message {
        "Hot reload applied."
        | "Mod workspace path is invalid; previous version kept."
        | "Enabled Mod set is invalid; previous version kept."
        | "Enabled Mod set is unreadable; previous version kept."
        | "Mod source path is invalid; previous version kept."
        | "Enabled Mod source is missing; previous version kept."
        | "Compilation failed; previous version kept."
        | "Runtime fault trapped; Mod paused until hot reload." => t(message),
        _ => message.to_owned(),
    }
}

fn mod_console_level(level: ModConsoleEntryLevel) -> &'static str {
    match level {
        ModConsoleEntryLevel::Editor => "EDITOR",
        ModConsoleEntryLevel::Event => "EVENT",
        ModConsoleEntryLevel::Runtime(ModLogLevel::Info) => "INFO",
        ModConsoleEntryLevel::Runtime(ModLogLevel::Warning) => "WARN",
        ModConsoleEntryLevel::Runtime(ModLogLevel::Error) => "ERROR",
    }
}

fn mod_runtime_event_text(event: &ModEventSnapshot) -> String {
    match (event.name.as_str(), event.values.as_slice()) {
        ("pre.enemy.identity", [target, config_hash, level]) => {
            return format!(
                "pre.enemy.identity target=0x{target:016X} \
                 config_hash=0x{config_hash:016X} level={level}"
            );
        }
        ("post.enemy.vitals", [target, hp, max_hp]) => {
            return format!("post.enemy.vitals target=0x{target:016X} hp={hp} max_hp={max_hp}");
        }
        ("post.enemy.cleared", [target]) => {
            return format!("post.enemy.cleared target=0x{target:016X}");
        }
        _ => {}
    }
    let mut text = event.name.clone();
    for (index, value) in event.values.iter().enumerate() {
        text.push_str(&format!(" v{index}={value}/0x{value:016X}"));
    }
    text
}

fn load_mod_editor_targets() -> Result<Vec<ModEditorTarget>, ModEditorTaskError> {
    let directory = crate::platform::mods_plugin::prepare_mod_workspace()
        .map_err(ModEditorTaskError::Deployment)?;
    load_mod_editor_targets_from_workspace(directory)
}

fn load_mod_editor_targets_from_workspace(
    directory: PathBuf,
) -> Result<Vec<ModEditorTarget>, ModEditorTaskError> {
    let workspace = load_mod_script_workspace(&directory).map_err(ModEditorTaskError::Script)?;
    Ok(vec![
        ModEditorTarget {
            region: ModsPluginGameRegion::China,
            directory: directory.clone(),
            workspace: workspace.clone(),
        },
        ModEditorTarget {
            region: ModsPluginGameRegion::Global,
            directory,
            workspace,
        },
    ])
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
        ModScriptError::MissingVersionHeader => t("The first statement must be NTE_SCRIPT(5)."),
        ModScriptError::MissingModDeclaration => t("The script must declare NTE_MOD(\"id\")."),
        ModScriptError::MismatchedModDeclaration => {
            t("The NTE_MOD(\"id\") declaration must match the file name.")
        }
        ModScriptError::MissingViewportTickHandler => {
            t("The script must define on_viewport_tick(event).")
        }
        ModScriptError::InvalidSourceLine(line) => tf(
            "The NTE C++ compiler rejected line {}.",
            &[&line.to_string()],
        ),
        ModScriptError::SourceBudgetExceeded => {
            t("The NTE C++ program exceeds the compiler resource budget.")
        }
        ModScriptError::CapabilityMismatch => {
            t("Declared Mod capabilities must exactly match the APIs used by the script.")
        }
        ModScriptError::ModSourceMissing(id) => tf("The Mod source file is missing: {}", &[id]),
    }
}

fn mod_source_error_line(validation: &Result<(), ModScriptError>) -> Option<usize> {
    match validation {
        Err(ModScriptError::InvalidSourceLine(line)) => Some(*line),
        _ => None,
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
                .or_else(|| {
                    line.strip_prefix("NTE_REQUIRES(\"")
                        .and_then(|line| line.strip_suffix("\");"))
                })
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
    diagnostic_line: Option<usize>,
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
                accept_completion = Some(prior_suggestions[completion.selected].clone());
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
                            .hint_text(t("Write NTE C++ code here."))
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
                                let line_number = index + 1;
                                let diagnostic = diagnostic_line == Some(line_number);
                                if diagnostic {
                                    painter.circle_filled(
                                        egui::pos2(gutter_rect.left() + 7.0, y),
                                        3.0,
                                        Color32::from_rgb(244, 71, 71),
                                    );
                                }
                                painter.text(
                                    egui::pos2(gutter_rect.right() - 8.0, y),
                                    egui::Align2::RIGHT_CENTER,
                                    line_number.to_string(),
                                    egui::TextStyle::Monospace.resolve(ui.style()),
                                    if diagnostic {
                                        Color32::from_rgb(244, 71, 71)
                                    } else if active_row == Some(index) {
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
        && let Some(new_cursor) = apply_completion(source, cursor_char, &selected.insert)
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
    let signature = signature_help_at_cursor(source, status_cursor_char);
    if !completion.open
        && focused
        && let Some(anchor) = completion_anchor
        && let Some(signature) = signature.as_ref()
    {
        signature_help_popup(ui, anchor, signature, palette);
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
) -> Vec<NteCompletionItem> {
    let prefix = completion_prefix(source, cursor_char).map(|(_, prefix)| prefix);
    if prefix.is_none() && !include_all_without_prefix {
        return Vec::new();
    }
    let query = prefix.unwrap_or_default().to_ascii_lowercase();
    let mut items: Vec<_> = mod_sdk_symbols()
        .map(static_completion_item)
        .chain(document_completion_items(source))
        .filter(|item| {
            if query.is_empty() {
                include_all_without_prefix
            } else {
                item.insert.to_ascii_lowercase().contains(&query)
                    || item.label.to_ascii_lowercase().contains(&query)
            }
        })
        .collect();
    items.sort_by(|left, right| {
        completion_match_rank(left, &query)
            .cmp(&completion_match_rank(right, &query))
            .then_with(|| left.label.cmp(&right.label))
    });
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.insert.clone()));
    items
}

fn static_completion_item(symbol: ModSdkSymbol) -> NteCompletionItem {
    let kind = match symbol.kind {
        ModSdkSymbolKind::Declaration => NteCompletionKind::Declaration,
        ModSdkSymbolKind::Snippet => NteCompletionKind::Snippet,
        ModSdkSymbolKind::Function => NteCompletionKind::Function,
        ModSdkSymbolKind::Property => NteCompletionKind::Property,
    };
    let detail = match kind {
        NteCompletionKind::Declaration => t("NTE C++ declaration"),
        NteCompletionKind::Snippet => t("Ready-to-edit code snippet"),
        NteCompletionKind::Function => {
            format!(
                "{} -> {}",
                symbol.label,
                symbol
                    .return_type
                    .expect("function schema entry has a return type")
            )
        }
        NteCompletionKind::Property => {
            format!(
                "{}: {}",
                symbol.label,
                symbol
                    .return_type
                    .expect("property schema entry has a return type")
            )
        }
        NteCompletionKind::Variable => symbol.label.to_owned(),
    };
    NteCompletionItem {
        label: symbol.label.to_owned(),
        insert: symbol.insert_text.to_owned(),
        kind,
        detail,
        documentation_key: symbol.documentation_key,
    }
}

fn completion_match_rank(item: &NteCompletionItem, query: &str) -> (u8, u8) {
    if query.is_empty() {
        return (0, completion_kind_rank(item.kind));
    }
    let label = item.label.to_ascii_lowercase();
    let insert = item.insert.to_ascii_lowercase();
    let prefix_rank = if insert.starts_with(query) || label.starts_with(query) {
        0
    } else if label
        .split([':', '.', '(', ' '])
        .any(|segment| segment.starts_with(query))
    {
        1
    } else {
        2
    };
    (prefix_rank, completion_kind_rank(item.kind))
}

fn completion_kind_rank(kind: NteCompletionKind) -> u8 {
    match kind {
        NteCompletionKind::Variable => 0,
        NteCompletionKind::Function => 1,
        NteCompletionKind::Property => 2,
        NteCompletionKind::Declaration => 3,
        NteCompletionKind::Snippet => 4,
    }
}

fn document_completion_items(source: &str) -> impl Iterator<Item = NteCompletionItem> {
    let mut items = Vec::new();
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("NTE_") {
            continue;
        }
        if let Some(open) = line.find('(') {
            let prefix = line[..open].trim();
            if !matches!(prefix, "if" | "for" | "while" | "switch")
                && !prefix.contains('=')
                && let Some((return_type, name)) = split_declared_symbol(prefix)
            {
                let parameters = line[open + 1..]
                    .split(')')
                    .next()
                    .unwrap_or_default()
                    .trim();
                items.push(NteCompletionItem {
                    label: format!("{name}({parameters})"),
                    insert: format!("{name}("),
                    kind: NteCompletionKind::Function,
                    detail: format!("{name}({parameters}) -> {return_type}"),
                    documentation_key: "Function declared in this Mod source file.",
                });
                for parameter in parameters.split(',').map(str::trim) {
                    if let Some((type_name, parameter_name)) = split_declared_symbol(parameter) {
                        items.push(NteCompletionItem {
                            label: parameter_name.to_owned(),
                            insert: parameter_name.to_owned(),
                            kind: NteCompletionKind::Variable,
                            detail: format!("{parameter_name}: {type_name}"),
                            documentation_key: "Parameter declared by the current Mod function.",
                        });
                    }
                }
            }
        }
        let declaration = line.trim_end_matches([';', '{']).trim();
        let declaration = declaration
            .split_once('=')
            .map_or(declaration, |(left, _)| left.trim());
        if let Some((type_name, name)) = split_declared_symbol(declaration) {
            items.push(NteCompletionItem {
                label: name.to_owned(),
                insert: name.to_owned(),
                kind: NteCompletionKind::Variable,
                detail: format!("{name}: {type_name}"),
                documentation_key: "Variable declared in this Mod source file.",
            });
        }
    }
    items.into_iter()
}

fn split_declared_symbol(declaration: &str) -> Option<(&str, &str)> {
    let name_start = declaration
        .char_indices()
        .rev()
        .find_map(|(index, character)| character.is_whitespace().then_some(index + 1))?;
    let type_name = declaration[..name_start].trim();
    let name = declaration[name_start..].trim_matches([' ', '\t', '&', '*']);
    (!type_name.is_empty()
        && !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'))
    .then_some((type_name, name))
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
    suggestions: &[NteCompletionItem],
    query: &str,
    selected: &mut usize,
    scroll_selected: bool,
    palette: ModEditorPalette,
) -> Option<NteCompletionItem> {
    let screen = ui.ctx().content_rect();
    let popup_size = completion_popup_size(screen.size(), suggestions.len(), query.is_empty());
    let width = popup_size.x;
    let height = popup_size.y;
    let row_height = 29.0;
    let visible_rows = suggestions.len().min(6) as f32;
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
    egui::Area::new(ui.make_persistent_id("nte_script_completion_popup_v2"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x, y))
        .default_size(popup_size)
        .constrain_to(screen)
        .show(ui.ctx(), |ui| {
            ui.set_min_size(popup_size);
            ui.set_max_size(popup_size);
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
                                let marker = completion_kind_marker(suggestion.kind);
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
                                    truncate_editor_text(&suggestion.label, 58),
                                    egui::FontId::monospace(13.0),
                                    palette.text,
                                );
                                ui.painter().text(
                                    egui::pos2(rect.right() - 10.0, rect.center().y),
                                    egui::Align2::RIGHT_CENTER,
                                    t(completion_kind_label(suggestion.kind)),
                                    egui::FontId::proportional(11.0),
                                    palette.muted,
                                );
                                if response.clicked() {
                                    clicked = Some(suggestion.clone());
                                }
                            }
                        });
                    ui.separator();
                    if let Some(active) = suggestions.get(*selected) {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&active.detail)
                                    .monospace()
                                    .size(11.0)
                                    .color(palette.selected_border),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&active.detail);
                        ui.add(
                            egui::Label::new(
                                RichText::new(t(active.documentation_key))
                                    .size(11.0)
                                    .color(palette.text),
                            )
                            .truncate(),
                        );
                    }
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

fn completion_popup_size(
    screen_size: egui::Vec2,
    suggestion_count: usize,
    query_empty: bool,
) -> egui::Vec2 {
    let available_width = (screen_size.x - 8.0).max(220.0);
    let width = (screen_size.x * 0.56)
        .clamp(320.0, 400.0)
        .min(available_width);
    let visible_rows = suggestion_count.min(6) as f32;
    let footer_height = if query_empty { 29.0 } else { 44.0 };
    let desired_height = visible_rows * 29.0 + 72.0 + footer_height;
    let height = desired_height.min((screen_size.y - 8.0).max(150.0));
    egui::vec2(width, height)
}

fn signature_help_at_cursor(source: &str, cursor_char: usize) -> Option<NteSignatureHelp> {
    let cursor_byte = char_to_byte_index(source, cursor_char)?;
    let (name, active_parameter) = active_call_context(source, cursor_byte)?;
    let item = mod_sdk_symbols()
        .map(static_completion_item)
        .chain(document_completion_items(source))
        .find(|item| {
            item.kind == NteCompletionKind::Function && item.label.starts_with(&format!("{name}("))
        })?;
    let parameters = item
        .label
        .split_once('(')
        .and_then(|(_, tail)| tail.rsplit_once(')'))
        .map(|(parameters, _)| {
            parameters
                .split(',')
                .map(str::trim)
                .filter(|parameter| !parameter.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(NteSignatureHelp {
        label: item.detail,
        active_parameter: active_parameter.min(parameters.len().saturating_sub(1)),
        parameters,
        documentation_key: item.documentation_key,
    })
}

fn active_call_context(source: &str, cursor_byte: usize) -> Option<(String, usize)> {
    let mut parentheses = Vec::new();
    let mut in_string = false;
    let mut in_character = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut escaped = false;
    let mut chars = source[..cursor_byte].char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let next = chars.peek().map(|(_, character)| *character);
        if in_line_comment {
            if character == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_block_comment {
            if character == '*' && next == Some('/') {
                chars.next();
                in_block_comment = false;
            }
            continue;
        }
        if in_string || in_character {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if (in_string && character == '"') || (in_character && character == '\'') {
                in_string = false;
                in_character = false;
            }
            continue;
        }
        if character == '/' && next == Some('/') {
            chars.next();
            in_line_comment = true;
        } else if character == '/' && next == Some('*') {
            chars.next();
            in_block_comment = true;
        } else if character == '"' {
            in_string = true;
        } else if character == '\'' {
            in_character = true;
        } else if character == '(' {
            parentheses.push(index);
        } else if character == ')' {
            parentheses.pop();
        }
    }
    let open = *parentheses.last()?;
    let name_end = source[..open].trim_end().len();
    let name_start = source[..name_end]
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!character.is_ascii_alphanumeric() && !matches!(character, '_' | ':'))
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    let name = source[name_start..name_end].to_owned();
    if name.is_empty() {
        return None;
    }
    let active_parameter = top_level_comma_count(&source[open + 1..cursor_byte]);
    Some((name, active_parameter))
}

fn top_level_comma_count(source: &str) -> usize {
    let mut depth = 0usize;
    let mut count = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for character in source.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else if character == '"' {
            in_string = true;
        } else if matches!(character, '(' | '[' | '{') {
            depth += 1;
        } else if matches!(character, ')' | ']' | '}') {
            depth = depth.saturating_sub(1);
        } else if character == ',' && depth == 0 {
            count += 1;
        }
    }
    count
}

fn signature_help_popup(
    ui: &egui::Ui,
    cursor_rect: egui::Rect,
    signature: &NteSignatureHelp,
    palette: ModEditorPalette,
) {
    let screen = ui.ctx().content_rect();
    let width = (screen.width() * 0.56)
        .clamp(300.0, 400.0)
        .min((screen.width() - 8.0).max(220.0));
    let x = cursor_rect.left().clamp(
        screen.left() + 4.0,
        (screen.right() - width - 4.0).max(screen.left()),
    );
    let y = (cursor_rect.top() - 82.0).max(screen.top() + 4.0);
    egui::Area::new(ui.make_persistent_id("nte_script_signature_help_v2"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(x, y))
        .default_size(egui::vec2(width, 82.0))
        .constrain_to(screen)
        .show(ui.ctx(), |ui| {
            ui.set_width(width);
            egui::Frame::new()
                .fill(palette.chrome)
                .stroke(Stroke::new(1.0_f32, palette.border))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&signature.label)
                                .monospace()
                                .color(palette.selected_border),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&signature.label);
                    if let Some(parameter) = signature.parameters.get(signature.active_parameter) {
                        ui.label(
                            RichText::new(tf(
                                "Parameter {} of {}: {}",
                                &[
                                    &(signature.active_parameter + 1).to_string(),
                                    &signature.parameters.len().to_string(),
                                    parameter,
                                ],
                            ))
                            .small()
                            .color(palette.text),
                        );
                    }
                    ui.label(
                        RichText::new(t(signature.documentation_key))
                            .small()
                            .color(palette.muted),
                    );
                });
        });
}

fn truncate_editor_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn completion_kind_label(kind: NteCompletionKind) -> &'static str {
    match kind {
        NteCompletionKind::Declaration => "Declaration",
        NteCompletionKind::Snippet => "Snippet",
        NteCompletionKind::Function => "Function",
        NteCompletionKind::Property => "Property",
        NteCompletionKind::Variable => "Variable",
    }
}

fn completion_kind_marker(kind: NteCompletionKind) -> &'static str {
    match kind {
        NteCompletionKind::Declaration => "D",
        NteCompletionKind::Snippet => "{ }",
        NteCompletionKind::Function => "ƒ",
        NteCompletionKind::Property => "◇",
        NteCompletionKind::Variable => "V",
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
            (!character.is_ascii_alphanumeric() && !matches!(character, '_' | ':' | '.' | '"'))
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
    let (keyword, type_name, macro_name, function, namespace, variable, string, number, comment) =
        if dark_mode {
            (
                format(Color32::from_rgb(197, 134, 192)),
                format(Color32::from_rgb(78, 201, 176)),
                format(Color32::from_rgb(86, 156, 214)),
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
                format(Color32::from_rgb(38, 127, 153)),
                format(Color32::from_rgb(0, 0, 255)),
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
    let mut expect_include_header = false;
    while index < source.len() {
        let character = source[index..]
            .chars()
            .next()
            .expect("index remains on a character boundary");
        let cpp_comment = source[index..].starts_with("//");
        let cpp_block_comment = source[index..].starts_with("/*");
        let preprocessor_end = (character == '#')
            .then(|| nte_cpp_preprocessor_directive_end(source, index))
            .flatten();
        let include_header_end = (expect_include_header && character == '<').then(|| {
            source[index..]
                .find('>')
                .map_or(source.len(), |offset| index + offset + 1)
        });
        let end = if cpp_comment {
            source[index..]
                .find('\n')
                .map_or(source.len(), |offset| index + offset)
        } else if cpp_block_comment {
            source[index + 2..]
                .find("*/")
                .map_or(source.len(), |offset| index + 2 + offset + 2)
        } else if let Some(end) = preprocessor_end {
            end
        } else if let Some(end) = include_header_end {
            end
        } else if matches!(character, '"' | '\'') {
            quoted_cpp_literal_end(source, index, character)
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
        let token_format = if cpp_comment || cpp_block_comment {
            comment.clone()
        } else if preprocessor_end.is_some() {
            expect_include_header = token
                .strip_prefix('#')
                .is_some_and(|directive| directive.trim() == "include");
            keyword.clone()
        } else if include_header_end.is_some() {
            expect_include_header = false;
            string.clone()
        } else if matches!(character, '"' | '\'') {
            string.clone()
        } else if character.is_ascii_digit() {
            number.clone()
        } else if character.is_ascii_alphabetic() || character == '_' {
            let followed_by_call =
                source[end..].chars().find(|value| !value.is_whitespace()) == Some('(');
            let kind = classify_nte_identifier(token, expect_function_name, followed_by_call);
            expect_function_name = matches!(token, "void" | "auto");
            match kind {
                NteIdentifierKind::Keyword => keyword.clone(),
                NteIdentifierKind::Type => type_name.clone(),
                NteIdentifierKind::Macro => macro_name.clone(),
                NteIdentifierKind::Function => function.clone(),
                NteIdentifierKind::Namespace => namespace.clone(),
                NteIdentifierKind::Variable => variable.clone(),
            }
        } else {
            base.clone()
        };
        if character == '\n' {
            expect_include_header = false;
        }
        job.append(token, 0.0, token_format);
        index = end;
    }
    if source.is_empty() {
        job.append("", 0.0, base);
    }
    job
}

fn quoted_cpp_literal_end(source: &str, start: usize, quote: char) -> usize {
    let mut escaped = false;
    for (offset, character) in source[start + quote.len_utf8()..].char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return start + quote.len_utf8() + offset + character.len_utf8();
        }
    }
    source.len()
}

fn nte_cpp_preprocessor_directive_end(source: &str, start: usize) -> Option<usize> {
    if !source[start..].starts_with('#') {
        return None;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    if !source[line_start..start].trim().is_empty() {
        return None;
    }
    let mut end = start + '#'.len_utf8();
    while source[end..]
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_whitespace() && character != '\n')
    {
        end += 1;
    }
    let name_start = end;
    end = consume_ascii_token(source, end, |character| {
        character.is_ascii_alphanumeric() || character == '_'
    });
    (end > name_start).then_some(end)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteIdentifierKind {
    Keyword,
    Type,
    Macro,
    Function,
    Namespace,
    Variable,
}

fn classify_nte_identifier(
    token: &str,
    function_definition_name: bool,
    followed_by_call: bool,
) -> NteIdentifierKind {
    if token.starts_with("NTE_") {
        NteIdentifierKind::Macro
    } else if matches!(
        token,
        "uint8_t"
            | "uint16_t"
            | "uint32_t"
            | "uint64_t"
            | "int8_t"
            | "int16_t"
            | "int32_t"
            | "int64_t"
            | "uintptr_t"
            | "size_t"
            | "viewport_tick_event"
    ) {
        NteIdentifierKind::Type
    } else if matches!(
        token,
        "alignas"
            | "alignof"
            | "and"
            | "and_eq"
            | "asm"
            | "auto"
            | "bitand"
            | "bitor"
            | "bool"
            | "break"
            | "case"
            | "catch"
            | "char"
            | "char8_t"
            | "char16_t"
            | "char32_t"
            | "class"
            | "compl"
            | "concept"
            | "const"
            | "consteval"
            | "constexpr"
            | "constinit"
            | "const_cast"
            | "continue"
            | "co_await"
            | "co_return"
            | "co_yield"
            | "decltype"
            | "default"
            | "delete"
            | "do"
            | "double"
            | "dynamic_cast"
            | "else"
            | "enum"
            | "explicit"
            | "export"
            | "extern"
            | "false"
            | "float"
            | "for"
            | "friend"
            | "goto"
            | "if"
            | "inline"
            | "int"
            | "long"
            | "mutable"
            | "namespace"
            | "new"
            | "noexcept"
            | "not"
            | "not_eq"
            | "nullptr"
            | "operator"
            | "or"
            | "or_eq"
            | "private"
            | "protected"
            | "public"
            | "register"
            | "reinterpret_cast"
            | "requires"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "static_assert"
            | "static_cast"
            | "struct"
            | "switch"
            | "template"
            | "this"
            | "thread_local"
            | "throw"
            | "true"
            | "try"
            | "typedef"
            | "typeid"
            | "typename"
            | "union"
            | "unsigned"
            | "using"
            | "virtual"
            | "void"
            | "volatile"
            | "wchar_t"
            | "while"
            | "xor"
            | "xor_eq"
    ) {
        NteIdentifierKind::Keyword
    } else if function_definition_name || followed_by_call {
        NteIdentifierKind::Function
    } else if matches!(
        token,
        "nte"
            | "std"
            | "game"
            | "memory"
            | "sdk"
            | "ipc"
            | "equipment"
            | "combat_clock"
            | "time"
            | "log"
            | "event"
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn runtime_event_text_keeps_enemy_target_and_hp_values() {
        let event = ModEventSnapshot {
            sequence: 7,
            timestamp_100ns: 0,
            mod_id: "enemy-telemetry".to_owned(),
            name: "post.enemy.vitals".to_owned(),
            values: vec![0x1234, 1_593_822, 2_220_578],
        };

        assert_eq!(
            mod_runtime_event_text(&event),
            "post.enemy.vitals target=0x0000000000001234 hp=1593822 max_hp=2220578"
        );
    }

    #[test]
    fn runtime_console_merges_logs_and_events_once_in_timestamp_order() {
        let logs = vec![ModLogSnapshot {
            sequence: 4,
            timestamp_100ns: 200,
            mod_id: "enemy-telemetry".to_owned(),
            level: ModLogLevel::Info,
            message: "enemy identity hash missing".to_owned(),
        }];
        let events = vec![ModEventSnapshot {
            sequence: 9,
            timestamp_100ns: 100,
            mod_id: "enemy-telemetry".to_owned(),
            name: "post.enemy.vitals".to_owned(),
            values: vec![0x1234, 1_593_822, 2_220_578],
        }];
        let mut console = ModConsoleState::default();

        console.ingest_runtime(ModConsolePollResult {
            logs: logs.clone(),
            events: events.clone(),
        });

        assert_eq!(console.entries.len(), 2);
        assert!(matches!(
            console.entries[0].level,
            ModConsoleEntryLevel::Event
        ));
        assert!(matches!(
            console.entries[1].level,
            ModConsoleEntryLevel::Runtime(ModLogLevel::Info)
        ));
        assert_eq!(console.last_runtime_sequence, 4);
        assert_eq!(console.last_event_sequence, 9);
        let copied = console.plain_text();
        assert!(copied.contains("[EVENT] [enemy-telemetry] post.enemy.vitals"));
        assert!(copied.contains("[INFO] [enemy-telemetry] enemy identity hash missing"));
        assert_eq!(copied.lines().count(), 2);

        console.ingest_runtime(ModConsolePollResult { logs, events });

        assert_eq!(console.entries.len(), 2);
    }

    fn temp_mod_workspace() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nte-mod-editor-{unique}"))
    }

    #[test]
    fn editor_loads_the_shared_workspace_without_a_game_installation() {
        let root = temp_mod_workspace();
        let mod_directory = root.join("nte-mods");
        fs::create_dir_all(&mod_directory).unwrap();
        fs::write(
            mod_directory.join("telemetry.nte"),
            new_mod_script_template("telemetry").unwrap(),
        )
        .unwrap();

        let targets = load_mod_editor_targets_from_workspace(root.clone()).unwrap();

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].directory, root);
        assert_eq!(targets[0].workspace.scripts[0].id, "telemetry");
        fs::remove_dir_all(targets[0].directory.clone()).unwrap();
    }

    #[test]
    fn completion_replaces_only_the_token_before_the_cursor() {
        let mut source = "auto value = nte::game::pla + suffix;".to_owned();
        let cursor = source.find(" +").unwrap();
        let cursor_char = source[..cursor].chars().count();

        let new_cursor = apply_completion(&mut source, cursor_char, "nte::game::player_state")
            .expect("completion prefix exists");

        assert_eq!(source, "auto value = nte::game::player_state + suffix;");
        assert_eq!(
            new_cursor,
            "auto value = nte::game::player_state".chars().count()
        );
    }

    #[test]
    fn completion_filters_host_apis_by_dotted_prefix() {
        let source = "nte::memory::read_";
        let suggestions = completion_candidates(source, source.chars().count(), false);

        assert!(suggestions.len() >= 6);
        assert!(
            suggestions
                .iter()
                .all(|item| item.insert.starts_with(source))
        );
    }

    #[test]
    fn completion_exposes_stable_game_session_builtins() {
        let source = "nte::game::player_";
        let suggestions = completion_candidates(source, source.chars().count(), false);
        let labels: Vec<_> = suggestions.iter().map(|item| item.label.as_str()).collect();

        assert!(labels.contains(&"nte::game::player_controller"));
        assert!(labels.contains(&"nte::game::player_state"));
        assert!(labels.contains(&"nte::game::player_character"));
    }

    #[test]
    fn completion_exposes_generic_typed_reads_and_cache() {
        let source = "nte::memory::read_";
        let suggestions = completion_candidates(source, source.chars().count(), false);
        let labels: Vec<_> = suggestions.iter().map(|item| item.label.as_str()).collect();

        assert!(labels.contains(&"nte::memory::read_f32_milli(base, offset)"));
        assert!(labels.contains(&"nte::memory::read_fname_hash(base, offset)"));
        assert!(mod_sdk_symbols().any(|item| item.label == "nte::cache::remember(key, value)"));
    }

    #[test]
    fn completion_exposes_generic_write_reflection_and_event_apis() {
        for expected in [
            "nte::memory::write_u64(base, offset, value)",
            "nte::unreal::find_function(object, \"Owner\", \"Function\")",
            "nte::unreal::params_write_u64(offset, value)",
            "nte::unreal::watch(object, function)",
            "nte::unreal::watch_class_array_u64(object, function, element_size, value_offset)",
            "nte::event::captured_u64()",
            "nte::event::read_u64(offset)",
        ] {
            assert!(mod_sdk_symbols().any(|item| item.label == expected));
        }
    }

    #[test]
    fn completion_navigation_wraps_in_both_directions() {
        assert_eq!(completion_selection_index(0, 3, true, false), 2);
        assert_eq!(completion_selection_index(2, 3, false, true), 0);
    }

    #[test]
    fn completion_popup_stays_compact_and_inside_the_viewport() {
        let desktop = completion_popup_size(egui::vec2(752.0, 648.0), 85, false);
        assert_eq!(desktop.x, 400.0);
        assert_eq!(desktop.y, 290.0);

        let narrow = completion_popup_size(egui::vec2(300.0, 220.0), 85, true);
        assert_eq!(narrow.x, 292.0);
        assert!(narrow.y <= 212.0);
    }

    #[test]
    fn explicit_completion_lists_entries_without_a_prefix() {
        let source = "void on_viewport_tick() { ";
        let suggestions = completion_candidates(source, source.chars().count(), true);

        assert!(suggestions.len() >= mod_sdk_symbols().len());
    }

    #[test]
    fn editor_gutter_reserves_rows_for_empty_and_trailing_lines() {
        assert_eq!(editor_content_row_count("", 12), 12);
        assert_eq!(editor_content_row_count("one\ntwo\n", 1), 3);
    }

    #[test]
    fn syntax_classification_distinguishes_language_roles() {
        assert_eq!(
            classify_nte_identifier("if", false, false),
            NteIdentifierKind::Keyword
        );
        assert_eq!(
            classify_nte_identifier("nte", false, false),
            NteIdentifierKind::Namespace
        );
        assert_eq!(
            classify_nte_identifier("on_viewport_tick", true, false),
            NteIdentifierKind::Function
        );
        assert_eq!(
            classify_nte_identifier("remember", false, true),
            NteIdentifierKind::Function
        );
        assert_eq!(
            classify_nte_identifier("character", false, false),
            NteIdentifierKind::Variable
        );
        assert_eq!(
            classify_nte_identifier("uint64_t", false, false),
            NteIdentifierKind::Type
        );
        assert_eq!(
            classify_nte_identifier("NTE_REQUIRES", false, true),
            NteIdentifierKind::Macro
        );
        assert_eq!(
            classify_nte_identifier("co_await", false, false),
            NteIdentifierKind::Keyword
        );
        assert_eq!(
            classify_nte_identifier("None", false, false),
            NteIdentifierKind::Variable
        );
    }

    #[test]
    fn completion_includes_symbols_declared_in_the_current_mod() {
        let source = concat!(
            "std::uint64_t last_character = 0;\n",
            "void sample(const std::uint64_t player_state)\n",
            "{\n",
            "    const auto current_character = nte::game::player_character;\n",
            "    current_\n",
            "}\n",
        );
        let cursor = source.find("current_\n").unwrap() + "current_".len();
        let suggestions = completion_candidates(source, source[..cursor].chars().count(), false);

        assert!(suggestions.iter().any(|item| {
            item.label == "current_character"
                && item.detail == "current_character: const auto"
                && item.kind == NteCompletionKind::Variable
        }));
        assert!(
            document_completion_items(source)
                .any(|item| item.label == "sample(const std::uint64_t player_state)")
        );
    }

    #[test]
    fn signature_help_tracks_nested_calls_and_the_active_parameter() {
        let source = "nte::ipc::emit(\"post.value\", nte::time::now_ms(), ";
        let signature = signature_help_at_cursor(source, source.chars().count()).unwrap();

        assert!(signature.label.starts_with("nte::ipc::emit("));
        assert_eq!(active_call_context(source, source.len()).unwrap().1, 2);
        assert_eq!(signature.active_parameter, 1);
        assert_eq!(signature.parameters.len(), 2);
    }

    #[test]
    fn quoted_literals_honor_escaped_quotes() {
        let source = "\"value: \\\"quoted\\\"\" + suffix";
        assert_eq!(
            quoted_cpp_literal_end(source, 0, '"'),
            "\"value: \\\"quoted\\\"\"".len()
        );
    }

    #[test]
    fn cpp_preprocessor_directives_are_not_python_comments() {
        let source = "#include <nte/mod.hpp>\nNTE_SCRIPT(5);";
        assert_eq!(nte_cpp_preprocessor_directive_end(source, 0), Some(8));
        assert_eq!(nte_cpp_preprocessor_directive_end(source, 23), None);
    }

    #[test]
    fn declared_capabilities_follow_cpp_macros() {
        let source = concat!(
            "#include <nte/mod.hpp>\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "NTE_REQUIRES(\"ipc\");\n",
        );
        assert_eq!(declared_capabilities(source), vec!["viewport.tick", "ipc"]);
    }
}
