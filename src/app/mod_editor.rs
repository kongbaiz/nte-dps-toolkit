use super::*;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ModStudioMode {
    #[default]
    Visual,
    Script,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LowCodeDamageTarget {
    #[default]
    All,
    Primary,
    FollowUp,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LowCodeNodeSelection {
    #[default]
    Event,
    Match,
    Variables,
    Rule(usize),
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BlueprintExecutionNode {
    Event,
    Match,
    Rule(u64),
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlueprintExecutionEdge {
    from: BlueprintExecutionNode,
    to: BlueprintExecutionNode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlueprintPendingConnection {
    Execution(BlueprintExecutionNode),
    Parameters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum BlueprintGraphNode {
    Event,
    Match,
    Variables,
    Rule(u64),
    Output,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlueprintPin {
    ExecutionInput(BlueprintExecutionNode),
    ExecutionOutput(BlueprintExecutionNode),
    ParameterInput(u64),
    ParameterOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlueprintPinAction {
    pin: BlueprintPin,
    disconnect: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BlueprintNodeResponse {
    selected: bool,
    pin_action: Option<BlueprintPinAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlueprintRuleTemplate {
    Custom,
    Scale,
    Bonus,
    Reduction,
    Flat,
    Clamp,
    Resistance,
    Threshold,
    FollowUp,
    Character,
}

#[derive(Clone, Debug, PartialEq)]
struct LowCodeVariable {
    name: String,
    value: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct LowCodeRule {
    id: u64,
    enabled: bool,
    target: LowCodeDamageTarget,
    expression: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BlueprintSkillOption {
    key: String,
    display_name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct DamageProjectionEditorState {
    name: String,
    character_ids: Vec<u32>,
    damage_attributes: Vec<String>,
    attack_types: Vec<String>,
    skill_names: Vec<String>,
    variables: Vec<LowCodeVariable>,
    rules: Vec<LowCodeRule>,
    next_rule_id: u64,
    execution_edges: Vec<BlueprintExecutionEdge>,
    parameter_connections: Vec<u64>,
    pending_connection: Option<BlueprintPendingConnection>,
    selected_node: LowCodeNodeSelection,
    event_node_position: egui::Pos2,
    match_node_position: egui::Pos2,
    variable_node_position: egui::Pos2,
    rule_node_positions: Vec<egui::Pos2>,
    output_node_position: egui::Pos2,
    scene_rect: egui::Rect,
    applied_to_home: bool,
    feedback: Option<String>,
}

impl Default for DamageProjectionEditorState {
    fn default() -> Self {
        Self {
            name: String::new(),
            character_ids: Vec::new(),
            damage_attributes: Vec::new(),
            attack_types: Vec::new(),
            skill_names: Vec::new(),
            variables: vec![LowCodeVariable {
                name: "factor".to_owned(),
                value: 1.0,
            }],
            rules: vec![LowCodeRule {
                id: 1,
                enabled: true,
                target: LowCodeDamageTarget::All,
                expression: "value * factor".to_owned(),
            }],
            next_rule_id: 2,
            execution_edges: default_blueprint_execution_edges(),
            parameter_connections: vec![1],
            pending_connection: None,
            selected_node: LowCodeNodeSelection::Event,
            event_node_position: egui::pos2(24.0, 70.0),
            match_node_position: egui::pos2(230.0, 70.0),
            variable_node_position: egui::pos2(230.0, 250.0),
            rule_node_positions: vec![egui::pos2(485.0, 70.0)],
            output_node_position: egui::pos2(805.0, 70.0),
            scene_rect: egui::Rect::from_min_max(
                egui::pos2(-20.0, -20.0),
                egui::pos2(1025.0, 500.0),
            ),
            applied_to_home: false,
            feedback: None,
        }
    }
}

impl DamageProjectionEditorState {
    fn load_resistance_recipe(&mut self) {
        self.name = t("20% Resistance Target");
        self.variables = vec![
            LowCodeVariable {
                name: "base_resistance".to_owned(),
                value: 0.0,
            },
            LowCodeVariable {
                name: "target_resistance".to_owned(),
                value: 20.0,
            },
        ];
        self.rules = vec![LowCodeRule {
            id: 1,
            enabled: true,
            target: LowCodeDamageTarget::All,
            expression: "value * resist(target_resistance) / resist(base_resistance)".to_owned(),
        }];
        self.next_rule_id = 2;
        self.execution_edges = default_blueprint_execution_edges();
        self.parameter_connections = vec![1];
        self.pending_connection = None;
        self.selected_node = LowCodeNodeSelection::Rule(0);
        self.rule_node_positions = vec![egui::pos2(485.0, 70.0)];
        self.feedback = Some(t("Resistance template added"));
    }

    fn execution_eq(&self, other: &Self) -> bool {
        self.character_ids == other.character_ids
            && self.damage_attributes == other.damage_attributes
            && self.attack_types == other.attack_types
            && self.skill_names == other.skill_names
            && self.variables == other.variables
            && self.rules == other.rules
            && self.execution_edges == other.execution_edges
            && self.parameter_connections == other.parameter_connections
            && self.applied_to_home == other.applied_to_home
    }

    fn reset_transform(&mut self) {
        let defaults = Self::default();
        self.variables = defaults.variables;
        self.rules = defaults.rules;
        self.next_rule_id = defaults.next_rule_id;
        self.execution_edges = defaults.execution_edges;
        self.parameter_connections = defaults.parameter_connections;
        self.pending_connection = None;
        self.rule_node_positions = defaults.rule_node_positions;
        self.output_node_position = defaults.output_node_position;
        self.selected_node = LowCodeNodeSelection::Rule(0);
        self.feedback = Some(t("Blueprint reset"));
    }
}

fn default_blueprint_execution_edges() -> Vec<BlueprintExecutionEdge> {
    vec![
        BlueprintExecutionEdge {
            from: BlueprintExecutionNode::Event,
            to: BlueprintExecutionNode::Match,
        },
        BlueprintExecutionEdge {
            from: BlueprintExecutionNode::Match,
            to: BlueprintExecutionNode::Rule(1),
        },
        BlueprintExecutionEdge {
            from: BlueprintExecutionNode::Rule(1),
            to: BlueprintExecutionNode::Output,
        },
    ]
}

#[derive(Default)]
pub(crate) struct ModEditorState {
    mode: ModStudioMode,
    projection: DamageProjectionEditorState,
    projection_error: Option<String>,
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

    pub(crate) fn refresh_applied_mod_projection(&mut self, force: bool) {
        if !self.mod_editor.projection.applied_to_home {
            if self.projected_state.take().is_some() {
                self.invalidate_mod_projection_views();
            }
            self.mod_projection_dirty = false;
            self.mod_editor.projection_error = None;
            return;
        }
        if !self.mod_projection_dirty && !force {
            return;
        }
        if !force
            && self
                .mod_projection_last_refresh
                .is_some_and(|last| last.elapsed() < Duration::from_millis(120))
        {
            return;
        }
        let projection = self.mod_editor.projection.clone();
        let projected = compile_low_code_mod(&projection).and_then(|compiled| {
            self.raw_presented_state()
                .try_transformed_damage_copy(|input| {
                    if !compiled.uses_match || low_code_component_matches(&projection, input) {
                        compiled.evaluate(input)
                    } else {
                        Ok(input.value)
                    }
                })
        });
        match projected {
            Ok(projected) => {
                self.projected_state = Some(Box::new(projected));
                self.mod_editor.projection_error = None;
            }
            Err(error) => {
                self.projected_state = None;
                self.mod_editor.projection_error = Some(error);
            }
        }
        self.mod_projection_dirty = false;
        self.mod_projection_last_refresh = Some(Instant::now());
        self.invalidate_mod_projection_views();
    }

    pub(crate) fn mod_projection_is_applied(&self) -> bool {
        self.mod_editor.projection.applied_to_home && self.projected_state.is_some()
    }

    fn invalidate_mod_projection_views(&mut self) {
        self.character_hit_cache = HitDetailCache::default();
        self.team_hit_cache = HitDetailCache::default();
        self.skill_summary_cache = SkillSummaryCache::default();
        self.timeline_cache = TimelineCache::default();
        self.skill_breakdown_cache = SkillBreakdownCache::default();
        self.session_epoch = self.session_epoch.wrapping_add(1);
    }

    pub(crate) fn mod_editor_contents(&mut self, ui: &mut egui::Ui) {
        self.drain_mod_editor_task(ui.ctx());
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
                        RichText::new(match self.mod_editor.mode {
                            ModStudioMode::Visual => t(
                                "Build software-side Mods as Blueprint-style event graphs with nodes, wires and expressions.",
                            ),
                            ModStudioMode::Script => t(
                                "Manage installed NTE Script v4 Mods and edit them with syntax assistance.",
                            ),
                        })
                        .color(palette.muted),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.mod_editor.mode,
                        ModStudioMode::Visual,
                        t("Blueprint Mods"),
                    );
                    ui.selectable_value(
                        &mut self.mod_editor.mode,
                        ModStudioMode::Script,
                        t("NTE Script editor"),
                    );
                    if matches!(self.mod_editor.mode, ModStudioMode::Visual) {
                        ui.separator();
                        ui.label(
                            RichText::new(t(
                                "Blueprint Mods run inside the DPS tool and modify a copy of software data.",
                            ))
                            .small()
                            .color(palette.muted),
                        );
                    }
                });
                if matches!(self.mod_editor.mode, ModStudioMode::Visual) {
                    return false;
                }
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
        if matches!(self.mod_editor.mode, ModStudioMode::Visual) {
            self.mod_projection_contents(ui);
            return;
        }
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

    fn mod_projection_contents(&mut self, ui: &mut egui::Ui) {
        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        let (characters, attributes, attack_types, skills) = {
            let state = self.raw_presented_state();
            let mut characters = state
                .stats
                .values()
                .map(|row| (row.char_id, row.name.clone()))
                .collect::<Vec<_>>();
            characters.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
            let mut attributes = state
                .hits
                .iter()
                .flat_map(|hit| {
                    [
                        hit.damage_attribute.as_deref(),
                        hit.follow_up_damage_attribute.as_deref(),
                    ]
                })
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            attributes.sort();
            attributes.dedup();
            let mut attack_types = state
                .hits
                .iter()
                .flat_map(|hit| {
                    [
                        hit.attack_type.as_deref(),
                        hit.follow_up_attack_type.as_deref(),
                    ]
                })
                .flatten()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            attack_types.sort();
            attack_types.dedup();
            let mut skills = state
                .hits
                .iter()
                .flat_map(blueprint_skill_options_for_hit)
                .collect::<Vec<_>>();
            skills.sort_by(|left, right| {
                left.key
                    .cmp(&right.key)
                    .then(left.display_name.cmp(&right.display_name))
            });
            skills.dedup_by(|left, right| left.key == right.key);
            skills.sort_by(|left, right| {
                left.display_name
                    .cmp(&right.display_name)
                    .then(left.key.cmp(&right.key))
            });
            (characters, attributes, attack_types, skills)
        };

        let mut projection = std::mem::take(&mut self.mod_editor.projection);
        let projection_before = projection.clone();

        egui::Frame::new()
            .fill(palette.editor)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .corner_radius(8)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(t("Software-side Blueprint Mod"))
                            .size(17.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.label(
                        RichText::new(t(
                            "A named event graph that can be edited, inspected and executed as a Mod.",
                        ))
                        .color(palette.muted),
                    );
                    ui.separator();
                    let (status, color) = if projection.applied_to_home {
                        (t("Mod enabled"), semantic_success(ui.visuals().dark_mode))
                    } else {
                        (t("Mod disabled"), palette.muted)
                    };
                    ui.label(RichText::new(status).small().strong().color(color));
                });
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Mod name")).small().color(palette.muted));
                    ui.add(
                        egui::TextEdit::singleline(&mut projection.name)
                            .hint_text(t("Untitled software Mod"))
                            .desired_width(220.0),
                    );
                    ui.separator();
                    mod_pipeline_chip(ui, "Event Mod", palette.selected_border, palette);
                    mod_pipeline_chip(ui, "Input: damage events", palette.muted, palette);
                    mod_pipeline_chip(ui, "Output: analysis views", palette.muted, palette);
                    ui.separator();
                    ui.label(
                        RichText::new(t("Starter recipes"))
                            .small()
                            .strong()
                            .color(palette.muted),
                    );
                    if ui.button(t("20% resistance target")).clicked() {
                        projection.load_resistance_recipe();
                    }
                    if ui.button(t("Reset transform")).clicked() {
                        projection.reset_transform();
                    }
                    if ui.button(t("Clear Mod")).clicked() {
                        projection = DamageProjectionEditorState::default();
                    }
                });
            });
        ui.add_space(8.0);

        let validation_error = compile_low_code_mod(&projection).err();

        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("blueprint_mod_scroll")
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    blueprint_mod_toolbar(
                        ui,
                        &mut projection,
                        validation_error.as_deref(),
                        palette,
                    );
                    ui.add_space(8.0);
                    if blueprint_details_use_side_panel(ui.available_width()) {
                        let workspace_width = ui.available_width();
                        ui.allocate_ui_with_layout(
                            egui::vec2(workspace_width, 500.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                let details_max_width = (workspace_width * 0.42).min(520.0);
                                egui::Panel::right("blueprint_node_details_panel")
                                    .resizable(true)
                                    .default_size(400.0)
                                    .size_range(320.0..=details_max_width)
                                    .show_separator_line(false)
                                    .frame(egui::Frame::new().inner_margin(egui::Margin {
                                        left: 8,
                                        ..egui::Margin::ZERO
                                    }))
                                    .show_inside(ui, |ui| {
                                        egui::ScrollArea::vertical()
                                            .id_salt("blueprint_node_details_scroll")
                                            .auto_shrink([false, false])
                                            .show(ui, |ui| {
                                                blueprint_mod_inspector(
                                                    ui,
                                                    &mut projection,
                                                    &characters,
                                                    &attributes,
                                                    &attack_types,
                                                    &skills,
                                                    palette,
                                                );
                                            });
                                    });
                                egui::CentralPanel::default()
                                    .frame(egui::Frame::NONE)
                                    .show_inside(ui, |ui| {
                                        blueprint_mod_canvas(ui, &mut projection, palette);
                                    });
                            },
                        );
                    } else {
                        blueprint_mod_canvas(ui, &mut projection, palette);
                        ui.add_space(8.0);
                        blueprint_mod_inspector(
                            ui,
                            &mut projection,
                            &characters,
                            &attributes,
                            &attack_types,
                            &skills,
                            palette,
                        );
                    }
                    ui.add_space(8.0);
                    low_code_source_preview(ui, &projection, palette);
                });
        });

        let projection_changed = projection != projection_before;
        let execution_changed = !projection.execution_eq(&projection_before);
        let projection_is_applied = projection.applied_to_home;
        self.mod_editor.projection = projection;
        if execution_changed {
            self.mod_projection_dirty = true;
            if projection_is_applied {
                self.refresh_applied_mod_projection(true);
            } else {
                self.refresh_applied_mod_projection(false);
            }
        }
        if projection_changed {
            ui.ctx().request_repaint();
        }
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

#[derive(Clone, Debug)]
struct CompiledLowCodeMod {
    variables: Vec<(String, f64)>,
    rules: Vec<CompiledLowCodeRule>,
    uses_match: bool,
}

#[derive(Clone, Debug)]
struct CompiledLowCodeRule {
    source_index: usize,
    target: LowCodeDamageTarget,
    expression: LowCodeExpression,
    uses_parameters: bool,
}

#[derive(Clone, Debug)]
enum LowCodeExpression {
    Number(f64),
    Variable(String),
    Negate(Box<Self>),
    Add(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
    Multiply(Box<Self>, Box<Self>),
    Divide(Box<Self>, Box<Self>),
    Call(String, Vec<Self>),
}

struct LowCodeExpressionParser<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> LowCodeExpressionParser<'a> {
    fn parse(source: &'a str) -> Result<LowCodeExpression, String> {
        let mut parser = Self {
            source,
            position: 0,
        };
        let expression = parser.parse_sum()?;
        parser.skip_whitespace();
        if parser.current().is_some() {
            return Err(tf(
                "Unexpected token at position {}",
                &[&(parser.position + 1).to_string()],
            ));
        }
        Ok(expression)
    }

    fn parse_sum(&mut self) -> Result<LowCodeExpression, String> {
        let mut expression = self.parse_product()?;
        loop {
            self.skip_whitespace();
            expression = match self.current() {
                Some('+') => {
                    self.advance();
                    LowCodeExpression::Add(Box::new(expression), Box::new(self.parse_product()?))
                }
                Some('-') => {
                    self.advance();
                    LowCodeExpression::Subtract(
                        Box::new(expression),
                        Box::new(self.parse_product()?),
                    )
                }
                _ => return Ok(expression),
            };
        }
    }

    fn parse_product(&mut self) -> Result<LowCodeExpression, String> {
        let mut expression = self.parse_unary()?;
        loop {
            self.skip_whitespace();
            expression = match self.current() {
                Some('*') => {
                    self.advance();
                    LowCodeExpression::Multiply(Box::new(expression), Box::new(self.parse_unary()?))
                }
                Some('/') => {
                    self.advance();
                    LowCodeExpression::Divide(Box::new(expression), Box::new(self.parse_unary()?))
                }
                _ => return Ok(expression),
            };
        }
    }

    fn parse_unary(&mut self) -> Result<LowCodeExpression, String> {
        self.skip_whitespace();
        match self.current() {
            Some('+') => {
                self.advance();
                self.parse_unary()
            }
            Some('-') => {
                self.advance();
                Ok(LowCodeExpression::Negate(Box::new(self.parse_unary()?)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<LowCodeExpression, String> {
        self.skip_whitespace();
        match self.current() {
            Some('(') => {
                self.advance();
                let expression = self.parse_sum()?;
                self.expect(')')?;
                Ok(expression)
            }
            Some(character) if character.is_ascii_digit() || character == '.' => {
                self.parse_number().map(LowCodeExpression::Number)
            }
            Some(character) if is_low_code_identifier_start(character) => {
                let identifier = self.parse_identifier();
                self.skip_whitespace();
                if self.current() != Some('(') {
                    return Ok(LowCodeExpression::Variable(identifier));
                }
                self.advance();
                let mut arguments = Vec::new();
                self.skip_whitespace();
                if self.current() != Some(')') {
                    loop {
                        arguments.push(self.parse_sum()?);
                        self.skip_whitespace();
                        if self.current() != Some(',') {
                            break;
                        }
                        self.advance();
                    }
                }
                self.expect(')')?;
                Ok(LowCodeExpression::Call(identifier, arguments))
            }
            _ => Err(tf(
                "Expected a number, variable or function at position {}",
                &[&(self.position + 1).to_string()],
            )),
        }
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.position;
        let mut has_decimal_point = false;
        while let Some(character) = self.current() {
            if character.is_ascii_digit() {
                self.advance();
            } else if character == '.' && !has_decimal_point {
                has_decimal_point = true;
                self.advance();
            } else {
                break;
            }
        }
        if matches!(self.current(), Some('e' | 'E')) {
            self.advance();
            if matches!(self.current(), Some('+' | '-')) {
                self.advance();
            }
            while self
                .current()
                .is_some_and(|character| character.is_ascii_digit())
            {
                self.advance();
            }
        }
        self.source[start..self.position]
            .parse()
            .map_err(|_| tf("Invalid number at position {}", &[&(start + 1).to_string()]))
    }

    fn parse_identifier(&mut self) -> String {
        let start = self.position;
        self.advance();
        while self.current().is_some_and(is_low_code_identifier_continue) {
            self.advance();
        }
        self.source[start..self.position].to_owned()
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        self.skip_whitespace();
        if self.current() != Some(expected) {
            return Err(tf(
                "Expected '{}' at position {}",
                &[&expected.to_string(), &(self.position + 1).to_string()],
            ));
        }
        self.advance();
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while self.current().is_some_and(char::is_whitespace) {
            self.advance();
        }
    }

    fn current(&self) -> Option<char> {
        self.source[self.position..].chars().next()
    }

    fn advance(&mut self) {
        self.position += self
            .current()
            .expect("advance is only called while a character is available")
            .len_utf8();
    }
}

fn is_low_code_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_low_code_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn blueprint_execution_path(
    projection: &DamageProjectionEditorState,
) -> Result<(Vec<usize>, bool), String> {
    let mut path = Vec::new();
    let mut uses_match = false;
    let mut current = BlueprintExecutionNode::Event;
    let mut visited = std::collections::HashSet::new();

    loop {
        if !visited.insert(current) {
            return Err(t("Execution flow contains a cycle"));
        }
        match current {
            BlueprintExecutionNode::Event => {}
            BlueprintExecutionNode::Match => uses_match = true,
            BlueprintExecutionNode::Rule(rule_id) => {
                let index = projection
                    .rules
                    .iter()
                    .position(|rule| rule.id == rule_id)
                    .expect("execution edges only reference existing rule nodes");
                path.push(index);
            }
            BlueprintExecutionNode::Output => return Ok((path, uses_match)),
        }

        let Some(edge) = projection
            .execution_edges
            .iter()
            .find(|edge| edge.from == current)
        else {
            let node = t(blueprint_execution_node_label(current));
            return Err(tf("Execution pin is not connected: {}", &[&node]));
        };
        current = edge.to;
    }
}

fn blueprint_execution_node_label(node: BlueprintExecutionNode) -> &'static str {
    match node {
        BlueprintExecutionNode::Event => "Damage Hit Event",
        BlueprintExecutionNode::Match => "Match Damage",
        BlueprintExecutionNode::Rule(_) => "Transform node",
        BlueprintExecutionNode::Output => "Analysis Views Output",
    }
}

fn compile_low_code_mod(
    projection: &DamageProjectionEditorState,
) -> Result<CompiledLowCodeMod, String> {
    let mut variables = Vec::with_capacity(projection.variables.len());
    for variable in &projection.variables {
        let name = variable.name.trim();
        if name.is_empty() || !is_valid_low_code_identifier(name) {
            return Err(tf("Invalid parameter name: {}", &[name]));
        }
        if is_low_code_builtin_variable(name) {
            return Err(tf("Parameter name is reserved: {}", &[name]));
        }
        if variables
            .iter()
            .any(|(existing, _): &(String, f64)| existing == name)
        {
            return Err(tf("Duplicate parameter name: {}", &[name]));
        }
        if !variable.value.is_finite() {
            return Err(tf("Parameter must be finite: {}", &[name]));
        }
        variables.push((name.to_owned(), variable.value));
    }

    let (execution_path, uses_match) = blueprint_execution_path(projection)?;
    let mut rules = Vec::new();
    for index in execution_path {
        let rule = &projection.rules[index];
        if !rule.enabled {
            continue;
        }
        if rule.expression.trim().is_empty() {
            return Err(tf(
                "Rule {} has an empty expression",
                &[&(index + 1).to_string()],
            ));
        }
        let expression = LowCodeExpressionParser::parse(&rule.expression)
            .map_err(|error| tf("Rule {}: {}", &[&(index + 1).to_string(), &error]))?;
        rules.push(CompiledLowCodeRule {
            source_index: index,
            target: rule.target,
            expression,
            uses_parameters: projection.parameter_connections.contains(&rule.id),
        });
    }

    let compiled = CompiledLowCodeMod {
        variables,
        rules,
        uses_match,
    };
    compiled.evaluate(DamageTransformInput {
        value: 100.0,
        char_id: 1,
        timestamp: 1.0,
        damage_attribute: None,
        attack_type: None,
        skill_name: None,
        follow_up: false,
    })?;
    compiled.evaluate(DamageTransformInput {
        value: 100.0,
        char_id: 1,
        timestamp: 1.0,
        damage_attribute: None,
        attack_type: None,
        skill_name: None,
        follow_up: true,
    })?;
    Ok(compiled)
}

fn is_valid_low_code_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next().is_some_and(is_low_code_identifier_start)
        && characters.all(is_low_code_identifier_continue)
}

fn is_low_code_builtin_variable(name: &str) -> bool {
    matches!(
        name,
        "value" | "raw" | "char_id" | "timestamp" | "is_follow_up"
    )
}

impl CompiledLowCodeMod {
    fn evaluate(&self, input: DamageTransformInput<'_>) -> Result<f64, String> {
        let raw = input.value;
        let mut value = raw;
        for rule in &self.rules {
            let target_matches = match rule.target {
                LowCodeDamageTarget::All => true,
                LowCodeDamageTarget::Primary => !input.follow_up,
                LowCodeDamageTarget::FollowUp => input.follow_up,
            };
            if !target_matches {
                continue;
            }
            value = evaluate_low_code_expression(
                &rule.expression,
                LowCodeEvaluationContext {
                    variables: if rule.uses_parameters {
                        &self.variables
                    } else {
                        &[]
                    },
                    value,
                    raw,
                    char_id: input.char_id,
                    timestamp: input.timestamp,
                    follow_up: input.follow_up,
                },
            )
            .map_err(|error| {
                tf(
                    "Rule {}: {}",
                    &[&(rule.source_index + 1).to_string(), &error],
                )
            })?;
            if !value.is_finite() {
                return Err(tf(
                    "Rule {} produced a non-finite value",
                    &[&(rule.source_index + 1).to_string()],
                ));
            }
            if value < 0.0 {
                return Err(tf(
                    "Rule {} produced a negative damage value",
                    &[&(rule.source_index + 1).to_string()],
                ));
            }
        }
        Ok(value)
    }
}

#[derive(Clone, Copy)]
struct LowCodeEvaluationContext<'a> {
    variables: &'a [(String, f64)],
    value: f64,
    raw: f64,
    char_id: u32,
    timestamp: f64,
    follow_up: bool,
}

fn evaluate_low_code_expression(
    expression: &LowCodeExpression,
    context: LowCodeEvaluationContext<'_>,
) -> Result<f64, String> {
    match expression {
        LowCodeExpression::Number(value) => Ok(*value),
        LowCodeExpression::Variable(name) => match name.as_str() {
            "value" => Ok(context.value),
            "raw" => Ok(context.raw),
            "char_id" => Ok(f64::from(context.char_id)),
            "timestamp" => Ok(context.timestamp),
            "is_follow_up" => Ok(f64::from(context.follow_up)),
            _ => context
                .variables
                .iter()
                .find(|(variable, _)| variable == name)
                .map(|(_, value)| *value)
                .ok_or_else(|| tf("Unknown variable: {}", &[name])),
        },
        LowCodeExpression::Negate(value) => Ok(-evaluate_low_code_expression(value, context)?),
        LowCodeExpression::Add(left, right) => Ok(evaluate_low_code_expression(left, context)?
            + evaluate_low_code_expression(right, context)?),
        LowCodeExpression::Subtract(left, right) => {
            Ok(evaluate_low_code_expression(left, context)?
                - evaluate_low_code_expression(right, context)?)
        }
        LowCodeExpression::Multiply(left, right) => {
            Ok(evaluate_low_code_expression(left, context)?
                * evaluate_low_code_expression(right, context)?)
        }
        LowCodeExpression::Divide(left, right) => {
            let divisor = evaluate_low_code_expression(right, context)?;
            if divisor == 0.0 {
                return Err(t("Division by zero"));
            }
            Ok(evaluate_low_code_expression(left, context)? / divisor)
        }
        LowCodeExpression::Call(name, arguments) => {
            let values = arguments
                .iter()
                .map(|argument| evaluate_low_code_expression(argument, context))
                .collect::<Result<Vec<_>, _>>()?;
            evaluate_low_code_function(name, &values)
        }
    }
}

fn evaluate_low_code_function(name: &str, arguments: &[f64]) -> Result<f64, String> {
    match (name, arguments) {
        ("resist", [resistance]) => {
            let resistance = resistance / 100.0;
            Ok(if resistance >= 0.0 {
                1.0 - resistance
            } else {
                1.0 - resistance / (1.0 - resistance)
            })
        }
        ("pct", [percent]) => Ok(percent / 100.0),
        ("abs", [value]) => Ok(value.abs()),
        ("round", [value]) => Ok(value.round()),
        ("floor", [value]) => Ok(value.floor()),
        ("ceil", [value]) => Ok(value.ceil()),
        ("min", [left, right]) => Ok(left.min(*right)),
        ("max", [left, right]) => Ok(left.max(*right)),
        ("lerp", [from, to, ratio]) => Ok(*from + (*to - *from) * *ratio),
        ("select", [condition, when_true, when_false]) => Ok(if *condition != 0.0 {
            *when_true
        } else {
            *when_false
        }),
        ("if_eq", [left, right, when_true, when_false]) => Ok(if left == right {
            *when_true
        } else {
            *when_false
        }),
        ("if_gt", [left, right, when_true, when_false]) => Ok(if left > right {
            *when_true
        } else {
            *when_false
        }),
        ("if_lt", [left, right, when_true, when_false]) => Ok(if left < right {
            *when_true
        } else {
            *when_false
        }),
        ("clamp", [value, minimum, maximum]) => {
            if minimum > maximum || !minimum.is_finite() || !maximum.is_finite() {
                return Err(t(
                    "clamp() requires a finite minimum not greater than maximum",
                ));
            }
            Ok(value.clamp(*minimum, *maximum))
        }
        ("pow", [base, exponent]) => Ok(base.powf(*exponent)),
        _ => Err(tf("Unknown function or argument count: {}", &[name])),
    }
}

fn low_code_component_matches(
    projection: &DamageProjectionEditorState,
    input: DamageTransformInput<'_>,
) -> bool {
    (projection.character_ids.is_empty() || projection.character_ids.contains(&input.char_id))
        && (projection.damage_attributes.is_empty()
            || input.damage_attribute.is_some_and(|attribute| {
                projection.damage_attributes.iter().any(|v| v == attribute)
            }))
        && (projection.attack_types.is_empty()
            || input.attack_type.is_some_and(|attack_type| {
                projection.attack_types.iter().any(|v| v == attack_type)
            }))
        && (projection.skill_names.is_empty()
            || input
                .skill_name
                .is_some_and(|skill| projection.skill_names.iter().any(|v| v == skill)))
}

fn blueprint_skill_options_for_hit(
    hit: &crate::engine::model::Hit,
) -> impl Iterator<Item = BlueprintSkillOption> {
    let primary_key = hit
        .ability_name
        .as_deref()
        .or(hit.damage_name.as_deref())
        .or(hit.damage_component.as_deref());
    let primary = primary_key.map(|key| BlueprintSkillOption {
        key: key.to_owned(),
        display_name: blueprint_skill_display_name(
            key,
            hit.ability_name.as_deref(),
            hit.gameplay_effect_name.as_deref(),
            hit.damage_component
                .as_deref()
                .or(hit.damage_name.as_deref()),
        ),
    });
    let follow_up = (hit.follow_up_damage != 0.0)
        .then(|| {
            hit.follow_up_damage_name
                .as_deref()
                .or(hit.ability_name.as_deref())
                .or(hit.damage_component.as_deref())
        })
        .flatten()
        .map(|key| BlueprintSkillOption {
            key: key.to_owned(),
            display_name: blueprint_skill_display_name(
                key,
                hit.ability_name.as_deref(),
                None,
                hit.follow_up_damage_name
                    .as_deref()
                    .or(hit.damage_component.as_deref()),
            ),
        });
    [primary, follow_up].into_iter().flatten()
}

fn blueprint_skill_display_name(
    key: &str,
    ability_name: Option<&str>,
    gameplay_effect_name: Option<&str>,
    fallback_name: Option<&str>,
) -> String {
    if ability_name == Some(key)
        && let Some(name) = crate::storage::ability_names::resolve_ability_name(key)
    {
        return name;
    }
    gameplay_effect_name
        .and_then(crate::storage::ability_names::resolve_damage_name)
        .or_else(|| crate::storage::ability_names::resolve_damage_name(key))
        .or_else(|| ability_name.and_then(crate::storage::ability_names::resolve_ability_name))
        .unwrap_or_else(|| translate_reaction_label(fallback_name.unwrap_or(key)))
}

fn mod_pipeline_chip(ui: &mut egui::Ui, label: &str, color: Color32, palette: ModEditorPalette) {
    egui::Frame::new()
        .fill(palette.chrome)
        .stroke(Stroke::new(1.0, color))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(t(label)).small().strong().color(color));
        });
}

impl BlueprintRuleTemplate {
    const ALL: [Self; 10] = [
        Self::Custom,
        Self::Scale,
        Self::Bonus,
        Self::Reduction,
        Self::Flat,
        Self::Clamp,
        Self::Resistance,
        Self::Threshold,
        Self::FollowUp,
        Self::Character,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Custom => "Custom expression",
            Self::Scale => "Scale damage",
            Self::Bonus => "Add damage bonus",
            Self::Reduction => "Reduce damage by percent",
            Self::Flat => "Add flat damage",
            Self::Clamp => "Clamp damage",
            Self::Resistance => "Simulate resistance",
            Self::Threshold => "Damage threshold",
            Self::FollowUp => "Follow-up damage condition",
            Self::Character => "Character ID condition",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Custom => "Start from the current damage value and write a custom expression.",
            Self::Scale => "Multiply damage by an exposed factor.",
            Self::Bonus => "Apply a percentage damage bonus.",
            Self::Reduction => "Apply a percentage damage reduction.",
            Self::Flat => "Add a fixed amount to every matching damage value.",
            Self::Clamp => "Keep the result between zero and the captured raw damage.",
            Self::Resistance => "Compare target resistance with a baseline resistance.",
            Self::Threshold => "Apply a factor only when raw damage is above a threshold.",
            Self::FollowUp => "Apply a factor only to follow-up damage components.",
            Self::Character => "Apply a factor only to one numeric character ID.",
        }
    }

    fn category(self) -> &'static str {
        match self {
            Self::Custom
            | Self::Scale
            | Self::Bonus
            | Self::Reduction
            | Self::Flat
            | Self::Clamp => "Basic transforms",
            Self::Resistance => "Simulation",
            Self::Threshold | Self::FollowUp | Self::Character => "Conditional transforms",
        }
    }

    fn expression(self) -> &'static str {
        match self {
            Self::Custom => "value",
            Self::Scale => "value * factor",
            Self::Bonus => "value * (1 + pct(bonus_percent))",
            Self::Reduction => "value * (1 - pct(reduction_percent))",
            Self::Flat => "value + flat_damage",
            Self::Clamp => "clamp(value, 0, raw)",
            Self::Resistance => "value * resist(target_resistance) / resist(base_resistance)",
            Self::Threshold => "if_gt(raw, threshold, value * factor, value)",
            Self::FollowUp => "select(is_follow_up, value * factor, value)",
            Self::Character => "if_eq(char_id, target_character_id, value * factor, value)",
        }
    }

    fn parameters(self) -> &'static [(&'static str, f64)] {
        match self {
            Self::Custom | Self::Clamp => &[],
            Self::Scale => &[("factor", 1.0)],
            Self::Bonus => &[("bonus_percent", 20.0)],
            Self::Reduction => &[("reduction_percent", 20.0)],
            Self::Flat => &[("flat_damage", 100.0)],
            Self::Resistance => &[("base_resistance", 0.0), ("target_resistance", 20.0)],
            Self::Threshold => &[("threshold", 1_000.0), ("factor", 1.2)],
            Self::FollowUp => &[("factor", 1.2)],
            Self::Character => &[("target_character_id", 0.0), ("factor", 1.2)],
        }
    }
}

