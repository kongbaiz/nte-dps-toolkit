use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::io_util::{atomic_write_file, atomic_write_text};

pub(crate) const MAX_MOD_SOURCE_BYTES: usize = 16 * 1024;
const MAX_MOD_BLUEPRINT_BYTES: usize = 64 * 1024;
const MAX_ENABLED_MODS: usize = 16;
const MAX_MOD_INSTRUCTIONS: usize = 256;
const MAX_MOD_VARIABLES: usize = 12;
const MAX_MOD_STATES: usize = 16;
const MAX_MOD_STRINGS: usize = 16;
const MAX_MOD_ROUTES: usize = 16;
const MAX_MOD_BLOCKS: usize = 8;
const MAX_MOD_BRANCHES: usize = 8;
const MAX_MOD_STRING_BYTES: usize = 31;
const MAX_MOD_VARIABLE_BYTES: usize = 32;
const MOD_DIRECTORY_NAME: &str = "nte-mods";
const MOD_SET_FILE_NAME: &str = "nte-mods.enabled";
const MOD_BLUEPRINT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModScriptDocument {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) source: String,
    pub(crate) blueprint: ModScriptBlueprint,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ModScriptWorkspace {
    pub(crate) scripts: Vec<ModScriptDocument>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModScriptBlueprint {
    #[serde(default)]
    pub(crate) nodes: Vec<ModScriptBlueprintNode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModScriptBlueprintNode {
    pub(crate) signature: String,
    pub(crate) position: [f32; 2],
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModScriptBlueprintFile {
    version: u32,
    #[serde(default)]
    nodes: Vec<ModScriptBlueprintNode>,
}

pub(crate) fn mod_script_workspace_directory() -> PathBuf {
    super::paths::software_dir().join("plugins")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ModScriptError {
    FileSystem(String),
    InvalidModSet,
    DuplicateModId(String),
    InvalidModId(String),
    TooManyEnabledMods,
    SourceTooLarge,
    SourceContainsNul,
    SourceNotUtf8(String),
    BlueprintTooLarge(String),
    BlueprintNotUtf8(String),
    InvalidBlueprint(String),
    UnsupportedBlueprintVersion { id: String, version: u32 },
    MissingVersionHeader,
    MissingModDeclaration,
    MismatchedModDeclaration,
    MissingViewportTickHandler,
    InvalidSourceLine(usize),
    SourceBudgetExceeded,
    CapabilityMismatch,
    ModSourceMissing(String),
}

pub(crate) fn load_mod_script_workspace(
    workspace_directory: &Path,
) -> Result<ModScriptWorkspace, ModScriptError> {
    let enabled = read_enabled_mods(workspace_directory)?;
    let mod_directory = workspace_directory.join(MOD_DIRECTORY_NAME);
    let entries = match fs::read_dir(&mod_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModScriptWorkspace::default());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };

    let mut scripts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        let is_nte = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("nte"));
        if !file_type.is_file() || !is_nte {
            continue;
        }
        let id = entry
            .path()
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                ModScriptError::InvalidModId(entry.file_name().to_string_lossy().into())
            })?
            .to_owned();
        validate_mod_id(&id)?;
        let bytes = fs::read(entry.path())
            .map_err(|error| ModScriptError::FileSystem(error.to_string()))?;
        if bytes.len() > MAX_MOD_SOURCE_BYTES {
            return Err(ModScriptError::SourceTooLarge);
        }
        let source =
            String::from_utf8(bytes).map_err(|_| ModScriptError::SourceNotUtf8(id.clone()))?;
        let blueprint = load_mod_script_blueprint(&mod_directory, &id)?;
        scripts.push(ModScriptDocument {
            enabled: enabled.contains(&id),
            id,
            source,
            blueprint,
        });
    }
    scripts.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(ModScriptWorkspace { scripts })
}

pub(crate) fn save_mod_script(
    workspace_directory: &Path,
    id: &str,
    source: &str,
    blueprint: &ModScriptBlueprint,
) -> Result<(), ModScriptError> {
    validate_mod_source(id, source)?;
    let file = ModScriptBlueprintFile {
        version: MOD_BLUEPRINT_VERSION,
        nodes: blueprint.nodes.clone(),
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|_| ModScriptError::InvalidBlueprint(id.to_owned()))?;
    if text.len() + 1 > MAX_MOD_BLUEPRINT_BYTES {
        return Err(ModScriptError::BlueprintTooLarge(id.to_owned()));
    }
    let mod_directory = workspace_directory.join(MOD_DIRECTORY_NAME);
    let source_path = mod_directory.join(format!("{id}.nte"));
    let blueprint_path = mod_script_blueprint_path(&mod_directory, id);
    let previous_blueprint =
        read_optional_file(&blueprint_path).map_err(ModScriptError::FileSystem)?;
    atomic_write_text(&blueprint_path, &format!("{text}\n")).map_err(ModScriptError::FileSystem)?;
    if let Err(error) = atomic_write_text(&source_path, source) {
        if let Err(rollback_error) = restore_file(&blueprint_path, previous_blueprint) {
            return Err(ModScriptError::FileSystem(format!(
                "{error}; failed to restore Blueprint metadata: {rollback_error}"
            )));
        }
        return Err(ModScriptError::FileSystem(error));
    }
    Ok(())
}

fn read_optional_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn restore_file(path: &Path, previous: Option<Vec<u8>>) -> Result<(), String> {
    match previous {
        Some(bytes) => atomic_write_file(path, |writer| {
            writer.write_all(&bytes).map_err(|error| error.to_string())
        }),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        },
    }
}

