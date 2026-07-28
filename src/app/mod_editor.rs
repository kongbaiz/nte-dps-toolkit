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
        blueprint: ModScriptBlueprint,
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
    Blueprint,
    Projection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NteBlueprintSelection {
    #[default]
    Manifest,
    Statement(u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NteSourceLanguage {
    Legacy,
    #[default]
    Cpp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NteBlueprintDeclaration {
    Capability(String),
    State {
        type_name: String,
        name: String,
        value: String,
    },
    IpcRoute {
        operation: String,
        service: String,
    },
    Other(String),
}

#[derive(Clone, Debug, PartialEq)]
struct NteBlueprintStatement {
    id: u64,
    indent: u8,
    leading_blank_lines: usize,
    source: String,
    position: egui::Pos2,
    description: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NteBlueprintBlockRange {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteBlueprintFlowEdgeKind {
    Next,
    True,
    False,
    Loop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NteBlueprintFlowEdge {
    from: usize,
    to: usize,
    kind: NteBlueprintFlowEdgeKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NteBlueprintControlFlow {
    blocks: Vec<NteBlueprintBlockRange>,
    edges: Vec<NteBlueprintFlowEdge>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteBlueprintWireRoute {
    Direct,
    Channel,
    OuterLeft,
    OuterRight,
}

#[derive(Clone, Copy, Debug)]
struct NteBlueprintRoutedWire {
    edge: NteBlueprintFlowEdge,
    start: egui::Pos2,
    end: egui::Pos2,
    route: NteBlueprintWireRoute,
    lane: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteBlueprintBlockCategory {
    Flow,
    Action,
    Values,
}

impl NteBlueprintBlockCategory {
    fn key(self) -> &'static str {
        match self {
            Self::Flow => "Condition",
            Self::Action => "Action",
            Self::Values => "Data",
        }
    }

    fn color(self) -> Color32 {
        match self {
            Self::Flow => Color32::from_rgb(111, 74, 156),
            Self::Action => Color32::from_rgb(45, 120, 82),
            Self::Values => Color32::from_rgb(42, 128, 142),
        }
    }
}

const NTE_BLUEPRINT_NODE_WIDTH: f32 = 340.0;
const NTE_BLUEPRINT_BASE_Y: f32 = 64.0;
const NTE_BLUEPRINT_GRAPH_COLUMN_STEP: f32 = 410.0;
const NTE_BLUEPRINT_GRAPH_ROW_GAP: f32 = 36.0;
const NTE_BLUEPRINT_VIEW_WIDTH: f32 = 1_680.0;
const NTE_BLUEPRINT_VIEW_HEIGHT: f32 = 760.0;
const NTE_BLUEPRINT_COMFORT_WIDTH: f32 = 1_120.0;
const NTE_BLUEPRINT_COMFORT_HEIGHT: f32 = 460.0;
const NTE_BLUEPRINT_OVERVIEW_MAX_WIDTH: f32 = 2_400.0;
const NTE_BLUEPRINT_OVERVIEW_MAX_HEIGHT: f32 = 760.0;
const NTE_BLUEPRINT_CPP_STATE_TYPES: &[&str] = &[
    "std::uint64_t",
    "std::uintptr_t",
    "std::int64_t",
    "std::uint32_t",
    "std::int32_t",
    "bool",
];
const NTE_BLUEPRINT_CPP_LOCAL_TYPES: &[&str] = &[
    "",
    "const auto",
    "auto",
    "std::uint64_t",
    "std::uintptr_t",
    "std::int64_t",
    "std::uint32_t",
    "std::int32_t",
    "bool",
];
const NTE_BLUEPRINT_CPP_BUILTIN_VALUES: &[&str] = &[
    "0",
    "1",
    "false",
    "true",
    "nullptr",
    "event.viewport",
    "nte::game::viewport",
    "nte::game::instance",
    "nte::game::local_player",
    "nte::game::player_controller",
    "nte::game::player_state",
    "nte::game::player_character",
];
const NTE_BLUEPRINT_LEGACY_BUILTIN_VALUES: &[&str] = &[
    "0",
    "1",
    "False",
    "True",
    "None",
    "event.viewport",
    "game.viewport",
    "game.instance",
    "game.local_player",
    "game.player_controller",
    "game.player_state",
    "game.player_character",
];
const NTE_BLUEPRINT_COMPARISON_OPERATORS: &[&str] = &["==", "!=", ">", ">=", "<", "<="];

#[derive(Clone, Debug, PartialEq)]
struct NteBlueprintEditorState {
    language: NteSourceLanguage,
    declarations: Vec<NteBlueprintDeclaration>,
    statements: Vec<NteBlueprintStatement>,
    trailing_blank_lines: usize,
    next_statement_id: u64,
    selected: NteBlueprintSelection,
    scene_rect: egui::Rect,
    source_snapshot: String,
    import_error: Option<String>,
    feedback: Option<String>,
}

impl Default for NteBlueprintEditorState {
    fn default() -> Self {
        Self {
            language: NteSourceLanguage::Cpp,
            declarations: Vec::new(),
            statements: Vec::new(),
            trailing_blank_lines: 0,
            next_statement_id: 1,
            selected: NteBlueprintSelection::Manifest,
            scene_rect: egui::Rect::from_min_size(
                egui::pos2(-40.0, -28.0),
                egui::vec2(NTE_BLUEPRINT_VIEW_WIDTH, NTE_BLUEPRINT_VIEW_HEIGHT),
            ),
            source_snapshot: String::new(),
            import_error: None,
            feedback: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteBlueprintStatementTemplate {
    Assignment,
    StateAssignment,
    If,
    Loop,
    GameValue,
    SdkCall,
    MemoryRead,
    MemoryWrite,
    Cache,
    UnrealCall,
    ProcessEvent,
    IpcEmit,
    IpcBind,
    Equipment,
    CombatClock,
    Log,
    Comment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NteBlueprintStatementKind {
    Assignment,
    If,
    Elif,
    Else,
    Loop,
    Call,
    Comment,
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
    source_editor_open: bool,
    nte_blueprint: NteBlueprintEditorState,
    projection: DamageProjectionEditorState,
    projection_error: Option<String>,
    loaded: bool,
    targets: Vec<ModEditorTarget>,
    selected_region: Option<ModsPluginGameRegion>,
    selected_mod_id: Option<String>,
    source: String,
    saved_source: String,
    saved_blueprint: ModScriptBlueprint,
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
        self.dirty_with_blueprint(&self.nte_blueprint)
    }

    fn dirty_with_blueprint(&self, blueprint: &NteBlueprintEditorState) -> bool {
        self.source != self.saved_source
            || nte_blueprint_metadata(blueprint) != self.saved_blueprint
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
            self.nte_blueprint = NteBlueprintEditorState::default();
            sync_nte_blueprint_from_source(&mut self.nte_blueprint, &self.source);
            apply_nte_blueprint_metadata(&mut self.nte_blueprint, &document.blueprint, false);
            self.saved_blueprint = nte_blueprint_metadata(&self.nte_blueprint);
        } else {
            self.source.clear();
            self.saved_source.clear();
            self.saved_blueprint = ModScriptBlueprint::default();
            self.nte_blueprint = NteBlueprintEditorState::default();
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
        label: "#include <nte/mod.hpp>",
        insert: "#include <nte/mod.hpp>",
    },
    NteCompletion {
        label: "NTE_SCRIPT(5);",
        insert: "NTE_SCRIPT(5);",
    },
    NteCompletion {
        label: "NTE_MOD(\"id\");",
        insert: "NTE_MOD(\"mod-id\");",
    },
    NteCompletion {
        label: "NTE_REQUIRES(\"capability\");",
        insert: "NTE_REQUIRES(\"viewport.tick\");",
    },
    NteCompletion {
        label: "NTE_ROUTE_IPC(operation, \"kernel.service\");",
        insert: "NTE_ROUTE_IPC(12, \"ipc.query_mod_events\");",
    },
    NteCompletion {
        label: "std::uint64_t state = 0;",
        insert: "std::uint64_t state_name = 0;",
    },
    NteCompletion {
        label: "void on_viewport_tick(const nte::viewport_tick_event& event)",
        insert: "void on_viewport_tick(const nte::viewport_tick_event& event)\n{\n    \n}",
    },
    NteCompletion {
        label: "event.viewport",
        insert: "event.viewport",
    },
    NteCompletion {
        label: "nte::game::viewport",
        insert: "nte::game::viewport",
    },
    NteCompletion {
        label: "nte::game::instance",
        insert: "nte::game::instance",
    },
    NteCompletion {
        label: "nte::game::local_player",
        insert: "nte::game::local_player",
    },
    NteCompletion {
        label: "nte::game::player_controller",
        insert: "nte::game::player_controller",
    },
    NteCompletion {
        label: "nte::game::player_state",
        insert: "nte::game::player_state",
    },
    NteCompletion {
        label: "nte::game::player_character",
        insert: "nte::game::player_character",
    },
    NteCompletion {
        label: "nte::memory::read_ptr(base, offset)",
        insert: "nte::memory::read_ptr(",
    },
    NteCompletion {
        label: "nte::memory::read_u8(base, offset)",
        insert: "nte::memory::read_u8(",
    },
    NteCompletion {
        label: "nte::memory::read_u16(base, offset)",
        insert: "nte::memory::read_u16(",
    },
    NteCompletion {
        label: "nte::memory::read_u32(base, offset)",
        insert: "nte::memory::read_u32(",
    },
    NteCompletion {
        label: "nte::memory::read_u64(base, offset)",
        insert: "nte::memory::read_u64(",
    },
    NteCompletion {
        label: "nte::memory::read_i32(base, offset)",
        insert: "nte::memory::read_i32(",
    },
    NteCompletion {
        label: "nte::memory::read_f32_milli(base, offset)",
        insert: "nte::memory::read_f32_milli(",
    },
    NteCompletion {
        label: "nte::memory::read_fname_hash(base, offset)",
        insert: "nte::memory::read_fname_hash(",
    },
    NteCompletion {
        label: "nte::memory::tarray_first(base, offset)",
        insert: "nte::memory::tarray_first(",
    },
    NteCompletion {
        label: "nte::memory::tarray_count(base, offset)",
        insert: "nte::memory::tarray_count(",
    },
    NteCompletion {
        label: "nte::memory::is_readable(pointer, size)",
        insert: "nte::memory::is_readable(",
    },
    NteCompletion {
        label: "nte::memory::write_u8(base, offset, value)",
        insert: "nte::memory::write_u8(",
    },
    NteCompletion {
        label: "nte::memory::write_u16(base, offset, value)",
        insert: "nte::memory::write_u16(",
    },
    NteCompletion {
        label: "nte::memory::write_u32(base, offset, value)",
        insert: "nte::memory::write_u32(",
    },
    NteCompletion {
        label: "nte::memory::write_u64(base, offset, value)",
        insert: "nte::memory::write_u64(",
    },
    NteCompletion {
        label: "nte::memory::write_i32(base, offset, value)",
        insert: "nte::memory::write_i32(",
    },
    NteCompletion {
        label: "nte::memory::write_f32_milli(base, offset, value)",
        insert: "nte::memory::write_f32_milli(",
    },
    NteCompletion {
        label: "nte::unreal::find_function(object, \"Owner\", \"Function\")",
        insert: "nte::unreal::find_function(",
    },
    NteCompletion {
        label: "nte::unreal::params_clear(size)",
        insert: "nte::unreal::params_clear(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_u8(offset, value)",
        insert: "nte::unreal::params_write_u8(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_u16(offset, value)",
        insert: "nte::unreal::params_write_u16(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_u32(offset, value)",
        insert: "nte::unreal::params_write_u32(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_u64(offset, value)",
        insert: "nte::unreal::params_write_u64(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_i32(offset, value)",
        insert: "nte::unreal::params_write_i32(",
    },
    NteCompletion {
        label: "nte::unreal::params_write_f32_milli(offset, value)",
        insert: "nte::unreal::params_write_f32_milli(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_u8(offset)",
        insert: "nte::unreal::params_read_u8(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_u16(offset)",
        insert: "nte::unreal::params_read_u16(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_u32(offset)",
        insert: "nte::unreal::params_read_u32(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_u64(offset)",
        insert: "nte::unreal::params_read_u64(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_i32(offset)",
        insert: "nte::unreal::params_read_i32(",
    },
    NteCompletion {
        label: "nte::unreal::params_read_f32_milli(offset)",
        insert: "nte::unreal::params_read_f32_milli(",
    },
    NteCompletion {
        label: "nte::unreal::call(object, function)",
        insert: "nte::unreal::call(",
    },
    NteCompletion {
        label: "nte::unreal::watch(object, function)",
        insert: "nte::unreal::watch(",
    },
    NteCompletion {
        label: "nte::unreal::unwatch(object, function)",
        insert: "nte::unreal::unwatch(",
    },
    NteCompletion {
        label: "nte::event::next()",
        insert: "nte::event::next()",
    },
    NteCompletion {
        label: "nte::event::object()",
        insert: "nte::event::object()",
    },
    NteCompletion {
        label: "nte::event::function()",
        insert: "nte::event::function()",
    },
    NteCompletion {
        label: "nte::event::params_size()",
        insert: "nte::event::params_size()",
    },
    NteCompletion {
        label: "nte::event::read_u8(offset)",
        insert: "nte::event::read_u8(",
    },
    NteCompletion {
        label: "nte::event::read_u16(offset)",
        insert: "nte::event::read_u16(",
    },
    NteCompletion {
        label: "nte::event::read_u32(offset)",
        insert: "nte::event::read_u32(",
    },
    NteCompletion {
        label: "nte::event::read_u64(offset)",
        insert: "nte::event::read_u64(",
    },
    NteCompletion {
        label: "nte::event::read_i32(offset)",
        insert: "nte::event::read_i32(",
    },
    NteCompletion {
        label: "nte::event::read_f32_milli(offset)",
        insert: "nte::event::read_f32_milli(",
    },
    NteCompletion {
        label: "nte::sdk::player_character(controller)",
        insert: "nte::sdk::player_character(",
    },
    NteCompletion {
        label: "nte::sdk::player_state(controller)",
        insert: "nte::sdk::player_state(",
    },
    NteCompletion {
        label: "nte::sdk::game_paused(controller)",
        insert: "nte::sdk::game_paused(",
    },
    NteCompletion {
        label: "nte::sdk::attack_target(character)",
        insert: "nte::sdk::attack_target(",
    },
    NteCompletion {
        label: "nte::sdk::current_weapon(character)",
        insert: "nte::sdk::current_weapon(",
    },
    NteCompletion {
        label: "nte::sdk::character_level(character)",
        insert: "nte::sdk::character_level(",
    },
    NteCompletion {
        label: "nte::sdk::character_hp_milli(character)",
        insert: "nte::sdk::character_hp_milli(",
    },
    NteCompletion {
        label: "nte::sdk::character_hp_max_milli(character, fixed)",
        insert: "nte::sdk::character_hp_max_milli(",
    },
    NteCompletion {
        label: "nte::sdk::character_is_alive(character)",
        insert: "nte::sdk::character_is_alive(",
    },
    NteCompletion {
        label: "nte::sdk::character_is_dead(character)",
        insert: "nte::sdk::character_is_dead(",
    },
    NteCompletion {
        label: "nte::sdk::character_is_controlled(character)",
        insert: "nte::sdk::character_is_controlled(",
    },
    NteCompletion {
        label: "nte::sdk::character_slomo_milli(character)",
        insert: "nte::sdk::character_slomo_milli(",
    },
    NteCompletion {
        label: "nte::cache::get(key)",
        insert: "nte::cache::get(",
    },
    NteCompletion {
        label: "nte::cache::remember(key, value)",
        insert: "nte::cache::remember(",
    },
    NteCompletion {
        label: "nte::equipment::cache_missing()",
        insert: "nte::equipment::cache_missing()",
    },
    NteCompletion {
        label: "nte::equipment::cache_ready(player_state)",
        insert: "nte::equipment::cache_ready(",
    },
    NteCompletion {
        label: "nte::equipment::prepare(player_state)",
        insert: "nte::equipment::prepare(",
    },
    NteCompletion {
        label: "nte::combat_clock::pause_mask(controller)",
        insert: "nte::combat_clock::pause_mask(",
    },
    NteCompletion {
        label: "nte::combat_clock::state_flags(controller)",
        insert: "nte::combat_clock::state_flags(",
    },
    NteCompletion {
        label: "nte::combat_clock::forward(pause_mask, state_flags)",
        insert: "nte::combat_clock::forward(",
    },
    NteCompletion {
        label: "nte::ipc::bind(player_state, controller)",
        insert: "nte::ipc::bind(",
    },
    NteCompletion {
        label: "nte::ipc::emit(\"event\", value...)",
        insert: "nte::ipc::emit(\"event.name\", ",
    },
    NteCompletion {
        label: "nte::ipc::emit(\"pre.event\", value...)",
        insert: "nte::ipc::emit(\"pre.event.name\", ",
    },
    NteCompletion {
        label: "nte::ipc::emit(\"post.event\", value...)",
        insert: "nte::ipc::emit(\"post.event.name\", ",
    },
    NteCompletion {
        label: "nte::time::now_ms()",
        insert: "nte::time::now_ms()",
    },
    NteCompletion {
        label: "nte::log::info(\"message\")",
        insert: "nte::log::info(\"message\")",
    },
    NteCompletion {
        label: "for (std::uint64_t index = 0; index < COUNT; ++index)",
        insert: "for (std::uint64_t index = 0; index < 1; ++index)\n{\n    \n}",
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
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Mod Studio")).size(18.0).strong());
                    ui.separator();
                    ui.selectable_value(
                        &mut self.mod_editor.mode,
                        ModStudioMode::Blueprint,
                        t("NTE Blueprint"),
                    );
                    ui.selectable_value(
                        &mut self.mod_editor.mode,
                        ModStudioMode::Projection,
                        t("Analysis Blueprint"),
                    );
                });
                if matches!(self.mod_editor.mode, ModStudioMode::Projection) {
                    return false;
                }
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
        if matches!(self.mod_editor.mode, ModStudioMode::Projection) {
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
            match self.mod_editor.mode {
                ModStudioMode::Blueprint => {
                    self.mod_editor_blueprint_panel(ui, pending);
                    self.mod_editor_source_window(ui.ctx(), pending);
                }
                ModStudioMode::Projection => {
                    unreachable!("analysis Blueprint returns before the script workspace")
                }
            }
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
                                self.mod_editor.saved_blueprint = ModScriptBlueprint::default();
                                self.mod_editor.is_new = true;
                                self.mod_editor.new_mod_id.clear();
                                self.mod_editor.message.clear();
                                self.mod_editor.completion = ModCompletionState::default();
                                self.mod_editor.nte_blueprint = NteBlueprintEditorState::default();
                            }
                            Err(error) => {
                                self.mod_editor.message = mod_script_error_text(&error);
                            }
                        }
                    }
                }
            });
    }

    fn mod_editor_blueprint_panel(&mut self, ui: &mut egui::Ui, pending: bool) {
        use egui_material_icons::icons::{ICON_ACCOUNT_TREE, ICON_CHECK_CIRCLE, ICON_ERROR};

        let Some(id) = self.mod_editor.selected_mod_id.clone() else {
            ui.centered_and_justified(|ui| {
                ui.label(t("Select a Mod or create a new one."));
            });
            return;
        };
        let palette = mod_editor_palette(self.preferences.dark_mode, self.preferences.accent);
        let mut blueprint = std::mem::take(&mut self.mod_editor.nte_blueprint);
        if blueprint.source_snapshot != self.mod_editor.source {
            sync_nte_blueprint_from_source(&mut blueprint, &self.mod_editor.source);
        }
        let validation = validate_mod_source(&id, &self.mod_editor.source)
            .map_err(|error| mod_script_error_text(&error))
            .and_then(|()| validate_nte_blueprint(&blueprint));
        let mut revert_existing = false;
        let mut revert_new = false;

        egui::Frame::new()
            .fill(palette.chrome)
            .stroke(Stroke::new(1.0_f32, palette.border))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    mod_editor_icon(ui, ICON_ACCOUNT_TREE, 16.0, palette.selected_border);
                    ui.label(
                        RichText::new(format!("{id}.nte"))
                            .monospace()
                            .strong()
                            .color(palette.text),
                    );
                    if self.mod_editor.dirty_with_blueprint(&blueprint) {
                        ui.label(RichText::new("●").size(8.0).color(palette.muted))
                            .on_hover_text(t("Unsaved changes"));
                    }
                    if self.mod_editor.is_new {
                        ui.label(RichText::new(t("New")).small().color(palette.status));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("Edit source")).clicked() {
                            self.mod_editor.source_editor_open = true;
                        }
                        if ui
                            .add_enabled(
                                !pending
                                    && self.mod_editor.dirty_with_blueprint(&blueprint)
                                    && validation.is_ok()
                                    && blueprint.import_error.is_none(),
                                egui::Button::new(t("Save Mod")).small(),
                            )
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
                                    blueprint: nte_blueprint_metadata(&blueprint),
                                },
                            );
                        }
                        if ui
                            .add_enabled(
                                !pending && self.mod_editor.dirty_with_blueprint(&blueprint),
                                egui::Button::new(t("Revert changes")).small(),
                            )
                            .clicked()
                        {
                            if self.mod_editor.is_new {
                                revert_new = true;
                            } else {
                                revert_existing = true;
                            }
                        }
                    });
                });
            });

        if revert_new {
            let selected = self
                .mod_editor
                .selected_target()
                .and_then(|target| target.workspace.scripts.first())
                .map(|script| script.id.clone());
            self.mod_editor.select_document(selected);
            self.mod_editor.message.clear();
            return;
        }
        if revert_existing {
            self.mod_editor
                .source
                .clone_from(&self.mod_editor.saved_source);
            blueprint = NteBlueprintEditorState::default();
            sync_nte_blueprint_from_source(&mut blueprint, &self.mod_editor.source);
            apply_nte_blueprint_metadata(&mut blueprint, &self.mod_editor.saved_blueprint, false);
            self.mod_editor.message.clear();
        }

        if let Some(error) = &blueprint.import_error {
            egui::Frame::new()
                .fill(palette.editor)
                .stroke(Stroke::new(
                    1.0_f32,
                    semantic_warning(ui.visuals().dark_mode),
                ))
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        mod_editor_icon(
                            ui,
                            ICON_ERROR,
                            18.0,
                            semantic_warning(ui.visuals().dark_mode),
                        );
                        ui.label(
                            RichText::new(t("Fix the source to show this Blueprint.")).strong(),
                        );
                    });
                    ui.label(error);
                });
            self.mod_editor.nte_blueprint = blueprint;
            return;
        }

        let blueprint_before = blueprint.clone();
        ui.add_space(3.0);
        nte_blueprint_toolbar(ui, &mut blueprint, palette);
        ui.add_space(3.0);
        let workspace_height = (ui.available_height() - 30.0).max(480.0);
        let canvas_accepts_scroll = !self.mod_editor.source_editor_open;
        let canvas = ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), workspace_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                nte_blueprint_canvas(ui, &mut blueprint, palette, canvas_accepts_scroll);
            },
        );
        nte_blueprint_inspector_window(ui.ctx(), &mut blueprint, palette, canvas.response.rect);

        if !nte_blueprint_document_eq(&blueprint, &blueprint_before) {
            sync_nte_blueprint_capabilities(&mut blueprint);
            let rendered_source = render_nte_blueprint_source(&id, &blueprint);
            if rendered_source != self.mod_editor.source {
                self.mod_editor.source = rendered_source;
                self.mod_editor.completion = ModCompletionState::default();
                self.mod_editor.message.clear();
            }
        }
        blueprint
            .source_snapshot
            .clone_from(&self.mod_editor.source);
        let validation = validate_mod_source(&id, &self.mod_editor.source)
            .map_err(|error| mod_script_error_text(&error))
            .and_then(|()| validate_nte_blueprint(&blueprint));
        egui::Frame::new()
            .fill(palette.status)
            .inner_margin(egui::Margin::symmetric(8, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| match &validation {
                    Ok(()) => {
                        mod_editor_icon(ui, ICON_CHECK_CIRCLE, 14.0, Color32::WHITE);
                        ui.label(
                            RichText::new(t("Blueprint is valid"))
                                .small()
                                .color(Color32::WHITE),
                        );
                    }
                    Err(error) => {
                        mod_editor_icon(ui, ICON_ERROR, 14.0, Color32::WHITE);
                        ui.label(RichText::new(error).small().color(Color32::WHITE));
                    }
                });
            });
        self.mod_editor.nte_blueprint = blueprint;
    }

    fn mod_editor_source_window(&mut self, ctx: &egui::Context, pending: bool) {
        if !self.mod_editor.source_editor_open {
            return;
        }
        let mut open = true;
        egui::Window::new(t("Edit source"))
            .id(egui::Id::new("nte_blueprint_source_editor_window"))
            .open(&mut open)
            .default_size(egui::vec2(980.0, 680.0))
            .min_size(egui::vec2(620.0, 360.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                let content_size = ui.available_size();
                let (content_rect, _) = ui.allocate_exact_size(content_size, egui::Sense::hover());
                let mut content_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(content_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                content_ui.set_clip_rect(content_rect);
                self.mod_editor_source_panel(&mut content_ui, pending);
            });
        self.mod_editor.source_editor_open = open;
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
                                self.mod_editor.nte_blueprint = NteBlueprintEditorState::default();
                                sync_nte_blueprint_from_source(
                                    &mut self.mod_editor.nte_blueprint,
                                    &self.mod_editor.source,
                                );
                                apply_nte_blueprint_metadata(
                                    &mut self.mod_editor.nte_blueprint,
                                    &self.mod_editor.saved_blueprint,
                                    false,
                                );
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
                                    blueprint: nte_blueprint_metadata(
                                        &self.mod_editor.nte_blueprint,
                                    ),
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
                            RichText::new("NTE C++")
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
                    blueprint,
                    ..
                } => save_mod_script(directory, id, source, blueprint)
                    .map_err(ModEditorTaskError::Script),
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
        .stroke(Stroke::new(1.0_f32, color))
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

const NTE_BLUEPRINT_CAPABILITIES: &[&str] = &[
    "viewport.tick",
    "game.session",
    "memory.read",
    "memory.write",
    "sdk.read",
    "unreal.reflection",
    "process.event",
    "ipc",
    "equipment",
    "combat-clock",
    "log",
];

const NTE_BLUEPRINT_IPC_SERVICES: [(&str, &str); 12] = [
    ("1", "equipment.equip_module"),
    ("2", "equipment.equip_core"),
    ("3", "equipment.unequip_module"),
    ("4", "equipment.unequip_core"),
    ("5", "equipment.unequip_all"),
    ("6", "equipment.equip_one_key"),
    ("7", "equipment.move_module_to_character"),
    ("8", "equipment.move_core_to_character"),
    ("9", "equipment.set_item_discarded"),
    ("10", "equipment.set_item_locked"),
    ("11", "combat_clock.query_transitions"),
    ("12", "ipc.query_mod_events"),
];

fn sync_nte_blueprint_from_source(editor: &mut NteBlueprintEditorState, source: &str) {
    let metadata = nte_blueprint_metadata(editor);
    match parse_nte_blueprint_source(source) {
        Ok(mut parsed) => {
            apply_nte_blueprint_metadata(&mut parsed, &metadata, true);
            parsed.source_snapshot = source.to_owned();
            *editor = parsed;
        }
        Err(error) => {
            editor.source_snapshot = source.to_owned();
            editor.import_error = Some(error);
        }
    }
}

fn parse_nte_blueprint_source(source: &str) -> Result<NteBlueprintEditorState, String> {
    if source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with('#'))
        == Some("NTE_SCRIPT(5);")
    {
        parse_cpp_blueprint_source(source)
    } else {
        parse_legacy_blueprint_source(source)
    }
}