fn blueprint_rule_template(rule: &LowCodeRule) -> BlueprintRuleTemplate {
    let expression = rule.expression.as_str();
    if expression.starts_with("value * factor") {
        BlueprintRuleTemplate::Scale
    } else if expression.starts_with("value * (1 + pct(bonus_percent") {
        BlueprintRuleTemplate::Bonus
    } else if expression.starts_with("value * (1 - pct(reduction_percent") {
        BlueprintRuleTemplate::Reduction
    } else if expression.starts_with("value + flat_damage") {
        BlueprintRuleTemplate::Flat
    } else if expression == BlueprintRuleTemplate::Clamp.expression() {
        BlueprintRuleTemplate::Clamp
    } else if expression.starts_with("value * resist(target_resistance")
        && expression.contains("/ resist(base_resistance")
    {
        BlueprintRuleTemplate::Resistance
    } else if expression.starts_with("if_gt(raw, threshold") {
        BlueprintRuleTemplate::Threshold
    } else if expression.starts_with("select(is_follow_up, value * factor") {
        BlueprintRuleTemplate::FollowUp
    } else if expression.starts_with("if_eq(char_id, target_character_id") {
        BlueprintRuleTemplate::Character
    } else {
        BlueprintRuleTemplate::Custom
    }
}

fn blueprint_node_palette(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    position: Option<egui::Pos2>,
) {
    ui.label(RichText::new(t("Transform nodes")).small().strong());
    let mut category = "";
    for template in BlueprintRuleTemplate::ALL {
        if category != template.category() {
            category = template.category();
            ui.separator();
            ui.label(RichText::new(t(category)).small().strong());
        }
        if ui
            .button(t(template.label()))
            .on_hover_text(t(template.description()))
            .clicked()
        {
            add_blueprint_rule(projection, template, position);
            ui.close();
        }
    }
}