fn load_mod_script_blueprint(
    mod_directory: &Path,
    id: &str,
) -> Result<ModScriptBlueprint, ModScriptError> {
    let bytes = match fs::read(mod_script_blueprint_path(mod_directory, id)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ModScriptBlueprint::default());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };
    if bytes.len() > MAX_MOD_BLUEPRINT_BYTES {
        return Err(ModScriptError::BlueprintTooLarge(id.to_owned()));
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| ModScriptError::BlueprintNotUtf8(id.to_owned()))?;
    let file = serde_json::from_str::<ModScriptBlueprintFile>(text)
        .map_err(|_| ModScriptError::InvalidBlueprint(id.to_owned()))?;
    if file.version != MOD_BLUEPRINT_VERSION {
        return Err(ModScriptError::UnsupportedBlueprintVersion {
            id: id.to_owned(),
            version: file.version,
        });
    }
    if file.nodes.iter().any(|node| {
        node.signature.is_empty()
            || node
                .position
                .iter()
                .any(|coordinate| !coordinate.is_finite())
    }) {
        return Err(ModScriptError::InvalidBlueprint(id.to_owned()));
    }
    Ok(ModScriptBlueprint { nodes: file.nodes })
}

fn mod_script_blueprint_path(mod_directory: &Path, id: &str) -> PathBuf {
    mod_directory.join(format!("{id}.blueprint.json"))
}

pub(crate) fn set_mod_enabled(
    workspace_directory: &Path,
    id: &str,
    enabled: bool,
) -> Result<(), ModScriptError> {
    validate_mod_id(id)?;
    let mut enabled_mods = read_enabled_mods(workspace_directory)?;
    if enabled {
        if !workspace_directory
            .join(MOD_DIRECTORY_NAME)
            .join(format!("{id}.nte"))
            .is_file()
        {
            return Err(ModScriptError::ModSourceMissing(id.to_owned()));
        }
        if !enabled_mods.contains(id) {
            if enabled_mods.len() == MAX_ENABLED_MODS {
                return Err(ModScriptError::TooManyEnabledMods);
            }
            enabled_mods.insert(id.to_owned());
        }
    } else {
        enabled_mods.remove(id);
    }
    write_enabled_mods(workspace_directory, &enabled_mods)
}

pub(crate) fn validate_mod_source(id: &str, source: &str) -> Result<(), ModScriptError> {
    validate_mod_id(id)?;
    if source.len() > MAX_MOD_SOURCE_BYTES {
        return Err(ModScriptError::SourceTooLarge);
    }
    if source.contains('\0') {
        return Err(ModScriptError::SourceContainsNul);
    }
    let mut lines = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    if lines.next() != Some("nte_mod(4)") {
        return Err(ModScriptError::MissingVersionHeader);
    }
    let Some(declaration) = lines.next() else {
        return Err(ModScriptError::MissingModDeclaration);
    };
    let expected_declaration = format!("mod(\"{id}\")");
    if !declaration.starts_with("mod(") {
        return Err(ModScriptError::MissingModDeclaration);
    }
    if declaration != expected_declaration {
        return Err(ModScriptError::MismatchedModDeclaration);
    }
    if !lines.any(|line| line == "def on_viewport_tick(event):") {
        return Err(ModScriptError::MissingViewportTickHandler);
    }
    ModSourceValidator::new(source)
        .validate(id)
        .map_err(|failure| match failure {
            ModSourceValidationFailure::InvalidLine(line) => {
                ModScriptError::InvalidSourceLine(line)
            }
            ModSourceValidationFailure::BudgetExceeded => ModScriptError::SourceBudgetExceeded,
            ModSourceValidationFailure::CapabilityMismatch => ModScriptError::CapabilityMismatch,
        })
}

const CAPABILITY_VIEWPORT_TICK: u8 = 1 << 0;
const CAPABILITY_MEMORY_READ: u8 = 1 << 1;
const CAPABILITY_IPC: u8 = 1 << 2;
const CAPABILITY_SDK_READ: u8 = 1 << 3;
const CAPABILITY_EQUIPMENT: u8 = 1 << 4;
const CAPABILITY_COMBAT_CLOCK: u8 = 1 << 5;
const CAPABILITY_LOG: u8 = 1 << 6;
const CAPABILITY_GAME_SESSION: u8 = 1 << 7;

#[derive(Clone, Copy)]
struct ModSourceLine<'a> {
    number: usize,
    indentation: usize,
    text: &'a str,
}

#[derive(Clone, Copy)]
enum ModSourceValidationFailure {
    InvalidLine(usize),
    BudgetExceeded,
    CapabilityMismatch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModSourceBlockKind {
    Conditional,
    Loop,
}

struct ModSourceBlock {
    kind: ModSourceBlockKind,
    indentation: usize,
    false_jump_open: bool,
    end_jump_count: usize,
}

struct ModSourceValidator<'a> {
    lines: Vec<ModSourceLine<'a>>,
    capabilities: u8,
    used_capabilities: u8,
    instruction_count: usize,
    states: Vec<&'a str>,
    variables: Vec<&'a str>,
    strings: Vec<&'a str>,
    routes: Vec<u16>,
}

impl<'a> ModSourceValidator<'a> {
    fn new(source: &'a str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let lines = source
            .lines()
            .enumerate()
            .filter_map(|(index, raw)| {
                let indentation = raw.bytes().take_while(|byte| *byte == b' ').count();
                let text = raw[indentation..].trim_end_matches([' ', '\t']);
                (!text.is_empty() && !text.starts_with('#')).then_some(ModSourceLine {
                    number: index + 1,
                    indentation,
                    text,
                })
            })
            .collect();
        Self {
            lines,
            capabilities: 0,
            used_capabilities: 0,
            instruction_count: 0,
            states: Vec::new(),
            variables: Vec::new(),
            strings: Vec::new(),
            routes: Vec::new(),
        }
    }