fn parse_legacy_blueprint_source(source: &str) -> Result<NteBlueprintEditorState, String> {
    let lines = source.lines().collect::<Vec<_>>();
    if lines.first().copied() != Some("nte_mod(4)") {
        return Err(t("The first statement must be nte_mod(4)."));
    }
    if !lines
        .get(1)
        .is_some_and(|line| line.starts_with("mod(\"") && line.ends_with("\")"))
    {
        return Err(t("The second statement must declare mod(\"id\")."));
    }
    let handler_index = lines
        .iter()
        .position(|line| *line == "def on_viewport_tick(event):")
        .ok_or_else(|| t("The script must define on_viewport_tick(event)."))?;
    let declarations = lines[2..handler_index]
        .iter()
        .map(|line| parse_nte_blueprint_declaration(line))
        .collect();
    let mut statements = Vec::new();
    let mut leading_blank_lines = 0;
    let mut next_statement_id = 1;
    for (relative_index, line) in lines[handler_index + 1..].iter().enumerate() {
        if line.trim().is_empty() {
            leading_blank_lines += 1;
            continue;
        }
        let leading_spaces = line.bytes().take_while(|byte| *byte == b' ').count();
        if leading_spaces < 4 || leading_spaces % 4 != 0 || line.starts_with('\t') {
            return Err(tf(
                "Blueprint import requires four-space indentation on line {}.",
                &[&(handler_index + relative_index + 2).to_string()],
            ));
        }
        let indent = leading_spaces / 4 - 1;
        if indent > 7 {
            return Err(t("NTE Script blocks can be nested at most eight levels."));
        }
        statements.push(NteBlueprintStatement {
            id: next_statement_id,
            indent: indent as u8,
            leading_blank_lines,
            source: line[leading_spaces..].trim_end().to_owned(),
            position: egui::Pos2::ZERO,
            description: String::new(),
        });
        next_statement_id += 1;
        leading_blank_lines = 0;
    }
    let mut editor = NteBlueprintEditorState {
        language: NteSourceLanguage::Legacy,
        declarations,
        statements,
        trailing_blank_lines: leading_blank_lines,
        next_statement_id,
        ..NteBlueprintEditorState::default()
    };
    layout_nte_blueprint(&mut editor);
    editor.scene_rect = nte_blueprint_initial_scene(&editor);
    Ok(editor)
}

fn parse_cpp_blueprint_source(source: &str) -> Result<NteBlueprintEditorState, String> {
    let lines = source.lines().collect::<Vec<_>>();
    let script_index = lines
        .iter()
        .position(|line| line.trim() == "NTE_SCRIPT(5);")
        .ok_or_else(|| t("The first statement must be NTE_SCRIPT(5)."))?;
    let mod_index = lines
        .iter()
        .skip(script_index + 1)
        .position(|line| line.trim().starts_with("NTE_MOD("))
        .map(|index| script_index + 1 + index)
        .ok_or_else(|| t("The script must declare NTE_MOD(\"id\")."))?;
    let handler_index = lines
        .iter()
        .skip(mod_index + 1)
        .position(|line| {
            line.trim() == "void on_viewport_tick(const nte::viewport_tick_event& event)"
        })
        .map(|index| mod_index + 1 + index)
        .ok_or_else(|| t("The script must define on_viewport_tick(event)."))?;
    let declarations = lines[mod_index + 1..handler_index]
        .iter()
        .map(|line| parse_cpp_blueprint_declaration(line.trim()))
        .collect();
    let mut next = handler_index + 1;
    while lines.get(next).is_some_and(|line| line.trim().is_empty()) {
        next += 1;
    }
    if lines.get(next).map(|line| line.trim()) != Some("{") {
        return Err(t("The on_viewport_tick function must open with a brace."));
    }
    next += 1;

    let mut depth = 1usize;
    let mut statements = Vec::new();
    let mut leading_blank_lines = 0;
    let mut next_statement_id = 1;
    while let Some(raw) = lines.get(next) {
        let line_number = next + 1;
        next += 1;
        let line = raw.trim();
        if line.is_empty() {
            leading_blank_lines += 1;
            continue;
        }
        if line == "{" {
            depth += 1;
            continue;
        }
        if line == "}" {
            depth = depth.checked_sub(1).ok_or_else(|| {
                tf(
                    "Blueprint import found an unmatched brace on line {}.",
                    &[&line_number.to_string()],
                )
            })?;
            if depth == 0 {
                break;
            }
            continue;
        }
        if depth == 0 || depth > 8 {
            return Err(t("NTE C++ blocks can be nested at most eight levels."));
        }
        let source = if line.starts_with("//") {
            line.to_owned()
        } else {
            line.strip_suffix(';').unwrap_or(line).trim().to_owned()
        };
        statements.push(NteBlueprintStatement {
            id: next_statement_id,
            indent: (depth - 1) as u8,
            leading_blank_lines,
            source,
            position: egui::Pos2::ZERO,
            description: String::new(),
        });
        next_statement_id += 1;
        leading_blank_lines = 0;
    }
    if depth != 0 {
        return Err(t(
            "The on_viewport_tick function is missing a closing brace.",
        ));
    }
    if lines[next..].iter().any(|line| !line.trim().is_empty()) {
        return Err(t("Only one on_viewport_tick function is supported."));
    }
    let mut editor = NteBlueprintEditorState {
        language: NteSourceLanguage::Cpp,
        declarations,
        statements,
        trailing_blank_lines: leading_blank_lines,
        next_statement_id,
        ..NteBlueprintEditorState::default()
    };
    layout_nte_blueprint(&mut editor);
    editor.scene_rect = nte_blueprint_initial_scene(&editor);
    Ok(editor)
}

fn parse_nte_blueprint_declaration(line: &str) -> NteBlueprintDeclaration {
    if let Some(capability) = line
        .strip_prefix("requires(\"")
        .and_then(|line| line.strip_suffix("\")"))
    {
        return NteBlueprintDeclaration::Capability(capability.to_owned());
    }
    if let Some(declaration) = line.strip_prefix("state.")
        && let Some((name, value)) = declaration.split_once('=')
    {
        return NteBlueprintDeclaration::State {
            type_name: "std::uint64_t".to_owned(),
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
        };
    }
    if let Some(arguments) = line
        .strip_prefix("route_ipc(")
        .and_then(|line| line.strip_suffix(')'))
        && let Some((operation, service)) = arguments.split_once(',')
    {
        let service = service.trim();
        if let Some(service) = service
            .strip_prefix('"')
            .and_then(|service| service.strip_suffix('"'))
        {
            return NteBlueprintDeclaration::IpcRoute {
                operation: operation.trim().to_owned(),
                service: service.to_owned(),
            };
        }
    }
    NteBlueprintDeclaration::Other(line.to_owned())
}

fn parse_cpp_blueprint_declaration(line: &str) -> NteBlueprintDeclaration {
    if let Some(capability) = line
        .strip_prefix("NTE_REQUIRES(\"")
        .and_then(|line| line.strip_suffix("\");"))
    {
        return NteBlueprintDeclaration::Capability(capability.to_owned());
    }
    if let Some((type_name, declaration)) = line.strip_suffix(';').and_then(|line| {
        NTE_BLUEPRINT_CPP_STATE_TYPES.iter().find_map(|type_name| {
            line.strip_prefix(&format!("{type_name} "))
                .map(|declaration| (*type_name, declaration))
        })
    }) && let Some((name, value)) = declaration.split_once('=')
    {
        return NteBlueprintDeclaration::State {
            type_name: type_name.to_owned(),
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
        };
    }
    if let Some(arguments) = line
        .strip_prefix("NTE_ROUTE_IPC(")
        .and_then(|line| line.strip_suffix(");"))
        && let Some((operation, service)) = arguments.split_once(',')
    {
        let service = service.trim();
        if let Some(service) = service
            .strip_prefix('"')
            .and_then(|service| service.strip_suffix('"'))
        {
            return NteBlueprintDeclaration::IpcRoute {
                operation: operation.trim().to_owned(),
                service: service.to_owned(),
            };
        }
    }
    NteBlueprintDeclaration::Other(line.to_owned())
}

fn render_nte_blueprint_source(id: &str, editor: &NteBlueprintEditorState) -> String {
    match editor.language {
        NteSourceLanguage::Legacy => render_legacy_blueprint_source(id, editor),
        NteSourceLanguage::Cpp => render_cpp_blueprint_source(id, editor),
    }
}

fn render_legacy_blueprint_source(id: &str, editor: &NteBlueprintEditorState) -> String {
    use std::fmt::Write as _;

    let mut source = String::new();
    let _ = writeln!(source, "nte_mod(4)");
    let _ = writeln!(source, "mod({id:?})");
    for declaration in &editor.declarations {
        match declaration {
            NteBlueprintDeclaration::Capability(capability) => {
                let _ = writeln!(source, "requires({capability:?})");
            }
            NteBlueprintDeclaration::State { name, value, .. } => {
                let _ = writeln!(source, "state.{name} = {value}");
            }
            NteBlueprintDeclaration::IpcRoute { operation, service } => {
                let _ = writeln!(source, "route_ipc({operation}, {service:?})");
            }
            NteBlueprintDeclaration::Other(line) => {
                let _ = writeln!(source, "{line}");
            }
        }
    }
    let _ = writeln!(source, "def on_viewport_tick(event):");
    for statement in &editor.statements {
        for _ in 0..statement.leading_blank_lines {
            source.push('\n');
        }
        source.push_str(&"    ".repeat(statement.indent as usize + 1));
        source.push_str(statement.source.trim());
        source.push('\n');
    }
    for _ in 0..editor.trailing_blank_lines {
        source.push('\n');
    }
    source
}

fn render_cpp_blueprint_source(id: &str, editor: &NteBlueprintEditorState) -> String {
    use std::fmt::Write as _;

    let mut source = String::new();
    let _ = writeln!(source, "#include <nte/mod.hpp>");
    let _ = writeln!(source);
    let _ = writeln!(source, "NTE_SCRIPT(5);");
    let _ = writeln!(source, "NTE_MOD({id:?});");
    for declaration in &editor.declarations {
        match declaration {
            NteBlueprintDeclaration::Capability(capability) => {
                let _ = writeln!(source, "NTE_REQUIRES({capability:?});");
            }
            NteBlueprintDeclaration::State {
                type_name,
                name,
                value,
            } => {
                let _ = writeln!(source, "{type_name} {name} = {value};");
            }
            NteBlueprintDeclaration::IpcRoute { operation, service } => {
                let _ = writeln!(source, "NTE_ROUTE_IPC({operation}, {service:?});");
            }
            NteBlueprintDeclaration::Other(line) => {
                let _ = writeln!(source, "{line}");
            }
        }
    }
    let _ = writeln!(
        source,
        "void on_viewport_tick(const nte::viewport_tick_event& event)"
    );
    let _ = writeln!(source, "{{");
    let mut open_blocks = 0usize;
    for statement in &editor.statements {
        let indent = statement.indent as usize;
        while open_blocks > indent {
            open_blocks -= 1;
            let _ = writeln!(source, "{}}}", "    ".repeat(open_blocks + 1));
        }
        for _ in 0..statement.leading_blank_lines {
            source.push('\n');
        }
        let statement_source = statement.source.trim();
        source.push_str(&"    ".repeat(indent + 1));
        source.push_str(statement_source);
        if statement_source.starts_with("//") {
            source.push('\n');
        } else if nte_blueprint_statement_opens_block(statement_source) {
            source.push('\n');
            let _ = writeln!(source, "{}{{", "    ".repeat(indent + 1));
            open_blocks = indent + 1;
        } else {
            source.push_str(";\n");
        }
    }
    while open_blocks > 0 {
        open_blocks -= 1;
        let _ = writeln!(source, "{}}}", "    ".repeat(open_blocks + 1));
    }
    let _ = writeln!(source, "}}");
    for _ in 0..editor.trailing_blank_lines {
        source.push('\n');
    }
    source
}

fn nte_blueprint_document_eq(
    left: &NteBlueprintEditorState,
    right: &NteBlueprintEditorState,
) -> bool {
    left.declarations == right.declarations
        && left.language == right.language
        && left.trailing_blank_lines == right.trailing_blank_lines
        && left.statements.len() == right.statements.len()
        && left
            .statements
            .iter()
            .zip(&right.statements)
            .all(|(left, right)| {
                left.id == right.id
                    && left.indent == right.indent
                    && left.leading_blank_lines == right.leading_blank_lines
                    && left.source == right.source
            })
}

fn sync_nte_blueprint_capabilities(editor: &mut NteBlueprintEditorState) {
    let required = required_nte_blueprint_capabilities(editor);
    let insertion_index = editor
        .declarations
        .iter()
        .position(|line| matches!(line, NteBlueprintDeclaration::Capability(_)))
        .unwrap_or(0);
    editor
        .declarations
        .retain(|line| !matches!(line, NteBlueprintDeclaration::Capability(_)));
    let insertion_index = insertion_index.min(editor.declarations.len());
    editor.declarations.splice(
        insertion_index..insertion_index,
        required
            .into_iter()
            .map(|capability| NteBlueprintDeclaration::Capability(capability.to_owned())),
    );
}

fn required_nte_blueprint_capabilities(editor: &NteBlueprintEditorState) -> Vec<&'static str> {
    NTE_BLUEPRINT_CAPABILITIES
        .iter()
        .copied()
        .filter(|capability| {
            *capability == "viewport.tick"
                || editor.declarations.iter().any(|declaration| {
                    let NteBlueprintDeclaration::IpcRoute { service, .. } = declaration else {
                        return false;
                    };
                    match *capability {
                        "ipc" => true,
                        "equipment" => service.starts_with("equipment."),
                        "combat-clock" => service.starts_with("combat_clock."),
                        _ => false,
                    }
                })
                || editor.statements.iter().any(|statement| {
                    let source = statement.source.trim();
                    !source.starts_with('#')
                        && match *capability {
                            "game.session" => {
                                nte_blueprint_statement_uses_namespace(source, "game.")
                            }
                            "memory.read" => {
                                ["memory.read_", "memory.tarray_", "memory.is_readable"]
                                    .iter()
                                    .any(|namespace| {
                                        nte_blueprint_statement_uses_namespace(source, namespace)
                                    })
                            }
                            "memory.write" => {
                                nte_blueprint_statement_uses_namespace(source, "memory.write_")
                            }
                            "sdk.read" => nte_blueprint_statement_uses_namespace(source, "sdk."),
                            "unreal.reflection" => {
                                ["unreal.find_function", "unreal.params_", "unreal.call"]
                                    .iter()
                                    .any(|namespace| {
                                        nte_blueprint_statement_uses_namespace(source, namespace)
                                    })
                            }
                            "process.event" => [
                                "unreal.watch",
                                "unreal.unwatch",
                                "event.next(",
                                "event.object(",
                                "event.function(",
                                "event.params_size(",
                                "event.read_",
                            ]
                            .iter()
                            .any(|namespace| {
                                nte_blueprint_statement_uses_namespace(source, namespace)
                            }),
                            "ipc" => nte_blueprint_statement_uses_namespace(source, "ipc."),
                            "equipment" => {
                                nte_blueprint_statement_uses_namespace(source, "equipment.")
                            }
                            "combat-clock" => {
                                nte_blueprint_statement_uses_namespace(source, "combat_clock.")
                            }
                            "log" => nte_blueprint_statement_uses_namespace(source, "log."),
                            "viewport.tick" => true,
                            _ => unreachable!("every Blueprint capability is classified"),
                        }
                })
        })
        .collect()
}

fn nte_blueprint_statement_uses_namespace(source: &str, namespace: &str) -> bool {
    nte_blueprint_statement_uses_token(source, namespace)
        || nte_blueprint_statement_uses_token(
            source,
            &format!("nte::{}", namespace.replace('.', "::")),
        )
}