fn add_blueprint_rule(
    projection: &mut DamageProjectionEditorState,
    template: BlueprintRuleTemplate,
    position: Option<egui::Pos2>,
) {
    let index = projection.rules.len();
    let rule_id = projection.next_rule_id;
    projection.next_rule_id += 1;
    let mut expression = template.expression().to_owned();
    for (name, value) in template.parameters() {
        let mut unique_name = (*name).to_owned();
        let mut suffix = 2;
        while projection
            .variables
            .iter()
            .any(|variable| variable.name == unique_name)
        {
            unique_name = format!("{name}_{suffix}");
            suffix += 1;
        }
        expression = expression.replace(name, &unique_name);
        projection.variables.push(LowCodeVariable {
            name: unique_name,
            value: *value,
        });
    }
    projection.rules.push(LowCodeRule {
        id: rule_id,
        enabled: true,
        target: LowCodeDamageTarget::All,
        expression,
    });
    projection
        .rule_node_positions
        .push(position.unwrap_or_else(|| egui::pos2(485.0 + index as f32 * 330.0, 70.0)));
    projection.output_node_position.x = projection
        .output_node_position
        .x
        .max(805.0 + index as f32 * 330.0);

    let predecessor = match projection.pending_connection {
        Some(BlueprintPendingConnection::Execution(from)) => Some(from),
        _ => projection
            .execution_edges
            .iter()
            .find(|edge| edge.to == BlueprintExecutionNode::Output)
            .map(|edge| edge.from),
    };
    if let Some(from) = predecessor {
        let old_target = projection
            .execution_edges
            .iter()
            .find(|edge| edge.from == from)
            .map(|edge| edge.to);
        connect_blueprint_execution_edge(projection, from, BlueprintExecutionNode::Rule(rule_id));
        if let Some(target) = old_target {
            connect_blueprint_execution_edge(
                projection,
                BlueprintExecutionNode::Rule(rule_id),
                target,
            );
        }
    }
    if !template.parameters().is_empty() {
        projection.parameter_connections.push(rule_id);
    }
    projection.pending_connection = None;
    projection.selected_node = LowCodeNodeSelection::Rule(index);
    projection.feedback = Some(tf("{} node added", &[&t(template.label())]));
}