    fn validate(mut self, expected_id: &str) -> Result<(), ModSourceValidationFailure> {
        let Some(version) = self.lines.first().copied() else {
            return Err(ModSourceValidationFailure::InvalidLine(1));
        };
        if version.indentation != 0 || parse_call(version.text, "nte_mod") != Some("4") {
            return Err(ModSourceValidationFailure::InvalidLine(version.number));
        }
        let Some(declaration) = self.lines.get(1).copied() else {
            return Err(ModSourceValidationFailure::InvalidLine(version.number));
        };
        let declared_id = parse_call(declaration.text, "mod")
            .and_then(parse_string_literal)
            .filter(|id| *id == expected_id);
        if declaration.indentation != 0 || declared_id.is_none() {
            return Err(ModSourceValidationFailure::InvalidLine(declaration.number));
        }

        let mut handler_index = None;
        for index in 2..self.lines.len() {
            let line = self.lines[index];
            if line.indentation != 0 {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            }
            if line.text == "def on_viewport_tick(event):" {
                self.used_capabilities |= CAPABILITY_VIEWPORT_TICK;
                handler_index = Some(index);
                break;
            }
            self.validate_declaration(line)?;
        }
        let Some(handler_index) = handler_index else {
            return Err(ModSourceValidationFailure::InvalidLine(declaration.number));
        };
        self.validate_body(handler_index + 1)?;
        if self.capabilities != self.used_capabilities {
            return Err(ModSourceValidationFailure::CapabilityMismatch);
        }
        Ok(())
    }

    fn validate_declaration(
        &mut self,
        line: ModSourceLine<'a>,
    ) -> Result<(), ModSourceValidationFailure> {
        if let Some(arguments) =
            parse_call(line.text, "requires").or_else(|| parse_call(line.text, "capability"))
        {
            let Some(capability) = parse_string_literal(arguments).and_then(mod_capability) else {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            };
            if self.capabilities & capability != 0 {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            }
            self.capabilities |= capability;
            return Ok(());
        }
        if let Some(arguments) = parse_call(line.text, "route_ipc") {
            if self.routes.len() == MAX_MOD_ROUTES {
                return Err(ModSourceValidationFailure::BudgetExceeded);
            }
            let Some((operation, service)) = split_two_arguments(arguments) else {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            };
            let Some(operation) = parse_mod_integer(operation)
                .filter(|operation| *operation <= u16::MAX as u64)
                .map(|operation| operation as u16)
            else {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            };
            let Some(service) = parse_string_literal(service) else {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            };
            let Some((expected_operation, capability)) = mod_ipc_service(service) else {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            };
            if operation != expected_operation || self.routes.contains(&operation) {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            }
            self.routes.push(operation);
            self.used_capabilities |= capability;
            return Ok(());
        }
        if let Some((target, expression)) = parse_mod_assignment(line.text)
            && let Some(name) = parse_state_name(target)
        {
            if self.states.len() == MAX_MOD_STATES {
                return Err(ModSourceValidationFailure::BudgetExceeded);
            }
            if !is_mod_variable_name(name)
                || self.states.contains(&name)
                || parse_mod_integer(expression).is_none()
            {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            }
            self.states.push(name);
            return Ok(());
        }
        Err(ModSourceValidationFailure::InvalidLine(line.number))
    }

    fn validate_body(&mut self, first_line: usize) -> Result<(), ModSourceValidationFailure> {
        let mut blocks = Vec::<ModSourceBlock>::new();
        let mut has_body = false;
        for index in first_line..self.lines.len() {
            let line = self.lines[index];
            let condition = parse_mod_condition(line.text, "if ");
            let elif_condition = parse_mod_condition(line.text, "elif ");
            let is_else = line.text == "else:";
            while blocks.last().is_some_and(|block| {
                line.indentation <= block.indentation
                    && !(line.indentation == block.indentation
                        && (is_else || elif_condition.is_some())
                        && block.kind == ModSourceBlockKind::Conditional)
            }) {
                self.close_block(
                    blocks
                        .pop()
                        .expect("the loop condition established a block"),
                )?;
            }
            let expected_indentation = blocks.last().map_or(4, |block| block.indentation + 4);
            if is_else || elif_condition.is_some() {
                let Some(block) = blocks.last_mut() else {
                    return Err(ModSourceValidationFailure::InvalidLine(line.number));
                };
                if line.indentation != block.indentation
                    || block.kind != ModSourceBlockKind::Conditional
                    || !block.false_jump_open
                {
                    return Err(ModSourceValidationFailure::InvalidLine(line.number));
                }
                if block.end_jump_count == MAX_MOD_BRANCHES {
                    return Err(ModSourceValidationFailure::BudgetExceeded);
                }
                block.end_jump_count += 1;
                block.false_jump_open = false;
                self.append_instructions(1)?;
                if let Some(expression) = elif_condition {
                    self.compile_expression(expression, line.number)?;
                    self.append_instructions(1)?;
                    blocks
                        .last_mut()
                        .expect("the branch block remains active")
                        .false_jump_open = true;
                }
                has_body = true;
                continue;
            }
            if line.indentation != expected_indentation {
                return Err(ModSourceValidationFailure::InvalidLine(line.number));
            }
            if let Some(expression) = condition {
                if blocks.len() == MAX_MOD_BLOCKS {
                    return Err(ModSourceValidationFailure::BudgetExceeded);
                }
                self.compile_expression(expression, line.number)?;
                self.append_instructions(1)?;
                blocks.push(ModSourceBlock {
                    kind: ModSourceBlockKind::Conditional,
                    indentation: line.indentation,
                    false_jump_open: true,
                    end_jump_count: 0,
                });
                has_body = true;
                continue;
            }
            if let Some((variable, _count)) = parse_mod_for_range(line.text) {
                if blocks.len() == MAX_MOD_BLOCKS {
                    return Err(ModSourceValidationFailure::BudgetExceeded);
                }
                self.assign_variable(variable, line.number)?;
                self.append_instructions(2)?;
                blocks.push(ModSourceBlock {
                    kind: ModSourceBlockKind::Loop,
                    indentation: line.indentation,
                    false_jump_open: false,
                    end_jump_count: 0,
                });
                has_body = true;
                continue;
            }
            self.compile_statement(line.text, line.number)?;
            has_body = true;
        }
        while let Some(block) = blocks.pop() {
            self.close_block(block)?;
        }
        if !has_body {
            return Err(ModSourceValidationFailure::InvalidLine(
                self.lines
                    .get(first_line.saturating_sub(1))
                    .map_or(1, |line| line.number),
            ));
        }
        Ok(())
    }