fn nte_blueprint_statement_uses_token(source: &str, namespace: &str) -> bool {
    let bytes = source.as_bytes();
    let namespace = namespace.as_bytes();
    let mut in_string = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => in_string = !in_string,
            b'#' if !in_string => break,
            b'/' if !in_string
                && bytes
                    .get(index + 1)
                    .is_some_and(|character| *character == b'/') =>
            {
                break;
            }
            _ => {}
        }
        if !in_string
            && bytes[index..].starts_with(namespace)
            && (index == 0
                || !matches!(
                    bytes[index - 1],
                    b'a'..=b'z'
                        | b'A'..=b'Z'
                        | b'0'..=b'9'
                        | b'_'
                        | b'.'
                        | b':'
                ))
        {
            return true;
        }
        index += 1;
    }
    false
}

fn validate_nte_blueprint(editor: &NteBlueprintEditorState) -> Result<(), String> {
    if editor.import_error.is_some() {
        return Err(t("Fix the source to show this Blueprint."));
    }
    if editor.statements.is_empty() {
        return Err(t("Add at least one step."));
    }
    for declaration in &editor.declarations {
        match declaration {
            NteBlueprintDeclaration::State { name, value, .. } => {
                if !is_nte_blueprint_identifier(name) {
                    return Err(tf("Invalid state variable name: {}", &[name]));
                }
                if !is_nte_blueprint_integer(value) {
                    return Err(tf(
                        "State variable {} must start with an integer, boolean, or None value.",
                        &[name],
                    ));
                }
            }
            NteBlueprintDeclaration::IpcRoute { operation, service } => {
                if !NTE_BLUEPRINT_IPC_SERVICES
                    .iter()
                    .any(|candidate| candidate.0 == operation && candidate.1 == service)
                {
                    return Err(tf(
                        "Invalid IPC route: operation {} cannot use service {}.",
                        &[operation, service],
                    ));
                }
            }
            NteBlueprintDeclaration::Capability(_) | NteBlueprintDeclaration::Other(_) => {}
        }
    }
    for (index, declaration) in editor.declarations.iter().enumerate() {
        let NteBlueprintDeclaration::IpcRoute { operation, .. } = declaration else {
            continue;
        };
        if editor.declarations[..index].iter().any(|candidate| {
            matches!(
                candidate,
                NteBlueprintDeclaration::IpcRoute {
                    operation: candidate_operation,
                    ..
                } if candidate_operation == operation
            )
        }) {
            return Err(tf(
                "IPC operation {} is routed more than once.",
                &[operation],
            ));
        }
    }
    for (index, statement) in editor.statements.iter().enumerate() {
        if statement.source.trim().is_empty() {
            return Err(tf(
                "Statement node {} is empty.",
                &[&(index + 1).to_string()],
            ));
        }
        if index == 0 && statement.indent != 0 {
            return Err(t("The first statement must connect directly to the event."));
        }
        if let Some(previous) = index.checked_sub(1).map(|index| &editor.statements[index]) {
            if statement.indent > previous.indent + 1 {
                return Err(tf(
                    "Statement node {} skips an indentation level.",
                    &[&(index + 1).to_string()],
                ));
            }
            if statement.indent == previous.indent + 1
                && !nte_blueprint_statement_opens_block(&previous.source)
            {
                return Err(tf(
                    "Statement node {} is indented under a node that does not open a block.",
                    &[&(index + 1).to_string()],
                ));
            }
        }
        if nte_blueprint_statement_opens_block(&statement.source)
            && editor
                .statements
                .get(index + 1)
                .is_none_or(|next| next.indent != statement.indent + 1)
        {
            return Err(tf(
                "Block node {} needs an indented child statement.",
                &[&(index + 1).to_string()],
            ));
        }
        if matches!(
            nte_blueprint_statement_kind(&statement.source),
            NteBlueprintStatementKind::Elif | NteBlueprintStatementKind::Else
        ) && !nte_blueprint_has_matching_if(editor, index)
        {
            return Err(tf(
                "Branch node {} has no matching if node.",
                &[&(index + 1).to_string()],
            ));
        }
    }
    Ok(())
}

fn is_nte_blueprint_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character == '_')
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn is_nte_blueprint_integer(value: &str) -> bool {
    if matches!(
        value,
        "None" | "False" | "True" | "nullptr" | "false" | "true"
    ) {
        return true;
    }
    if let Some(value) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return !value.is_empty() && u64::from_str_radix(value, 16).is_ok();
    }
    value.parse::<u64>().is_ok()
}

fn nte_blueprint_has_matching_if(editor: &NteBlueprintEditorState, index: usize) -> bool {
    let statement = &editor.statements[index];
    editor.statements[..index]
        .iter()
        .rev()
        .find(|candidate| candidate.indent <= statement.indent)
        .is_some_and(|candidate| {
            candidate.indent == statement.indent
                && matches!(
                    nte_blueprint_statement_kind(&candidate.source),
                    NteBlueprintStatementKind::If | NteBlueprintStatementKind::Elif
                )
        })
}

fn nte_blueprint_statement_opens_block(source: &str) -> bool {
    matches!(
        nte_blueprint_statement_kind(source),
        NteBlueprintStatementKind::If
            | NteBlueprintStatementKind::Elif
            | NteBlueprintStatementKind::Else
            | NteBlueprintStatementKind::Loop
    ) && (source.trim_end().ends_with(':')
        || source.trim_end().ends_with(')')
        || source.trim() == "else")
}

fn nte_blueprint_statement_kind(source: &str) -> NteBlueprintStatementKind {
    let source = source.trim();
    if source.starts_with('#') || source.starts_with("//") {
        NteBlueprintStatementKind::Comment
    } else if source.starts_with("if ") || source.starts_with("if (") {
        NteBlueprintStatementKind::If
    } else if source.starts_with("elif ") || source.starts_with("else if (") {
        NteBlueprintStatementKind::Elif
    } else if matches!(source, "else:" | "else") {
        NteBlueprintStatementKind::Else
    } else if source.starts_with("for ") || source.starts_with("for (") {
        NteBlueprintStatementKind::Loop
    } else if split_nte_blueprint_assignment(source).is_some() {
        NteBlueprintStatementKind::Assignment
    } else {
        NteBlueprintStatementKind::Call
    }
}

fn nte_blueprint_blocks(editor: &NteBlueprintEditorState) -> Vec<NteBlueprintBlockRange> {
    if editor.language == NteSourceLanguage::Cpp {
        let mut blocks = Vec::new();
        let mut start = 0;
        while start < editor.statements.len() {
            let base_indent = editor.statements[start].indent;
            let mut end = start + 1;
            if !nte_blueprint_statement_opens_block(&editor.statements[start].source) {
                while let Some(statement) = editor.statements.get(end) {
                    if statement.indent != base_indent
                        || statement.leading_blank_lines != 0
                        || nte_blueprint_statement_opens_block(&statement.source)
                    {
                        break;
                    }
                    end += 1;
                }
            }
            blocks.push(NteBlueprintBlockRange { start, end });
            start = end;
        }
        return blocks;
    }
    let mut blocks = Vec::new();
    let mut start = 0;
    while start < editor.statements.len() {
        let base_indent = editor.statements[start].indent;
        let mut end = start + 1;
        while let Some(statement) = editor.statements.get(end) {
            if statement.indent < base_indent
                || (statement.indent == base_indent && statement.leading_blank_lines != 0)
            {
                break;
            }
            end += 1;
        }
        blocks.push(NteBlueprintBlockRange { start, end });
        start = end;
    }
    blocks
}

fn nte_blueprint_block_subtree_end(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> usize {
    let indent = editor.statements[block.start].indent;
    editor.statements[block.end..]
        .iter()
        .position(|statement| statement.indent <= indent)
        .map_or(editor.statements.len(), |offset| block.end + offset)
}

fn nte_blueprint_can_indent_block(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> bool {
    let indent = editor.statements[block.start].indent;
    editor.statements[..block.start]
        .iter()
        .rposition(|statement| statement.indent <= indent)
        .is_some_and(|index| {
            editor.statements[index].indent == indent
                && nte_blueprint_statement_opens_block(&editor.statements[index].source)
        })
}

fn nte_blueprint_can_outdent_block(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> bool {
    let indent = editor.statements[block.start].indent;
    if indent == 0 {
        return false;
    }
    let Some(parent) = editor.statements[..block.start]
        .iter()
        .rposition(|statement| statement.indent < indent)
    else {
        return false;
    };
    if editor.statements[parent].indent + 1 != indent
        || !nte_blueprint_statement_opens_block(&editor.statements[parent].source)
    {
        return false;
    }
    let subtree_end = nte_blueprint_block_subtree_end(editor, block);
    let parent_end = editor.statements[parent + 1..]
        .iter()
        .position(|statement| statement.indent <= editor.statements[parent].indent)
        .map_or(editor.statements.len(), |offset| parent + 1 + offset);
    editor.statements[parent + 1..block.start]
        .iter()
        .chain(editor.statements[subtree_end..parent_end].iter())
        .any(|statement| statement.indent == indent)
}

fn adjust_nte_blueprint_block_indent(
    editor: &mut NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
    increase: bool,
) {
    let subtree_end = nte_blueprint_block_subtree_end(editor, block);
    if block.start != 0 {
        editor.statements[block.start].leading_blank_lines =
            editor.statements[block.start].leading_blank_lines.max(1);
    }
    for statement in &mut editor.statements[block.start..subtree_end] {
        if increase {
            statement.indent += 1;
        } else {
            statement.indent -= 1;
        }
    }
}

fn nte_blueprint_block_flow_kind(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> NteBlueprintStatementKind {
    let last = &editor.statements[block.end - 1].source;
    if nte_blueprint_statement_opens_block(last) {
        nte_blueprint_statement_kind(last)
    } else {
        nte_blueprint_statement_kind(&editor.statements[block.start].source)
    }
}

fn nte_blueprint_control_flow(editor: &NteBlueprintEditorState) -> NteBlueprintControlFlow {
    let blocks = nte_blueprint_blocks(editor);
    let mut edges = Vec::new();
    if editor.language == NteSourceLanguage::Cpp {
        if !blocks.is_empty() {
            let indent = editor.statements[blocks[0].start].indent;
            build_nte_cpp_flow_sequence(editor, &blocks, 0, blocks.len(), indent, None, &mut edges);
        }
    } else {
        for index in 0..blocks.len().saturating_sub(1) {
            edges.push(NteBlueprintFlowEdge {
                from: index,
                to: index + 1,
                kind: NteBlueprintFlowEdgeKind::Next,
            });
        }
    }
    NteBlueprintControlFlow { blocks, edges }
}

fn build_nte_cpp_flow_sequence(
    editor: &NteBlueprintEditorState,
    blocks: &[NteBlueprintBlockRange],
    start: usize,
    end: usize,
    indent: u8,
    continuation: Option<usize>,
    edges: &mut Vec<NteBlueprintFlowEdge>,
) -> Option<usize> {
    let top_level = (start..end)
        .filter(|index| editor.statements[blocks[*index].start].indent == indent)
        .collect::<Vec<_>>();
    if top_level.is_empty() {
        return continuation;
    }

    let mut units = Vec::new();
    let mut cursor = 0;
    while cursor < top_level.len() {
        let index = top_level[cursor];
        let kind = nte_blueprint_block_flow_kind(editor, blocks[index]);
        if kind == NteBlueprintStatementKind::If {
            let mut branch_end = cursor + 1;
            while branch_end < top_level.len()
                && matches!(
                    nte_blueprint_block_flow_kind(editor, blocks[top_level[branch_end]]),
                    NteBlueprintStatementKind::Elif | NteBlueprintStatementKind::Else
                )
            {
                branch_end += 1;
            }
            units.push((cursor, branch_end));
            cursor = branch_end;
        } else {
            units.push((cursor, cursor + 1));
            cursor += 1;
        }
    }

    let mut next_entry = continuation;
    for (unit_start, unit_end) in units.into_iter().rev() {
        let first = top_level[unit_start];
        let first_kind = nte_blueprint_block_flow_kind(editor, blocks[first]);
        if first_kind == NteBlueprintStatementKind::If {
            for branch_offset in (unit_start..unit_end).rev() {
                let branch = top_level[branch_offset];
                let branch_body_end = if branch_offset + 1 < unit_end {
                    top_level[branch_offset + 1]
                } else if unit_end < top_level.len() {
                    top_level[unit_end]
                } else {
                    end
                };
                let branch_kind = nte_blueprint_block_flow_kind(editor, blocks[branch]);
                let body_entry = build_nte_cpp_flow_sequence(
                    editor,
                    blocks,
                    branch + 1,
                    branch_body_end,
                    indent + 1,
                    next_entry,
                    edges,
                );
                if branch_kind == NteBlueprintStatementKind::Else {
                    if let Some(to) = body_entry {
                        push_nte_blueprint_flow_edge(
                            edges,
                            branch,
                            to,
                            NteBlueprintFlowEdgeKind::Next,
                        );
                    }
                } else {
                    if let Some(to) = body_entry {
                        push_nte_blueprint_flow_edge(
                            edges,
                            branch,
                            to,
                            NteBlueprintFlowEdgeKind::True,
                        );
                    }
                    let false_target = if branch_offset + 1 < unit_end {
                        Some(top_level[branch_offset + 1])
                    } else {
                        next_entry
                    };
                    if let Some(to) = false_target {
                        push_nte_blueprint_flow_edge(
                            edges,
                            branch,
                            to,
                            NteBlueprintFlowEdgeKind::False,
                        );
                    }
                }
            }
            next_entry = Some(first);
            continue;
        }

        let item_end = if unit_end < top_level.len() {
            top_level[unit_end]
        } else {
            end
        };
        if first_kind == NteBlueprintStatementKind::Loop {
            let body_entry = build_nte_cpp_flow_sequence(
                editor,
                blocks,
                first + 1,
                item_end,
                indent + 1,
                Some(first),
                edges,
            );
            if let Some(to) = body_entry {
                push_nte_blueprint_flow_edge(edges, first, to, NteBlueprintFlowEdgeKind::True);
            }
            if let Some(to) = next_entry {
                push_nte_blueprint_flow_edge(edges, first, to, NteBlueprintFlowEdgeKind::False);
            }
        } else if let Some(to) = next_entry {
            push_nte_blueprint_flow_edge(edges, first, to, NteBlueprintFlowEdgeKind::Next);
        }
        next_entry = Some(first);
    }
    next_entry
}

fn push_nte_blueprint_flow_edge(
    edges: &mut Vec<NteBlueprintFlowEdge>,
    from: usize,
    to: usize,
    mut kind: NteBlueprintFlowEdgeKind,
) {
    if to <= from {
        kind = NteBlueprintFlowEdgeKind::Loop;
    }
    let edge = NteBlueprintFlowEdge { from, to, kind };
    if !edges.contains(&edge) {
        edges.push(edge);
    }
}

fn nte_blueprint_metadata(editor: &NteBlueprintEditorState) -> ModScriptBlueprint {
    ModScriptBlueprint {
        nodes: nte_blueprint_blocks(editor)
            .into_iter()
            .map(|block| {
                let statement = &editor.statements[block.start];
                ModScriptBlueprintNode {
                    signature: nte_blueprint_block_signature(editor, block),
                    position: [statement.position.x, statement.position.y],
                    description: statement.description.clone(),
                }
            })
            .collect(),
    }
}

fn apply_nte_blueprint_metadata(
    editor: &mut NteBlueprintEditorState,
    metadata: &ModScriptBlueprint,
    fallback_by_order: bool,
) {
    let blocks = nte_blueprint_blocks(editor);
    let signatures = blocks
        .iter()
        .map(|block| nte_blueprint_block_signature(editor, *block))
        .collect::<Vec<_>>();
    let mut matches = vec![None; blocks.len()];
    let mut used = vec![false; metadata.nodes.len()];
    for (block_index, signature) in signatures.iter().enumerate() {
        if let Some(metadata_index) = metadata
            .nodes
            .iter()
            .enumerate()
            .position(|(index, node)| !used[index] && node.signature == *signature)
        {
            matches[block_index] = Some(metadata_index);
            used[metadata_index] = true;
        }
    }
    if fallback_by_order {
        for block_index in 0..blocks.len() {
            if matches[block_index].is_none()
                && metadata.nodes.get(block_index).is_some()
                && !used[block_index]
            {
                matches[block_index] = Some(block_index);
                used[block_index] = true;
            }
        }
    }
    for (block, metadata_index) in blocks.into_iter().zip(matches) {
        let Some(node) = metadata_index.map(|index| &metadata.nodes[index]) else {
            continue;
        };
        let position = egui::pos2(node.position[0], node.position[1]);
        for statement in &mut editor.statements[block.start..block.end] {
            statement.position = position;
        }
        editor.statements[block.start]
            .description
            .clone_from(&node.description);
    }
    editor.scene_rect = nte_blueprint_initial_scene(editor);
}

fn nte_blueprint_block_signature(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> String {
    use std::fmt::Write as _;

    let mut signature = String::new();
    for statement in &editor.statements[block.start..block.end] {
        let _ = writeln!(
            signature,
            "{}:{}",
            statement.indent,
            statement.source.trim()
        );
    }
    signature
}

fn nte_blueprint_selected_block(
    editor: &NteBlueprintEditorState,
) -> Option<NteBlueprintBlockRange> {
    let NteBlueprintSelection::Statement(id) = editor.selected else {
        return None;
    };
    nte_blueprint_blocks(editor).into_iter().find(|block| {
        editor.statements[block.start..block.end]
            .iter()
            .any(|statement| statement.id == id)
    })
}

fn nte_blueprint_block_height(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> f32 {
    if editor.language == NteSourceLanguage::Cpp
        && nte_blueprint_statement_opens_block(&editor.statements[block.start].source)
    {
        68.0
    } else if editor.language == NteSourceLanguage::Cpp {
        48.0 + (block.end - block.start) as f32 * 15.0
    } else if block.end - block.start == 1 {
        88.0
    } else {
        92.0
    }
}

fn nte_blueprint_block_summary(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> String {
    let description = editor.statements[block.start].description.trim();
    if !description.is_empty() {
        return description.to_owned();
    }
    if let Some((_, description)) = nte_blueprint_known_step(editor, block) {
        return t(description);
    }
    t(match nte_blueprint_block_category(editor, block) {
        NteBlueprintBlockCategory::Flow => "Run when the condition is met.",
        NteBlueprintBlockCategory::Action => "Run this action.",
        NteBlueprintBlockCategory::Values => "Read or save data.",
    })
}

fn nte_blueprint_block_category(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> NteBlueprintBlockCategory {
    let has_flow = editor.statements[block.start..block.end]
        .iter()
        .any(|statement| nte_blueprint_statement_opens_block(&statement.source));
    let has_action = editor.statements[block.start..block.end]
        .iter()
        .any(|statement| nte_blueprint_call_name(&statement.source).is_some());
    if editor.language == NteSourceLanguage::Cpp && has_flow {
        NteBlueprintBlockCategory::Flow
    } else if has_action {
        NteBlueprintBlockCategory::Action
    } else if has_flow {
        NteBlueprintBlockCategory::Flow
    } else {
        NteBlueprintBlockCategory::Values
    }
}

fn nte_blueprint_block_title(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> String {
    match nte_blueprint_block_flow_kind(editor, block) {
        NteBlueprintStatementKind::If => return t("Conditional Branch"),
        NteBlueprintStatementKind::Elif => return t("Alternative Condition"),
        NteBlueprintStatementKind::Else => return t("Fallback Branch"),
        NteBlueprintStatementKind::Loop => return t("Repeat actions"),
        NteBlueprintStatementKind::Assignment
        | NteBlueprintStatementKind::Call
        | NteBlueprintStatementKind::Comment => {}
    }
    if let Some((title, _)) = nte_blueprint_known_step(editor, block) {
        return t(title);
    }
    let statements = &editor.statements[block.start..block.end];
    if let Some(comment) = statements
        .first()
        .and_then(|statement| statement.source.strip_prefix('#'))
    {
        return nte_blueprint_truncate(comment.trim(), 48);
    }
    let first = &statements[0].source;
    if let Some(condition) = first
        .strip_prefix("if ")
        .and_then(|source| source.strip_suffix(':'))
    {
        if condition.ends_with(" != None") {
            return t("Run when data is available");
        }
        if condition.ends_with(" == True") {
            return t("Run when condition is true");
        }
        if condition.ends_with(" == False") {
            return t("Run when condition is false");
        }
        return t("Check a condition");
    }
    if first
        .strip_prefix("for ")
        .and_then(|source| source.strip_suffix(':'))
        .is_some()
    {
        return t("Repeat actions");
    }
    let targets = statements
        .iter()
        .take_while(|statement| statement.indent == statements[0].indent)
        .filter_map(|statement| split_nte_blueprint_assignment(&statement.source))
        .map(|(target, _)| target)
        .take(2)
        .collect::<Vec<_>>();
    if !targets.is_empty() {
        let read_only = statements
            .iter()
            .take(targets.len())
            .filter_map(|statement| split_nte_blueprint_assignment(&statement.source))
            .all(|(_, expression)| {
                expression.starts_with("game.")
                    || expression.starts_with("time.")
                    || expression.starts_with("sdk.")
            });
        return if read_only {
            t("Read game data")
        } else {
            t("Update values")
        };
    }
    if block.end - block.start == 1 {
        t(nte_blueprint_statement_title(first))
    } else {
        t("Custom logic")
    }
}

fn nte_blueprint_block_code_lines(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> Vec<String> {
    if editor.language != NteSourceLanguage::Cpp {
        return Vec::new();
    }
    editor.statements[block.start..block.end]
        .iter()
        .map(|statement| {
            let source = statement.source.trim();
            let source = if source.starts_with("//") || nte_blueprint_statement_opens_block(source)
            {
                source.to_owned()
            } else {
                format!("{source};")
            };
            nte_blueprint_truncate(&source, 46)
        })
        .collect()
}

fn nte_blueprint_known_step(
    editor: &NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
) -> Option<(&'static str, &'static str)> {
    let source = editor.statements[block.start..block.end]
        .iter()
        .map(|statement| statement.source.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .replace("nte::", "")
        .replace("::", ".");
    if source.contains("equipment.cache_ready(") || source.contains("equipment.prepare(") {
        Some((
            "Enable equipment actions",
            "Load equipment data and prepare equipment commands.",
        ))
    } else if source.contains("sdk.character_hp_milli(") {
        Some((
            "Send character health",
            "Send current health and health percentage.",
        ))
    } else if source.contains("combat_clock.forward(") {
        Some(("Send pause state", "Send the latest pause state."))
    } else if source.contains("combat_clock.pause_mask(")
        || source.contains("combat_clock.state_flags(")
    {
        Some(("Read combat state", "Read pause and combat status."))
    } else if source.contains("cache.remember(") {
        Some(("Cache data", "Keep the first value for each key."))
    } else if source.contains("state.last_") && source.contains("changed") {
        Some(("Detect state changes", "Continue when the state changes."))
    } else if source.contains("ipc.emit(") {
        Some(("Send Mod data", "Send the result to the desktop tool."))
    } else if source.contains("ipc.bind(") {
        Some((
            "Connect desktop service",
            "Allow the desktop tool to use this service.",
        ))
    } else {
        None
    }
}

fn nte_blueprint_call_name(source: &str) -> Option<&str> {
    let parenthesis = source.find('(')?;
    let prefix = &source[..parenthesis];
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_ascii_alphanumeric()
                && *character != '_'
                && *character != '.'
                && *character != ':'
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    let call = &prefix[start..];
    (!call.is_empty() && call.contains('.')).then_some(call)
}

fn nte_blueprint_truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() > max_chars {
        format!("{}…", value.chars().take(max_chars).collect::<String>())
    } else {
        value.to_owned()
    }
}

fn split_nte_blueprint_assignment(source: &str) -> Option<(&str, &str)> {
    let bytes = source.as_bytes();
    let mut in_string = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if byte == b'"' {
            in_string = !in_string;
        }
        if byte != b'=' || in_string {
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|index| bytes.get(index))
            .copied();
        let next = bytes.get(index + 1).copied();
        if matches!(previous, Some(b'!' | b'<' | b'>' | b'=')) || matches!(next, Some(b'=')) {
            continue;
        }
        let target = source[..index].trim();
        let expression = source[index + 1..].trim();
        if !target.is_empty() && !expression.is_empty() {
            return Some((target, expression));
        }
    }
    None
}

fn split_nte_blueprint_typed_target(target: &str) -> (&str, &str) {
    for type_name in NTE_BLUEPRINT_CPP_LOCAL_TYPES
        .iter()
        .copied()
        .filter(|type_name| !type_name.is_empty())
    {
        if let Some(name) = target
            .strip_prefix(type_name)
            .and_then(|target| target.strip_prefix(' '))
        {
            return (type_name, name.trim());
        }
    }
    ("", target.trim())
}

fn split_nte_blueprint_comparison(condition: &str) -> Option<(&str, &str, &str)> {
    let bytes = condition.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => in_string = !in_string,
            b'(' if !in_string => depth += 1,
            b')' if !in_string && depth != 0 => depth -= 1,
            _ => {}
        }
        if !in_string && depth == 0 {
            for operator in ["==", "!=", ">=", "<=", ">", "<"] {
                if bytes[index..].starts_with(operator.as_bytes()) {
                    let left = condition[..index].trim();
                    let right = condition[index + operator.len()..].trim();
                    if !left.is_empty()
                        && !right.is_empty()
                        && !left.contains("&&")
                        && !left.contains("||")
                        && !right.contains("&&")
                        && !right.contains("||")
                    {
                        return Some((left, operator, right));
                    }
                }
            }
        }
        index += 1;
    }
    None
}

fn nte_blueprint_value_options(editor: &NteBlueprintEditorState) -> Vec<String> {
    let builtins = match editor.language {
        NteSourceLanguage::Legacy => NTE_BLUEPRINT_LEGACY_BUILTIN_VALUES,
        NteSourceLanguage::Cpp => NTE_BLUEPRINT_CPP_BUILTIN_VALUES,
    };
    let mut options = builtins
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    for declaration in &editor.declarations {
        if let NteBlueprintDeclaration::State { name, .. } = declaration
            && !options.contains(name)
        {
            options.push(name.clone());
        }
    }
    for statement in &editor.statements {
        if let Some((target, _)) = split_nte_blueprint_assignment(&statement.source) {
            let (_, name) = split_nte_blueprint_typed_target(target);
            if !name.is_empty() && !options.iter().any(|option| option == name) {
                options.push(name.to_owned());
            }
        }
    }
    options
}

fn nte_blueprint_value_option_label(value: &str) -> String {
    match value {
        "false" | "False" => format!("{} · {value}", t("False")),
        "true" | "True" => format!("{} · {value}", t("True")),
        _ => value.to_owned(),
    }
}

fn nte_blueprint_value_selector(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut String,
    options: &[String],
) -> bool {
    const CUSTOM_VALUE: &str = "\u{0}custom";

    let mut selection = if options.iter().any(|option| option == value) {
        value.clone()
    } else {
        CUSTOM_VALUE.to_owned()
    };
    let previous = selection.clone();
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width())
        .selected_text(if selection == CUSTOM_VALUE {
            t("Custom expression")
        } else {
            nte_blueprint_value_option_label(&selection)
        })
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(
                    &mut selection,
                    option.clone(),
                    nte_blueprint_value_option_label(option),
                );
            }
            ui.separator();
            ui.selectable_value(
                &mut selection,
                CUSTOM_VALUE.to_owned(),
                t("Custom expression"),
            );
        });

    let mut changed = false;
    if selection != previous {
        if selection == CUSTOM_VALUE {
            value.clear();
        } else {
            value.clone_from(&selection);
        }
        changed = true;
    }
    if selection == CUSTOM_VALUE {
        changed |= ui
            .add(
                egui::TextEdit::singleline(value)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width()),
            )
            .changed();
    }
    changed
}

fn nte_blueprint_advanced_settings(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
) {
    ui.label(
        RichText::new(t("Configure persistent values and desktop commands."))
            .small()
            .color(palette.muted),
    );
    ui.add_space(4.0);
    nte_blueprint_manifest(ui, editor, palette);
}

fn nte_blueprint_manifest(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
) {
    egui::Frame::new()
        .fill(palette.editor)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t("Mod Manifest"))
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    RichText::new(t(
                        "Capabilities are calculated from flow block statements and IPC routes.",
                    ))
                    .small()
                    .color(palette.muted),
                );
                for capability in required_nte_blueprint_capabilities(editor) {
                    mod_pipeline_chip(ui, capability, palette.selected_border, palette);
                }
            });
            ui.add_space(3.0);
            let state_count = editor
                .declarations
                .iter()
                .filter(|line| matches!(line, NteBlueprintDeclaration::State { .. }))
                .count();
            egui::CollapsingHeader::new(
                RichText::new(tf("Persistent state · {}", &[&state_count.to_string()]))
                    .small()
                    .strong()
                    .color(palette.muted),
            )
            .id_salt("nte_blueprint_manifest_state")
            .default_open(false)
            .show(ui, |ui| {
                nte_blueprint_manifest_state(ui, editor);
            });
            let route_count = editor
                .declarations
                .iter()
                .filter(|line| matches!(line, NteBlueprintDeclaration::IpcRoute { .. }))
                .count();
            egui::CollapsingHeader::new(
                RichText::new(tf("IPC service routes · {}", &[&route_count.to_string()]))
                    .small()
                    .strong()
                    .color(palette.muted),
            )
            .id_salt("nte_blueprint_manifest_ipc_routes")
            .default_open(false)
            .show(ui, |ui| {
                nte_blueprint_manifest_ipc_routes(ui, editor, palette);
            });
        });
}