fn blueprint_details_use_side_panel(available_width: f32) -> bool {
    available_width >= 960.0
}

fn blueprint_mod_toolbar(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    validation_error: Option<&str>,
    palette: ModEditorPalette,
) {
    let canvas_palette = blueprint_canvas_palette(ui.visuals().dark_mode);
    egui::Frame::new()
        .fill(palette.chrome)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t("Mod Blueprint"))
                        .size(15.0)
                        .strong()
                        .color(palette.text),
                );
                ui.separator();
                ui.menu_button(t("Add node"), |ui| {
                    blueprint_node_palette(ui, projection, None);
                });
                if ui.button(t("Add parameter")).clicked() {
                    projection.variables.push(LowCodeVariable {
                        name: format!("parameter_{}", projection.variables.len() + 1),
                        value: 0.0,
                    });
                    projection.selected_node = LowCodeNodeSelection::Variables;
                    projection.feedback = Some(t("Parameter added"));
                }
                if ui.button(t("Frame all")).clicked() {
                    projection.scene_rect = blueprint_graph_bounds(projection);
                    projection.feedback = Some(t("Graph framed"));
                }
                ui.separator();
                mod_pipeline_chip(ui, "Execution wire", canvas_palette.execution, palette);
                mod_pipeline_chip(ui, "Parameter wire", canvas_palette.data, palette);
                ui.label(RichText::new("ⓘ").strong().color(palette.muted))
                    .on_hover_text(format!(
                        "{}\n{}",
                        t("Drag blank canvas to pan · Ctrl+wheel to zoom"),
                        t(
                            "Click an output pin, then an input pin to connect; click a connected input pin to cut.",
                        ),
                    ));
                if projection.pending_connection.is_some()
                    && ui.button(t("Cancel connection")).clicked()
                {
                    projection.pending_connection = None;
                    projection.feedback = Some(t("Connection cancelled"));
                }
                ui.separator();
                let button_text = if projection.applied_to_home {
                    t("Stop Blueprint Mod")
                } else {
                    t("Run Blueprint Mod")
                };
                let button_fill = if projection.applied_to_home {
                    palette.editor
                } else {
                    palette.selected_border
                };
                if ui
                    .add_enabled(
                        projection.applied_to_home || validation_error.is_none(),
                        egui::Button::new(
                            RichText::new(button_text).color(contrast_text(button_fill)),
                        )
                        .fill(button_fill),
                    )
                    .clicked()
                {
                    projection.applied_to_home = !projection.applied_to_home;
                    projection.feedback = Some(if projection.applied_to_home {
                        t("Blueprint applied to analysis views")
                    } else {
                        t("Blueprint stopped")
                    });
                }
                if let Some(error) = validation_error {
                    ui.label(
                        RichText::new(error)
                            .small()
                            .color(semantic_warning(ui.visuals().dark_mode)),
                    );
                } else if let Some(feedback) = &projection.feedback {
                    ui.label(
                        RichText::new(format!("✓ {feedback}"))
                            .small()
                            .strong()
                            .color(semantic_success(ui.visuals().dark_mode)),
                    );
                }
            });
        });
}

fn blueprint_graph_bounds(projection: &DamageProjectionEditorState) -> egui::Rect {
    let mut bounds =
        egui::Rect::from_min_size(projection.event_node_position, egui::vec2(176.0, 98.0));
    for (position, size) in [
        (projection.match_node_position, egui::vec2(214.0, 112.0)),
        (projection.variable_node_position, egui::vec2(214.0, 108.0)),
        (projection.output_node_position, egui::vec2(188.0, 102.0)),
    ] {
        bounds = bounds.union(egui::Rect::from_min_size(position, size));
    }
    for position in &projection.rule_node_positions {
        bounds = bounds.union(egui::Rect::from_min_size(
            *position,
            egui::vec2(300.0, 122.0),
        ));
    }
    bounds.expand(60.0)
}

#[derive(Clone, Copy)]
struct BlueprintNodeStyle {
    size: egui::Vec2,
    header: Color32,
    input_pin: bool,
    output_pin: bool,
    data_pin: bool,
}

#[derive(Clone, Copy)]
struct BlueprintCanvasPalette {
    background: Color32,
    grid_minor: Color32,
    grid_major: Color32,
    node_fill: Color32,
    node_border: Color32,
    text: Color32,
    muted: Color32,
    execution: Color32,
    data: Color32,
    selection: Color32,
}

fn blueprint_canvas_palette(dark_mode: bool) -> BlueprintCanvasPalette {
    if dark_mode {
        BlueprintCanvasPalette {
            background: Color32::from_rgb(20, 23, 28),
            grid_minor: Color32::from_rgba_unmultiplied(255, 255, 255, 10),
            grid_major: Color32::from_rgba_unmultiplied(255, 255, 255, 22),
            node_fill: Color32::from_rgb(34, 38, 45),
            node_border: Color32::from_rgb(78, 84, 94),
            text: Color32::from_rgb(232, 236, 243),
            muted: Color32::from_rgb(164, 174, 188),
            execution: Color32::from_rgb(214, 220, 230),
            data: Color32::from_rgb(64, 190, 210),
            selection: Color32::from_rgb(92, 174, 255),
        }
    } else {
        BlueprintCanvasPalette {
            background: Color32::from_rgb(241, 244, 248),
            grid_minor: Color32::from_rgba_unmultiplied(45, 55, 70, 16),
            grid_major: Color32::from_rgba_unmultiplied(45, 55, 70, 34),
            node_fill: Color32::from_rgb(252, 253, 255),
            node_border: Color32::from_rgb(174, 182, 194),
            text: Color32::from_rgb(34, 40, 50),
            muted: Color32::from_rgb(92, 102, 116),
            execution: Color32::from_rgb(73, 82, 96),
            data: Color32::from_rgb(20, 142, 166),
            selection: Color32::from_rgb(35, 118, 210),
        }
    }
}