    fn close_block(&mut self, block: ModSourceBlock) -> Result<(), ModSourceValidationFailure> {
        if block.kind == ModSourceBlockKind::Loop {
            self.append_instructions(1)?;
        }
        Ok(())
    }

    fn compile_statement(
        &mut self,
        line: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        if let Some((target, expression)) = parse_mod_assignment(line) {
            if let Some(state) = parse_state_name(target) {
                if !self.states.contains(&state) {
                    return Err(ModSourceValidationFailure::InvalidLine(line_number));
                }
                self.compile_expression(expression, line_number)?;
                return self.append_instructions(1);
            }
            self.assign_variable(target, line_number)?;
            return self.compile_expression(expression, line_number);
        }
        if let Some(arguments) = parse_call(line, "equipment.prepare") {
            let arguments = split_mod_arguments(arguments, 4)
                .filter(|arguments| arguments.len() == 1)
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            self.materialize_atom(arguments[0], line_number)?;
            self.used_capabilities |= CAPABILITY_EQUIPMENT;
            return self.append_instructions(1);
        }
        if let Some(arguments) = parse_call(line, "combat_clock.forward") {
            let arguments = split_mod_arguments(arguments, 4)
                .filter(|arguments| arguments.len() == 2)
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            self.materialize_atom(arguments[0], line_number)?;
            self.materialize_atom(arguments[1], line_number)?;
            self.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
            return self.append_instructions(1);
        }
        if let Some(arguments) = parse_call(line, "ipc.bind") {
            let arguments = split_mod_arguments(arguments, 4)
                .filter(|arguments| arguments.len() == 2)
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            if arguments[0] == "None" && arguments[1] == "None" {
                return Err(ModSourceValidationFailure::InvalidLine(line_number));
            }
            for argument in arguments {
                if argument != "None" {
                    self.materialize_atom(argument, line_number)?;
                }
            }
            self.used_capabilities |= CAPABILITY_IPC;
            return self.append_instructions(1);
        }
        if let Some(arguments) = parse_call(line, "ipc.emit") {
            let arguments = split_mod_arguments(arguments, 4)
                .filter(|arguments| !arguments.is_empty())
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            let event_name = parse_string_literal(arguments[0])
                .filter(|event_name| is_mod_event_name(event_name))
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            self.add_string(event_name, line_number)?;
            for argument in &arguments[1..] {
                self.materialize_atom(argument, line_number)?;
            }
            self.used_capabilities |= CAPABILITY_IPC;
            return self.append_instructions(1);
        }
        if let Some(arguments) = parse_call(line, "log.info") {
            let arguments = split_mod_arguments(arguments, 4)
                .filter(|arguments| arguments.len() == 1)
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            let message = parse_string_literal(arguments[0])
                .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
            self.add_string(message, line_number)?;
            self.used_capabilities |= CAPABILITY_LOG;
            return self.append_instructions(1);
        }
        Err(ModSourceValidationFailure::InvalidLine(line_number))
    }

    fn compile_expression(
        &mut self,
        expression: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        let expression = expression.trim_matches([' ', '\t']);
        if let Some(operand) = expression.strip_prefix("not ") {
            self.compile_expression(operand.trim_matches([' ', '\t']), line_number)?;
            return self.append_instructions(1);
        }
        if let Some(operand) = expression.strip_prefix('-')
            && !operand.is_empty()
        {
            self.compile_atom(operand.trim_matches([' ', '\t']), line_number)?;
            return self.append_instructions(1);
        }
        if let Some((left, right)) = split_mod_binary_expression(expression) {
            self.materialize_atom(left, line_number)?;
            self.materialize_atom(right, line_number)?;
            return self.append_instructions(1);
        }
        if self.compile_call_expression(expression, line_number)? {
            return Ok(());
        }
        self.compile_atom(expression, line_number)
    }