fn nte_blueprint_manifest_state(ui: &mut egui::Ui, editor: &mut NteBlueprintEditorState) {
    if ui
        .add(egui::Button::new(t("Add state variable")).small())
        .clicked()
    {
        let count = editor
            .declarations
            .iter()
            .filter(|line| matches!(line, NteBlueprintDeclaration::State { .. }))
            .count();
        let insertion_index = editor
            .declarations
            .iter()
            .rposition(|line| matches!(line, NteBlueprintDeclaration::State { .. }))
            .map_or(editor.declarations.len(), |index| index + 1);
        editor.declarations.insert(
            insertion_index,
            NteBlueprintDeclaration::State {
                type_name: "std::uint64_t".to_owned(),
                name: format!("value_{}", count + 1),
                value: "0".to_owned(),
            },
        );
        editor.feedback = Some(t("State variable added"));
    }
    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("nte_blueprint_manifest_state_scroll")
        .max_height(150.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, declaration) in editor.declarations.iter_mut().enumerate() {
                if let NteBlueprintDeclaration::State {
                    type_name,
                    name,
                    value,
                } = declaration
                {
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt(("nte_state_type", index))
                            .selected_text(type_name.as_str())
                            .show_ui(ui, |ui| {
                                for candidate in NTE_BLUEPRINT_CPP_STATE_TYPES {
                                    if ui
                                        .selectable_value(
                                            type_name,
                                            (*candidate).to_owned(),
                                            *candidate,
                                        )
                                        .changed()
                                    {
                                        if *candidate == "bool"
                                            && !matches!(value.as_str(), "true" | "false")
                                        {
                                            *value = "false".to_owned();
                                        } else if *candidate != "bool"
                                            && matches!(value.as_str(), "true" | "false")
                                        {
                                            *value = "0".to_owned();
                                        }
                                    }
                                }
                            });
                        ui.add(
                            egui::TextEdit::singleline(name)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(150.0),
                        );
                        ui.label(RichText::new("=").monospace());
                        if type_name == "bool" {
                            egui::ComboBox::from_id_salt(("nte_state_bool", index))
                                .selected_text(t(if value == "true" { "True" } else { "False" }))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(value, "false".to_owned(), t("False"));
                                    ui.selectable_value(value, "true".to_owned(), t("True"));
                                });
                        } else {
                            ui.add(
                                egui::TextEdit::singleline(value)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(110.0),
                            );
                        }
                        if ui
                            .add(egui::Button::new(t("Remove")).small())
                            .on_hover_text(t("Remove state variable"))
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                }
            }
        });
    if let Some(index) = remove {
        editor.declarations.remove(index);
        editor.feedback = Some(t("State variable removed"));
    }
}

fn nte_blueprint_manifest_ipc_routes(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
) {
    ui.horizontal_wrapped(|ui| {
        if ui
            .add(egui::Button::new(t("Add IPC route")).small())
            .clicked()
        {
            let route = NTE_BLUEPRINT_IPC_SERVICES
                .iter()
                .find(|candidate| {
                    !editor.declarations.iter().any(|declaration| {
                        matches!(
                            declaration,
                            NteBlueprintDeclaration::IpcRoute { operation, .. }
                                if operation == candidate.0
                        )
                    })
                })
                .copied()
                .unwrap_or(NTE_BLUEPRINT_IPC_SERVICES[0]);
            editor.declarations.push(NteBlueprintDeclaration::IpcRoute {
                operation: route.0.to_owned(),
                service: route.1.to_owned(),
            });
            sync_nte_blueprint_capabilities(editor);
            editor.feedback = Some(t("IPC route added"));
        }
        ui.label(
            RichText::new(t(
                "External routes choose which kernel service handles each wire operation.",
            ))
            .small()
            .color(palette.muted),
        );
    });
    let mut remove_route = None;
    egui::ScrollArea::vertical()
        .id_salt("nte_blueprint_manifest_ipc_routes_scroll")
        .max_height(178.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (index, declaration) in editor.declarations.iter_mut().enumerate() {
                let NteBlueprintDeclaration::IpcRoute { operation, service } = declaration else {
                    continue;
                };
                let selected_index = NTE_BLUEPRINT_IPC_SERVICES
                    .iter()
                    .position(|candidate| candidate.0 == operation && candidate.1 == service)
                    .unwrap_or(0);
                let mut next_index = selected_index;
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Operation")).small().color(palette.muted));
                    egui::ComboBox::from_id_salt(("nte_ipc_route", index))
                        .width(360.0)
                        .selected_text(format!(
                            "{} → {}",
                            NTE_BLUEPRINT_IPC_SERVICES[selected_index].0,
                            NTE_BLUEPRINT_IPC_SERVICES[selected_index].1
                        ))
                        .show_ui(ui, |ui| {
                            for (candidate_index, candidate) in
                                NTE_BLUEPRINT_IPC_SERVICES.iter().enumerate()
                            {
                                ui.selectable_value(
                                    &mut next_index,
                                    candidate_index,
                                    format!("{} → {}", candidate.0, candidate.1),
                                );
                            }
                        });
                    if ui
                        .add(egui::Button::new(t("Remove")).small())
                        .on_hover_text(t("Remove IPC route"))
                        .clicked()
                    {
                        remove_route = Some(index);
                    }
                });
                if next_index != selected_index {
                    let route = NTE_BLUEPRINT_IPC_SERVICES[next_index];
                    *operation = route.0.to_owned();
                    *service = route.1.to_owned();
                }
            }
        });
    if let Some(index) = remove_route {
        editor.declarations.remove(index);
        sync_nte_blueprint_capabilities(editor);
        editor.feedback = Some(t("IPC route removed"));
    }
}

fn nte_blueprint_toolbar(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
) {
    use egui_material_icons::icons::ICON_CHECK_CIRCLE;

    egui::Frame::new()
        .fill(palette.chrome)
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(t("Mod Flow"))
                        .size(15.0)
                        .strong()
                        .color(palette.text),
                );
                mod_pipeline_chip(
                    ui,
                    "Runs continuously",
                    Color32::from_rgb(168, 48, 58),
                    palette,
                );
                ui.menu_button(format!("+ {}", t("Add step")), |ui| {
                    nte_blueprint_statement_palette(ui, editor);
                });
                ui.menu_button(t("View"), |ui| {
                    if ui.button(t("Arrange control-flow graph")).clicked() {
                        layout_nte_blueprint(editor);
                        editor.scene_rect = nte_blueprint_initial_scene(editor);
                        editor.feedback = Some(t("Control-flow graph arranged"));
                        ui.close();
                    }
                    if ui.button(t("Fit graph to window")).clicked() {
                        editor.scene_rect = nte_blueprint_initial_scene(editor);
                        editor.feedback = Some(t("Graph fitted to window"));
                        ui.close();
                    }
                });
                ui.menu_button(t("Advanced Mod settings"), |ui| {
                    ui.set_min_width(520.0);
                    nte_blueprint_advanced_settings(ui, editor, palette);
                });
                if let Some(feedback) = &editor.feedback {
                    ui.separator();
                    mod_editor_icon(
                        ui,
                        ICON_CHECK_CIRCLE,
                        15.0,
                        semantic_success(ui.visuals().dark_mode),
                    );
                    ui.label(
                        RichText::new(feedback)
                            .small()
                            .strong()
                            .color(semantic_success(ui.visuals().dark_mode)),
                    );
                }
            });
        });
}

fn nte_blueprint_statement_palette(ui: &mut egui::Ui, editor: &mut NteBlueprintEditorState) {
    nte_blueprint_template_group(
        ui,
        editor,
        "Read data",
        &[
            NteBlueprintStatementTemplate::GameValue,
            NteBlueprintStatementTemplate::SdkCall,
            NteBlueprintStatementTemplate::MemoryRead,
            NteBlueprintStatementTemplate::UnrealCall,
            NteBlueprintStatementTemplate::ProcessEvent,
        ],
    );
    ui.separator();
    nte_blueprint_template_group(
        ui,
        editor,
        "Perform an action",
        &[
            NteBlueprintStatementTemplate::IpcEmit,
            NteBlueprintStatementTemplate::IpcBind,
            NteBlueprintStatementTemplate::Equipment,
            NteBlueprintStatementTemplate::CombatClock,
            NteBlueprintStatementTemplate::MemoryWrite,
            NteBlueprintStatementTemplate::Log,
        ],
    );
    ui.separator();
    nte_blueprint_template_group(
        ui,
        editor,
        "Store data",
        &[
            NteBlueprintStatementTemplate::Assignment,
            NteBlueprintStatementTemplate::StateAssignment,
            NteBlueprintStatementTemplate::Cache,
        ],
    );
    ui.separator();
    nte_blueprint_template_group(
        ui,
        editor,
        "Advanced logic",
        &[
            NteBlueprintStatementTemplate::If,
            NteBlueprintStatementTemplate::Loop,
            NteBlueprintStatementTemplate::Comment,
        ],
    );
}

fn nte_blueprint_template_group(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    title: &str,
    templates: &[NteBlueprintStatementTemplate],
) {
    ui.label(RichText::new(t(title)).small().strong());
    for template in templates {
        if ui
            .button(t(nte_blueprint_template_label(*template)))
            .clicked()
        {
            add_nte_blueprint_statement(editor, *template);
            ui.close();
        }
    }
}

fn nte_blueprint_template_label(template: NteBlueprintStatementTemplate) -> &'static str {
    match template {
        NteBlueprintStatementTemplate::Assignment => "Store a value",
        NteBlueprintStatementTemplate::StateAssignment => "Remember a value",
        NteBlueprintStatementTemplate::If => "Run when a condition matches",
        NteBlueprintStatementTemplate::Loop => "Repeat actions",
        NteBlueprintStatementTemplate::GameValue => "Read game data",
        NteBlueprintStatementTemplate::SdkCall => "Read character data",
        NteBlueprintStatementTemplate::MemoryRead => "Read an advanced game value",
        NteBlueprintStatementTemplate::MemoryWrite => "Write an advanced game value",
        NteBlueprintStatementTemplate::Cache => "Cache a value",
        NteBlueprintStatementTemplate::UnrealCall => "Call an Unreal function",
        NteBlueprintStatementTemplate::ProcessEvent => "Observe an Unreal event",
        NteBlueprintStatementTemplate::IpcEmit => "Send data to the desktop tool",
        NteBlueprintStatementTemplate::IpcBind => "Connect a desktop service",
        NteBlueprintStatementTemplate::Equipment => "Prepare equipment data",
        NteBlueprintStatementTemplate::CombatClock => "Send combat state",
        NteBlueprintStatementTemplate::Log => "Write a debug message",
        NteBlueprintStatementTemplate::Comment => "Add a note",
    }
}

fn nte_blueprint_template_source(
    template: NteBlueprintStatementTemplate,
    language: NteSourceLanguage,
) -> &'static str {
    if language == NteSourceLanguage::Cpp {
        return match template {
            NteBlueprintStatementTemplate::Assignment => "auto value = 0",
            NteBlueprintStatementTemplate::StateAssignment => "value = value",
            NteBlueprintStatementTemplate::If => "if (value == true)",
            NteBlueprintStatementTemplate::Loop => {
                "for (std::uint64_t index = 0; index < 1; ++index)"
            }
            NteBlueprintStatementTemplate::GameValue => {
                "const auto character = nte::game::player_character"
            }
            NteBlueprintStatementTemplate::SdkCall => {
                "const auto hp = nte::sdk::character_hp_milli(nte::game::player_character)"
            }
            NteBlueprintStatementTemplate::MemoryRead => {
                "const auto value = nte::memory::read_u64(nte::game::player_character, 0x0)"
            }
            NteBlueprintStatementTemplate::MemoryWrite => {
                "nte::memory::write_u64(nte::game::player_character, 0x0, value)"
            }
            NteBlueprintStatementTemplate::Cache => {
                "const auto cached = nte::cache::remember(1, value)"
            }
            NteBlueprintStatementTemplate::UnrealCall => {
                "const auto called = nte::unreal::call(nte::game::player_controller, function)"
            }
            NteBlueprintStatementTemplate::ProcessEvent => "const auto ready = nte::event::next()",
            NteBlueprintStatementTemplate::IpcEmit => "nte::ipc::emit(\"event.name\", value)",
            NteBlueprintStatementTemplate::IpcBind => {
                "nte::ipc::bind(nte::game::player_state, nte::game::player_controller)"
            }
            NteBlueprintStatementTemplate::Equipment => {
                "nte::equipment::prepare(nte::game::player_state)"
            }
            NteBlueprintStatementTemplate::CombatClock => {
                "nte::combat_clock::forward(pause_mask, state_flags)"
            }
            NteBlueprintStatementTemplate::Log => "nte::log::info(\"message\")",
            NteBlueprintStatementTemplate::Comment => "// Describe this step",
        };
    }
    match template {
        NteBlueprintStatementTemplate::Assignment => "value = 0",
        NteBlueprintStatementTemplate::StateAssignment => "state.value = value",
        NteBlueprintStatementTemplate::If => "if value == True:",
        NteBlueprintStatementTemplate::Loop => "for index in range(1):",
        NteBlueprintStatementTemplate::GameValue => "character = game.player_character",
        NteBlueprintStatementTemplate::SdkCall => {
            "hp = sdk.character_hp_milli(game.player_character)"
        }
        NteBlueprintStatementTemplate::MemoryRead => {
            "value = memory.read_u64(game.player_character, 0x0)"
        }
        NteBlueprintStatementTemplate::MemoryWrite => {
            "memory.write_u64(game.player_character, 0x0, value)"
        }
        NteBlueprintStatementTemplate::Cache => "cached = cache.remember(1, value)",
        NteBlueprintStatementTemplate::UnrealCall => {
            "called = unreal.call(game.player_controller, function)"
        }
        NteBlueprintStatementTemplate::ProcessEvent => "ready = event.next()",
        NteBlueprintStatementTemplate::IpcEmit => "ipc.emit(\"event.name\", value)",
        NteBlueprintStatementTemplate::IpcBind => {
            "ipc.bind(game.player_state, game.player_controller)"
        }
        NteBlueprintStatementTemplate::Equipment => "equipment.prepare(game.player_state)",
        NteBlueprintStatementTemplate::CombatClock => {
            "combat_clock.forward(pause_mask, state_flags)"
        }
        NteBlueprintStatementTemplate::Log => "log.info(\"message\")",
        NteBlueprintStatementTemplate::Comment => "# Describe this step",
    }
}