fn blueprint_mod_canvas(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    palette: ModEditorPalette,
) {
    let mut canvas_palette = blueprint_canvas_palette(ui.visuals().dark_mode);
    canvas_palette.selection = palette.selected_border;
    let mut scene_rect = projection.scene_rect;
    let grid_rect = scene_rect.expand(96.0);
    let canvas_size = egui::vec2(ui.available_width(), 500.0);
    let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
    let canvas_clip = canvas_rect.intersect(ui.clip_rect());
    let mut canvas_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(canvas_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    canvas_ui.set_clip_rect(canvas_clip);
    egui::Frame::new()
        .fill(canvas_palette.background)
        .stroke(Stroke::new(1.0, palette.border))
        .corner_radius(8)
        .show(&mut canvas_ui, |ui| {
            // Scene expands its layer clip to the full canvas, which leaks through a parent
            // ScrollArea. Keep its pan/zoom controller but own the layer clip here.
            let (outer_rect, _) =
                ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::hover());
            let zoom_range = egui::Rangef::new(0.45, 2.0);
            let scale = (outer_rect.size() / scene_rect.size())
                .min_elem()
                .clamp(zoom_range.min, zoom_range.max);
            let mut to_global = egui::emath::TSTransform::from_translation(
                outer_rect.center().to_vec2() - scale * scene_rect.center().to_vec2(),
            ) * egui::emath::TSTransform::from_scaling(scale);
            let scene_layer_id =
                egui::LayerId::new(ui.layer_id().order, ui.id().with("blueprint_scene_area"));
            ui.ctx().set_sublayer(ui.layer_id(), scene_layer_id);
            let visible_clip = outer_rect.intersect(ui.clip_rect());
            let mut scene_ui =
                ui.new_child(egui::UiBuilder::new().layer_id(scene_layer_id).max_rect(
                    egui::Rect::from_center_size(egui::Pos2::ZERO, egui::vec2(8_000.0, 8_000.0)),
                ));
            scene_ui.set_clip_rect(to_global.inverse() * visible_clip);
            scene_ui
                .ctx()
                .set_transform_layer(scene_layer_id, to_global);
            let mut pan_response = scene_ui.interact(
                to_global.inverse() * outer_rect,
                scene_ui.id().with("blueprint_scene_background"),
                egui::Sense::click_and_drag(),
            );
            egui::Scene::new()
                .zoom_range(zoom_range)
                .drag_pan_buttons(egui::DragPanButtons::PRIMARY)
                .register_pan_and_zoom(&scene_ui, &mut pan_response, &mut to_global);
            scene_ui.set_clip_rect(to_global.inverse() * visible_clip);
            scene_ui
                .ctx()
                .set_transform_layer(scene_layer_id, to_global);
            {
                let ui = &mut scene_ui;
                paint_blueprint_grid(ui.painter(), grid_rect, canvas_palette);
                paint_blueprint_wires(ui.painter(), egui::Pos2::ZERO, projection, canvas_palette);
                if let Some(pointer_global) = ui.input(|input| input.pointer.hover_pos())
                    && visible_clip.contains(pointer_global)
                {
                    let pointer_scene = to_global.inverse() * pointer_global;
                    match projection.pending_connection {
                        Some(BlueprintPendingConnection::Execution(from)) => {
                            let start = blueprint_execution_output_position(projection, from)
                                .expect("pending execution connections start at an output pin");
                            paint_blueprint_wire(
                                ui.painter(),
                                start,
                                pointer_scene,
                                canvas_palette.execution,
                            );
                        }
                        Some(BlueprintPendingConnection::Parameters) => {
                            paint_blueprint_wire(
                                ui.painter(),
                                blueprint_parameter_output_position(projection),
                                pointer_scene,
                                canvas_palette.data,
                            );
                        }
                        None => {}
                    }
                }
                let match_summary = blueprint_match_summary(projection);
                let variable_summary = tf(
                    "{} exposed values",
                    &[&projection.variables.len().to_string()],
                );

                let response = blueprint_node(
                    ui,
                    egui::Pos2::ZERO,
                    &mut projection.event_node_position,
                    "blueprint_event_node",
                    BlueprintGraphNode::Event,
                    "EVENT",
                    "Damage Hit",
                    &["damage.hit", "Captured outgoing damage"],
                    projection.selected_node == LowCodeNodeSelection::Event,
                    projection.pending_connection,
                    BlueprintNodeStyle {
                        size: egui::vec2(176.0, 98.0),
                        header: Color32::from_rgb(168, 48, 58),
                        input_pin: false,
                        output_pin: true,
                        data_pin: false,
                    },
                    canvas_palette,
                );
                if response.selected {
                    projection.selected_node = LowCodeNodeSelection::Event;
                }
                if let Some(action) = response.pin_action {
                    handle_blueprint_pin_action(projection, action);
                }
                let response = blueprint_node(
                    ui,
                    egui::Pos2::ZERO,
                    &mut projection.match_node_position,
                    "blueprint_match_node",
                    BlueprintGraphNode::Match,
                    "BRANCH",
                    "Match Damage",
                    &[&match_summary, "Pass matching components"],
                    projection.selected_node == LowCodeNodeSelection::Match,
                    projection.pending_connection,
                    BlueprintNodeStyle {
                        size: egui::vec2(214.0, 112.0),
                        header: Color32::from_rgb(111, 74, 156),
                        input_pin: true,
                        output_pin: true,
                        data_pin: false,
                    },
                    canvas_palette,
                );
                if response.selected {
                    projection.selected_node = LowCodeNodeSelection::Match;
                }
                if let Some(action) = response.pin_action {
                    handle_blueprint_pin_action(projection, action);
                }
                let response = blueprint_node(
                    ui,
                    egui::Pos2::ZERO,
                    &mut projection.variable_node_position,
                    "blueprint_variable_node",
                    BlueprintGraphNode::Variables,
                    "VARIABLES",
                    "Parameters",
                    &[&variable_summary, "Feeds expression nodes"],
                    projection.selected_node == LowCodeNodeSelection::Variables,
                    projection.pending_connection,
                    BlueprintNodeStyle {
                        size: egui::vec2(214.0, 108.0),
                        header: Color32::from_rgb(42, 128, 142),
                        input_pin: false,
                        output_pin: false,
                        data_pin: true,
                    },
                    canvas_palette,
                );
                if response.selected {
                    projection.selected_node = LowCodeNodeSelection::Variables;
                }
                if let Some(action) = response.pin_action {
                    handle_blueprint_pin_action(projection, action);
                }

                for index in 0..projection.rules.len() {
                    let rule = projection.rules[index].clone();
                    let expression = if rule.expression.chars().count() > 30 {
                        format!("{}…", rule.expression.chars().take(30).collect::<String>())
                    } else {
                        rule.expression.clone()
                    };
                    let response = blueprint_node(
                        ui,
                        egui::Pos2::ZERO,
                        &mut projection.rule_node_positions[index],
                        ("blueprint_rule_node", rule.id),
                        BlueprintGraphNode::Rule(rule.id),
                        if rule.enabled { "SET" } else { "BYPASSED" },
                        blueprint_rule_template(&rule).label(),
                        &[low_code_target_label(rule.target), &expression],
                        projection.selected_node == LowCodeNodeSelection::Rule(index),
                        projection.pending_connection,
                        BlueprintNodeStyle {
                            size: egui::vec2(300.0, 122.0),
                            header: if rule.enabled {
                                Color32::from_rgb(45, 120, 82)
                            } else {
                                Color32::from_rgb(92, 98, 108)
                            },
                            input_pin: true,
                            output_pin: true,
                            data_pin: true,
                        },
                        canvas_palette,
                    );
                    if response.selected {
                        projection.selected_node = LowCodeNodeSelection::Rule(index);
                    }
                    if let Some(action) = response.pin_action {
                        handle_blueprint_pin_action(projection, action);
                    }
                }

                let response = blueprint_node(
                    ui,
                    egui::Pos2::ZERO,
                    &mut projection.output_node_position,
                    "blueprint_output_node",
                    BlueprintGraphNode::Output,
                    "EMIT",
                    "Analysis Views",
                    &["Homepage · HUD", "Skills · Timeline"],
                    projection.selected_node == LowCodeNodeSelection::Output,
                    projection.pending_connection,
                    BlueprintNodeStyle {
                        size: egui::vec2(188.0, 102.0),
                        header: Color32::from_rgb(170, 112, 37),
                        input_pin: true,
                        output_pin: false,
                        data_pin: false,
                    },
                    canvas_palette,
                );
                if response.selected {
                    projection.selected_node = LowCodeNodeSelection::Output;
                }
                if let Some(action) = response.pin_action {
                    handle_blueprint_pin_action(projection, action);
                }
            }
            let context_position = ui
                .input(|input| input.pointer.hover_pos())
                .map(|position| to_global.inverse() * position);
            pan_response.context_menu(|ui| {
                ui.label(RichText::new(t("Add node at cursor")).strong());
                blueprint_node_palette(ui, projection, context_position);
            });
            if pan_response.changed() {
                scene_rect = to_global.inverse() * outer_rect;
            }
            let pointer_over_canvas = ui
                .input(|input| input.pointer.hover_pos())
                .is_some_and(|position| visible_clip.contains(position));
            if pointer_over_canvas {
                // Scene already consumed this frame's wheel input for pan/zoom.
                // Clear it before the parent page ScrollArea handles the same event.
                ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        });
    projection.scene_rect = scene_rect;
}

fn paint_blueprint_grid(
    painter: &egui::Painter,
    canvas: egui::Rect,
    palette: BlueprintCanvasPalette,
) {
    let step = 24.0;
    let first_column = (canvas.left() / step).floor() as i32;
    let last_column = (canvas.right() / step).ceil() as i32;
    let first_row = (canvas.top() / step).floor() as i32;
    let last_row = (canvas.bottom() / step).ceil() as i32;
    for column in first_column..=last_column {
        let x = column as f32 * step;
        painter.line_segment(
            [egui::pos2(x, canvas.top()), egui::pos2(x, canvas.bottom())],
            Stroke::new(
                1.0,
                if column.rem_euclid(4) == 0 {
                    palette.grid_major
                } else {
                    palette.grid_minor
                },
            ),
        );
    }
    for row in first_row..=last_row {
        let y = row as f32 * step;
        painter.line_segment(
            [egui::pos2(canvas.left(), y), egui::pos2(canvas.right(), y)],
            Stroke::new(
                1.0,
                if row.rem_euclid(4) == 0 {
                    palette.grid_major
                } else {
                    palette.grid_minor
                },
            ),
        );
    }
}

fn paint_blueprint_wires(
    painter: &egui::Painter,
    origin: egui::Pos2,
    projection: &DamageProjectionEditorState,
    palette: BlueprintCanvasPalette,
) {
    for edge in &projection.execution_edges {
        let start = blueprint_execution_output_position(projection, edge.from)
            .expect("execution edge starts at an output pin");
        let end = blueprint_execution_input_position(projection, edge.to)
            .expect("execution edge ends at an input pin");
        paint_blueprint_wire(
            painter,
            origin + start.to_vec2(),
            origin + end.to_vec2(),
            palette.execution,
        );
    }
    let data_start = origin + blueprint_parameter_output_position(projection).to_vec2();
    for rule_id in &projection.parameter_connections {
        let data_input = blueprint_parameter_input_position(projection, *rule_id)
            .expect("parameter connections reference existing rule nodes");
        paint_blueprint_wire(
            painter,
            data_start,
            origin + data_input.to_vec2(),
            palette.data,
        );
    }
}

fn blueprint_execution_input_position(
    projection: &DamageProjectionEditorState,
    node: BlueprintExecutionNode,
) -> Option<egui::Pos2> {
    match node {
        BlueprintExecutionNode::Event => None,
        BlueprintExecutionNode::Match => {
            Some(projection.match_node_position + egui::vec2(0.0, 50.0))
        }
        BlueprintExecutionNode::Rule(rule_id) => blueprint_rule_position(projection, rule_id)
            .map(|position| position + egui::vec2(0.0, 50.0)),
        BlueprintExecutionNode::Output => {
            Some(projection.output_node_position + egui::vec2(0.0, 50.0))
        }
    }
}

fn blueprint_execution_output_position(
    projection: &DamageProjectionEditorState,
    node: BlueprintExecutionNode,
) -> Option<egui::Pos2> {
    match node {
        BlueprintExecutionNode::Event => {
            Some(projection.event_node_position + egui::vec2(176.0, 50.0))
        }
        BlueprintExecutionNode::Match => {
            Some(projection.match_node_position + egui::vec2(214.0, 50.0))
        }
        BlueprintExecutionNode::Rule(rule_id) => blueprint_rule_position(projection, rule_id)
            .map(|position| position + egui::vec2(300.0, 50.0)),
        BlueprintExecutionNode::Output => None,
    }
}

fn blueprint_parameter_output_position(projection: &DamageProjectionEditorState) -> egui::Pos2 {
    projection.variable_node_position + egui::vec2(214.0, 90.0)
}

fn blueprint_parameter_input_position(
    projection: &DamageProjectionEditorState,
    rule_id: u64,
) -> Option<egui::Pos2> {
    blueprint_rule_position(projection, rule_id).map(|position| position + egui::vec2(0.0, 104.0))
}

fn blueprint_rule_position(
    projection: &DamageProjectionEditorState,
    rule_id: u64,
) -> Option<egui::Pos2> {
    projection
        .rules
        .iter()
        .position(|rule| rule.id == rule_id)
        .map(|index| projection.rule_node_positions[index])
}

fn paint_blueprint_wire(
    painter: &egui::Painter,
    start: egui::Pos2,
    end: egui::Pos2,
    color: Color32,
) {
    let control_offset = ((end.x - start.x).abs() * 0.45).max(60.0);
    let control_a = start + egui::vec2(control_offset, 0.0);
    let control_b = end - egui::vec2(control_offset, 0.0);
    let points = (0..=24)
        .map(|index| {
            let t = index as f32 / 24.0;
            let inverse = 1.0 - t;
            start.to_vec2() * inverse.powi(3)
                + control_a.to_vec2() * (3.0 * inverse.powi(2) * t)
                + control_b.to_vec2() * (3.0 * inverse * t.powi(2))
                + end.to_vec2() * t.powi(3)
        })
        .map(|point| egui::pos2(point.x, point.y))
        .collect::<Vec<_>>();
    painter.add(egui::Shape::line(points, Stroke::new(2.0, color)));
}

#[allow(clippy::too_many_arguments)]
fn blueprint_node(
    ui: &mut egui::Ui,
    canvas_origin: egui::Pos2,
    position: &mut egui::Pos2,
    id_source: impl std::hash::Hash,
    graph_node: BlueprintGraphNode,
    category: &str,
    title: &str,
    lines: &[&str],
    selected: bool,
    pending_connection: Option<BlueprintPendingConnection>,
    style: BlueprintNodeStyle,
    palette: BlueprintCanvasPalette,
) -> BlueprintNodeResponse {
    let rect = egui::Rect::from_min_size(canvas_origin + position.to_vec2(), style.size);
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 30.0));
    let response = ui.interact(rect, ui.id().with(id_source), egui::Sense::click_and_drag());
    if response.dragged() {
        *position += response.drag_delta();
    }

    ui.painter().rect(
        rect,
        7.0,
        palette.node_fill,
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected {
                palette.selection
            } else {
                palette.node_border
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(header, 7.0, style.header);
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(header.left(), header.bottom() - 7.0),
            header.right_bottom(),
        ),
        0.0,
        style.header,
    );
    ui.painter().text(
        header.left_center() + egui::vec2(9.0, 0.0),
        egui::Align2::LEFT_CENTER,
        t(category),
        egui::FontId::proportional(10.0),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.left_top() + egui::vec2(10.0, 40.0),
        egui::Align2::LEFT_TOP,
        t(title),
        egui::FontId::proportional(14.0),
        palette.text,
    );
    for (index, line) in lines.iter().enumerate() {
        ui.painter().text(
            rect.left_top() + egui::vec2(10.0, 64.0 + index as f32 * 18.0),
            egui::Align2::LEFT_TOP,
            t(line),
            egui::FontId::monospace(10.5),
            palette.muted,
        );
    }
    let mut node_response = BlueprintNodeResponse {
        selected: response.clicked() || response.drag_started(),
        pin_action: None,
    };
    let execution_node = match graph_node {
        BlueprintGraphNode::Event => Some(BlueprintExecutionNode::Event),
        BlueprintGraphNode::Match => Some(BlueprintExecutionNode::Match),
        BlueprintGraphNode::Rule(rule_id) => Some(BlueprintExecutionNode::Rule(rule_id)),
        BlueprintGraphNode::Output => Some(BlueprintExecutionNode::Output),
        BlueprintGraphNode::Variables => None,
    };
    if style.input_pin {
        let node = execution_node.expect("execution input pin belongs to an execution node");
        let center = egui::pos2(rect.left(), rect.top() + 50.0);
        node_response.pin_action = blueprint_pin_interaction(
            ui,
            ("execution_input", node),
            center,
            BlueprintPin::ExecutionInput(node),
            false,
            matches!(
                pending_connection,
                Some(BlueprintPendingConnection::Execution(_))
            ),
            palette.execution,
        );
    }
    if style.output_pin {
        let node = execution_node.expect("execution output pin belongs to an execution node");
        let center = egui::pos2(rect.right(), rect.top() + 50.0);
        node_response.pin_action = node_response.pin_action.or_else(|| {
            blueprint_pin_interaction(
                ui,
                ("execution_output", node),
                center,
                BlueprintPin::ExecutionOutput(node),
                pending_connection == Some(BlueprintPendingConnection::Execution(node)),
                false,
                palette.execution,
            )
        });
    }
    if style.data_pin {
        let (center, pin, pending, compatible) = match graph_node {
            BlueprintGraphNode::Variables => (
                egui::pos2(rect.right(), rect.bottom() - 18.0),
                BlueprintPin::ParameterOutput,
                pending_connection == Some(BlueprintPendingConnection::Parameters),
                false,
            ),
            BlueprintGraphNode::Rule(rule_id) => (
                egui::pos2(rect.left(), rect.bottom() - 18.0),
                BlueprintPin::ParameterInput(rule_id),
                false,
                pending_connection == Some(BlueprintPendingConnection::Parameters),
            ),
            _ => unreachable!("data pins only belong to parameter and transform nodes"),
        };
        node_response.pin_action = node_response.pin_action.or_else(|| {
            blueprint_pin_interaction(
                ui,
                ("parameter_pin", graph_node),
                center,
                pin,
                pending,
                compatible,
                palette.data,
            )
        });
    }
    node_response
}