    fn compile_call_expression(
        &mut self,
        expression: &'a str,
        line_number: usize,
    ) -> Result<bool, ModSourceValidationFailure> {
        if let Some(arguments) = parse_call(expression, "time.now_ms") {
            if !arguments.is_empty() {
                return Err(ModSourceValidationFailure::InvalidLine(line_number));
            }
            self.append_instructions(1)?;
            return Ok(true);
        }
        if let Some(arguments) = parse_call(expression, "equipment.cache_missing") {
            if !arguments.is_empty() {
                return Err(ModSourceValidationFailure::InvalidLine(line_number));
            }
            self.used_capabilities |= CAPABILITY_EQUIPMENT;
            self.append_instructions(1)?;
            return Ok(true);
        }
        if let Some(arguments) = parse_call(expression, "equipment.cache_ready") {
            self.compile_single_argument_call(arguments, line_number)?;
            self.used_capabilities |= CAPABILITY_EQUIPMENT;
            return Ok(true);
        }
        if let Some(arguments) = parse_call(expression, "combat_clock.sample") {
            self.compile_single_argument_call(arguments, line_number)?;
            self.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
            return Ok(true);
        }
        for name in ["combat_clock.pause_mask", "combat_clock.state_flags"] {
            if let Some(arguments) = parse_call(expression, name) {
                self.compile_single_argument_call(arguments, line_number)?;
                self.used_capabilities |= CAPABILITY_COMBAT_CLOCK;
                return Ok(true);
            }
        }
        for name in [
            "memory.read_ptr",
            "memory.read_u8",
            "memory.read_u16",
            "memory.read_u32",
            "memory.read_u64",
            "memory.read_i32",
            "memory.tarray_first",
            "memory.tarray_count",
            "memory.is_readable",
        ] {
            if let Some(arguments) = parse_call(expression, name) {
                let arguments = split_mod_arguments(arguments, 3)
                    .filter(|arguments| arguments.len() == 2)
                    .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
                self.materialize_atom(arguments[0], line_number)?;
                self.materialize_atom(arguments[1], line_number)?;
                self.used_capabilities |= CAPABILITY_MEMORY_READ;
                self.append_instructions(1)?;
                return Ok(true);
            }
        }
        for (name, expected_arguments) in [
            ("sdk.player_character", 1),
            ("sdk.player_state", 1),
            ("sdk.game_paused", 1),
            ("sdk.attack_target", 1),
            ("sdk.current_weapon", 1),
            ("sdk.character_level", 1),
            ("sdk.character_hp_milli", 1),
            ("sdk.character_hp_max_milli", 2),
            ("sdk.character_is_alive", 1),
            ("sdk.character_is_dead", 1),
            ("sdk.character_is_controlled", 1),
            ("sdk.character_slomo_milli", 1),
        ] {
            if let Some(arguments) = parse_call(expression, name) {
                let arguments = split_mod_arguments(arguments, 3)
                    .filter(|arguments| arguments.len() == expected_arguments)
                    .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
                self.materialize_atom(arguments[0], line_number)?;
                if expected_arguments == 2 {
                    self.materialize_atom(arguments[1], line_number)?;
                } else {
                    self.append_instructions(1)?;
                }
                self.used_capabilities |= CAPABILITY_SDK_READ;
                self.append_instructions(1)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn compile_single_argument_call(
        &mut self,
        arguments: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        let arguments = split_mod_arguments(arguments, 3)
            .filter(|arguments| arguments.len() == 1)
            .ok_or(ModSourceValidationFailure::InvalidLine(line_number))?;
        self.materialize_atom(arguments[0], line_number)?;
        self.append_instructions(1)
    }

    fn materialize_atom(
        &mut self,
        expression: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        let expression = expression.trim_matches([' ', '\t']);
        if self.variables.contains(&expression) {
            return Ok(());
        }
        self.compile_atom(expression, line_number)
    }

    fn compile_atom(
        &mut self,
        expression: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        if parse_mod_integer(expression).is_some() {
            return self.append_instructions(1);
        }
        if expression == "event.viewport" {
            self.used_capabilities |= CAPABILITY_VIEWPORT_TICK;
            return self.append_instructions(1);
        }
        if matches!(
            expression,
            "game.viewport"
                | "game.instance"
                | "game.local_player"
                | "game.player_controller"
                | "game.player_state"
                | "game.player_character"
        ) {
            self.used_capabilities |= CAPABILITY_GAME_SESSION;
            return self.append_instructions(1);
        }
        if let Some(state) = parse_state_name(expression)
            && self.states.contains(&state)
        {
            return self.append_instructions(1);
        }
        if self.variables.contains(&expression) {
            return self.append_instructions(1);
        }
        Err(ModSourceValidationFailure::InvalidLine(line_number))
    }

    fn assign_variable(
        &mut self,
        name: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        if !is_mod_variable_name(name) {
            return Err(ModSourceValidationFailure::InvalidLine(line_number));
        }
        if self.variables.contains(&name) {
            return Ok(());
        }
        if self.variables.len() == MAX_MOD_VARIABLES {
            return Err(ModSourceValidationFailure::BudgetExceeded);
        }
        self.variables.push(name);
        Ok(())
    }

    fn add_string(
        &mut self,
        value: &'a str,
        line_number: usize,
    ) -> Result<(), ModSourceValidationFailure> {
        if value.is_empty() || value.len() > MAX_MOD_STRING_BYTES {
            return Err(ModSourceValidationFailure::InvalidLine(line_number));
        }
        if self.strings.contains(&value) {
            return Ok(());
        }
        if self.strings.len() == MAX_MOD_STRINGS {
            return Err(ModSourceValidationFailure::BudgetExceeded);
        }
        self.strings.push(value);
        Ok(())
    }

    fn append_instructions(&mut self, count: usize) -> Result<(), ModSourceValidationFailure> {
        if self.instruction_count + count > MAX_MOD_INSTRUCTIONS {
            return Err(ModSourceValidationFailure::BudgetExceeded);
        }
        self.instruction_count += count;
        Ok(())
    }
}

fn parse_call<'a>(expression: &'a str, function_name: &str) -> Option<&'a str> {
    expression
        .strip_prefix(function_name)
        .and_then(|expression| expression.strip_prefix('('))
        .and_then(|expression| expression.strip_suffix(')'))
        .map(|arguments| arguments.trim_matches([' ', '\t']))
}

fn parse_string_literal(text: &str) -> Option<&str> {
    let value = text.strip_prefix('"')?.strip_suffix('"')?;
    (!value.bytes().any(|byte| matches!(byte, b'"' | b'\\'))).then_some(value)
}

fn split_two_arguments(arguments: &str) -> Option<(&str, &str)> {
    let mut parts = arguments.split(',');
    let first = parts.next()?.trim_matches([' ', '\t']);
    let second = parts.next()?.trim_matches([' ', '\t']);
    (!first.is_empty() && !second.is_empty() && parts.next().is_none()).then_some((first, second))
}

fn parse_mod_assignment(line: &str) -> Option<(&str, &str)> {
    let bytes = line.as_bytes();
    let mut separator = None;
    for index in 0..bytes.len() {
        if bytes[index] != b'=' {
            continue;
        }
        let comparison = index != 0 && matches!(bytes[index - 1], b'=' | b'!' | b'<' | b'>')
            || index + 1 < bytes.len() && bytes[index + 1] == b'=';
        if comparison {
            continue;
        }
        if separator.replace(index).is_some() {
            return None;
        }
    }
    let separator = separator?;
    let target = line[..separator].trim_matches([' ', '\t']);
    let expression = line[separator + 1..].trim_matches([' ', '\t']);
    (!target.is_empty() && !expression.is_empty()).then_some((target, expression))
}

fn parse_mod_integer(text: &str) -> Option<u64> {
    match text {
        "None" | "False" => Some(0),
        "True" => Some(1),
        _ => {
            let (digits, base) = text
                .strip_prefix("0x")
                .or_else(|| text.strip_prefix("0X"))
                .map_or((text, 10), |digits| (digits, 16));
            if digits.is_empty() {
                return None;
            }
            digits.bytes().try_fold(0u64, |value, digit| {
                let digit = match digit {
                    b'0'..=b'9' => u64::from(digit - b'0'),
                    b'a'..=b'f' if base == 16 => u64::from(digit - b'a' + 10),
                    b'A'..=b'F' if base == 16 => u64::from(digit - b'A' + 10),
                    _ => return None,
                };
                (digit < base).then_some(())?;
                value.checked_mul(base)?.checked_add(digit)
            })
        }
    }
}

fn parse_state_name(text: &str) -> Option<&str> {
    text.strip_prefix("state.").filter(|name| !name.is_empty())
}

fn is_mod_variable_name(name: &str) -> bool {
    name.len() <= MAX_MOD_VARIABLE_BYTES
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte == b'_')
        && name
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && !matches!(name, "event" | "state" | "None" | "True" | "False")
}