fn add_nte_blueprint_statement(
    editor: &mut NteBlueprintEditorState,
    template: NteBlueprintStatementTemplate,
) {
    let (insertion_index, indent) = match editor.selected {
        NteBlueprintSelection::Manifest => (
            editor.statements.len(),
            editor
                .statements
                .first()
                .map(|statement| statement.indent)
                .unwrap_or(0),
        ),
        NteBlueprintSelection::Statement(_) => {
            let block = nte_blueprint_selected_block(editor)
                .expect("selected Blueprint statement belongs to a flow block");
            (block.end, editor.statements[block.start].indent)
        }
    };
    let id = editor.next_statement_id;
    editor.next_statement_id += 1;
    let source = nte_blueprint_template_source(template, editor.language).to_owned();
    let opens_block = nte_blueprint_statement_opens_block(&source);
    editor.statements.insert(
        insertion_index,
        NteBlueprintStatement {
            id,
            indent,
            leading_blank_lines: usize::from(insertion_index != 0),
            source,
            position: egui::Pos2::ZERO,
            description: String::new(),
        },
    );
    if opens_block {
        let child_id = editor.next_statement_id;
        editor.next_statement_id += 1;
        editor.statements.insert(
            insertion_index + 1,
            NteBlueprintStatement {
                id: child_id,
                indent: indent + 1,
                leading_blank_lines: 0,
                source: match editor.language {
                    NteSourceLanguage::Legacy => "# Add actions to this step",
                    NteSourceLanguage::Cpp => "// Add actions to this step",
                }
                .to_owned(),
                position: egui::Pos2::ZERO,
                description: String::new(),
            },
        );
    }
    editor.selected = NteBlueprintSelection::Statement(id);
    layout_nte_blueprint(editor);
    editor.scene_rect = nte_blueprint_selection_scene(editor);
    sync_nte_blueprint_capabilities(editor);
    editor.feedback = Some(t("Step added"));
}

fn add_nte_blueprint_action_to_block(
    editor: &mut NteBlueprintEditorState,
    block: NteBlueprintBlockRange,
    template: NteBlueprintStatementTemplate,
) {
    if editor.language == NteSourceLanguage::Cpp
        && nte_blueprint_statement_opens_block(&editor.statements[block.start].source)
    {
        let child_index = block.end;
        let child_indent = editor.statements[block.start].indent + 1;
        let source = nte_blueprint_template_source(template, editor.language).to_owned();
        let child = editor
            .statements
            .get(child_index)
            .filter(|statement| statement.indent == child_indent)
            .expect("a C++ Blueprint block retains its indented child");
        let placeholder = child.source == "// Add actions to this step";
        if placeholder {
            editor.statements[child_index].source = source;
            editor.selected = NteBlueprintSelection::Statement(editor.statements[child_index].id);
        } else {
            let id = editor.next_statement_id;
            editor.next_statement_id += 1;
            let position = child.position;
            let description = child.description.clone();
            let leading_blank_lines = child.leading_blank_lines;
            editor.statements.insert(
                child_index,
                NteBlueprintStatement {
                    id,
                    indent: child_indent,
                    leading_blank_lines,
                    source,
                    position,
                    description,
                },
            );
            editor.statements[child_index + 1].description.clear();
            editor.statements[child_index + 1].leading_blank_lines = 0;
            editor.selected = NteBlueprintSelection::Statement(id);
        }
        sync_nte_blueprint_capabilities(editor);
        editor.feedback = Some(t("Action added to step"));
        return;
    }

    let structural = nte_blueprint_statement_opens_block(&editor.statements[block.end - 1].source);
    let insertion_index = if structural { block.end - 1 } else { block.end };
    let starts_block = insertion_index == block.start;
    let position = editor.statements[block.start].position;
    let description = if starts_block {
        editor.statements[block.start].description.clone()
    } else {
        String::new()
    };
    let id = editor.next_statement_id;
    editor.next_statement_id += 1;
    editor.statements.insert(
        insertion_index,
        NteBlueprintStatement {
            id,
            indent: editor.statements[block.start].indent,
            leading_blank_lines: 0,
            source: nte_blueprint_template_source(template, editor.language).to_owned(),
            position,
            description,
        },
    );
    if starts_block {
        editor.statements[block.start + 1].description.clear();
    }
    editor.selected = NteBlueprintSelection::Statement(editor.statements[block.start].id);
    sync_nte_blueprint_capabilities(editor);
    editor.feedback = Some(t("Action added to step"));
}

fn layout_nte_blueprint(editor: &mut NteBlueprintEditorState) {
    let flow = nte_blueprint_control_flow(editor);
    if flow.blocks.is_empty() {
        return;
    }

    let ranks = nte_blueprint_flow_ranks(&flow);
    let mut columns = vec![0.0_f32; flow.blocks.len()];
    let mut column_proposals = vec![Vec::new(); flow.blocks.len()];
    for index in 0..flow.blocks.len() {
        if !column_proposals[index].is_empty() {
            columns[index] =
                column_proposals[index].iter().sum::<f32>() / column_proposals[index].len() as f32;
        }
        for edge in flow
            .edges
            .iter()
            .filter(|edge| edge.from == index && edge.to > index)
        {
            let offset = match edge.kind {
                NteBlueprintFlowEdgeKind::True => -1.0,
                NteBlueprintFlowEdgeKind::False => 1.0,
                NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => 0.0,
            };
            column_proposals[edge.to].push(columns[index] + offset);
        }
    }

    let rank_count = ranks.iter().copied().max().unwrap_or(0) + 1;
    let mut rank_heights = vec![0.0_f32; rank_count];
    for (index, block) in flow.blocks.iter().copied().enumerate() {
        rank_heights[ranks[index]] =
            rank_heights[ranks[index]].max(nte_blueprint_block_height(editor, block));
    }
    let mut rank_y = vec![NTE_BLUEPRINT_BASE_Y; rank_count];
    for rank in 1..rank_count {
        rank_y[rank] = rank_y[rank - 1] + rank_heights[rank - 1] + NTE_BLUEPRINT_GRAPH_ROW_GAP;
    }

    let mut positions = vec![egui::Pos2::ZERO; flow.blocks.len()];
    for (rank, y) in rank_y.iter().copied().enumerate() {
        let mut nodes = (0..flow.blocks.len())
            .filter(|index| ranks[*index] == rank)
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            columns[*left]
                .total_cmp(&columns[*right])
                .then(left.cmp(right))
        });
        let first_x = -(nodes.len() as f32 - 1.0) * NTE_BLUEPRINT_GRAPH_COLUMN_STEP / 2.0;
        for (slot, index) in nodes.into_iter().enumerate() {
            positions[index] =
                egui::pos2(first_x + slot as f32 * NTE_BLUEPRINT_GRAPH_COLUMN_STEP, y);
        }
    }
    let min_x = positions
        .iter()
        .map(|position| position.x)
        .fold(f32::INFINITY, f32::min);
    let translate_x = 40.0 - min_x;
    for (block, mut position) in flow.blocks.into_iter().zip(positions) {
        position.x += translate_x;
        for statement in &mut editor.statements[block.start..block.end] {
            statement.position = position;
        }
    }
}

fn nte_blueprint_flow_ranks(flow: &NteBlueprintControlFlow) -> Vec<usize> {
    let mut ranks = vec![0usize; flow.blocks.len()];
    for index in 0..flow.blocks.len() {
        for edge in flow
            .edges
            .iter()
            .filter(|edge| edge.from == index && edge.to > index)
        {
            ranks[edge.to] = ranks[edge.to].max(ranks[index] + 1);
        }
    }
    ranks
}

fn nte_blueprint_initial_scene(editor: &NteBlueprintEditorState) -> egui::Rect {
    let bounds = nte_blueprint_graph_bounds(editor);
    if bounds.width() <= NTE_BLUEPRINT_OVERVIEW_MAX_WIDTH
        && bounds.height() <= NTE_BLUEPRINT_OVERVIEW_MAX_HEIGHT
    {
        return egui::Rect::from_center_size(
            bounds.center(),
            egui::vec2(
                bounds.width().max(NTE_BLUEPRINT_COMFORT_WIDTH),
                bounds.height().max(NTE_BLUEPRINT_COMFORT_HEIGHT),
            ),
        );
    }
    egui::Rect::from_min_size(
        bounds.min,
        egui::vec2(NTE_BLUEPRINT_VIEW_WIDTH, NTE_BLUEPRINT_VIEW_HEIGHT),
    )
}

fn nte_blueprint_selection_scene(editor: &NteBlueprintEditorState) -> egui::Rect {
    let Some(block) = nte_blueprint_selected_block(editor) else {
        return nte_blueprint_initial_scene(editor);
    };
    let position = editor.statements[block.start].position;
    egui::Rect::from_center_size(
        position + egui::vec2(NTE_BLUEPRINT_NODE_WIDTH / 2.0, 46.0),
        egui::vec2(NTE_BLUEPRINT_COMFORT_WIDTH, NTE_BLUEPRINT_COMFORT_HEIGHT),
    )
}

fn nte_blueprint_graph_bounds(editor: &NteBlueprintEditorState) -> egui::Rect {
    let blocks = nte_blueprint_blocks(editor);
    let Some(first) = blocks.first().copied() else {
        return egui::Rect::from_min_size(
            egui::pos2(40.0, NTE_BLUEPRINT_BASE_Y),
            egui::vec2(NTE_BLUEPRINT_NODE_WIDTH, 92.0),
        )
        .expand(40.0);
    };
    let mut bounds = egui::Rect::from_min_size(
        editor.statements[first.start].position,
        egui::vec2(
            NTE_BLUEPRINT_NODE_WIDTH,
            nte_blueprint_block_height(editor, first),
        ),
    );
    for block in blocks.into_iter().skip(1) {
        let statement = &editor.statements[block.start];
        bounds = bounds.union(egui::Rect::from_min_size(
            statement.position,
            egui::vec2(
                NTE_BLUEPRINT_NODE_WIDTH,
                nte_blueprint_block_height(editor, block),
            ),
        ));
    }
    let flow = nte_blueprint_control_flow(editor);
    for wire in nte_blueprint_routed_wires(editor, &flow) {
        for point in
            nte_blueprint_wire_points(wire.start, wire.end, wire.edge.kind, wire.route, wire.lane)
        {
            bounds.min.x = bounds.min.x.min(point.x);
            bounds.min.y = bounds.min.y.min(point.y);
            bounds.max.x = bounds.max.x.max(point.x);
            bounds.max.y = bounds.max.y.max(point.y);
        }
    }
    bounds.expand(40.0)
}

fn nte_blueprint_canvas(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
    accepts_scroll: bool,
) {
    let mut canvas_palette = blueprint_canvas_palette(ui.visuals().dark_mode);
    canvas_palette.selection = palette.selected_border;
    let mut scene_rect = editor.scene_rect;
    let grid_rect = scene_rect.expand(120.0);
    let canvas_size = egui::vec2(ui.available_width(), ui.available_height().max(480.0));
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
        .stroke(Stroke::new(1.0_f32, palette.border))
        .corner_radius(8)
        .show(&mut canvas_ui, |ui| {
            let (outer_rect, _) =
                ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::hover());
            let zoom_range = egui::Rangef::new(0.05, 2.0);
            let scale = (outer_rect.size() / scene_rect.size())
                .min_elem()
                .clamp(zoom_range.min, zoom_range.max);
            let mut to_global = egui::emath::TSTransform::from_translation(
                outer_rect.center().to_vec2() - scale * scene_rect.center().to_vec2(),
            ) * egui::emath::TSTransform::from_scaling(scale);
            let scene_layer_id =
                egui::LayerId::new(ui.layer_id().order, ui.id().with("nte_blueprint_scene"));
            ui.ctx().set_sublayer(ui.layer_id(), scene_layer_id);
            let visible_clip = outer_rect.intersect(ui.clip_rect());
            let scene_content_rect = nte_blueprint_graph_bounds(editor)
                .union(scene_rect)
                .expand(1_200.0);
            let mut scene_ui = ui.new_child(
                egui::UiBuilder::new()
                    .layer_id(scene_layer_id)
                    .max_rect(scene_content_rect),
            );
            scene_ui.set_clip_rect(to_global.inverse() * visible_clip);
            scene_ui
                .ctx()
                .set_transform_layer(scene_layer_id, to_global);
            let mut pan_response = scene_ui.interact(
                to_global.inverse() * outer_rect,
                scene_ui.id().with("nte_blueprint_scene_background"),
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
                let flow = nte_blueprint_control_flow(editor);
                paint_nte_blueprint_wires(ui.painter(), editor, &flow, canvas_palette);
                for (block_index, block) in flow.blocks.iter().copied().enumerate() {
                    let category = nte_blueprint_block_category(editor, block);
                    let title = nte_blueprint_block_title(editor, block);
                    let summary =
                        nte_blueprint_truncate(&nte_blueprint_block_summary(editor, block), 52);
                    let code_lines = nte_blueprint_block_code_lines(editor, block);
                    let height = nte_blueprint_block_height(editor, block);
                    let selected = matches!(
                        editor.selected,
                        NteBlueprintSelection::Statement(selected_id)
                            if editor.statements[block.start..block.end]
                                .iter()
                                .any(|statement| statement.id == selected_id)
                    );
                    let first_id = editor.statements[block.start].id;
                    let badge = tf("Block {}", &[&(block_index + 1).to_string()]);
                    let has_input = flow.edges.iter().any(|edge| edge.to == block_index);
                    let output_kinds = flow
                        .edges
                        .iter()
                        .filter(|edge| edge.from == block_index)
                        .map(|edge| edge.kind)
                        .collect::<Vec<_>>();
                    let structured = editor.language == NteSourceLanguage::Cpp
                        && nte_blueprint_statement_opens_block(
                            &editor.statements[block.start].source,
                        );
                    let response = if structured {
                        let source = editor.statements[block.start].source.clone();
                        nte_blueprint_control_node(
                            ui,
                            &mut editor.statements[block.start].position,
                            ("nte_blueprint_statement", first_id),
                            &source,
                            selected,
                            egui::vec2(NTE_BLUEPRINT_NODE_WIDTH, height),
                            category.color(),
                            canvas_palette,
                            has_input,
                            &output_kinds,
                            &badge,
                        )
                    } else {
                        nte_blueprint_node(
                            ui,
                            &mut editor.statements[block.start].position,
                            ("nte_blueprint_statement", first_id),
                            category.key(),
                            &title,
                            &summary,
                            selected,
                            egui::vec2(NTE_BLUEPRINT_NODE_WIDTH, height),
                            category.color(),
                            canvas_palette,
                            has_input,
                            &output_kinds,
                            &badge,
                            &code_lines,
                        )
                    };
                    let position = editor.statements[block.start].position;
                    for statement in &mut editor.statements[block.start + 1..block.end] {
                        statement.position = position;
                    }
                    if response.clicked() || response.drag_started() {
                        editor.selected = NteBlueprintSelection::Statement(first_id);
                    }
                }
            }
            pan_response.context_menu(|ui| {
                ui.label(RichText::new(t("Add step")).strong());
                nte_blueprint_statement_palette(ui, editor);
            });
            if pan_response.changed() {
                scene_rect = to_global.inverse() * outer_rect;
            }
            if accepts_scroll && pan_response.hovered() {
                ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
            }
        });
    editor.scene_rect = scene_rect;
}

fn paint_nte_blueprint_wires(
    painter: &egui::Painter,
    editor: &NteBlueprintEditorState,
    flow: &NteBlueprintControlFlow,
    palette: BlueprintCanvasPalette,
) {
    let wires = nte_blueprint_routed_wires(editor, flow);
    for wire in wires {
        let junction = (wire.edge.kind != NteBlueprintFlowEdgeKind::Loop)
            .then(|| nte_blueprint_merge_junction(editor, flow, wire.edge.to))
            .flatten();
        paint_nte_blueprint_wire(
            painter,
            wire,
            junction.unwrap_or(wire.end),
            nte_blueprint_flow_edge_color(wire.edge.kind, palette),
            junction.is_none(),
        );
    }
    for target in 0..flow.blocks.len() {
        let Some(junction) = nte_blueprint_merge_junction(editor, flow, target) else {
            continue;
        };
        let block = flow.blocks[target];
        let end = editor.statements[block.start].position
            + egui::vec2(NTE_BLUEPRINT_NODE_WIDTH * 0.5, 0.0);
        painter.line_segment([junction, end], Stroke::new(2.0_f32, palette.execution));
        painter.circle_filled(junction, 4.5, palette.execution);
        paint_nte_blueprint_arrow(painter, end, palette.execution);
    }
}

fn nte_blueprint_merge_junction(
    editor: &NteBlueprintEditorState,
    flow: &NteBlueprintControlFlow,
    target: usize,
) -> Option<egui::Pos2> {
    (flow
        .edges
        .iter()
        .filter(|edge| edge.to == target && edge.kind != NteBlueprintFlowEdgeKind::Loop)
        .count()
        > 1)
    .then(|| {
        let block = flow.blocks[target];
        editor.statements[block.start].position + egui::vec2(NTE_BLUEPRINT_NODE_WIDTH * 0.5, -18.0)
    })
}

fn nte_blueprint_flow_edge_color(
    kind: NteBlueprintFlowEdgeKind,
    palette: BlueprintCanvasPalette,
) -> Color32 {
    match kind {
        NteBlueprintFlowEdgeKind::Next => palette.execution,
        NteBlueprintFlowEdgeKind::True => Color32::from_rgb(73, 184, 108),
        NteBlueprintFlowEdgeKind::False => Color32::from_rgb(224, 86, 94),
        NteBlueprintFlowEdgeKind::Loop => Color32::from_rgb(82, 156, 224),
    }
}

fn nte_blueprint_routed_wires(
    editor: &NteBlueprintEditorState,
    flow: &NteBlueprintControlFlow,
) -> Vec<NteBlueprintRoutedWire> {
    let ranks = nte_blueprint_flow_ranks(flow);
    let mut left_lanes: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut right_lanes: Vec<Vec<(f32, f32)>> = Vec::new();
    flow.edges
        .iter()
        .copied()
        .map(|edge| {
            let from = flow.blocks[edge.from];
            let to = flow.blocks[edge.to];
            let start_position = editor.statements[from.start].position;
            let start_x = match edge.kind {
                NteBlueprintFlowEdgeKind::True => NTE_BLUEPRINT_NODE_WIDTH * 0.32,
                NteBlueprintFlowEdgeKind::False => NTE_BLUEPRINT_NODE_WIDTH * 0.68,
                NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => {
                    NTE_BLUEPRINT_NODE_WIDTH * 0.5
                }
            };
            let start =
                start_position + egui::vec2(start_x, nte_blueprint_block_height(editor, from));
            let end = editor.statements[to.start].position
                + egui::vec2(NTE_BLUEPRINT_NODE_WIDTH * 0.5, 0.0);
            let rank_gap = ranks[edge.to].saturating_sub(ranks[edge.from]);
            let route = nte_blueprint_wire_route(start, end, edge.kind, rank_gap);
            let interval = (start.y.min(end.y), start.y.max(end.y));
            let lane = match route {
                NteBlueprintWireRoute::OuterLeft => {
                    reserve_nte_blueprint_wire_lane(&mut left_lanes, interval)
                }
                NteBlueprintWireRoute::OuterRight => {
                    reserve_nte_blueprint_wire_lane(&mut right_lanes, interval)
                }
                NteBlueprintWireRoute::Direct | NteBlueprintWireRoute::Channel => 0,
            };
            NteBlueprintRoutedWire {
                edge,
                start,
                end,
                route,
                lane,
            }
        })
        .collect()
}

fn reserve_nte_blueprint_wire_lane(
    lanes: &mut Vec<Vec<(f32, f32)>>,
    interval: (f32, f32),
) -> usize {
    let lane = lanes
        .iter()
        .position(|occupied| {
            occupied
                .iter()
                .all(|other| interval.1 + 8.0 < other.0 || interval.0 > other.1 + 8.0)
        })
        .unwrap_or(lanes.len());
    if lane == lanes.len() {
        lanes.push(Vec::new());
    }
    lanes[lane].push(interval);
    lane
}

fn nte_blueprint_wire_route(
    start: egui::Pos2,
    end: egui::Pos2,
    kind: NteBlueprintFlowEdgeKind,
    rank_gap: usize,
) -> NteBlueprintWireRoute {
    if matches!(kind, NteBlueprintFlowEdgeKind::Loop) || end.y <= start.y {
        return NteBlueprintWireRoute::OuterLeft;
    }
    if rank_gap <= 1 {
        return if (start.x - end.x).abs() < 1.0 {
            NteBlueprintWireRoute::Direct
        } else {
            NteBlueprintWireRoute::Channel
        };
    }
    match kind {
        NteBlueprintFlowEdgeKind::True => NteBlueprintWireRoute::OuterLeft,
        NteBlueprintFlowEdgeKind::False => NteBlueprintWireRoute::OuterRight,
        NteBlueprintFlowEdgeKind::Next => {
            if end.x < start.x {
                NteBlueprintWireRoute::OuterLeft
            } else {
                NteBlueprintWireRoute::OuterRight
            }
        }
        NteBlueprintFlowEdgeKind::Loop => {
            unreachable!("loop edges are routed before forward edges")
        }
    }
}