fn blueprint_pin_interaction(
    ui: &mut egui::Ui,
    id_source: impl std::hash::Hash,
    center: egui::Pos2,
    pin: BlueprintPin,
    pending: bool,
    compatible: bool,
    color: Color32,
) -> Option<BlueprintPinAction> {
    let response = ui
        .interact(
            egui::Rect::from_center_size(center, egui::vec2(18.0, 18.0)),
            ui.id().with(id_source),
            egui::Sense::click(),
        )
        .on_hover_text(t(
            "Click to connect or cut this pin; right-click to cut its current wire.",
        ));
    let hovered_compatible = compatible && response.hovered();
    let pin_color = if hovered_compatible {
        semantic_success(ui.visuals().dark_mode)
    } else {
        color
    };
    ui.painter().circle_filled(
        center,
        if pending {
            8.0
        } else if hovered_compatible {
            7.0
        } else {
            5.0
        },
        pin_color,
    );
    if response.secondary_clicked() {
        Some(BlueprintPinAction {
            pin,
            disconnect: true,
        })
    } else if response.clicked() {
        Some(BlueprintPinAction {
            pin,
            disconnect: false,
        })
    } else {
        None
    }
}

fn handle_blueprint_pin_action(
    projection: &mut DamageProjectionEditorState,
    action: BlueprintPinAction,
) {
    match action.pin {
        BlueprintPin::ExecutionOutput(from) => {
            if action.disconnect {
                projection.execution_edges.retain(|edge| edge.from != from);
                projection.pending_connection = None;
                projection.feedback = Some(t("Execution wire cut"));
            } else if projection.pending_connection
                == Some(BlueprintPendingConnection::Execution(from))
            {
                projection.pending_connection = None;
                projection.feedback = Some(t("Connection cancelled"));
            } else {
                projection.pending_connection = Some(BlueprintPendingConnection::Execution(from));
                projection.feedback = Some(t("Choose a compatible execution input"));
            }
        }
        BlueprintPin::ExecutionInput(to) => {
            if !action.disconnect
                && let Some(BlueprintPendingConnection::Execution(from)) =
                    projection.pending_connection
            {
                connect_blueprint_execution_edge(projection, from, to);
                projection.feedback = Some(t("Execution wire connected"));
            } else {
                projection.execution_edges.retain(|edge| edge.to != to);
                projection.feedback = Some(t("Execution wire cut"));
            }
            projection.pending_connection = None;
        }
        BlueprintPin::ParameterOutput => {
            if action.disconnect {
                projection.parameter_connections.clear();
                projection.pending_connection = None;
                projection.feedback = Some(t("Parameter wires cut"));
            } else if projection.pending_connection == Some(BlueprintPendingConnection::Parameters)
            {
                projection.pending_connection = None;
                projection.feedback = Some(t("Connection cancelled"));
            } else {
                projection.pending_connection = Some(BlueprintPendingConnection::Parameters);
                projection.feedback = Some(t("Choose a parameter input"));
            }
        }
        BlueprintPin::ParameterInput(rule_id) => {
            if !action.disconnect
                && projection.pending_connection == Some(BlueprintPendingConnection::Parameters)
            {
                if !projection.parameter_connections.contains(&rule_id) {
                    projection.parameter_connections.push(rule_id);
                }
                projection.feedback = Some(t("Parameter wire connected"));
            } else {
                projection
                    .parameter_connections
                    .retain(|connected| *connected != rule_id);
                projection.feedback = Some(t("Parameter wire cut"));
            }
            projection.pending_connection = None;
        }
    }
}

fn connect_blueprint_execution_edge(
    projection: &mut DamageProjectionEditorState,
    from: BlueprintExecutionNode,
    to: BlueprintExecutionNode,
) {
    projection
        .execution_edges
        .retain(|edge| edge.from != from && edge.to != to);
    projection
        .execution_edges
        .push(BlueprintExecutionEdge { from, to });
}

fn blueprint_match_summary(projection: &DamageProjectionEditorState) -> String {
    let summary = [
        blueprint_filter_count_summary(projection.character_ids.len()),
        blueprint_filter_count_summary(projection.damage_attributes.len()),
        blueprint_filter_count_summary(projection.attack_types.len()),
        blueprint_filter_count_summary(projection.skill_names.len()),
    ]
    .join(" · ");
    if summary.chars().count() > 30 {
        format!("{}…", summary.chars().take(30).collect::<String>())
    } else {
        summary
    }
}

fn blueprint_filter_count_summary(selected: usize) -> String {
    if selected == 0 {
        "*".to_owned()
    } else {
        selected.to_string()
    }
}

fn blueprint_mod_inspector(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    characters: &[(u32, String)],
    attributes: &[String],
    attack_types: &[String],
    skills: &[BlueprintSkillOption],
    palette: ModEditorPalette,
) {
    egui::Frame::new()
        .fill(palette.editor)
        .stroke(Stroke::new(1.0_f32, palette.selected_border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t("Node Details"))
                        .size(15.0)
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    RichText::new(t("Select and drag nodes on the graph; edit the selected node here."))
                        .color(palette.muted),
                );
            });
            ui.add_space(7.0);
            match projection.selected_node {
                LowCodeNodeSelection::Event => {
                    blueprint_readonly_node_details(
                        ui,
                        "Damage Hit Event",
                        "Starts once for every captured outgoing primary or follow-up damage component.",
                        "damage.hit(value, char_id, timestamp, is_follow_up)",
                        palette,
                    );
                }
                LowCodeNodeSelection::Match => blueprint_match_inspector(
                    ui,
                    projection,
                    characters,
                    attributes,
                    attack_types,
                    skills,
                    palette,
                ),
                LowCodeNodeSelection::Variables => {
                    blueprint_variable_inspector(ui, projection, palette);
                }
                LowCodeNodeSelection::Rule(index) => {
                    blueprint_rule_inspector(ui, projection, index, palette);
                }
                LowCodeNodeSelection::Output => {
                    blueprint_readonly_node_details(
                        ui,
                        "Analysis Views Output",
                        "Routes the transformed CombatState copy into homepage, HUD, character, skill and timeline views.",
                        "emit analysis.views",
                        palette,
                    );
                }
            }
        });
}

fn blueprint_readonly_node_details(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    signature: &str,
    palette: ModEditorPalette,
) {
    ui.label(RichText::new(t(title)).strong().color(palette.text));
    ui.label(RichText::new(t(description)).color(palette.muted));
    ui.add_space(4.0);
    ui.label(
        RichText::new(signature)
            .monospace()
            .color(palette.selected_border),
    );
}

fn blueprint_match_inspector(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    characters: &[(u32, String)],
    attributes: &[String],
    attack_types: &[String],
    skills: &[BlueprintSkillOption],
    palette: ModEditorPalette,
) {
    ui.label(
        RichText::new(t("Structured WHERE conditions"))
            .strong()
            .color(palette.text),
    );
    ui.horizontal_wrapped(|ui| {
        ui.label(t("Character")).on_hover_text(t(
            "Only damage caused by selected characters enters this branch.",
        ));
        egui::ComboBox::from_id_salt("blueprint_match_character")
            .width(180.0)
            .height(300.0)
            .selected_text(blueprint_selection_label(
                projection.character_ids.len(),
                "All characters",
            ))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(projection.character_ids.is_empty(), t("All characters"))
                    .clicked()
                {
                    projection.character_ids.clear();
                }
                ui.separator();
                for (char_id, name) in characters {
                    let mut selected = projection.character_ids.contains(char_id);
                    if ui.checkbox(&mut selected, name).changed() {
                        if selected {
                            projection.character_ids.push(*char_id);
                            projection.character_ids.sort_unstable();
                        } else {
                            projection.character_ids.retain(|id| id != char_id);
                        }
                    }
                }
            });
        ui.label(t("Attribute")).on_hover_text(t(
            "The elemental or damage attribute recorded on each captured component.",
        ));
        egui::ComboBox::from_id_salt("blueprint_match_attribute")
            .width(150.0)
            .height(300.0)
            .selected_text(blueprint_selection_label(
                projection.damage_attributes.len(),
                "All attributes",
            ))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(projection.damage_attributes.is_empty(), t("All attributes"))
                    .clicked()
                {
                    projection.damage_attributes.clear();
                }
                ui.separator();
                for attribute in attributes {
                    let mut selected = projection.damage_attributes.contains(attribute);
                    if ui.checkbox(&mut selected, attribute).changed() {
                        toggle_blueprint_string_selection(
                            &mut projection.damage_attributes,
                            attribute,
                            selected,
                        );
                    }
                }
            });
        ui.label(t("Damage source")).on_hover_text(t(
            "The captured attack type, such as a normal attack, skill, or other source.",
        ));
        egui::ComboBox::from_id_salt("blueprint_match_source")
            .width(180.0)
            .height(300.0)
            .selected_text(blueprint_selection_label(
                projection.attack_types.len(),
                "All damage sources",
            ))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(projection.attack_types.is_empty(), t("All damage sources"))
                    .clicked()
                {
                    projection.attack_types.clear();
                }
                ui.separator();
                for attack_type in attack_types {
                    let mut selected = projection.attack_types.contains(attack_type);
                    if ui.checkbox(&mut selected, attack_type).changed() {
                        toggle_blueprint_string_selection(
                            &mut projection.attack_types,
                            attack_type,
                            selected,
                        );
                    }
                }
            });
        ui.label(t("Skill")).on_hover_text(t(
            "The localized skill name; hover an option to see its stable internal key.",
        ));
        egui::ComboBox::from_id_salt("blueprint_match_skill")
            .width(240.0)
            .height(320.0)
            .selected_text(blueprint_skill_selection_label(
                &projection.skill_names,
                skills,
            ))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(projection.skill_names.is_empty(), t("All skills"))
                    .clicked()
                {
                    projection.skill_names.clear();
                }
                ui.separator();
                for skill in skills {
                    let mut selected = projection.skill_names.contains(&skill.key);
                    if ui
                        .checkbox(&mut selected, &skill.display_name)
                        .on_hover_text(&skill.key)
                        .changed()
                    {
                        toggle_blueprint_string_selection(
                            &mut projection.skill_names,
                            &skill.key,
                            selected,
                        );
                    }
                }
            });
    });
    ui.label(
        RichText::new(t(
            "The branch forwards matching damage components and leaves all other captured data unchanged.",
        ))
        .small()
        .color(palette.muted),
    );
}