fn is_mod_event_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MOD_STRING_BYTES
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

fn mod_capability(name: &str) -> Option<u8> {
    Some(match name {
        "viewport.tick" => CAPABILITY_VIEWPORT_TICK,
        "memory.read" => CAPABILITY_MEMORY_READ,
        "ipc" => CAPABILITY_IPC,
        "sdk.read" => CAPABILITY_SDK_READ,
        "equipment" => CAPABILITY_EQUIPMENT,
        "combat-clock" => CAPABILITY_COMBAT_CLOCK,
        "log" => CAPABILITY_LOG,
        "game.session" => CAPABILITY_GAME_SESSION,
        _ => return None,
    })
}

fn mod_ipc_service(name: &str) -> Option<(u16, u8)> {
    Some(match name {
        "equipment.equip_module" => (1, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.equip_core" => (2, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.unequip_module" => (3, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.unequip_core" => (4, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.unequip_all" => (5, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.equip_one_key" => (6, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.move_module_to_character" => (7, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.move_core_to_character" => (8, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.set_item_discarded" => (9, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "equipment.set_item_locked" => (10, CAPABILITY_IPC | CAPABILITY_EQUIPMENT),
        "combat_clock.query_transitions" => (11, CAPABILITY_IPC | CAPABILITY_COMBAT_CLOCK),
        "ipc.query_mod_events" => (12, CAPABILITY_IPC),
        _ => return None,
    })
}

fn split_mod_arguments(arguments: &str, capacity: usize) -> Option<Vec<&str>> {
    if arguments.is_empty() {
        return Some(Vec::new());
    }
    let bytes = arguments.as_bytes();
    let mut output = Vec::new();
    let mut in_string = false;
    let mut depth = 0usize;
    let mut first = 0usize;
    for index in 0..=bytes.len() {
        let value = bytes.get(index).copied().unwrap_or(b',');
        match value {
            b'"' => in_string = !in_string,
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth = depth.checked_sub(1)?,
            b',' if !in_string && depth == 0 => {
                if output.len() == capacity {
                    return None;
                }
                let argument = arguments[first..index].trim_matches([' ', '\t']);
                if argument.is_empty() {
                    return None;
                }
                output.push(argument);
                first = index + 1;
            }
            _ => {}
        }
    }
    (!in_string && depth == 0).then_some(output)
}

fn split_mod_binary_expression(expression: &str) -> Option<(&str, &str)> {
    const OPERATORS: [&str; 18] = [
        " or ", " and ", "==", "!=", "<=", ">=", "<<", ">>", "<", ">", "+", "-", "*", "/", "%",
        "&", "|", "^",
    ];
    let bytes = expression.as_bytes();
    let mut found = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => in_string = !in_string,
            b'(' if !in_string => depth += 1,
            b')' if !in_string => depth = depth.checked_sub(1)?,
            _ => {}
        }
        if !in_string
            && depth == 0
            && let Some(operator) = OPERATORS
                .iter()
                .find(|operator| bytes[index..].starts_with(operator.as_bytes()))
        {
            if found.replace((index, operator.len())).is_some() {
                return None;
            }
            index += operator.len();
            continue;
        }
        index += 1;
    }
    if in_string || depth != 0 {
        return None;
    }
    let (index, length) = found?;
    let left = expression[..index].trim_matches([' ', '\t']);
    let right = expression[index + length..].trim_matches([' ', '\t']);
    (!left.is_empty() && !right.is_empty()).then_some((left, right))
}

fn parse_mod_condition<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let expression = line.strip_prefix(prefix)?.strip_suffix(':')?;
    let expression = expression.trim_matches([' ', '\t']);
    (!expression.is_empty()).then_some(expression)
}

fn parse_mod_for_range(line: &str) -> Option<(&str, u8)> {
    let header = line.strip_prefix("for ")?.strip_suffix(':')?;
    let (variable, count) = header.split_once(" in range(")?;
    let count = count.strip_suffix(')')?.trim_matches([' ', '\t']);
    let variable = variable.trim_matches([' ', '\t']);
    let count = parse_mod_integer(count)?;
    (is_mod_variable_name(variable) && count <= 64).then_some((variable, count as u8))
}

pub(crate) fn new_mod_script_template(id: &str) -> Result<String, ModScriptError> {
    validate_mod_id(id)?;
    Ok(format!(
        "# Declare only the host capabilities this Mod actually uses.\n\
         nte_mod(4)\n\
         mod(\"{id}\")\n\
         requires(\"viewport.tick\")\n\
         requires(\"game.session\")\n\
         requires(\"ipc\")\n\
         route_ipc(12, \"ipc.query_mod_events\")\n\
         state.last_character = 0\n\
         \n\
         # This handler is the Mod's control flow and runs on the shared tick hook.\n\
         def on_viewport_tick(event):\n\
         \x20\x20\x20\x20character = game.player_character\n\
         \x20\x20\x20\x20if character != state.last_character:\n\
         \x20\x20\x20\x20\x20\x20\x20\x20ipc.emit(\"pre.session.changed\", state.last_character, character)\n\
         \x20\x20\x20\x20\x20\x20\x20\x20state.last_character = character\n\
         \x20\x20\x20\x20\x20\x20\x20\x20ipc.emit(\"post.session.changed\", character)\n"
    ))
}

pub(crate) fn validate_enabled_mod_set(source: &str) -> Result<(), ModScriptError> {
    parse_enabled_mods(source).map(|_| ())
}

fn validate_mod_id(id: &str) -> Result<(), ModScriptError> {
    if id.is_empty()
        || id.len() > 31
        || id.bytes().any(|value| {
            !value.is_ascii_lowercase()
                && !value.is_ascii_digit()
                && !matches!(value, b'-' | b'_' | b'.')
        })
    {
        return Err(ModScriptError::InvalidModId(id.to_owned()));
    }
    Ok(())
}

fn read_enabled_mods(workspace_directory: &Path) -> Result<HashSet<String>, ModScriptError> {
    let path = workspace_directory.join(MOD_SET_FILE_NAME);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(error) => return Err(ModScriptError::FileSystem(error.to_string())),
    };
    parse_enabled_mods(&text)
}

fn parse_enabled_mods(text: &str) -> Result<HashSet<String>, ModScriptError> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'));
    if lines.next() != Some("nte_mod_set 1") {
        return Err(ModScriptError::InvalidModSet);
    }
    let mut enabled = HashSet::new();
    for line in lines {
        let Some(id) = line.strip_prefix("load ") else {
            return Err(ModScriptError::InvalidModSet);
        };
        validate_mod_id(id)?;
        if !enabled.insert(id.to_owned()) {
            return Err(ModScriptError::DuplicateModId(id.to_owned()));
        }
    }
    if enabled.len() > MAX_ENABLED_MODS {
        return Err(ModScriptError::TooManyEnabledMods);
    }
    Ok(enabled)
}