fn nte_blueprint_wire_points(
    start: egui::Pos2,
    end: egui::Pos2,
    kind: NteBlueprintFlowEdgeKind,
    route: NteBlueprintWireRoute,
    lane: usize,
) -> Vec<egui::Pos2> {
    match route {
        NteBlueprintWireRoute::Direct => vec![start, end],
        NteBlueprintWireRoute::Channel => {
            let track_y = (start.y + end.y) / 2.0;
            vec![
                start,
                egui::pos2(start.x, track_y),
                egui::pos2(end.x, track_y),
                end,
            ]
        }
        NteBlueprintWireRoute::OuterLeft | NteBlueprintWireRoute::OuterRight => {
            let start_offset = match kind {
                NteBlueprintFlowEdgeKind::True => NTE_BLUEPRINT_NODE_WIDTH * 0.32,
                NteBlueprintFlowEdgeKind::False => NTE_BLUEPRINT_NODE_WIDTH * 0.68,
                NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => {
                    NTE_BLUEPRINT_NODE_WIDTH * 0.5
                }
            };
            let start_left = start.x - start_offset;
            let start_right = start_left + NTE_BLUEPRINT_NODE_WIDTH;
            let end_left = end.x - NTE_BLUEPRINT_NODE_WIDTH * 0.5;
            let end_right = end.x + NTE_BLUEPRINT_NODE_WIDTH * 0.5;
            let lane_offset = lane as f32 * 14.0;
            let rail_x = match route {
                NteBlueprintWireRoute::OuterLeft => start_left.min(end_left) - 28.0 - lane_offset,
                NteBlueprintWireRoute::OuterRight => {
                    start_right.max(end_right) + 28.0 + lane_offset
                }
                NteBlueprintWireRoute::Direct | NteBlueprintWireRoute::Channel => {
                    unreachable!("outer routes select an outer rail")
                }
            };
            vec![
                start,
                start + egui::vec2(0.0, 18.0),
                egui::pos2(rail_x, start.y + 18.0),
                egui::pos2(rail_x, end.y - 18.0),
                end - egui::vec2(0.0, 18.0),
                end,
            ]
        }
    }
}

fn paint_nte_blueprint_wire(
    painter: &egui::Painter,
    wire: NteBlueprintRoutedWire,
    end: egui::Pos2,
    color: Color32,
    paint_arrow: bool,
) {
    painter.add(egui::Shape::line(
        nte_blueprint_wire_points(wire.start, end, wire.edge.kind, wire.route, wire.lane),
        Stroke::new(2.0_f32, color),
    ));
    if paint_arrow {
        paint_nte_blueprint_arrow(painter, end, color);
    }
}

fn paint_nte_blueprint_arrow(painter: &egui::Painter, end: egui::Pos2, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            end,
            end + egui::vec2(-5.0, -8.0),
            end + egui::vec2(5.0, -8.0),
        ],
        color,
        Stroke::NONE,
    ));
}

#[allow(clippy::too_many_arguments)]
fn nte_blueprint_control_node(
    ui: &mut egui::Ui,
    position: &mut egui::Pos2,
    id_source: impl std::hash::Hash,
    source: &str,
    selected: bool,
    size: egui::Vec2,
    color: Color32,
    palette: BlueprintCanvasPalette,
    has_input: bool,
    output_kinds: &[NteBlueprintFlowEdgeKind],
    badge: &str,
) -> egui::Response {
    let rect = egui::Rect::from_min_size(*position, size);
    let response = ui.interact(rect, ui.id().with(id_source), egui::Sense::click_and_drag());
    if response.dragged() {
        *position += response.drag_delta();
    }
    ui.painter().rect(
        rect,
        10.0,
        color,
        Stroke::new(
            if selected { 2.5_f32 } else { 1.0_f32 },
            if selected {
                palette.selection
            } else {
                Color32::from_white_alpha(90)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let (keyword, expression) = nte_blueprint_control_parts(source);
    ui.painter().text(
        rect.left_top() + egui::vec2(11.0, 11.0),
        egui::Align2::LEFT_TOP,
        keyword,
        egui::FontId::monospace(12.0),
        Color32::WHITE,
    );
    ui.painter().text(
        rect.right_top() + egui::vec2(-10.0, 11.0),
        egui::Align2::RIGHT_TOP,
        badge,
        egui::FontId::proportional(9.0),
        Color32::from_white_alpha(220),
    );
    if expression.is_empty() {
        ui.painter().text(
            rect.left_bottom() + egui::vec2(11.0, -11.0),
            egui::Align2::LEFT_BOTTOM,
            t("Fallback Branch"),
            egui::FontId::proportional(10.0),
            Color32::from_white_alpha(220),
        );
    } else {
        let expression_rect = egui::Rect::from_min_max(
            rect.left_top() + egui::vec2(40.0, 27.0),
            rect.right_bottom() - egui::vec2(10.0, 20.0),
        );
        ui.painter().rect_filled(
            expression_rect,
            expression_rect.height() / 2.0,
            palette.node_fill,
        );
        ui.painter().text(
            expression_rect.left_center() + egui::vec2(9.0, 0.0),
            egui::Align2::LEFT_CENTER,
            nte_blueprint_truncate(&expression, 44),
            egui::FontId::monospace(9.5),
            palette.text,
        );
    }
    if has_input {
        let center = rect.center_top();
        ui.painter().circle_filled(center, 5.5, palette.execution);
        ui.painter()
            .circle_stroke(center, 7.0, Stroke::new(1.5_f32, color));
    }
    for kind in output_kinds {
        let x = match kind {
            NteBlueprintFlowEdgeKind::True => rect.left() + rect.width() * 0.32,
            NteBlueprintFlowEdgeKind::False => rect.left() + rect.width() * 0.68,
            NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => rect.center().x,
        };
        let center = egui::pos2(x, rect.bottom());
        let edge_color = nte_blueprint_flow_edge_color(*kind, palette);
        ui.painter().circle_filled(center, 5.5, edge_color);
        ui.painter()
            .circle_stroke(center, 7.0, Stroke::new(1.5_f32, color));
        if keyword != "for"
            && matches!(
                kind,
                NteBlueprintFlowEdgeKind::True | NteBlueprintFlowEdgeKind::False
            )
        {
            ui.painter().text(
                center - egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_BOTTOM,
                t(match kind {
                    NteBlueprintFlowEdgeKind::True => "True",
                    NteBlueprintFlowEdgeKind::False => "False",
                    NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => {
                        unreachable!("only branch outputs have labels")
                    }
                }),
                egui::FontId::proportional(9.0),
                Color32::from_white_alpha(230),
            );
        }
    }
    response
}

fn nte_blueprint_control_parts(source: &str) -> (&'static str, String) {
    let source = source.trim();
    let (keyword, expression) = if let Some(expression) = source.strip_prefix("else if") {
        ("else if", expression)
    } else if let Some(expression) = source.strip_prefix("if") {
        ("if", expression)
    } else if let Some(expression) = source.strip_prefix("for") {
        ("for", expression)
    } else {
        ("else", "")
    };
    (
        keyword,
        expression
            .trim()
            .strip_prefix('(')
            .and_then(|expression| expression.strip_suffix(')'))
            .unwrap_or_else(|| expression.trim())
            .to_owned(),
    )
}

#[allow(clippy::too_many_arguments)]
fn nte_blueprint_node(
    ui: &mut egui::Ui,
    position: &mut egui::Pos2,
    id_source: impl std::hash::Hash,
    category: &str,
    title: &str,
    summary: &str,
    selected: bool,
    size: egui::Vec2,
    header_color: Color32,
    palette: BlueprintCanvasPalette,
    has_input: bool,
    output_kinds: &[NteBlueprintFlowEdgeKind],
    badge: &str,
    code_lines: &[String],
) -> egui::Response {
    let rect = egui::Rect::from_min_size(*position, size);
    let header = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 22.0));
    let response = ui.interact(rect, ui.id().with(id_source), egui::Sense::click_and_drag());
    if response.dragged() {
        *position += response.drag_delta();
    }
    ui.painter().rect(
        rect,
        5.0,
        palette.node_fill,
        Stroke::new(
            if selected { 2.0_f32 } else { 1.0_f32 },
            if selected {
                palette.selection
            } else {
                palette.node_border
            },
        ),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(header, 5.0, header_color);
    ui.painter().rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(header.left(), header.bottom() - 5.0),
            header.right_bottom(),
        ),
        0.0,
        header_color,
    );
    ui.painter().text(
        header.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        t(category),
        egui::FontId::proportional(9.0),
        Color32::WHITE,
    );
    ui.painter().text(
        header.right_center() - egui::vec2(8.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        badge,
        egui::FontId::proportional(9.0),
        Color32::from_white_alpha(220),
    );
    ui.painter().text(
        rect.left_top() + egui::vec2(9.0, 29.0),
        egui::Align2::LEFT_TOP,
        t(title),
        egui::FontId::proportional(12.0),
        palette.text,
    );
    if code_lines.is_empty() {
        ui.painter().text(
            rect.left_top() + egui::vec2(9.0, 46.0),
            egui::Align2::LEFT_TOP,
            summary,
            egui::FontId::proportional(10.0),
            palette.muted,
        );
    } else {
        for (line_index, line) in code_lines.iter().enumerate() {
            ui.painter().text(
                rect.left_top() + egui::vec2(9.0, 45.0 + line_index as f32 * 15.0),
                egui::Align2::LEFT_TOP,
                line,
                egui::FontId::monospace(9.5),
                if line.starts_with("//") {
                    palette.muted
                } else {
                    palette.text
                },
            );
        }
    }
    if has_input {
        let center = rect.center_top();
        ui.painter().circle_filled(center, 5.5, palette.execution);
        ui.painter()
            .circle_stroke(center, 7.0, Stroke::new(1.5_f32, palette.node_fill));
    }
    for kind in output_kinds {
        let x = match kind {
            NteBlueprintFlowEdgeKind::True => rect.left() + rect.width() * 0.32,
            NteBlueprintFlowEdgeKind::False => rect.left() + rect.width() * 0.68,
            NteBlueprintFlowEdgeKind::Next | NteBlueprintFlowEdgeKind::Loop => rect.center().x,
        };
        let center = egui::pos2(x, rect.bottom());
        let color = nte_blueprint_flow_edge_color(*kind, palette);
        ui.painter().circle_filled(center, 5.5, color);
        ui.painter()
            .circle_stroke(center, 7.0, Stroke::new(1.5_f32, palette.node_fill));
    }
    response
}

fn nte_blueprint_statement_title(source: &str) -> &'static str {
    match nte_blueprint_statement_kind(source) {
        NteBlueprintStatementKind::Assignment => "Set Value",
        NteBlueprintStatementKind::If => "Conditional Branch",
        NteBlueprintStatementKind::Elif => "Alternative Condition",
        NteBlueprintStatementKind::Else => "Fallback Branch",
        NteBlueprintStatementKind::Loop => "Bounded Loop",
        NteBlueprintStatementKind::Call => "Host API Call",
        NteBlueprintStatementKind::Comment => "Comment",
    }
}

fn nte_blueprint_inspector_window(
    ctx: &egui::Context,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
    canvas_rect: egui::Rect,
) {
    if nte_blueprint_selected_block(editor).is_none() {
        return;
    }

    let constrain_rect = canvas_rect.intersect(ctx.content_rect());
    let default_width = constrain_rect.width().min(400.0);
    let default_height = constrain_rect.height().min(520.0);
    let default_position = egui::pos2(
        constrain_rect.right() - default_width - 12.0,
        constrain_rect.top() + 12.0,
    );
    let mut open = true;
    egui::Window::new(t("Edit step"))
        .id(egui::Id::new("nte_blueprint_node_inspector_window"))
        .open(&mut open)
        .default_pos(default_position)
        .default_size(egui::vec2(default_width, default_height))
        .min_size(egui::vec2(300.0, 220.0))
        .max_size(egui::vec2(
            constrain_rect.width().min(520.0),
            constrain_rect.height(),
        ))
        .resizable(true)
        .collapsible(false)
        .constrain_to(constrain_rect)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("nte_blueprint_node_details_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    nte_blueprint_inspector(ui, editor, palette);
                });
        });
    if !open {
        editor.selected = NteBlueprintSelection::Manifest;
    }
}

fn nte_blueprint_inspector(
    ui: &mut egui::Ui,
    editor: &mut NteBlueprintEditorState,
    palette: ModEditorPalette,
) {
    use egui_material_icons::icons::{ICON_ARROW_DOWNWARD, ICON_ARROW_UPWARD, ICON_DELETE};

    let value_options = nte_blueprint_value_options(editor);
    egui::Frame::new()
        .fill(palette.editor)
        .stroke(Stroke::new(1.0_f32, palette.selected_border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let block = nte_blueprint_selected_block(editor)
                .expect("step details are only shown for a selected flow block");
            let blocks = nte_blueprint_blocks(editor);
            let block_index = blocks
                .iter()
                .position(|candidate| *candidate == block)
                .expect("selected Blueprint block exists");
            let structural =
                nte_blueprint_statement_opens_block(&editor.statements[block.end - 1].source);
            let block_title = nte_blueprint_block_title(editor, block);
            let block_summary = nte_blueprint_block_summary(editor, block);
            let mut move_up = false;
            let mut move_down = false;
            let mut indent = false;
            let mut outdent = false;
            let mut remove = false;
            ui.label(
                RichText::new(block_title)
                    .size(17.0)
                    .strong()
                    .color(palette.text),
            );
            ui.label(RichText::new(block_summary).color(palette.muted));
            ui.add_space(8.0);
            ui.label(
                RichText::new(t("Function description"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            ui.add(
                egui::TextEdit::multiline(&mut editor.statements[block.start].description)
                    .desired_rows(2)
                    .desired_width(ui.available_width())
                    .hint_text(t("Describe what this step does.")),
            );
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("Order")).small().color(palette.muted));
                if mod_editor_icon_button(
                    ui,
                    ICON_ARROW_UPWARD,
                    t("Move step earlier"),
                    block_index > 0,
                    palette,
                )
                .clicked()
                {
                    move_up = true;
                }
                if mod_editor_icon_button(
                    ui,
                    ICON_ARROW_DOWNWARD,
                    t("Move step later"),
                    block_index + 1 < blocks.len(),
                    palette,
                )
                .clicked()
                {
                    move_down = true;
                }
                if mod_editor_icon_button(ui, ICON_DELETE, t("Delete step"), true, palette)
                    .clicked()
                {
                    remove = true;
                }
            });
            ui.add_space(6.0);
            let mut remove_statement = None;
            egui::CollapsingHeader::new(
                RichText::new(tf(
                    "Advanced · {} items",
                    &[&(block.end - block.start).to_string()],
                ))
                .small()
                .strong()
                .color(palette.muted),
            )
            .id_salt((
                "nte_blueprint_step_logic",
                editor.statements[block.start].id,
            ))
            .default_open(false)
            .show(ui, |ui| {
                let can_outdent = nte_blueprint_can_outdent_block(editor, block);
                let can_indent = nte_blueprint_can_indent_block(editor, block);
                ui.label(
                    RichText::new(tf(
                        "Nesting level · {}",
                        &[&(editor.statements[block.start].indent + 1).to_string()],
                    ))
                    .small()
                    .strong()
                    .color(palette.muted),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            can_outdent,
                            egui::Button::new(t("Move outside current branch")).small(),
                        )
                        .on_disabled_hover_text(t(
                            "The current branch must keep at least one child step.",
                        ))
                        .clicked()
                    {
                        outdent = true;
                    }
                    if ui
                        .add_enabled(
                            can_indent,
                            egui::Button::new(t("Nest under previous branch")).small(),
                        )
                        .on_disabled_hover_text(t(
                            "Place this step after a condition or loop before nesting it.",
                        ))
                        .clicked()
                    {
                        indent = true;
                    }
                });
                ui.add_space(5.0);
                for statement_index in block.start..block.end {
                    if !structural {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(tf(
                                    "Item {}",
                                    &[&(statement_index - block.start + 1).to_string()],
                                ))
                                .small()
                                .strong()
                                .color(palette.muted),
                            );
                            if mod_editor_icon_button(
                                ui,
                                ICON_DELETE,
                                t("Remove item from step"),
                                true,
                                palette,
                            )
                            .clicked()
                            {
                                remove_statement = Some(statement_index);
                            }
                        });
                    }
                    nte_blueprint_statement_editor(
                        ui,
                        &mut editor.statements[statement_index],
                        editor.language,
                        palette,
                        &value_options,
                    );
                    if statement_index + 1 < block.end {
                        ui.separator();
                    }
                }
                ui.menu_button(t("Add action inside this step"), |ui| {
                    for template in [
                        NteBlueprintStatementTemplate::Assignment,
                        NteBlueprintStatementTemplate::StateAssignment,
                        NteBlueprintStatementTemplate::GameValue,
                        NteBlueprintStatementTemplate::SdkCall,
                        NteBlueprintStatementTemplate::MemoryRead,
                        NteBlueprintStatementTemplate::MemoryWrite,
                        NteBlueprintStatementTemplate::Cache,
                        NteBlueprintStatementTemplate::UnrealCall,
                        NteBlueprintStatementTemplate::ProcessEvent,
                        NteBlueprintStatementTemplate::IpcEmit,
                        NteBlueprintStatementTemplate::IpcBind,
                        NteBlueprintStatementTemplate::Equipment,
                        NteBlueprintStatementTemplate::CombatClock,
                        NteBlueprintStatementTemplate::Log,
                        NteBlueprintStatementTemplate::Comment,
                    ] {
                        if ui
                            .button(t(nte_blueprint_template_label(template)))
                            .clicked()
                        {
                            add_nte_blueprint_action_to_block(editor, block, template);
                            ui.close();
                        }
                    }
                });
            });

            if move_up {
                let previous = blocks[block_index - 1];
                let previous_blank_lines = editor.statements[previous.start].leading_blank_lines;
                let block_blank_lines = editor.statements[block.start].leading_blank_lines;
                let block_len = block.end - block.start;
                editor.statements[previous.start..block.end]
                    .rotate_left(previous.end - previous.start);
                editor.statements[previous.start].leading_blank_lines = previous_blank_lines;
                editor.statements[previous.start + block_len].leading_blank_lines =
                    block_blank_lines;
                editor.feedback = Some(t("Step moved"));
            } else if move_down {
                let next = blocks[block_index + 1];
                let block_blank_lines = editor.statements[block.start].leading_blank_lines;
                let next_blank_lines = editor.statements[next.start].leading_blank_lines;
                let next_len = next.end - next.start;
                editor.statements[block.start..next.end].rotate_left(block.end - block.start);
                editor.statements[block.start].leading_blank_lines = block_blank_lines;
                editor.statements[block.start + next_len].leading_blank_lines = next_blank_lines;
                editor.feedback = Some(t("Step moved"));
            } else if outdent {
                adjust_nte_blueprint_block_indent(editor, block, false);
                editor.feedback = Some(t("Nesting reduced"));
            } else if indent {
                adjust_nte_blueprint_block_indent(editor, block, true);
                editor.feedback = Some(t("Nesting increased"));
            } else if remove {
                let leading_blank_lines = editor.statements[block.start].leading_blank_lines;
                editor.statements.drain(block.start..block.end);
                if block.start == 0 && !editor.statements.is_empty() {
                    editor.statements[0].leading_blank_lines = leading_blank_lines;
                }
                editor.selected = NteBlueprintSelection::Manifest;
                editor.feedback = Some(t("Step deleted"));
            } else if let Some(statement_index) = remove_statement {
                let leading_blank_lines = editor.statements[statement_index].leading_blank_lines;
                let description = editor.statements[block.start].description.clone();
                editor.statements.remove(statement_index);
                if block.end - block.start == 1 {
                    editor.selected = NteBlueprintSelection::Manifest;
                } else {
                    if statement_index == block.start {
                        editor.statements[block.start].leading_blank_lines = leading_blank_lines;
                        editor.statements[block.start].description = description;
                    }
                    editor.selected =
                        NteBlueprintSelection::Statement(editor.statements[block.start].id);
                }
                editor.feedback = Some(t("Item removed from step"));
            }
            if move_up || move_down || outdent || indent || remove || remove_statement.is_some() {
                layout_nte_blueprint(editor);
                editor.scene_rect = nte_blueprint_selection_scene(editor);
                sync_nte_blueprint_capabilities(editor);
            }
        });
}

fn mod_editor_icon_button(
    ui: &mut egui::Ui,
    icon: egui_material_icons::MaterialIcon,
    hover_text: String,
    enabled: bool,
    palette: ModEditorPalette,
) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(icon.codepoint)
                .font(egui::FontId::new(16.0, icon.font_family()))
                .color(palette.text),
        )
        .small(),
    )
    .on_hover_text(hover_text)
}