fn blueprint_selection_label(selected: usize, all_key: &str) -> String {
    if selected == 0 {
        t(all_key)
    } else {
        tf("{} selected", &[&selected.to_string()])
    }
}

fn blueprint_skill_selection_label(selected: &[String], skills: &[BlueprintSkillOption]) -> String {
    match selected {
        [] => t("All skills"),
        [key] => skills
            .iter()
            .find(|skill| skill.key == *key)
            .map(|skill| skill.display_name.clone())
            .unwrap_or_else(|| key.clone()),
        _ => tf("{} selected", &[&selected.len().to_string()]),
    }
}

fn toggle_blueprint_string_selection(selections: &mut Vec<String>, value: &str, selected: bool) {
    if selected {
        selections.push(value.to_owned());
        selections.sort();
        selections.dedup();
    } else {
        selections.retain(|selection| selection != value);
    }
}

fn blueprint_variable_inspector(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    palette: ModEditorPalette,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(t("Exposed parameters / LET"))
                .strong()
                .color(palette.text),
        );
        ui.label(
            RichText::new(t(
                "These are the spreadsheet-like cells users tune without changing rule expressions.",
            ))
            .color(palette.muted),
        );
        if ui.button(t("Add parameter")).clicked() {
            projection.variables.push(LowCodeVariable {
                name: format!("parameter_{}", projection.variables.len() + 1),
                value: 0.0,
            });
            projection.feedback = Some(t("Parameter added"));
        }
    });
    ui.add_space(6.0);
    let mut remove = None;
    for (index, variable) in projection.variables.iter_mut().enumerate() {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("let")
                    .monospace()
                    .color(palette.selected_border),
            );
            ui.add(
                egui::TextEdit::singleline(&mut variable.name)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(180.0),
            );
            ui.label(RichText::new("=").monospace());
            ui.add(
                egui::DragValue::new(&mut variable.value)
                    .speed(0.1)
                    .max_decimals(4),
            );
            if ui
                .small_button("×")
                .on_hover_text(t("Remove parameter"))
                .clicked()
            {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        projection.variables.remove(index);
    }
}

fn blueprint_rule_inspector(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    index: usize,
    palette: ModEditorPalette,
) {
    if index >= projection.rules.len() {
        projection.selected_node = LowCodeNodeSelection::Event;
        return;
    }
    let mut remove = false;
    {
        let rule = &mut projection.rules[index];
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut rule.enabled, t("Enabled"));
            ui.label(
                RichText::new(t(blueprint_rule_template(rule).label()))
                    .strong()
                    .color(palette.text),
            );
            egui::ComboBox::from_id_salt(("blueprint_rule_target", index))
                .selected_text(t(low_code_target_label(rule.target)))
                .show_ui(ui, |ui| {
                    for target in [
                        LowCodeDamageTarget::All,
                        LowCodeDamageTarget::Primary,
                        LowCodeDamageTarget::FollowUp,
                    ] {
                        ui.selectable_value(
                            &mut rule.target,
                            target,
                            t(low_code_target_label(target)),
                        );
                    }
                });
            if ui.button(t("Delete node")).clicked() {
                remove = true;
            }
        });
        ui.label(
            RichText::new(t(
                "The target chooses which damage component this expression changes.",
            ))
            .small()
            .color(palette.muted),
        );
        ui.add_space(6.0);
        ui.label(
            RichText::new(t("Expression"))
                .small()
                .strong()
                .color(palette.muted),
        );
        ui.add(
            egui::TextEdit::singleline(&mut rule.expression)
                .font(egui::TextStyle::Monospace)
                .desired_width(ui.available_width()),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(t("Insert pattern"))
                    .small()
                    .color(palette.muted),
            );
            if ui.button(t("Scale by parameter")).clicked() {
                rule.expression = "value * factor".to_owned();
            }
            if ui.button(t("Resistance ratio")).clicked() {
                rule.expression =
                    "value * resist(target_resistance) / resist(base_resistance)".to_owned();
            }
            if ui.button(t("Clamp to raw value")).clicked() {
                rule.expression = "clamp(value, 0, raw)".to_owned();
            }
        });
    }
    blueprint_expression_reference(ui, palette);
    if remove {
        let rule_id = projection.rules[index].id;
        let execution_node = BlueprintExecutionNode::Rule(rule_id);
        projection
            .execution_edges
            .retain(|edge| edge.from != execution_node && edge.to != execution_node);
        projection
            .parameter_connections
            .retain(|connected| *connected != rule_id);
        projection.pending_connection = None;
        projection.rules.remove(index);
        projection.rule_node_positions.remove(index);
        projection.selected_node = LowCodeNodeSelection::Match;
        projection.feedback = Some(t("Node deleted"));
    }
}

fn blueprint_expression_reference(ui: &mut egui::Ui, palette: ModEditorPalette) {
    egui::CollapsingHeader::new(t("Expression reference"))
        .default_open(true)
        .show(ui, |ui| {
            ui.label(
                RichText::new(t("Variables"))
                    .small()
                    .strong()
                    .color(palette.text),
            );
            blueprint_reference_entries(
                ui,
                &[
                    (
                        "value",
                        "Current damage after previous connected transform nodes.",
                    ),
                    (
                        "raw",
                        "Original captured damage before this Blueprint runs.",
                    ),
                    (
                        "char_id",
                        "Numeric character template ID for this damage component.",
                    ),
                    ("timestamp", "Capture timestamp in seconds."),
                    ("is_follow_up", "1 for follow-up damage, otherwise 0."),
                ],
                palette,
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(t("Functions"))
                    .small()
                    .strong()
                    .color(palette.text),
            );
            blueprint_reference_entries(
                ui,
                &[
                    (
                        "pct(percent)",
                        "Converts a percentage such as 20 into 0.2.",
                    ),
                    (
                        "resist(percent)",
                        "Returns the game resistance multiplier for a resistance percentage.",
                    ),
                    (
                        "min(a, b) · max(a, b) · clamp(value, min, max)",
                        "Returns the smaller, larger, or range-limited value.",
                    ),
                    (
                        "abs(value) · round(value) · floor(value) · ceil(value) · pow(base, exponent)",
                        "Absolute value, rounding, and exponent helpers.",
                    ),
                    (
                        "lerp(from, to, ratio)",
                        "Interpolates from one value to another by a ratio.",
                    ),
                    (
                        "select(condition, when_true, when_false)",
                        "Chooses between two values; zero is false and non-zero is true.",
                    ),
                    (
                        "if_eq(a, b, when_true, when_false) · if_gt(...) · if_lt(...)",
                        "Numeric comparisons that choose between two values.",
                    ),
                ],
                palette,
            );
        });
}

fn blueprint_reference_entries(
    ui: &mut egui::Ui,
    entries: &[(&str, &str)],
    palette: ModEditorPalette,
) {
    for (signature, description) in entries {
        ui.label(
            RichText::new(*signature)
                .small()
                .monospace()
                .color(palette.text),
        );
        ui.label(RichText::new(t(description)).small().color(palette.muted));
        ui.add_space(4.0);
    }
}

fn low_code_target_label(target: LowCodeDamageTarget) -> &'static str {
    match target {
        LowCodeDamageTarget::All => "each damage value",
        LowCodeDamageTarget::Primary => "primary damage",
        LowCodeDamageTarget::FollowUp => "follow-up damage",
    }
}

fn low_code_source_preview(
    ui: &mut egui::Ui,
    projection: &DamageProjectionEditorState,
    palette: ModEditorPalette,
) {
    let source = generated_low_code_source(projection);
    egui::Frame::new()
        .fill(palette.chrome)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t("Generated Blueprint source"))
                        .size(14.0)
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    RichText::new(t(
                        "The graph and this readable rule source describe the same Mod program.",
                    ))
                    .color(palette.muted),
                );
                if ui.button(t("Copy generated source")).clicked() {
                    ui.ctx().copy_text(source.clone());
                }
            });
            ui.add_space(6.0);
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.label(RichText::new(source).monospace().color(palette.text));
            });
        });
}

fn generated_low_code_source(projection: &DamageProjectionEditorState) -> String {
    let mut source = String::new();
    let name = if projection.name.trim().is_empty() {
        "untitled"
    } else {
        projection.name.trim()
    };
    let _ = writeln!(source, "mod {name:?}");
    let _ = writeln!(source, "on damage.hit as hit:");
    let (execution_path, uses_match) = match blueprint_execution_path(projection) {
        Ok(path) => path,
        Err(error) => {
            let _ = write!(source, "  graph_error {error:?}");
            return source;
        }
    };
    if uses_match {
        let _ = writeln!(
            source,
            "  where character in {:?} and attribute in {:?} and source in {:?} and skill in {:?}",
            projection.character_ids,
            projection.damage_attributes,
            projection.attack_types,
            projection.skill_names,
        );
    }
    if execution_path.iter().any(|index| {
        projection
            .parameter_connections
            .contains(&projection.rules[*index].id)
    }) {
        for variable in &projection.variables {
            let _ = writeln!(source, "  let {} = {}", variable.name, variable.value);
        }
    }
    for index in execution_path {
        let rule = &projection.rules[index];
        if rule.enabled {
            let _ = writeln!(
                source,
                "  set {} = {}",
                low_code_target_source(rule.target),
                rule.expression
            );
        }
    }
    let _ = write!(source, "  emit analysis.views");
    source
}