fn write_enabled_mods(
    workspace_directory: &Path,
    enabled: &HashSet<String>,
) -> Result<(), ModScriptError> {
    let mut ids: Vec<_> = enabled.iter().map(String::as_str).collect();
    ids.sort_unstable();
    let mut text = String::from("nte_mod_set 1\n");
    for id in ids {
        text.push_str("load ");
        text.push_str(id);
        text.push('\n');
    }
    atomic_write_text(&workspace_directory.join(MOD_SET_FILE_NAME), &text)
        .map_err(ModScriptError::FileSystem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nte-mod-script-test-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn workspace_round_trip_preserves_source_and_enabled_state() {
        let root = temp_workspace();
        let source = new_mod_script_template("telemetry").unwrap();
        assert!(source.starts_with("# Declare only the host capabilities"));
        assert!(source.contains("# This handler is the Mod's control flow"));
        assert!(source.contains("game.player_character"));
        assert!(source.contains("ipc.emit(\"pre.session.changed\""));
        assert!(source.contains("ipc.emit(\"post.session.changed\""));
        let blueprint = ModScriptBlueprint {
            nodes: vec![ModScriptBlueprintNode {
                signature: "0:character = game.player_character\n".to_owned(),
                position: [184.5, 92.25],
                description: "Read the active character.".to_owned(),
            }],
        };

        save_mod_script(&root, "telemetry", &source, &blueprint).unwrap();
        set_mod_enabled(&root, "telemetry", true).unwrap();
        let workspace = load_mod_script_workspace(&root).unwrap();

        assert_eq!(
            workspace.scripts,
            vec![ModScriptDocument {
                id: "telemetry".to_owned(),
                enabled: true,
                source,
                blueprint,
            }]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_loads_legacy_script_without_blueprint_metadata() {
        let root = temp_workspace();
        let source = new_mod_script_template("legacy").unwrap();
        atomic_write_text(&root.join("nte-mods").join("legacy.nte"), &source).unwrap();

        let workspace = load_mod_script_workspace(&root).unwrap();

        assert_eq!(
            workspace.scripts[0].blueprint,
            ModScriptBlueprint::default()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_blueprint_is_rejected_before_source_is_written() {
        let root = temp_workspace();
        let source = new_mod_script_template("oversized").unwrap();
        let blueprint = ModScriptBlueprint {
            nodes: vec![ModScriptBlueprintNode {
                signature: "0:value = 1\n".to_owned(),
                position: [0.0, 0.0],
                description: "x".repeat(MAX_MOD_BLUEPRINT_BYTES),
            }],
        };

        assert_eq!(
            save_mod_script(&root, "oversized", &source, &blueprint),
            Err(ModScriptError::BlueprintTooLarge("oversized".to_owned()))
        );
        assert!(!root.join("nte-mods").join("oversized.nte").exists());
    }

    #[test]
    fn source_write_failure_restores_previous_blueprint_metadata() {
        let root = temp_workspace();
        let mod_directory = root.join(MOD_DIRECTORY_NAME);
        fs::create_dir_all(mod_directory.join("blocked.nte")).unwrap();
        let blueprint_path = mod_script_blueprint_path(&mod_directory, "blocked");
        atomic_write_text(&blueprint_path, "previous blueprint").unwrap();
        let source = new_mod_script_template("blocked").unwrap();
        let blueprint = ModScriptBlueprint {
            nodes: vec![ModScriptBlueprintNode {
                signature: "0:value = 1\n".to_owned(),
                position: [10.0, 20.0],
                description: String::new(),
            }],
        };

        assert!(matches!(
            save_mod_script(&root, "blocked", &source, &blueprint),
            Err(ModScriptError::FileSystem(_))
        ));
        assert_eq!(
            fs::read_to_string(blueprint_path).unwrap(),
            "previous blueprint"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_validation_requires_matching_v4_declaration_and_handler() {
        assert_eq!(
            validate_mod_source(
                "telemetry",
                "nte_mod(4)\nmod(\"other\")\ndef on_viewport_tick(event):\n    value = 1\n"
            ),
            Err(ModScriptError::MismatchedModDeclaration)
        );
        assert_eq!(
            validate_mod_source("telemetry", "nte_mod(4)\nmod(\"telemetry\")\n"),
            Err(ModScriptError::MissingViewportTickHandler)
        );
    }

    #[test]
    fn source_validation_accepts_bundled_programs_with_crlf() {
        for (id, source) in [
            (
                "equipment",
                include_str!("../../plugins/nte-mods/equipment.nte"),
            ),
            (
                "combat-clock",
                include_str!("../../plugins/nte-mods/combat-clock.nte"),
            ),
            (
                "character-telemetry",
                include_str!("../../plugins/examples/character-telemetry.nte"),
            ),
        ] {
            let crlf = source.replace("\r\n", "\n").replace('\n', "\r\n");
            validate_mod_source(id, &crlf).unwrap();
        }
    }

    #[test]
    fn source_validation_rejects_native_grammar_errors() {
        let prefix = concat!(
            "nte_mod(4)\n",
            "mod(\"telemetry\")\n",
            "requires(\"viewport.tick\")\n",
            "def on_viewport_tick(event):\n",
        );
        assert_eq!(
            validate_mod_source("telemetry", &format!("{prefix}  value = 1\n")),
            Err(ModScriptError::InvalidSourceLine(5))
        );
        assert_eq!(
            validate_mod_source("telemetry", &format!("{prefix}    unknown.call()\n")),
            Err(ModScriptError::InvalidSourceLine(5))
        );
        assert_eq!(
            validate_mod_source(
                "telemetry",
                concat!(
                    "nte_mod(4)\n",
                    "mod(\"telemetry\")\n",
                    "requires(\"unknown\")\n",
                    "def on_viewport_tick(event):\n",
                    "    value = 1\n",
                )
            ),
            Err(ModScriptError::InvalidSourceLine(3))
        );
    }

    #[test]
    fn save_rejects_invalid_native_grammar_before_writing_files() {
        let root = temp_workspace();
        let source = concat!(
            "nte_mod(4)\n",
            "mod(\"telemetry\")\n",
            "requires(\"viewport.tick\")\n",
            "def on_viewport_tick(event):\n",
            "    unknown.call()\n",
        );

        assert_eq!(
            save_mod_script(&root, "telemetry", source, &ModScriptBlueprint::default(),),
            Err(ModScriptError::InvalidSourceLine(5))
        );
        assert!(!root.join(MOD_DIRECTORY_NAME).exists());
    }

    #[test]
    fn source_validation_rejects_capability_and_instruction_budget_mismatches() {
        assert_eq!(
            validate_mod_source(
                "telemetry",
                concat!(
                    "nte_mod(4)\n",
                    "mod(\"telemetry\")\n",
                    "requires(\"viewport.tick\")\n",
                    "requires(\"log\")\n",
                    "def on_viewport_tick(event):\n",
                    "    value = 1\n",
                )
            ),
            Err(ModScriptError::CapabilityMismatch)
        );

        let mut oversized = concat!(
            "nte_mod(4)\n",
            "mod(\"telemetry\")\n",
            "requires(\"viewport.tick\")\n",
            "def on_viewport_tick(event):\n",
        )
        .to_owned();
        oversized.push_str(&"    value = 1\n".repeat(MAX_MOD_INSTRUCTIONS + 1));
        assert_eq!(
            validate_mod_source("telemetry", &oversized),
            Err(ModScriptError::SourceBudgetExceeded)
        );
    }

    #[test]
    fn enabled_set_rejects_duplicates_and_path_characters() {
        assert_eq!(
            parse_enabled_mods("nte_mod_set 1\nload telemetry\nload telemetry\n"),
            Err(ModScriptError::DuplicateModId("telemetry".to_owned()))
        );
        assert_eq!(
            parse_enabled_mods("nte_mod_set 1\nload ../telemetry\n"),
            Err(ModScriptError::InvalidModId("../telemetry".to_owned()))
        );
    }
}