fn nte_blueprint_statement_editor(
    ui: &mut egui::Ui,
    statement: &mut NteBlueprintStatement,
    language: NteSourceLanguage,
    palette: ModEditorPalette,
    value_options: &[String],
) {
    match nte_blueprint_statement_kind(&statement.source) {
        NteBlueprintStatementKind::Assignment => {
            let (mut target, mut expression) = split_nte_blueprint_assignment(&statement.source)
                .map(|(target, expression)| (target.to_owned(), expression.to_owned()))
                .expect("assignment statement has a target and expression");
            let target_changed = if language == NteSourceLanguage::Cpp {
                let (parsed_type, parsed_name) = split_nte_blueprint_typed_target(&target);
                let mut type_name = parsed_type.to_owned();
                let mut name = parsed_name.to_owned();
                ui.label(
                    RichText::new(t("Variable type"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                let previous_type = type_name.clone();
                egui::ComboBox::from_id_salt((statement.id, "assignment_type"))
                    .width(ui.available_width())
                    .selected_text(if type_name.is_empty() {
                        t("Existing variable")
                    } else {
                        type_name.clone()
                    })
                    .show_ui(ui, |ui| {
                        for candidate in NTE_BLUEPRINT_CPP_LOCAL_TYPES {
                            ui.selectable_value(
                                &mut type_name,
                                (*candidate).to_owned(),
                                if candidate.is_empty() {
                                    t("Existing variable")
                                } else {
                                    (*candidate).to_owned()
                                },
                            );
                        }
                    });
                ui.label(
                    RichText::new(t("Variable name"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                let name_changed = ui
                    .add(
                        egui::TextEdit::singleline(&mut name)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(ui.available_width()),
                    )
                    .changed();
                if type_name == "bool"
                    && previous_type != "bool"
                    && !matches!(expression.as_str(), "true" | "false")
                {
                    expression = "false".to_owned();
                }
                let changed = type_name != previous_type || name_changed;
                if changed {
                    target = if type_name.is_empty() {
                        name.trim().to_owned()
                    } else {
                        format!("{} {}", type_name, name.trim())
                    };
                }
                changed
            } else {
                ui.label(
                    RichText::new(t("Target"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut target)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                )
                .changed()
            };
            ui.label(
                RichText::new(t("Value source"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            let expression_changed = nte_blueprint_value_selector(
                ui,
                egui::Id::new((statement.id, "assignment_value")),
                &mut expression,
                value_options,
            );
            if target_changed || expression_changed {
                statement.source = format!("{} = {}", target.trim(), expression.trim());
            }
        }
        NteBlueprintStatementKind::If | NteBlueprintStatementKind::Elif => {
            let prefix = if statement.source.trim_start().starts_with("elif ") {
                "elif"
            } else if statement.source.trim_start().starts_with("else if") {
                "else if"
            } else {
                "if"
            };
            let mut condition = statement
                .source
                .trim()
                .strip_prefix(prefix)
                .expect("branch prefix matches its statement kind")
                .trim()
                .trim_end_matches(':')
                .trim()
                .strip_prefix('(')
                .and_then(|condition| condition.strip_suffix(')'))
                .unwrap_or_else(|| {
                    statement
                        .source
                        .trim()
                        .strip_prefix(prefix)
                        .expect("branch prefix matches its statement kind")
                        .trim()
                        .trim_end_matches(':')
                })
                .to_owned();
            ui.label(
                RichText::new(t("Condition"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            let condition_changed = if let Some((left, operator, right)) =
                split_nte_blueprint_comparison(&condition)
            {
                let mut left = left.to_owned();
                let mut operator = operator.to_owned();
                let mut right = right.to_owned();
                ui.label(
                    RichText::new(t("Left value"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                let left_changed = nte_blueprint_value_selector(
                    ui,
                    egui::Id::new((statement.id, "condition_left")),
                    &mut left,
                    value_options,
                );
                ui.label(
                    RichText::new(t("Comparison"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                let previous_operator = operator.clone();
                egui::ComboBox::from_id_salt((statement.id, "condition_operator"))
                    .width(ui.available_width())
                    .selected_text(&operator)
                    .show_ui(ui, |ui| {
                        for candidate in NTE_BLUEPRINT_COMPARISON_OPERATORS {
                            ui.selectable_value(&mut operator, (*candidate).to_owned(), *candidate);
                        }
                    });
                ui.label(
                    RichText::new(t("Right value"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                let right_changed = nte_blueprint_value_selector(
                    ui,
                    egui::Id::new((statement.id, "condition_right")),
                    &mut right,
                    value_options,
                );
                let changed = left_changed || operator != previous_operator || right_changed;
                if changed {
                    condition = format!("{} {} {}", left.trim(), operator, right.trim());
                }
                changed
            } else {
                ui.label(
                    RichText::new(t("Custom expression"))
                        .small()
                        .strong()
                        .color(palette.muted),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut condition)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                )
                .changed()
            };
            if condition_changed {
                statement.source = match language {
                    NteSourceLanguage::Legacy => format!("{prefix} {}:", condition.trim()),
                    NteSourceLanguage::Cpp => format!("{prefix} ({})", condition.trim()),
                };
            }
        }
        NteBlueprintStatementKind::Loop => {
            let mut variable = "index".to_owned();
            let mut count = "1".to_owned();
            if let Some(header) = statement.source.trim().strip_prefix("for ")
                && let Some((parsed_variable, parsed_count)) = header.split_once(" in range(")
                && let Some(parsed_count) = parsed_count.strip_suffix("):")
            {
                variable = parsed_variable.trim().to_owned();
                count = parsed_count.trim().to_owned();
            } else if let Some(header) = statement
                .source
                .trim()
                .strip_prefix("for (std::uint64_t ")
                .and_then(|header| header.strip_suffix(')'))
            {
                let mut clauses = header.split(';').map(str::trim);
                if let Some((parsed_variable, "0")) =
                    clauses.next().and_then(|clause| clause.split_once('='))
                    && let Some((condition_variable, parsed_count)) =
                        clauses.next().and_then(|clause| clause.split_once('<'))
                    && parsed_variable.trim() == condition_variable.trim()
                {
                    variable = parsed_variable.trim().to_owned();
                    count = parsed_count.trim().to_owned();
                }
            }
            ui.label(
                RichText::new(t("Loop variable"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            let variable_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut variable)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                )
                .changed();
            ui.label(
                RichText::new(t("Iteration count (0-64)"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            let count_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut count)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width()),
                )
                .changed();
            if variable_changed || count_changed {
                statement.source = match language {
                    NteSourceLanguage::Legacy => {
                        format!("for {} in range({}):", variable.trim(), count.trim())
                    }
                    NteSourceLanguage::Cpp => format!(
                        "for (std::uint64_t {0} = 0; {0} < {1}; ++{0})",
                        variable.trim(),
                        count.trim()
                    ),
                };
            }
        }
        NteBlueprintStatementKind::Else => {
            ui.label(
                RichText::new(t("Run when earlier conditions do not match.")).color(palette.muted),
            );
        }
        NteBlueprintStatementKind::Call | NteBlueprintStatementKind::Comment => {
            ui.label(
                RichText::new(t("Statement"))
                    .small()
                    .strong()
                    .color(palette.muted),
            );
            ui.add(
                egui::TextEdit::singleline(&mut statement.source)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width()),
            );
        }
    }
}

fn blueprint_mod_toolbar(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    validation_error: Option<&str>,
    palette: ModEditorPalette,
) {
    use egui_material_icons::icons::{ICON_CHECK_CIRCLE, ICON_INFO};

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
                mod_editor_icon(ui, ICON_INFO, 17.0, palette.muted).on_hover_text(format!(
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
                    mod_editor_icon(
                        ui,
                        ICON_CHECK_CIRCLE,
                        15.0,
                        semantic_success(ui.visuals().dark_mode),
                    );
                    ui.label(
                        RichText::new(feedback)
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
        .stroke(Stroke::new(1.0_f32, palette.border))
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
                1.0_f32,
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
                1.0_f32,
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
    painter.add(egui::Shape::line(points, Stroke::new(2.0_f32, color)));
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
            if selected { 2.0_f32 } else { 1.0_f32 },
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
    ui.add_space(5.0);
    if ui.available_width() >= 520.0 {
        ui.columns(2, |columns| {
            blueprint_character_filter(&mut columns[0], projection, characters);
            columns[0].add_space(7.0);
            blueprint_damage_source_filter(&mut columns[0], projection, attack_types);
            blueprint_attribute_filter(&mut columns[1], projection, attributes);
            columns[1].add_space(7.0);
            blueprint_skill_filter(&mut columns[1], projection, skills);
        });
    } else {
        blueprint_character_filter(ui, projection, characters);
        ui.add_space(7.0);
        blueprint_attribute_filter(ui, projection, attributes);
        ui.add_space(7.0);
        blueprint_damage_source_filter(ui, projection, attack_types);
        ui.add_space(7.0);
        blueprint_skill_filter(ui, projection, skills);
    }
    ui.label(
        RichText::new(t(
            "The branch forwards matching damage components and leaves all other captured data unchanged.",
        ))
        .small()
        .color(palette.muted),
    );
}

fn blueprint_character_filter(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    characters: &[(u32, String)],
) {
    ui.label(t("Character")).on_hover_text(t(
        "Only damage caused by selected characters enters this branch.",
    ));
    egui::ComboBox::from_id_salt("blueprint_match_character")
        .width(ui.available_width())
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
}

fn blueprint_attribute_filter(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    attributes: &[String],
) {
    ui.label(t("Attribute")).on_hover_text(t(
        "The elemental or damage attribute recorded on each captured component.",
    ));
    egui::ComboBox::from_id_salt("blueprint_match_attribute")
        .width(ui.available_width())
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
}

fn blueprint_damage_source_filter(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    attack_types: &[String],
) {
    ui.label(t("Damage source")).on_hover_text(t(
        "The captured attack type, such as a normal attack, skill, or other source.",
    ));
    egui::ComboBox::from_id_salt("blueprint_match_source")
        .width(ui.available_width())
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
}

fn blueprint_skill_filter(
    ui: &mut egui::Ui,
    projection: &mut DamageProjectionEditorState,
    skills: &[BlueprintSkillOption],
) {
    ui.label(t("Skill")).on_hover_text(t(
        "The localized skill name; hover an option to see its stable internal key.",
    ));
    egui::ComboBox::from_id_salt("blueprint_match_skill")
        .width(ui.available_width())
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
        ModScriptError::BlueprintTooLarge(id) => tf("Blueprint metadata is too large: {}", &[id]),
        ModScriptError::BlueprintNotUtf8(id) => {
            tf("Blueprint metadata is not valid UTF-8: {}", &[id])
        }
        ModScriptError::InvalidBlueprint(id) => {
            tf("Blueprint metadata has an invalid format: {}", &[id])
        }
        ModScriptError::UnsupportedBlueprintVersion { id, version } => tf(
            "Blueprint metadata {} uses unsupported version {}.",
            &[id, &version.to_string()],
        ),
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
    if completion.label.starts_with("#include")
        || completion.label.starts_with("NTE_")
        || completion.label.starts_with("std::")
    {
        "Declaration"
    } else if completion.label.starts_with("void ") || completion.label.starts_with("for ") {
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
    let mut expect_include_header = false;
    while index < source.len() {
        let character = source[index..]
            .chars()
            .next()
            .expect("index remains on a character boundary");
        let cpp_comment = source[index..].starts_with("//");
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
        } else if let Some(end) = preprocessor_end {
            end
        } else if let Some(end) = include_header_end {
            end
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
        let token_format = if cpp_comment {
            comment.clone()
        } else if preprocessor_end.is_some() {
            expect_include_header = token
                .strip_prefix('#')
                .is_some_and(|directive| directive.trim() == "include");
            keyword.clone()
        } else if include_header_end.is_some() {
            expect_include_header = false;
            string.clone()
        } else if character == '"' {
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

fn nte_cpp_preprocessor_directive_end(source: &str, start: usize) -> Option<usize> {
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
        "alignas"
            | "alignof"
            | "auto"
            | "bool"
            | "break"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "constexpr"
            | "continue"
            | "default"
            | "delete"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "explicit"
            | "false"
            | "float"
            | "for"
            | "if"
            | "inline"
            | "int"
            | "namespace"
            | "new"
            | "noexcept"
            | "nullptr"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "signed"
            | "sizeof"
            | "static"
            | "struct"
            | "switch"
            | "template"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typedef"
            | "typename"
            | "union"
            | "unsigned"
            | "using"
            | "virtual"
            | "void"
            | "volatile"
            | "while"
            | "def"
            | "elif"
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn nte_blueprint_round_trips_existing_script_source() {
        let source = concat!(
            "nte_mod(4)\n",
            "mod(\"telemetry\")\n",
            "requires(\"viewport.tick\")\n",
            "requires(\"game.session\")\n",
            "requires(\"sdk.read\")\n",
            "requires(\"ipc\")\n",
            "state.last_hp = 0\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    character = game.player_character\n",
            "    if character != None:\n",
            "        hp = sdk.character_hp_milli(character)\n",
            "        if hp != state.last_hp:\n",
            "            ipc.emit(\"post.character.health\", hp)\n",
            "            state.last_hp = hp\n",
        );

        let blueprint = parse_nte_blueprint_source(source).unwrap();

        assert_eq!(render_nte_blueprint_source("telemetry", &blueprint), source);
        assert!(validate_nte_blueprint(&blueprint).is_ok());
    }

    #[test]
    fn nte_blueprint_imports_all_bundled_mods() {
        for (id, source, expected_flow_blocks) in [
            (
                "equipment",
                include_str!("../../plugins/nte-mods/equipment.nte"),
                10,
            ),
            (
                "combat-clock",
                include_str!("../../plugins/nte-mods/combat-clock.nte"),
                12,
            ),
            (
                "enemy-telemetry",
                include_str!("../../plugins/nte-mods/enemy-telemetry.nte"),
                32,
            ),
            (
                "character-telemetry",
                include_str!("../../plugins/examples/character-telemetry.nte"),
                8,
            ),
            (
                "reflection-events",
                include_str!("../../plugins/examples/reflection-events.nte"),
                8,
            ),
        ] {
            let source = source.replace("\r\n", "\n").replace('\n', "\r\n");
            let blueprint = parse_nte_blueprint_source(&source).unwrap();
            assert_eq!(
                render_nte_blueprint_source(id, &blueprint),
                source.replace("\r\n", "\n")
            );
            assert_eq!(
                nte_blueprint_blocks(&blueprint).len(),
                expected_flow_blocks,
                "{id} should open as semantic flow blocks"
            );
            assert!(
                validate_nte_blueprint(&blueprint).is_ok(),
                "{id} should be a valid Blueprint"
            );
        }
    }

    #[test]
    fn enemy_telemetry_blueprint_uses_the_control_flow_layout_without_metadata() {
        let root = temp_mod_workspace();
        let mod_directory = root.join("nte-mods");
        fs::create_dir_all(&mod_directory).unwrap();
        fs::write(root.join("nte-mods.enabled"), "nte_mod_set 1\n").unwrap();
        fs::write(
            mod_directory.join("enemy-telemetry.nte"),
            include_str!("../../plugins/nte-mods/enemy-telemetry.nte"),
        )
        .unwrap();
        fs::write(
            mod_directory.join("enemy-telemetry.blueprint.json"),
            include_str!("../../plugins/nte-mods/enemy-telemetry.blueprint.json"),
        )
        .unwrap();

        let workspace = load_mod_script_workspace(&root).unwrap();
        let document = workspace
            .scripts
            .iter()
            .find(|script| script.id == "enemy-telemetry")
            .unwrap();
        let mut blueprint = parse_nte_blueprint_source(&document.source).unwrap();
        apply_nte_blueprint_metadata(&mut blueprint, &document.blueprint, false);
        let positions = nte_blueprint_blocks(&blueprint)
            .into_iter()
            .map(|block| blueprint.statements[block.start].position)
            .collect::<Vec<_>>();
        let bounds = nte_blueprint_graph_bounds(&blueprint);

        assert_eq!(positions.len(), 32);
        assert_eq!(positions[0].y, NTE_BLUEPRINT_BASE_Y);
        assert!(
            positions
                .iter()
                .any(|position| position.x != positions[0].x)
        );
        assert!(bounds.width() <= 1_250.0, "graph bounds: {bounds:?}");
        assert!(bounds.height() <= 3_200.0, "graph bounds: {bounds:?}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nte_blueprint_derives_capabilities_from_statement_nodes() {
        let mut blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "requires(\"log\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    hp = sdk.character_hp_milli(game.player_character)\n",
            "    ipc.emit(\"event.hp\", hp)\n",
        ))
        .unwrap();

        sync_nte_blueprint_capabilities(&mut blueprint);

        assert_eq!(
            required_nte_blueprint_capabilities(&blueprint),
            vec!["viewport.tick", "game.session", "sdk.read", "ipc"]
        );
        let source = render_nte_blueprint_source("example", &blueprint);
        assert!(source.contains("requires(\"game.session\")"));
        assert!(source.contains("requires(\"sdk.read\")"));
        assert!(source.contains("requires(\"ipc\")"));
        assert!(!source.contains("requires(\"log\")"));
    }

    #[test]
    fn nte_blueprint_ignores_capability_names_inside_string_literals() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "requires(\"ipc\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    ipc.emit(\"game.session.sdk.memory.equipment.combat_clock.log\", value)\n",
        ))
        .unwrap();

        assert_eq!(
            required_nte_blueprint_capabilities(&blueprint),
            vec!["viewport.tick", "ipc"]
        );
    }

    #[test]
    fn nte_blueprint_derives_generic_runtime_capabilities_independently() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    viewport = event.viewport\n",
            "    value = memory.read_u32(viewport, 0x20)\n",
            "    memory.write_u32(viewport, 0x24, value)\n",
            "    function = unreal.find_function(viewport, \"Owner\", \"Function\")\n",
            "    unreal.watch(viewport, function)\n",
            "    ready = event.next()\n",
        ))
        .unwrap();

        assert_eq!(
            required_nte_blueprint_capabilities(&blueprint),
            vec![
                "viewport.tick",
                "memory.read",
                "memory.write",
                "unreal.reflection",
                "process.event"
            ]
        );
    }

    #[test]
    fn nte_blueprint_groups_a_logical_phase_with_its_nested_subtree() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    first = 1\n",
            "    second = 2\n",
            "    ipc.emit(\"event.values\", first, second)\n",
            "    if first != second:\n",
            "        state.changed = 1\n",
            "        log.info(\"changed\")\n",
        ))
        .unwrap();

        let blocks = nte_blueprint_blocks(&blueprint);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].end - blocks[0].start, 6);
        assert_eq!(nte_blueprint_block_height(&blueprint, blocks[0]), 92.0);
        assert_eq!(
            nte_blueprint_block_category(&blueprint, blocks[0]),
            NteBlueprintBlockCategory::Action
        );
        assert_eq!(
            nte_blueprint_block_title(&blueprint, blocks[0]),
            t("Send Mod data")
        );
        assert_eq!(
            nte_blueprint_block_summary(&blueprint, blocks[0]),
            t("Send the result to the desktop tool.")
        );
    }

    #[test]
    fn nte_cpp_blueprint_groups_combat_clock_into_basic_blocks() {
        let blueprint =
            parse_nte_blueprint_source(include_str!("../../plugins/nte-mods/combat-clock.nte"))
                .unwrap();
        let blocks = nte_blueprint_blocks(&blueprint);
        let flow = nte_blueprint_control_flow(&blueprint);
        let positions = blocks
            .iter()
            .map(|block| blueprint.statements[block.start].position)
            .collect::<Vec<_>>();
        let bounds = nte_blueprint_graph_bounds(&blueprint);

        assert!(blocks.iter().any(|block| block.end - block.start > 1));
        assert!(blocks.iter().all(|block| {
            !nte_blueprint_statement_opens_block(&blueprint.statements[block.start].source)
                || block.end == block.start + 1
        }));
        let first_condition = blocks
            .iter()
            .position(|block| {
                nte_blueprint_statement_opens_block(&blueprint.statements[block.start].source)
            })
            .unwrap();
        assert!(first_condition > 0);
        assert!(
            !blueprint.statements
                [blocks[first_condition - 1].start..blocks[first_condition - 1].end]
                .iter()
                .any(|statement| nte_blueprint_statement_opens_block(&statement.source))
        );
        assert!(positions.windows(2).all(|pair| pair[0] != pair[1]));
        assert!(bounds.width() <= 500.0);
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == NteBlueprintFlowEdgeKind::True)
        );
        assert!(
            flow.edges
                .iter()
                .any(|edge| edge.kind == NteBlueprintFlowEdgeKind::False)
        );
    }

    #[test]
    fn nte_cpp_control_flow_splits_true_false_paths_and_merges_them() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"branches\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "NTE_REQUIRES(\"log\");\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    const auto value = event.viewport;\n",
            "    if (value != nullptr)\n",
            "    {\n",
            "        nte::log::info(\"true\");\n",
            "    }\n",
            "    else\n",
            "    {\n",
            "        nte::log::info(\"false\");\n",
            "    }\n",
            "    nte::log::info(\"merged\");\n",
            "}\n",
        ))
        .unwrap();
        let flow = nte_blueprint_control_flow(&blueprint);
        let block_index = |needle: &str| {
            flow.blocks
                .iter()
                .position(|block| {
                    blueprint.statements[block.start..block.end]
                        .iter()
                        .any(|statement| statement.source.contains(needle))
                })
                .unwrap()
        };
        let condition = block_index("if (value");
        let true_body = block_index("\"true\"");
        let fallback = block_index("else");
        let false_body = block_index("\"false\"");
        let merge = block_index("\"merged\"");

        for edge in [
            NteBlueprintFlowEdge {
                from: condition,
                to: true_body,
                kind: NteBlueprintFlowEdgeKind::True,
            },
            NteBlueprintFlowEdge {
                from: condition,
                to: fallback,
                kind: NteBlueprintFlowEdgeKind::False,
            },
            NteBlueprintFlowEdge {
                from: true_body,
                to: merge,
                kind: NteBlueprintFlowEdgeKind::Next,
            },
            NteBlueprintFlowEdge {
                from: fallback,
                to: false_body,
                kind: NteBlueprintFlowEdgeKind::Next,
            },
            NteBlueprintFlowEdge {
                from: false_body,
                to: merge,
                kind: NteBlueprintFlowEdgeKind::Next,
            },
        ] {
            assert!(flow.edges.contains(&edge), "missing flow edge {edge:?}");
        }
        let condition_position = blueprint.statements[flow.blocks[condition].start].position;
        let true_position = blueprint.statements[flow.blocks[true_body].start].position;
        let fallback_position = blueprint.statements[flow.blocks[fallback].start].position;
        let false_position = blueprint.statements[flow.blocks[false_body].start].position;
        let merge_position = blueprint.statements[flow.blocks[merge].start].position;
        assert!(condition_position.y < true_position.y);
        assert_eq!(true_position.y, fallback_position.y);
        assert_ne!(true_position.x, fallback_position.x);
        assert!(fallback_position.y < false_position.y);
        assert!(false_position.y < merge_position.y);
        let junction = nte_blueprint_merge_junction(&blueprint, &flow, merge).unwrap();
        assert_eq!(
            junction,
            merge_position + egui::vec2(NTE_BLUEPRINT_NODE_WIDTH * 0.5, -18.0)
        );
        let wires = nte_blueprint_routed_wires(&blueprint, &flow);
        assert!(
            wires
                .iter()
                .any(|wire| wire.edge.kind == NteBlueprintFlowEdgeKind::True)
        );
        assert!(
            wires
                .iter()
                .any(|wire| wire.edge.kind == NteBlueprintFlowEdgeKind::False)
        );
    }

    #[test]
    fn nte_cpp_control_flow_marks_loop_back_edges() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"loop\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "NTE_REQUIRES(\"log\");\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    for (std::uint64_t index = 0; index < 2; ++index)\n",
            "    {\n",
            "        nte::log::info(\"body\");\n",
            "    }\n",
            "    nte::log::info(\"after\");\n",
            "}\n",
        ))
        .unwrap();
        let flow = nte_blueprint_control_flow(&blueprint);
        let loop_block = flow
            .blocks
            .iter()
            .position(|block| {
                blueprint.statements[block.start]
                    .source
                    .starts_with("for (")
            })
            .unwrap();
        let body = flow
            .blocks
            .iter()
            .position(|block| {
                blueprint.statements[block.start]
                    .source
                    .contains("\"body\"")
            })
            .unwrap();
        let after = flow
            .blocks
            .iter()
            .position(|block| {
                blueprint.statements[block.start]
                    .source
                    .contains("\"after\"")
            })
            .unwrap();

        assert!(flow.edges.contains(&NteBlueprintFlowEdge {
            from: loop_block,
            to: body,
            kind: NteBlueprintFlowEdgeKind::True,
        }));
        assert!(flow.edges.contains(&NteBlueprintFlowEdge {
            from: loop_block,
            to: after,
            kind: NteBlueprintFlowEdgeKind::False,
        }));
        assert!(flow.edges.contains(&NteBlueprintFlowEdge {
            from: body,
            to: loop_block,
            kind: NteBlueprintFlowEdgeKind::Loop,
        }));
    }

    #[test]
    fn nte_blueprint_wires_keep_short_edges_local_and_long_edges_outside_nodes() {
        let aligned_start = egui::pos2(300.0, 180.0);
        let aligned_end = egui::pos2(300.0, 260.0);
        assert_eq!(
            nte_blueprint_wire_route(
                aligned_start,
                aligned_end,
                NteBlueprintFlowEdgeKind::Next,
                1,
            ),
            NteBlueprintWireRoute::Direct
        );
        assert_eq!(
            nte_blueprint_wire_points(
                aligned_start,
                aligned_end,
                NteBlueprintFlowEdgeKind::Next,
                NteBlueprintWireRoute::Direct,
                0,
            ),
            vec![aligned_start, aligned_end]
        );

        let branch_end = egui::pos2(710.0, 300.0);
        assert_eq!(
            nte_blueprint_wire_route(aligned_start, branch_end, NteBlueprintFlowEdgeKind::True, 1,),
            NteBlueprintWireRoute::Channel
        );

        let long_route = nte_blueprint_wire_route(
            aligned_start,
            branch_end,
            NteBlueprintFlowEdgeKind::False,
            3,
        );
        assert_eq!(long_route, NteBlueprintWireRoute::OuterRight);
        let long_points = nte_blueprint_wire_points(
            aligned_start,
            branch_end,
            NteBlueprintFlowEdgeKind::False,
            long_route,
            2,
        );
        let start_right = aligned_start.x + NTE_BLUEPRINT_NODE_WIDTH * (1.0 - 0.68);
        let end_right = branch_end.x + NTE_BLUEPRINT_NODE_WIDTH / 2.0;
        assert!(long_points[2].x > start_right.max(end_right));

        let loop_route =
            nte_blueprint_wire_route(branch_end, aligned_start, NteBlueprintFlowEdgeKind::Loop, 0);
        assert_eq!(loop_route, NteBlueprintWireRoute::OuterLeft);
        let loop_points = nte_blueprint_wire_points(
            branch_end,
            aligned_start,
            NteBlueprintFlowEdgeKind::Loop,
            loop_route,
            0,
        );
        let start_left = branch_end.x - NTE_BLUEPRINT_NODE_WIDTH / 2.0;
        let end_left = aligned_start.x - NTE_BLUEPRINT_NODE_WIDTH / 2.0;
        assert!(loop_points[2].x < start_left.min(end_left));

        let mut lanes = Vec::new();
        assert_eq!(
            reserve_nte_blueprint_wire_lane(&mut lanes, (100.0, 180.0)),
            0
        );
        assert_eq!(
            reserve_nte_blueprint_wire_lane(&mut lanes, (200.0, 280.0)),
            0
        );
        assert_eq!(
            reserve_nte_blueprint_wire_lane(&mut lanes, (160.0, 220.0)),
            1
        );
    }

    #[test]
    fn nte_cpp_basic_block_metadata_survives_source_sync() {
        let source = concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"sync\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    auto value = 1;\n",
            "    value = value + 1;\n",
            "}\n",
        );
        let mut blueprint = parse_nte_blueprint_source(source).unwrap();
        assert_eq!(nte_blueprint_blocks(&blueprint).len(), 1);
        blueprint.statements[0].position = egui::pos2(420.0, 260.0);
        blueprint.statements[0].description = "Update the local value.".to_owned();

        sync_nte_blueprint_from_source(
            &mut blueprint,
            &source.replace("auto value = 1;", "auto value = 2;"),
        );

        assert_eq!(nte_blueprint_blocks(&blueprint).len(), 1);
        assert_eq!(blueprint.statements[0].position, egui::pos2(420.0, 260.0));
        assert_eq!(
            blueprint.statements[0].description,
            "Update the local value."
        );
    }

    #[test]
    fn nte_cpp_action_added_to_a_condition_becomes_a_child() {
        let source = concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"insert\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    if (event.viewport != nullptr)\n",
            "    {\n",
            "\n",
            "        nte::log::info(\"child\");\n",
            "    }\n",
            "    nte::log::info(\"after\");\n",
            "}\n",
        );
        let mut blueprint = parse_nte_blueprint_source(source).unwrap();
        let blocks = nte_blueprint_blocks(&blueprint);
        let selected = blocks[0];
        let condition_id = blueprint.statements[selected.start].id;
        let condition_position = blueprint.statements[selected.start].position;
        let child_id = blueprint.statements[blocks[1].start].id;
        let child_position = blueprint.statements[blocks[1].start].position;
        blueprint.statements[blocks[1].start].description = "Keep this child node.".to_owned();
        blueprint.statements[selected.start].description = "Keep this node.".to_owned();

        add_nte_blueprint_action_to_block(
            &mut blueprint,
            selected,
            NteBlueprintStatementTemplate::Assignment,
        );

        let updated_blocks = nte_blueprint_blocks(&blueprint);
        assert_eq!(updated_blocks.len(), blocks.len());
        assert_eq!(
            blueprint.statements[updated_blocks[0].start].id,
            condition_id
        );
        assert_eq!(
            blueprint.statements[updated_blocks[0].start].position,
            condition_position
        );
        assert_eq!(
            blueprint.statements[updated_blocks[0].start].description,
            "Keep this node."
        );
        assert_eq!(
            blueprint.statements[updated_blocks[1].start].source,
            "auto value = 0"
        );
        assert_eq!(
            blueprint.statements[updated_blocks[1].start].indent,
            blueprint.statements[updated_blocks[0].start].indent + 1
        );
        assert_eq!(
            blueprint.statements[updated_blocks[1].start].position,
            child_position
        );
        assert_eq!(
            blueprint.statements[updated_blocks[1].start].description,
            "Keep this child node."
        );
        assert_eq!(
            blueprint
                .statements
                .iter()
                .find(|statement| statement.id == child_id)
                .unwrap()
                .position,
            child_position
        );
        assert_eq!(
            blueprint
                .statements
                .iter()
                .find(|statement| statement.id == child_id)
                .unwrap()
                .description,
            ""
        );
    }

    #[test]
    fn nte_cpp_nesting_moves_the_whole_subtree_without_merging_nodes() {
        let source = concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"nest\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    if (event.viewport != nullptr)\n",
            "    {\n",
            "        nte::log::info(\"child\");\n",
            "    }\n",
            "    nte::log::info(\"after\");\n",
            "}\n",
        );
        let mut blueprint = parse_nte_blueprint_source(source).unwrap();
        let blocks = nte_blueprint_blocks(&blueprint);
        let after = *blocks.last().unwrap();
        let after_id = blueprint.statements[after.start].id;
        assert!(nte_blueprint_can_indent_block(&blueprint, after));

        adjust_nte_blueprint_block_indent(&mut blueprint, after, true);

        let nested_blocks = nte_blueprint_blocks(&blueprint);
        assert_eq!(nested_blocks.len(), blocks.len());
        let nested = nested_blocks
            .iter()
            .copied()
            .find(|block| blueprint.statements[block.start].id == after_id)
            .unwrap();
        assert_eq!(blueprint.statements[nested.start].indent, 1);
        assert!(nte_blueprint_can_outdent_block(&blueprint, nested));

        adjust_nte_blueprint_block_indent(&mut blueprint, nested, false);

        assert_eq!(nte_blueprint_blocks(&blueprint).len(), blocks.len());
        assert_eq!(
            blueprint
                .statements
                .iter()
                .find(|statement| statement.id == after_id)
                .unwrap()
                .indent,
            0
        );
    }

    #[test]
    fn nte_cpp_state_types_and_structured_values_round_trip() {
        let source = concat!(
            "#include <nte/mod.hpp>\n",
            "\n",
            "NTE_SCRIPT(5);\n",
            "NTE_MOD(\"types\");\n",
            "NTE_REQUIRES(\"viewport.tick\");\n",
            "bool enabled = false;\n",
            "std::int32_t count = 1;\n",
            "\n",
            "void on_viewport_tick(const nte::viewport_tick_event& event)\n",
            "{\n",
            "    auto ready = enabled;\n",
            "}\n",
        );
        let blueprint = parse_nte_blueprint_source(source).unwrap();
        let rendered = render_nte_blueprint_source("types", &blueprint);
        let values = nte_blueprint_value_options(&blueprint);

        assert!(rendered.contains("bool enabled = false;"));
        assert!(rendered.contains("std::int32_t count = 1;"));
        assert!(values.iter().any(|value| value == "enabled"));
        assert!(values.iter().any(|value| value == "ready"));
        assert_eq!(
            split_nte_blueprint_comparison("状态 == true"),
            Some(("状态", "==", "true"))
        );
    }

    #[test]
    fn nte_blueprint_metadata_restores_node_positions_and_descriptions() {
        let source = concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    value = 1\n",
            "\n",
            "    ipc.emit(\"event.value\", value)\n",
        );
        let mut blueprint = parse_nte_blueprint_source(source).unwrap();
        let blocks = nte_blueprint_blocks(&blueprint);
        blueprint.statements[blocks[0].start].position = egui::pos2(126.5, 244.25);
        blueprint.statements[blocks[0].start].description =
            "Read the value used by this Mod.".to_owned();
        blueprint.statements[blocks[1].start].position = egui::pos2(612.0, 318.0);
        blueprint.statements[blocks[1].start].description =
            "Send the value to the desktop tool.".to_owned();
        let metadata = nte_blueprint_metadata(&blueprint);
        let mut restored = parse_nte_blueprint_source(source).unwrap();

        apply_nte_blueprint_metadata(&mut restored, &metadata, false);

        let restored_blocks = nte_blueprint_blocks(&restored);
        assert_eq!(
            restored.statements[restored_blocks[0].start].position,
            egui::pos2(126.5, 244.25)
        );
        assert_eq!(
            restored.statements[restored_blocks[0].start].description,
            "Read the value used by this Mod."
        );
        assert_eq!(
            restored.statements[restored_blocks[1].start].position,
            egui::pos2(612.0, 318.0)
        );
        assert_eq!(
            nte_blueprint_block_summary(&restored, restored_blocks[1]),
            "Send the value to the desktop tool."
        );
    }

    #[test]
    fn nte_blueprint_source_edit_keeps_metadata_on_the_same_step() {
        let mut blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    value = 1\n",
        ))
        .unwrap();
        blueprint.statements[0].position = egui::pos2(320.0, 180.0);
        blueprint.statements[0].description = "Prepare the outgoing value.".to_owned();

        sync_nte_blueprint_from_source(
            &mut blueprint,
            concat!(
                "nte_mod(4)\n",
                "mod(\"example\")\n",
                "requires(\"viewport.tick\")\n",
                "\n",
                "def on_viewport_tick(event):\n",
                "    value = 2\n",
            ),
        );

        assert_eq!(blueprint.statements[0].position, egui::pos2(320.0, 180.0));
        assert_eq!(
            blueprint.statements[0].description,
            "Prepare the outgoing value."
        );
    }

    #[test]
    fn nte_blueprint_bundled_mod_opens_at_the_entry_of_the_complete_graph() {
        let blueprint =
            parse_nte_blueprint_source(include_str!("../../plugins/nte-mods/equipment.nte"))
                .unwrap();
        let bounds = nte_blueprint_graph_bounds(&blueprint);

        let blocks = nte_blueprint_blocks(&blueprint);
        assert_eq!(blocks.len(), 10);
        assert!(bounds.width() <= NTE_BLUEPRINT_OVERVIEW_MAX_WIDTH);
        assert!(bounds.height() > NTE_BLUEPRINT_OVERVIEW_MAX_HEIGHT);
        assert!(blueprint.scene_rect.width() >= NTE_BLUEPRINT_COMFORT_WIDTH);
        assert!(blueprint.scene_rect.height() >= NTE_BLUEPRINT_COMFORT_HEIGHT);
        assert!(blueprint.scene_rect.contains(bounds.min));
        assert!(
            blueprint
                .scene_rect
                .contains(blueprint.statements[blocks[0].start].position)
        );
    }

    #[test]
    fn nte_blueprint_routes_are_manifest_data_and_drive_capabilities() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "requires(\"ipc\")\n",
            "requires(\"equipment\")\n",
            "route_ipc(1, \"equipment.equip_module\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    value = 0\n",
        ))
        .unwrap();

        assert_eq!(
            required_nte_blueprint_capabilities(&blueprint),
            vec!["viewport.tick", "ipc", "equipment"]
        );
        assert_eq!(
            render_nte_blueprint_source("example", &blueprint),
            concat!(
                "nte_mod(4)\n",
                "mod(\"example\")\n",
                "requires(\"viewport.tick\")\n",
                "requires(\"ipc\")\n",
                "requires(\"equipment\")\n",
                "route_ipc(1, \"equipment.equip_module\")\n",
                "\n",
                "def on_viewport_tick(event):\n",
                "    value = 0\n",
            )
        );
        assert!(validate_nte_blueprint(&blueprint).is_ok());
    }

    #[test]
    fn nte_blueprint_rejects_missing_block_children() {
        let blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    if value == True:\n",
            "    value = 1\n",
        ))
        .unwrap();

        assert!(validate_nte_blueprint(&blueprint).is_err());
    }

    #[test]
    fn nte_blueprint_templates_generate_editable_source() {
        let mut blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    value = 0\n",
        ))
        .unwrap();
        add_nte_blueprint_statement(&mut blueprint, NteBlueprintStatementTemplate::IpcEmit);
        add_nte_blueprint_statement(&mut blueprint, NteBlueprintStatementTemplate::Cache);

        let source = render_nte_blueprint_source("example", &blueprint);

        assert_eq!(nte_blueprint_blocks(&blueprint).len(), 3);
        assert!(source.contains("ipc.emit(\"event.name\", value)"));
        assert!(source.contains("cached = cache.remember(1, value)"));
        assert!(source.contains("requires(\"ipc\")"));
        assert!(validate_mod_source("example", &source).is_ok());
    }

    #[test]
    fn nte_blueprint_condition_template_starts_as_a_complete_step() {
        let mut blueprint = parse_nte_blueprint_source(concat!(
            "nte_mod(4)\n",
            "mod(\"example\")\n",
            "requires(\"viewport.tick\")\n",
            "\n",
            "def on_viewport_tick(event):\n",
            "    value = 0\n",
        ))
        .unwrap();

        add_nte_blueprint_statement(&mut blueprint, NteBlueprintStatementTemplate::If);

        let blocks = nte_blueprint_blocks(&blueprint);
        let condition = blocks[1];
        assert_eq!(condition.end - condition.start, 2);
        assert_eq!(
            blueprint.statements[condition.start + 1].indent,
            blueprint.statements[condition.start].indent + 1
        );
        assert!(validate_nte_blueprint(&blueprint).is_ok());
    }

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
        save_mod_script(
            &directory,
            "local-test",
            &source,
            &ModScriptBlueprint::default(),
        )
        .unwrap();
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
                        blueprint: ModScriptBlueprint::default(),
                    }]
        }));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn completion_replaces_only_the_token_before_the_cursor() {
        let mut source = "const auto value = nte::sdk::char".to_owned();
        let cursor = source.chars().count();

        let next = apply_completion(&mut source, cursor, "nte::sdk::character_hp_milli(").unwrap();

        assert_eq!(source, "const auto value = nte::sdk::character_hp_milli(");
        assert_eq!(next, source.chars().count());
    }

    #[test]
    fn completion_filters_host_apis_by_dotted_prefix() {
        let source = "    const auto hp = nte::sdk::character_hp";
        let suggestions = completion_candidates(source, source.chars().count(), false);

        assert!(
            suggestions
                .iter()
                .any(|completion| { completion.insert == "nte::sdk::character_hp_milli(" })
        );
        assert!(suggestions.iter().all(|completion| {
            completion.insert.starts_with("nte::sdk::character_hp")
                || completion.label.starts_with("nte::sdk::character_hp")
        }));
    }

    #[test]
    fn completion_exposes_stable_game_session_builtins() {
        let source = "    const auto controller = nte::game::player_";
        let suggestions = completion_candidates(source, source.chars().count(), false);

        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "nte::game::player_controller")
        );
        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "nte::game::player_state")
        );
        assert!(
            suggestions
                .iter()
                .any(|completion| completion.insert == "nte::game::player_character")
        );
    }

    #[test]
    fn completion_exposes_generic_typed_reads_and_cache() {
        let memory_source = "    const auto hp = nte::memory::read_";
        let memory = completion_candidates(memory_source, memory_source.chars().count(), false);
        assert!(
            memory
                .iter()
                .any(|completion| completion.insert == "nte::memory::read_f32_milli(")
        );
        assert!(
            memory
                .iter()
                .any(|completion| completion.insert == "nte::memory::read_fname_hash(")
        );

        let cache_source = "    const auto value = nte::cache::";
        let cache = completion_candidates(cache_source, cache_source.chars().count(), false);
        assert!(
            cache
                .iter()
                .any(|completion| completion.insert == "nte::cache::get(")
        );
        assert!(
            cache
                .iter()
                .any(|completion| completion.insert == "nte::cache::remember(")
        );
    }

    #[test]
    fn completion_exposes_generic_write_reflection_and_event_apis() {
        for (source, expected) in [
            ("    nte::memory::write_", "nte::memory::write_f32_milli("),
            (
                "    const auto function = nte::unreal::find_",
                "nte::unreal::find_function(",
            ),
            (
                "    nte::unreal::params_write_",
                "nte::unreal::params_write_u64(",
            ),
            ("    nte::unreal::un", "nte::unreal::unwatch("),
            ("    const auto ready = nte::event::", "nte::event::next()"),
            (
                "    const auto value = nte::event::read_",
                "nte::event::read_u32(",
            ),
        ] {
            let suggestions = completion_candidates(source, source.chars().count(), false);
            assert!(
                suggestions
                    .iter()
                    .any(|completion| completion.insert == expected),
                "{expected} should be suggested"
            );
        }
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
            classify_nte_identifier("constexpr", false, false),
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
            classify_nte_identifier("nte", false, false),
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
    fn cpp_preprocessor_directives_are_not_python_comments() {
        let source = "  #include <nte/mod.hpp>\n// comment\nvalue # other";

        assert_eq!(
            nte_cpp_preprocessor_directive_end(source, source.find('#').unwrap()),
            Some("  #include".len())
        );
        assert_eq!(
            nte_cpp_preprocessor_directive_end(source, source.rfind('#').unwrap()),
            None
        );
    }

    #[test]
    fn cpp_control_headers_split_keyword_from_their_expression() {
        assert_eq!(
            nte_blueprint_control_parts("if (value != nullptr)"),
            ("if", "value != nullptr".to_owned())
        );
        assert_eq!(
            nte_blueprint_control_parts("else if (ready)"),
            ("else if", "ready".to_owned())
        );
        assert_eq!(nte_blueprint_control_parts("else"), ("else", String::new()));
    }

    #[test]
    fn declared_capabilities_follow_cpp_macros() {
        assert_eq!(
            declared_capabilities(
                "NTE_SCRIPT(5);\nNTE_REQUIRES(\"viewport.tick\");\nNTE_REQUIRES(\"sdk.read\");\n"
            ),
            vec!["viewport.tick", "sdk.read"]
        );
    }
}