fn low_code_target_source(target: LowCodeDamageTarget) -> &'static str {
    match target {
        LowCodeDamageTarget::All => "hit.damage[*]",
        LowCodeDamageTarget::Primary => "hit.damage",
        LowCodeDamageTarget::FollowUp => "hit.follow_up_damage",
    }
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_mod_workspace() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nte-mod-editor-test-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn resistance_blueprint_recipe_executes_its_expression() {
        let mut projection = DamageProjectionEditorState::default();
        projection.load_resistance_recipe();
        let compiled = compile_low_code_mod(&projection).unwrap();
        let transformed = compiled
            .evaluate(DamageTransformInput {
                value: 100.0,
                char_id: 7,
                timestamp: 1.0,
                damage_attribute: Some("光"),
                attack_type: Some("普通攻击"),
                skill_name: Some("测试技能"),
                follow_up: false,
            })
            .unwrap();

        assert_eq!(transformed, 80.0);
    }

    #[test]
    fn match_node_combines_multi_select_dimensions() {
        let mut projection = DamageProjectionEditorState::default();
        projection.character_ids = vec![7, 8];
        projection.damage_attributes = vec!["光".to_owned(), "灵".to_owned()];
        projection.attack_types = vec!["普通攻击".to_owned(), "技能".to_owned()];
        projection.skill_names = vec!["技能甲".to_owned(), "技能乙".to_owned()];

        assert!(low_code_component_matches(
            &projection,
            DamageTransformInput {
                value: 100.0,
                char_id: 8,
                timestamp: 1.0,
                damage_attribute: Some("灵"),
                attack_type: Some("技能"),
                skill_name: Some("技能乙"),
                follow_up: false,
            }
        ));
        assert!(!low_code_component_matches(
            &projection,
            DamageTransformInput {
                value: 100.0,
                char_id: 9,
                timestamp: 1.0,
                damage_attribute: Some("灵"),
                attack_type: Some("技能"),
                skill_name: Some("技能乙"),
                follow_up: false,
            }
        ));
        assert!(!low_code_component_matches(
            &projection,
            DamageTransformInput {
                value: 100.0,
                char_id: 8,
                timestamp: 1.0,
                damage_attribute: Some("火"),
                attack_type: Some("技能"),
                skill_name: Some("技能乙"),
                follow_up: false,
            }
        ));
    }

    #[test]
    fn blueprint_skill_selection_displays_localized_text_and_keeps_the_match_key() {
        let skills = vec![BlueprintSkillOption {
            key: "GA_Female051_UltraSkill".to_owned(),
            display_name: "本地化终结技".to_owned(),
        }];
        assert_eq!(
            blueprint_skill_selection_label(&["GA_Female051_UltraSkill".to_owned()], &skills,),
            "本地化终结技"
        );
        assert_eq!(skills[0].key, "GA_Female051_UltraSkill");
    }

    #[test]
    fn frame_all_includes_nodes_moved_above_and_left_of_the_origin() {
        let mut projection = DamageProjectionEditorState::default();
        projection.event_node_position = egui::pos2(-240.0, -160.0);

        let bounds = blueprint_graph_bounds(&projection);

        assert!(bounds.left() < projection.event_node_position.x);
        assert!(bounds.top() < projection.event_node_position.y);
        assert!(bounds.contains(projection.event_node_position));
    }

    #[test]
    fn blueprint_details_use_the_side_panel_at_the_workspace_breakpoint() {
        assert!(!blueprint_details_use_side_panel(959.0));
        assert!(blueprint_details_use_side_panel(960.0));
    }

    #[test]
    fn node_template_inserts_parameters_and_execution_flow() {
        let mut projection = DamageProjectionEditorState::default();

        add_blueprint_rule(&mut projection, BlueprintRuleTemplate::Bonus, None);

        assert_eq!(projection.rules.len(), 2);
        assert_eq!(
            projection.rules[1].expression,
            "value * (1 + pct(bonus_percent))"
        );
        assert!(
            projection
                .variables
                .iter()
                .any(|variable| variable.name == "bonus_percent" && variable.value == 20.0)
        );
        assert!(projection.parameter_connections.contains(&2));
        assert!(
            projection
                .execution_edges
                .contains(&BlueprintExecutionEdge {
                    from: BlueprintExecutionNode::Rule(1),
                    to: BlueprintExecutionNode::Rule(2),
                })
        );
        assert!(
            projection
                .execution_edges
                .contains(&BlueprintExecutionEdge {
                    from: BlueprintExecutionNode::Rule(2),
                    to: BlueprintExecutionNode::Output,
                })
        );
        assert!(compile_low_code_mod(&projection).is_ok());
    }

    #[test]
    fn repeated_node_templates_receive_independent_parameters() {
        let mut projection = DamageProjectionEditorState::default();

        add_blueprint_rule(&mut projection, BlueprintRuleTemplate::Bonus, None);
        add_blueprint_rule(&mut projection, BlueprintRuleTemplate::Bonus, None);

        assert_eq!(
            projection.rules[1].expression,
            "value * (1 + pct(bonus_percent))"
        );
        assert_eq!(
            projection.rules[2].expression,
            "value * (1 + pct(bonus_percent_2))"
        );
        assert!(
            projection
                .variables
                .iter()
                .any(|variable| variable.name == "bonus_percent_2" && variable.value == 20.0)
        );
        assert!(compile_low_code_mod(&projection).is_ok());
    }

    #[test]
    fn conditional_template_applies_only_above_its_threshold() {
        let mut projection = DamageProjectionEditorState::default();
        add_blueprint_rule(&mut projection, BlueprintRuleTemplate::Threshold, None);
        let compiled = compile_low_code_mod(&projection).unwrap();

        let evaluate = |value| {
            compiled
                .evaluate(DamageTransformInput {
                    value,
                    char_id: 7,
                    timestamp: 1.0,
                    damage_attribute: None,
                    attack_type: None,
                    skill_name: None,
                    follow_up: false,
                })
                .unwrap()
        };

        assert_eq!(
            projection.rules[1].expression,
            "if_gt(raw, threshold, value * factor_2, value)"
        );
        assert_eq!(evaluate(1_200.0), 1_440.0);
        assert_eq!(evaluate(800.0), 800.0);
    }

    #[test]
    fn low_code_branch_and_interpolation_functions_choose_expected_values() {
        assert_eq!(
            evaluate_low_code_function("lerp", &[10.0, 20.0, 0.25]).unwrap(),
            12.5
        );
        assert_eq!(
            evaluate_low_code_function("select", &[1.0, 20.0, 10.0]).unwrap(),
            20.0
        );
        assert_eq!(
            evaluate_low_code_function("select", &[0.0, 20.0, 10.0]).unwrap(),
            10.0
        );
        assert_eq!(
            evaluate_low_code_function("if_eq", &[7.0, 7.0, 2.0, 1.0]).unwrap(),
            2.0
        );
        assert_eq!(
            evaluate_low_code_function("if_gt", &[8.0, 7.0, 2.0, 1.0]).unwrap(),
            2.0
        );
        assert_eq!(
            evaluate_low_code_function("if_lt", &[6.0, 7.0, 2.0, 1.0]).unwrap(),
            2.0
        );
    }

    #[test]
    fn low_code_expression_supports_parameters_and_sequential_rules() {
        let mut projection = DamageProjectionEditorState::default();
        projection.variables[0].value = 0.5;
        projection.rules.push(LowCodeRule {
            id: 2,
            enabled: true,
            target: LowCodeDamageTarget::All,
            expression: "value + pct(10) * raw".to_owned(),
        });
        connect_blueprint_execution_edge(
            &mut projection,
            BlueprintExecutionNode::Rule(1),
            BlueprintExecutionNode::Rule(2),
        );
        connect_blueprint_execution_edge(
            &mut projection,
            BlueprintExecutionNode::Rule(2),
            BlueprintExecutionNode::Output,
        );
        let compiled = compile_low_code_mod(&projection).unwrap();
        let transformed = compiled
            .evaluate(DamageTransformInput {
                value: 100.0,
                char_id: 7,
                timestamp: 1.0,
                damage_attribute: None,
                attack_type: None,
                skill_name: None,
                follow_up: false,
            })
            .unwrap();

        assert_eq!(transformed, 60.0);
    }

    #[test]
    fn blueprint_execution_uses_only_connected_rules() {
        let mut projection = DamageProjectionEditorState::default();
        projection.rules[0].expression = "value * 0.5".to_owned();
        connect_blueprint_execution_edge(
            &mut projection,
            BlueprintExecutionNode::Match,
            BlueprintExecutionNode::Output,
        );

        let compiled = compile_low_code_mod(&projection).unwrap();
        let transformed = compiled
            .evaluate(DamageTransformInput {
                value: 100.0,
                char_id: 7,
                timestamp: 1.0,
                damage_attribute: None,
                attack_type: None,
                skill_name: None,
                follow_up: false,
            })
            .unwrap();

        assert_eq!(transformed, 100.0);
    }

    #[test]
    fn blueprint_parameter_wire_controls_parameter_visibility() {
        let mut projection = DamageProjectionEditorState::default();
        projection.parameter_connections.clear();
        assert!(compile_low_code_mod(&projection).is_err());

        projection.parameter_connections.push(1);
        assert!(compile_low_code_mod(&projection).is_ok());
    }

    #[test]
    fn blueprint_execution_can_bypass_the_match_node() {
        let mut projection = DamageProjectionEditorState::default();
        connect_blueprint_execution_edge(
            &mut projection,
            BlueprintExecutionNode::Event,
            BlueprintExecutionNode::Rule(1),
        );

        let compiled = compile_low_code_mod(&projection).unwrap();

        assert!(!compiled.uses_match);
    }

    #[test]
    fn blueprint_execution_reports_incomplete_and_cyclic_flows() {
        let mut incomplete = DamageProjectionEditorState::default();
        incomplete
            .execution_edges
            .retain(|edge| edge.from != BlueprintExecutionNode::Rule(1));
        assert!(compile_low_code_mod(&incomplete).is_err());

        let mut cyclic = DamageProjectionEditorState::default();
        cyclic.execution_edges = vec![
            BlueprintExecutionEdge {
                from: BlueprintExecutionNode::Event,
                to: BlueprintExecutionNode::Match,
            },
            BlueprintExecutionEdge {
                from: BlueprintExecutionNode::Match,
                to: BlueprintExecutionNode::Rule(1),
            },
            BlueprintExecutionEdge {
                from: BlueprintExecutionNode::Rule(1),
                to: BlueprintExecutionNode::Match,
            },
        ];
        assert!(compile_low_code_mod(&cyclic).is_err());
    }

    #[test]
    fn blueprint_connection_replaces_existing_input_and_output_wires() {
        let mut projection = DamageProjectionEditorState::default();
        connect_blueprint_execution_edge(
            &mut projection,
            BlueprintExecutionNode::Match,
            BlueprintExecutionNode::Output,
        );

        assert!(
            projection
                .execution_edges
                .contains(&BlueprintExecutionEdge {
                    from: BlueprintExecutionNode::Match,
                    to: BlueprintExecutionNode::Output,
                })
        );
        assert!(
            !projection
                .execution_edges
                .iter()
                .any(|edge| edge.from == BlueprintExecutionNode::Rule(1)
                    || edge.to == BlueprintExecutionNode::Rule(1))
        );
    }

    #[test]
    fn low_code_expression_accepts_unicode_parameter_names() {
        let mut projection = DamageProjectionEditorState::default();
        projection.variables = vec![LowCodeVariable {
            name: "抗性".to_owned(),
            value: 20.0,
        }];
        projection.rules[0].expression = "value * resist(抗性)".to_owned();

        assert!(compile_low_code_mod(&projection).is_ok());
    }

    #[test]
    fn low_code_expression_reports_unknown_variables() {
        let mut projection = DamageProjectionEditorState::default();
        projection.rules[0].expression = "value * missing".to_owned();

        assert!(compile_low_code_mod(&projection).is_err());
    }

    #[test]
    fn low_code_clamp_rejects_an_inverted_range() {
        let mut projection = DamageProjectionEditorState::default();
        projection.rules[0].expression = "clamp(value, 10, 0)".to_owned();

        assert!(compile_low_code_mod(&projection).is_err());
    }

    #[test]
    fn editor_loads_the_shared_workspace_without_a_game_installation() {
        let directory = temp_mod_workspace();
        std::fs::create_dir_all(directory.join("nte-mods")).unwrap();
        let source = new_mod_script_template("local-test").unwrap();
        save_mod_script(&directory, "local-test", &source).unwrap();
        set_mod_enabled(&directory, "local-test", true).unwrap();

        let targets = load_mod_editor_targets_from_workspace(directory.clone()).unwrap();

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].region, ModsPluginGameRegion::China);
        assert_eq!(targets[1].region, ModsPluginGameRegion::Global);
        assert!(targets.iter().all(|target| {
            target.directory == directory
                && target.workspace.scripts
                    == [ModScriptDocument {
                        id: "local-test".to_owned(),
                        enabled: true,
                        source: source.clone(),
                    }]
        }));
        std::fs::remove_dir_all(directory).unwrap();
    }

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
