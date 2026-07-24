use std::collections::{BTreeMap, HashSet};

use crate::core::snapshot::{
    CHARACTER_LOADOUT_MAX_JSON_BYTES, CharacterLoadoutError, ItemUid, ValidatedCharacterLoadout,
    export_character_loadout_json, parse_character_loadout_json, validate_character_loadout,
};
use crate::engine::model::{EmptyCurtainCharacter, EmptyCurtainItem, EquipmentStat, HtItemNetId};
use crate::engine::parser::{EMPTY_CURTAIN_MAX_STAT_ROWS, EquipmentCatalog, EquipmentKind};

use super::*;

// Cards flex between these bounds: the min drives how many columns fit, then
// each card stretches to fill its share of the row so neither side is left with
// redundant margin. The max keeps a lone card from ballooning on a narrow window.
const EQUIPMENT_CARD_MIN_WIDTH: f32 = 210.0;
const EQUIPMENT_CARD_MAX_WIDTH: f32 = 280.0;
const EQUIPMENT_CARD_HEIGHT: f32 = 210.0;
const EQUIPMENT_CARD_GAP: f32 = 8.0;
const EQUIPMENT_CARD_ROW_HEIGHT: f32 = EQUIPMENT_CARD_HEIGHT + EQUIPMENT_CARD_GAP;
const EQUIPMENT_CARD_HEADER_HEIGHT: f32 = 62.0;
const EQUIPMENT_STAT_ROW_HEIGHT: f32 = 22.0;
const EQUIPMENT_ICON_SIZE: f32 = 42.0;
const EQUIPPED_CHARACTER_AVATAR_SIZE: f32 = 28.0;
const FILTER_TILE_WIDTH: f32 = 104.0;
const FILTER_TILE_HEIGHT: f32 = 88.0;
const FILTER_ICON_SIZE: f32 = 48.0;
const EQUIPMENT_PLUGIN_RISK_LOCK_DURATION: Duration = Duration::from_secs(5);

// Modules and cassettes share the same closed set of rarity tiers, ordered from
// lowest to highest. The parser rejects any other value, so this list is total.
const EQUIPMENT_QUALITIES: [&str; 3] = ["blue", "purple", "orange"];

#[derive(Clone)]
struct PendingModulePlacement {
    character: EmptyCurtainCharacter,
    equipment: HtItemNetId,
    item_id: String,
    move_from_other_character: bool,
}

enum EquipmentEquipSelection {
    Module(PendingModulePlacement),
    Submit {
        character: HtItemNetId,
        operation: EquipmentPluginOperation,
    },
}

#[derive(Clone, Copy)]
enum CharacterEquipmentAction {
    ImportLoadout,
    ExportLoadout(EmptyCurtainCharacter),
    EquipOneKey(EmptyCurtainCharacter),
    UnequipAll(EmptyCurtainCharacter),
}

struct PendingPluginRequest {
    request_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PluginDeploymentAction {
    Inspect,
    Install(EquipmentPluginGameRegion),
    Remove(EquipmentPluginGameRegion),
}

struct PendingPluginDeployment {
    action: PluginDeploymentAction,
    receiver: Receiver<Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError>>,
}

#[derive(Clone, Copy)]
struct PluginRiskConfirmation {
    opened_at: Instant,
    viewport: egui::ViewportId,
    region: EquipmentPluginGameRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OneKeyPlanError {
    MissingTemplate,
    MissingEquipment,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum EquipmentFilterKey {
    Module(String),
    Core(String),
}

#[derive(Clone)]
struct EquipmentFilterOption {
    key: EquipmentFilterKey,
    name_zh: String,
    name_en: String,
    name_ja: String,
    icon: String,
}

struct EmptyCurtainVisuals<'a> {
    catalog: &'a EquipmentCatalog,
    equipment_textures: &'a HashMap<String, egui::TextureHandle>,
    characters: &'a HashMap<u32, CharacterInfo>,
    avatar_textures: &'a HashMap<String, egui::TextureHandle>,
    dark_mode: bool,
}

struct EquipmentFilterCandidate<'a> {
    equipment: Option<&'a EquipmentFilterKey>,
    quality: Option<&'a str>,
    character_id: Option<u32>,
    sub_stats: &'a [EquipmentStat],
}

#[derive(Default)]
struct EmptyCurtainFilterCache {
    valid: bool,
    inventory_generation: u64,
    filter_revision: u64,
    source_len: usize,
    indices: Vec<usize>,
}

#[derive(Default)]
pub(crate) struct KongmuUiState {
    filter_open: bool,
    selected_equipment: HashSet<EquipmentFilterKey>,
    selected_characters: HashSet<u32>,
    selected_qualities: HashSet<String>,
    selected_substats: HashSet<String>,
    filter_revision: u64,
    filter_cache: EmptyCurtainFilterCache,
    pending_module_placement: Option<PendingModulePlacement>,
    plugin_request: Option<PendingPluginRequest>,
    plugin_deployment_status:
        Option<Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError>>,
    plugin_deployment: Option<PendingPluginDeployment>,
    plugin_risk_confirmation: Option<PluginRiskConfirmation>,
    selected_plugin_region: Option<EquipmentPluginGameRegion>,
}

impl KongmuUiState {
    pub(crate) fn invalidate_inventory(&mut self) {
        self.filter_cache.valid = false;
    }

    pub(crate) fn reset_session_state(&mut self) {
        self.invalidate_inventory();
        self.pending_module_placement = None;
        self.plugin_request = None;
    }

    fn active_filter_count(&self) -> usize {
        self.selected_equipment.len()
            + self.selected_characters.len()
            + self.selected_qualities.len()
            + self.selected_substats.len()
    }

    fn toggle_equipment(&mut self, key: EquipmentFilterKey) {
        if !self.selected_equipment.remove(&key) {
            self.selected_equipment.insert(key);
        }
        self.filter_revision = self.filter_revision.wrapping_add(1);
    }

    fn toggle_quality(&mut self, quality: String) {
        if !self.selected_qualities.remove(&quality) {
            self.selected_qualities.insert(quality);
        }
        self.filter_revision = self.filter_revision.wrapping_add(1);
    }

    fn toggle_character(&mut self, character_id: u32) {
        if !self.selected_characters.remove(&character_id) {
            self.selected_characters.insert(character_id);
        }
        self.filter_revision = self.filter_revision.wrapping_add(1);
    }

    fn toggle_substat(&mut self, property: String) {
        if !self.selected_substats.remove(&property) {
            self.selected_substats.insert(property);
        }
        self.filter_revision = self.filter_revision.wrapping_add(1);
    }

    fn clear_filters(&mut self) {
        if self.selected_equipment.is_empty()
            && self.selected_characters.is_empty()
            && self.selected_qualities.is_empty()
            && self.selected_substats.is_empty()
        {
            return;
        }
        self.selected_equipment.clear();
        self.selected_characters.clear();
        self.selected_qualities.clear();
        self.selected_substats.clear();
        self.filter_revision = self.filter_revision.wrapping_add(1);
    }

    fn refresh_filter_cache(
        &mut self,
        items: &[EmptyCurtainItem],
        inventory_generation: u64,
        catalog: &EquipmentCatalog,
    ) {
        if self.filter_cache.valid
            && self.filter_cache.inventory_generation == inventory_generation
            && self.filter_cache.filter_revision == self.filter_revision
            && self.filter_cache.source_len == items.len()
        {
            return;
        }

        self.filter_cache.indices.clear();
        for (index, item) in items.iter().enumerate() {
            let definition = catalog.items.get(&item.item_id);
            let item_key = equipment_filter_key(item, catalog);
            let item_quality = definition.map(|definition| definition.quality.as_str());
            if empty_curtain_filter_matches(
                &self.selected_equipment,
                &self.selected_qualities,
                &self.selected_substats,
                &self.selected_characters,
                EquipmentFilterCandidate {
                    equipment: item_key.as_ref(),
                    quality: item_quality,
                    character_id: item.equipped_character_id,
                    sub_stats: &item.sub_stats,
                },
            ) {
                self.filter_cache.indices.push(index);
            }
        }
        self.filter_cache.valid = true;
        self.filter_cache.inventory_generation = inventory_generation;
        self.filter_cache.filter_revision = self.filter_revision;
        self.filter_cache.source_len = items.len();
    }
}

impl DpsApp {
    pub(crate) fn empty_curtain_contents(&mut self, ui: &mut egui::Ui) {
        self.drain_equipment_plugin_response(ui.ctx());
        self.drain_plugin_deployment(ui.ctx());
        if self.kongmu_ui.plugin_deployment_status.is_none()
            && self.kongmu_ui.plugin_deployment.is_none()
        {
            self.start_plugin_deployment(ui.ctx(), PluginDeploymentAction::Inspect);
        }
        self.plugin_deployment_controls(ui);
        ui.add_space(8.0);
        self.kongmu_ui.refresh_filter_cache(
            &self.state.empty_curtain,
            self.state.empty_curtain_generation,
            &self.equipment_catalog,
        );

        let active_filter_count = self.kongmu_ui.active_filter_count();
        let mut open_filter = false;
        let mut clear_filters = false;
        let mut export = false;
        let mut character_action = None;
        inline_controls(ui, |ui| {
            let filter_label = if active_filter_count == 0 {
                t("Filter")
            } else {
                tf("Filter ({})", &[&active_filter_count.to_string()])
            };
            if ui.button(filter_label).clicked() {
                open_filter = true;
            }
            if active_filter_count > 0 && ui.button(t("Clear Filters")).clicked() {
                clear_filters = true;
            }
            let visible = self.kongmu_ui.filter_cache.indices.len().to_string();
            let total = self.state.empty_curtain.len().to_string();
            ui.label(inline_text(
                tf("{} of {} items", &[&visible, &total]),
                ui.visuals().weak_text_color(),
            ));
            if !self.state.empty_curtain.is_empty() {
                ui.separator();
                if ui
                    .button(t("Export for Drive Calculator"))
                    .on_hover_text(t(
                        "Save the full inventory as a real_inventory.json for NTE Drive Calculator",
                    ))
                    .clicked()
                {
                    export = true;
                }
            }
            if !self.state.empty_curtain_characters.is_empty() {
                ui.separator();
                ui.menu_button(t("Character Equipment"), |ui| {
                    if self.kongmu_ui.plugin_request.is_some() {
                        ui.label(t("Waiting for the equipment plugin..."));
                        return;
                    }
                    if ui
                        .button(t("Import Loadout..."))
                        .on_hover_text(t(
                            "Import a character Console loadout JSON and switch equipment through the plugin",
                        ))
                        .clicked()
                    {
                        character_action = Some(CharacterEquipmentAction::ImportLoadout);
                        ui.close();
                    }
                    ui.separator();
                    for character in &self.state.empty_curtain_characters {
                        let fallback =
                            tf("Character ID {}", &[&character.character_id.to_string()]);
                        let name = character_display_name(
                            &self.characters,
                            character.character_id,
                            &fallback,
                        );
                        ui.menu_button(name, |ui| {
                            if ui
                                .button(t("Export Loadout..."))
                                .on_hover_text(t(
                                    "Save this character's equipped Console loadout as JSON",
                                ))
                                .clicked()
                            {
                                character_action =
                                    Some(CharacterEquipmentAction::ExportLoadout(*character));
                                ui.close();
                            }
                            if ui.button(t("One-click Equip")).clicked() {
                                character_action =
                                    Some(CharacterEquipmentAction::EquipOneKey(*character));
                                ui.close();
                            }
                            if ui.button(t("Unequip All")).clicked() {
                                character_action =
                                    Some(CharacterEquipmentAction::UnequipAll(*character));
                                ui.close();
                            }
                        });
                    }
                });
                ui.separator();
                ui.label(inline_text(
                    t("Right-click equipment to manage it through the plugin"),
                    ui.visuals().weak_text_color(),
                ));
            }
        });
        if open_filter {
            self.kongmu_ui.filter_open = true;
        }
        if export {
            let ctx = ui.ctx().clone();
            self.export_empty_curtain(&ctx);
        }
        if clear_filters {
            self.kongmu_ui.clear_filters();
            self.kongmu_ui.refresh_filter_cache(
                &self.state.empty_curtain,
                self.state.empty_curtain_generation,
                &self.equipment_catalog,
            );
        }
        if let Some(action) = character_action {
            match action {
                CharacterEquipmentAction::ImportLoadout => {
                    self.import_character_loadout(ui.ctx());
                }
                CharacterEquipmentAction::ExportLoadout(character) => {
                    self.export_character_loadout(ui.ctx(), character);
                }
                CharacterEquipmentAction::EquipOneKey(character) => {
                    match build_one_key_plan(
                        character,
                        &self.state.empty_curtain,
                        &self.equipment_catalog,
                    ) {
                        Ok(operation) => self.submit_equipment_plugin_request(
                            ui.ctx(),
                            character.net_id,
                            operation,
                        ),
                        Err(OneKeyPlanError::MissingTemplate) => self.set_last_error_in(
                            ui.ctx(),
                            t("No character equipment template is available"),
                            None,
                        ),
                        Err(OneKeyPlanError::MissingEquipment) => self.set_last_error_in(
                            ui.ctx(),
                            t("No complete one-click loadout is available in the inventory"),
                            None,
                        ),
                    }
                }
                CharacterEquipmentAction::UnequipAll(character) => {
                    self.submit_equipment_plugin_request(
                        ui.ctx(),
                        character.net_id,
                        EquipmentPluginOperation::UnequipAll,
                    );
                }
            }
        }
        ui.add_space(6.0);

        if self.state.empty_curtain.is_empty() {
            self.capture_data_empty_state(
                ui,
                t("Waiting for Console equipment data"),
                t("Start capture or import a replay to collect equipment data."),
            );
        } else if self.kongmu_ui.filter_cache.indices.is_empty() {
            let theme = self.theme();
            empty_state_card(
                ui,
                theme,
                t("No equipment matches the current filters"),
                t("Clear the filters to show all recorded equipment."),
                |ui| {
                    if ui.button(t("Clear Filters")).clicked() {
                        self.kongmu_ui.clear_filters();
                        self.kongmu_ui.refresh_filter_cache(
                            &self.state.empty_curtain,
                            self.state.empty_curtain_generation,
                            &self.equipment_catalog,
                        );
                    }
                },
            );
        } else {
            let selection = draw_empty_curtain_grid(
                ui,
                &self.state.empty_curtain,
                &self.kongmu_ui.filter_cache.indices,
                &self.state.empty_curtain_characters,
                self.kongmu_ui.plugin_request.is_some(),
                &EmptyCurtainVisuals {
                    catalog: &self.equipment_catalog,
                    equipment_textures: &self.equipment_textures,
                    characters: &self.characters,
                    avatar_textures: &self.avatar_textures,
                    dark_mode: self.preferences.dark_mode,
                },
            );
            if let Some(selection) = selection {
                match selection {
                    EquipmentEquipSelection::Module(placement) => {
                        self.kongmu_ui.pending_module_placement = Some(placement);
                    }
                    EquipmentEquipSelection::Submit {
                        character,
                        operation,
                    } => self.submit_equipment_plugin_request(ui.ctx(), character, operation),
                }
            }
        }

        if self.kongmu_ui.filter_open {
            let ctx = ui.ctx().clone();
            show_empty_curtain_filter_window(
                &ctx,
                &self.state.empty_curtain,
                &EmptyCurtainVisuals {
                    catalog: &self.equipment_catalog,
                    equipment_textures: &self.equipment_textures,
                    characters: &self.characters,
                    avatar_textures: &self.avatar_textures,
                    dark_mode: self.preferences.dark_mode,
                },
                &mut self.kongmu_ui,
            );
        }

        if let Some((placement, row, column)) = show_module_placement_window(
            ui.ctx(),
            &mut self.kongmu_ui.pending_module_placement,
            &self.characters,
            &self.equipment_catalog,
        ) {
            let operation = if placement.move_from_other_character {
                EquipmentPluginOperation::MoveModuleToCharacter {
                    equipment: placement.equipment,
                    row,
                    column,
                }
            } else {
                EquipmentPluginOperation::EquipModule {
                    equipment: placement.equipment,
                    row,
                    column,
                }
            };
            self.submit_equipment_plugin_request(ui.ctx(), placement.character.net_id, operation);
        }
        if self.kongmu_ui.plugin_request.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    fn plugin_deployment_controls(&mut self, ui: &mut egui::Ui) {
        let theme = self.theme();
        let pending_action = self
            .kongmu_ui
            .plugin_deployment
            .as_ref()
            .map(|pending| pending.action);
        let deployment_status = self.kongmu_ui.plugin_deployment_status.as_ref();
        let games = deployment_status
            .and_then(|result| result.as_ref().ok())
            .map(|status| status.games.as_slice())
            .unwrap_or_default();
        let mut selected_region = self
            .kongmu_ui
            .selected_plugin_region
            .filter(|selected| games.iter().any(|game| game.region == *selected))
            .or_else(|| {
                games
                    .iter()
                    .find(|game| game.installed)
                    .or_else(|| games.first())
                    .map(|game| game.region)
            });
        let mut enabled = selected_region.is_some_and(|selected| {
            games
                .iter()
                .any(|game| game.region == selected && game.installed)
        });
        let source_available = deployment_status
            .and_then(|result| result.as_ref().ok())
            .is_some_and(|status| status.source_available);
        let interactive = pending_action.is_none()
            && !self.update_client.busy()
            && selected_region.is_some()
            && (enabled || source_available);
        let status_text =
            plugin_deployment_status_text(pending_action, deployment_status, selected_region);
        let mut requested = None;

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("⚠").color(theme.warning))
                .on_hover_text(t(
                    "This is a third-party mod loaded by the game process. Read the risk warning before enabling it.",
                ));
            ui.label(RichText::new(t("In-game Equipment Plugin")).strong());
            if let Some(region) = &mut selected_region {
                ui.add_enabled_ui(pending_action.is_none(), |ui| {
                    egui::ComboBox::from_id_salt("equipment_plugin_game_region")
                        .width(132.0)
                        .selected_text(t(plugin_game_region_label(*region)))
                        .show_ui(ui, |ui| {
                            for game in games {
                                ui.selectable_value(
                                    region,
                                    game.region,
                                    t(plugin_game_region_label(game.region)),
                                );
                            }
                        });
                });
            }
            ui.label(RichText::new(status_text).color(theme.fg_muted));
            let response = ui.add_enabled(
                interactive,
                egui::Checkbox::new(&mut enabled, t("Enable")),
            );
            if response.changed() {
                requested = Some(enabled);
            }
        });

        self.kongmu_ui.selected_plugin_region = selected_region;

        match (requested, selected_region) {
            (Some(_), _) if self.capture_ui.game_process_detected => self.set_last_error_in(
                ui.ctx(),
                t("Close HTGame.exe before changing the equipment plugin."),
                None,
            ),
            (Some(true), Some(region)) => {
                self.kongmu_ui.plugin_risk_confirmation = Some(PluginRiskConfirmation {
                    opened_at: Instant::now(),
                    viewport: ui.ctx().viewport_id(),
                    region,
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            (Some(false), Some(region)) => {
                self.start_plugin_deployment(ui.ctx(), PluginDeploymentAction::Remove(region))
            }
            _ => {}
        }
    }

    fn start_plugin_deployment(&mut self, ctx: &egui::Context, action: PluginDeploymentAction) {
        if self.kongmu_ui.plugin_deployment.is_some() {
            return;
        }
        let (sender, receiver) = bounded(1);
        let repaint = ctx.clone();
        thread::spawn(move || {
            let plugin = if matches!(action, PluginDeploymentAction::Remove(_)) {
                Ok(None)
            } else {
                read_equipment_plugin()
                    .map_err(|error| EquipmentPluginDeploymentError::FileSystem(error.to_string()))
            };
            let result = plugin.and_then(|plugin| match action {
                PluginDeploymentAction::Inspect => {
                    crate::platform::equipment_plugin::inspect_plugin_deployment(plugin.as_deref())
                }
                PluginDeploymentAction::Install(region) => {
                    crate::platform::equipment_plugin::install_equipment_plugin(
                        region,
                        plugin
                            .as_deref()
                            .ok_or(EquipmentPluginDeploymentError::PluginSourceNotFound)?,
                    )
                }
                PluginDeploymentAction::Remove(region) => {
                    crate::platform::equipment_plugin::remove_equipment_plugin(region)
                }
            });
            let _ = sender.send(result);
            repaint.request_repaint();
        });
        self.kongmu_ui.plugin_deployment = Some(PendingPluginDeployment { action, receiver });
    }

    pub(crate) fn equipment_plugin_deployment_idle(&self) -> bool {
        self.kongmu_ui.plugin_deployment.is_none()
    }

    fn drain_plugin_deployment(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.kongmu_ui.plugin_deployment.as_ref() else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                panic!("equipment plugin deployment worker must return a result")
            }
        };
        let action = pending.action;
        self.kongmu_ui.plugin_deployment = None;
        match result {
            Ok(status) => {
                self.kongmu_ui.plugin_deployment_status = Some(Ok(status));
                match action {
                    PluginDeploymentAction::Inspect => {}
                    PluginDeploymentAction::Install(_) => {
                        self.notifications.status = t("Equipment plugin enabled");
                        self.clear_last_error();
                    }
                    PluginDeploymentAction::Remove(_) => {
                        self.notifications.status = t("Equipment plugin removed");
                        self.clear_last_error();
                        self.start_plugin_deployment(ctx, PluginDeploymentAction::Inspect);
                    }
                }
            }
            Err(error) => {
                if matches!(action, PluginDeploymentAction::Inspect) {
                    self.kongmu_ui.plugin_deployment_status = Some(Err(error.clone()));
                }
                self.set_last_error_in(ctx, plugin_deployment_error_text(&error), None);
                if !matches!(action, PluginDeploymentAction::Inspect) {
                    self.start_plugin_deployment(ctx, PluginDeploymentAction::Inspect);
                }
            }
        }
    }

    pub(crate) fn show_equipment_plugin_risk_dialog(&mut self, ctx: &egui::Context) {
        let Some(confirmation) = self.kongmu_ui.plugin_risk_confirmation else {
            return;
        };
        if confirmation.viewport != ctx.viewport_id() {
            return;
        }
        let remaining = plugin_risk_confirmation_remaining(confirmation.opened_at, Instant::now());
        let unlocked = remaining.is_zero();
        if !unlocked {
            ctx.request_repaint_after(Duration::from_millis(100).min(remaining));
        }
        let mut confirm = unlocked
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let mut cancel =
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if !unlocked {
            ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        }
        let theme = self.theme();
        egui::Modal::new(egui::Id::new("equipment_plugin_risk_confirmation"))
            .backdrop_color(theme.modal_backdrop)
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(theme.bg_elevated)
                    .stroke(Stroke::new(1.0_f32, theme.danger))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(320.0, 500.0));
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(t("Warning: third-party game plugin"))
                            .size(20.0)
                            .strong()
                            .color(theme.danger),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(t(
                            "This plugin lets the tool equip, move, unequip, lock, discard, and apply one-key equipment plans in the game. Leave it disabled if you do not use these features.",
                        ))
                        .strong()
                        .color(theme.success),
                    );
                    for message in [
                        "Enabling this option installs a third-party mod into the game directory.",
                        "It copies dwmapi.dll beside HTGame.exe. The game loads the proxy at startup, and the proxy exposes a local named pipe used for equipment operations.",
                        "Changing the game directory may trigger integrity or anti-cheat checks and may cause client or account risk. Enable it only after accepting these risks.",
                    ] {
                        ui.label(RichText::new(t(message)).color(theme.danger));
                    }
                    ui.add_space(12.0);
                    if !unlocked {
                        let seconds = remaining.as_millis().div_ceil(1_000).to_string();
                        ui.label(
                            RichText::new(tf(
                                "Please read the warning. Enable unlocks in {} seconds; you can close this dialog at any time.",
                                &[&seconds],
                            ))
                            .strong()
                            .color(theme.warning),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(unlocked, egui::Button::new(t("Accept Risk and Enable")))
                            .clicked()
                        {
                            confirm = true;
                        }
                        if ui.button(t("Cancel")).clicked() {
                            cancel = true;
                        }
                    });
                });
            });
        if confirm {
            self.kongmu_ui.plugin_risk_confirmation = None;
            self.start_plugin_deployment(ctx, PluginDeploymentAction::Install(confirmation.region));
        } else if cancel {
            self.kongmu_ui.plugin_risk_confirmation = None;
        }
    }

    pub(crate) fn retarget_equipment_plugin_dialog(
        &mut self,
        from: egui::ViewportId,
        to: egui::ViewportId,
    ) {
        if let Some(confirmation) = &mut self.kongmu_ui.plugin_risk_confirmation
            && confirmation.viewport == from
        {
            confirmation.viewport = to;
        }
    }

    fn submit_equipment_plugin_request(
        &mut self,
        ctx: &egui::Context,
        character: HtItemNetId,
        operation: EquipmentPluginOperation,
    ) {
        match self.equipment_plugin.submit(character, operation) {
            Ok(request_id) => {
                self.kongmu_ui.plugin_request = Some(PendingPluginRequest { request_id });
                self.notifications.status = t("Sending equipment request...");
                self.clear_last_error();
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            Err(EquipmentPluginSubmitError::Busy) => {
                self.set_last_error_in(ctx, t("Equipment plugin is busy; try again shortly"), None)
            }
        }
    }

    fn drain_equipment_plugin_response(&mut self, ctx: &egui::Context) {
        let Some(response) = self.equipment_plugin.try_recv() else {
            return;
        };
        if self
            .kongmu_ui
            .plugin_request
            .as_ref()
            .is_none_or(|request| request.request_id != response.request_id)
        {
            return;
        }
        self.kongmu_ui.plugin_request = None;
        match response.status {
            Ok(0) => {
                self.notifications.status =
                    t("Equipment RPC dispatched; waiting for game synchronization");
                self.clear_last_error();
            }
            Ok(1) => {
                self.notifications.status = t("Equipment request passed plugin dry-run validation");
                self.clear_last_error();
            }
            Ok(status) => self.set_last_error_in(
                ctx,
                tf(
                    "Equipment plugin rejected the request (status {})",
                    &[&status.to_string()],
                ),
                None,
            ),
            Err(error) => self.set_last_error_in(
                ctx,
                tf("Equipment plugin is unavailable: {}", &[&error]),
                None,
            ),
        }
    }

    fn import_character_loadout(&mut self, ctx: &egui::Context) {
        let filter = t("Console character loadout");
        self.spawn_file_dialog(
            ctx,
            FileDialogPurpose::CharacterLoadoutImport,
            move |owner| {
                with_owner(rfd::FileDialog::new().add_filter(filter, &["json"]), owner).pick_file()
            },
        );
    }

    fn export_character_loadout(&mut self, ctx: &egui::Context, character: EmptyCurtainCharacter) {
        let json = match export_character_loadout_json(
            character,
            &self.state.empty_curtain,
            &self.equipment_catalog,
        ) {
            Ok(json) => json,
            Err(error) => {
                self.set_last_error_in(ctx, character_loadout_error_text(&error), None);
                return;
            }
        };
        let default_name = format!(
            "nte_loadout_{}_{}.json",
            character.character_id,
            Local::now().format("%Y%m%d_%H%M%S")
        );
        let filter = t("Console character loadout");
        self.spawn_file_dialog(
            ctx,
            FileDialogPurpose::CharacterLoadoutExport { json },
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

    pub(crate) fn finish_character_loadout_import(
        &mut self,
        ctx: &egui::Context,
        viewport: egui::ViewportId,
        path: &std::path::Path,
    ) {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.set_last_error_for(
                    viewport,
                    tf(
                        "Failed to read character loadout: {}",
                        &[&error.to_string()],
                    ),
                    None,
                );
                return;
            }
        };
        if metadata.len() > CHARACTER_LOADOUT_MAX_JSON_BYTES as u64 {
            self.set_last_error_for(
                viewport,
                character_loadout_error_text(&CharacterLoadoutError::JsonTooLarge),
                None,
            );
            return;
        }
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(error) => {
                self.set_last_error_for(
                    viewport,
                    tf(
                        "Failed to read character loadout: {}",
                        &[&error.to_string()],
                    ),
                    None,
                );
                return;
            }
        };
        let file = match parse_character_loadout_json(&json) {
            Ok(file) => file,
            Err(error) => {
                self.set_last_error_for(viewport, character_loadout_error_text(&error), None);
                return;
            }
        };
        let loadout = match validate_character_loadout(
            &file,
            &self.state.empty_curtain_characters,
            &self.state.empty_curtain,
            &self.equipment_catalog,
        ) {
            Ok(loadout) => loadout,
            Err(error) => {
                self.set_last_error_for(viewport, character_loadout_error_text(&error), None);
                return;
            }
        };
        let operation = character_loadout_plugin_operation(&loadout);
        self.submit_equipment_plugin_request(ctx, loadout.character.net_id, operation);
    }

    pub(crate) fn finish_character_loadout_export(
        &mut self,
        viewport: egui::ViewportId,
        path: &std::path::Path,
        json: &str,
    ) {
        match atomic_write_text(path, json) {
            Ok(()) => {
                self.notifications.status = t("Character loadout exported");
                self.clear_last_error();
            }
            Err(error) => self.set_last_error_for(
                viewport,
                tf(
                    "Failed to export character loadout: {}",
                    &[&error.to_string()],
                ),
                None,
            ),
        }
    }

    /// Serialize the whole inventory into the `real_inventory.json` schema that
    /// the third-party NTE Drive Calculator imports, then prompt for a save path.
    pub(crate) fn export_empty_curtain(&mut self, ctx: &egui::Context) {
        if self.state.empty_curtain.is_empty() {
            self.set_last_error_in(ctx, t("No Console equipment to export"), None);
            return;
        }
        let export =
            build_drive_calculator_inventory(&self.state.empty_curtain, &self.equipment_catalog);
        let Ok(json) = serde_json::to_string_pretty(&export) else {
            self.set_last_error_in(ctx, t("Failed to serialize Console equipment"), None);
            return;
        };
        let filter = t("Drive Calculator inventory");
        self.spawn_file_dialog(
            ctx,
            FileDialogPurpose::EmptyCurtainExport { json },
            move |owner| {
                with_owner(
                    rfd::FileDialog::new()
                        .add_filter(filter, &["json"])
                        .set_file_name("real_inventory.json"),
                    owner,
                )
                .save_file()
            },
        );
    }

    pub(crate) fn finish_empty_curtain_export(
        &mut self,
        viewport: egui::ViewportId,
        path: &std::path::Path,
        json: &str,
    ) {
        match atomic_write_text(path, json) {
            Ok(()) => {
                self.notifications.status = t("Console equipment exported");
                self.clear_last_error();
            }
            Err(error) => self.set_last_error_for(
                viewport,
                tf(
                    "Failed to export Console equipment: {}",
                    &[&error.to_string()],
                ),
                None,
            ),
        }
    }
}

fn plugin_deployment_status_text(
    pending: Option<PluginDeploymentAction>,
    status: Option<&Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError>>,
    selected_region: Option<EquipmentPluginGameRegion>,
) -> String {
    if let Some(action) = pending {
        return match action {
            PluginDeploymentAction::Inspect => t("Checking equipment plugin status..."),
            PluginDeploymentAction::Install(_) => t("Installing equipment plugin..."),
            PluginDeploymentAction::Remove(_) => t("Removing equipment plugin..."),
        };
    }
    let selected_game = status
        .and_then(|result| result.as_ref().ok())
        .and_then(|status| {
            selected_region
                .and_then(|selected| status.games.iter().find(|game| game.region == selected))
        });
    let source_available = status
        .and_then(|result| result.as_ref().ok())
        .is_some_and(|status| status.source_available);
    match (status, selected_game) {
        (_, Some(_)) if !source_available => {
            t("Equipment plugin file plugins/dwmapi.dll was not found")
        }
        (_, Some(game)) if !game.installed => t("Equipment plugin is disabled"),
        (_, Some(game)) if !game.current => {
            t("Equipment plugin is enabled, but the installed copy differs from this app version")
        }
        (_, Some(_)) => t("Equipment plugin is enabled for the selected game client"),
        (Some(Err(EquipmentPluginDeploymentError::GameInstallationNotFound)), _) => {
            t("Game installation not detected")
        }
        (Some(Err(_)), _) => t("Equipment plugin status check failed"),
        _ => t("Checking equipment plugin status..."),
    }
}

fn plugin_game_region_label(region: EquipmentPluginGameRegion) -> &'static str {
    match region {
        EquipmentPluginGameRegion::China => "China client",
        EquipmentPluginGameRegion::Global => "Global client",
    }
}

fn plugin_risk_confirmation_remaining(opened_at: Instant, now: Instant) -> Duration {
    EQUIPMENT_PLUGIN_RISK_LOCK_DURATION.saturating_sub(now.saturating_duration_since(opened_at))
}

fn plugin_deployment_error_text(error: &EquipmentPluginDeploymentError) -> String {
    match error {
        EquipmentPluginDeploymentError::GameRunning => {
            t("Close HTGame.exe before changing the equipment plugin.")
        }
        EquipmentPluginDeploymentError::GameProcessProbe(error) => tf(
            "Failed to check whether HTGame.exe is running: {}",
            &[error],
        ),
        EquipmentPluginDeploymentError::GameInstallationNotFound => t(
            "No supported game installation was found. Repair or reinstall the official launcher registration, then try again.",
        ),
        EquipmentPluginDeploymentError::Registry(error) => tf(
            "Failed to locate the game installation from the registry: {}",
            &[error],
        ),
        EquipmentPluginDeploymentError::PluginSourceNotFound => {
            t("Equipment plugin file plugins/dwmapi.dll was not found")
        }
        EquipmentPluginDeploymentError::ConflictingDwmapi => t(
            "The game directory already contains a dwmapi.dll that is not managed by this tool. Remove the conflicting mod manually before enabling this plugin.",
        ),
        EquipmentPluginDeploymentError::InstalledPluginChanged => t(
            "The installed dwmapi.dll or its ownership marker changed outside this tool. Check the game directory manually before trying again.",
        ),
        EquipmentPluginDeploymentError::FileSystem(error) => {
            tf("Failed to update the equipment plugin files: {}", &[error])
        }
    }
}

fn character_loadout_plugin_operation(
    loadout: &ValidatedCharacterLoadout,
) -> EquipmentPluginOperation {
    let mut placements = Vec::with_capacity(loadout.placements.len());
    for placement in &loadout.placements {
        placements.push(EquipmentPluginPlacement {
            equipment: placement.equipment,
            row: placement.row,
            column: placement.column,
        });
    }
    EquipmentPluginOperation::EquipOneKey {
        placements,
        core: loadout.core,
    }
}

fn character_loadout_error_text(error: &CharacterLoadoutError) -> String {
    match error {
        CharacterLoadoutError::JsonTooLarge => t("Character loadout file is too large"),
        CharacterLoadoutError::InvalidJson => t("Character loadout JSON is invalid"),
        CharacterLoadoutError::UnsupportedVersion(version) => tf(
            "Unsupported character loadout version: {}",
            &[&version.to_string()],
        ),
        CharacterLoadoutError::CharacterUnavailable(character_id) => tf(
            "Character {} is not available in the current session",
            &[&character_id.to_string()],
        ),
        CharacterLoadoutError::MissingCharacterPlan(_) => {
            t("No character equipment template is available")
        }
        CharacterLoadoutError::MissingCore => t("Character loadout has no cassette"),
        CharacterLoadoutError::MultipleCores => t("Character loadout has multiple cassettes"),
        CharacterLoadoutError::MissingModules => t("Character loadout has no drive modules"),
        CharacterLoadoutError::TooManyModules => t("Character loadout has too many drive modules"),
        CharacterLoadoutError::MissingPlacement(uid) => tf(
            "Equipped drive module {} has no captured position",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::InvalidUid(uid) => tf(
            "Character loadout contains an invalid equipment UID: {}",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::DuplicateItem(uid) => tf(
            "Character loadout references equipment more than once: {}",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::UnknownItem(item_id) => tf(
            "Character loadout references unknown equipment: {}",
            &[item_id],
        ),
        CharacterLoadoutError::MissingInventoryItem(uid) => tf(
            "Equipment {} is not present in the current inventory",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::ItemMismatch(uid) => tf(
            "Equipment {} no longer matches the exported item",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::WrongKind(uid) => tf(
            "Equipment {} has the wrong type",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::ItemInUse(uid) => tf(
            "Equipment {} is currently equipped by another character",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::InvalidPosition(uid) => tf(
            "Equipment {} has an invalid position for this character",
            &[&character_loadout_item_label(*uid)],
        ),
        CharacterLoadoutError::OverlappingModules => {
            t("Character loadout contains overlapping drive modules")
        }
    }
}

fn character_loadout_item_label(uid: ItemUid) -> String {
    format!("{}:{}", uid.slot, uid.serial)
}

fn draw_empty_curtain_grid(
    ui: &mut egui::Ui,
    items: &[EmptyCurtainItem],
    filtered_indices: &[usize],
    characters: &[EmptyCurtainCharacter],
    request_pending: bool,
    visuals: &EmptyCurtainVisuals<'_>,
) -> Option<EquipmentEquipSelection> {
    let mut selection = None;
    let available_width = ui.available_width();
    let column_count = |width: f32| {
        (((width + EQUIPMENT_CARD_GAP) / (EQUIPMENT_CARD_MIN_WIDTH + EQUIPMENT_CARD_GAP)).floor()
            as usize)
            .max(1)
    };
    // The scroll area reserves this gutter on the right for its bar; fold it out
    // of the layout width when the grid overflows so the last column never slips
    // underneath the bar. A short list that doesn't scroll keeps the full width.
    let scroll = ui.spacing().scroll;
    let scrollbar_gutter = scroll.bar_width + scroll.bar_inner_margin + scroll.bar_outer_margin;
    let unscrolled_rows = filtered_indices
        .len()
        .div_ceil(column_count(available_width));
    let content_width =
        if unscrolled_rows as f32 * EQUIPMENT_CARD_ROW_HEIGHT > ui.available_height() {
            (available_width - scrollbar_gutter).max(EQUIPMENT_CARD_MIN_WIDTH)
        } else {
            available_width
        };
    let columns = column_count(content_width);
    // Stretch each card to fill its share of the row so the grid spans the full
    // width instead of centering a fixed-width block with wasted side margins.
    let card_width = ((content_width - EQUIPMENT_CARD_GAP * (columns - 1) as f32) / columns as f32)
        .min(EQUIPMENT_CARD_MAX_WIDTH);
    let row_count = filtered_indices.len().div_ceil(columns);

    egui::ScrollArea::vertical()
        .id_salt("empty_curtain_cards")
        .max_height(ui.available_height())
        .show_rows(
            ui,
            EQUIPMENT_CARD_ROW_HEIGHT,
            row_count,
            |ui, visible_rows| {
                for row_index in visible_rows {
                    let first = row_index * columns;
                    let count = (filtered_indices.len() - first).min(columns);
                    let (row_rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), EQUIPMENT_CARD_ROW_HEIGHT),
                        egui::Sense::hover(),
                    );
                    let cards_rect = egui::Rect::from_min_size(
                        row_rect.min,
                        egui::vec2(row_rect.width(), EQUIPMENT_CARD_HEIGHT),
                    );
                    let mut cards = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(cards_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Min)),
                    );
                    cards.spacing_mut().item_spacing.x = EQUIPMENT_CARD_GAP;
                    for item_index in filtered_indices[first..first + count].iter().copied() {
                        draw_empty_curtain_card(
                            &mut cards,
                            &items[item_index],
                            card_width,
                            characters,
                            request_pending,
                            visuals,
                            &mut selection,
                        );
                    }
                }
            },
        );
    selection
}

fn draw_empty_curtain_card(
    ui: &mut egui::Ui,
    item: &EmptyCurtainItem,
    card_width: f32,
    characters: &[EmptyCurtainCharacter],
    request_pending: bool,
    visuals: &EmptyCurtainVisuals<'_>,
    selection: &mut Option<EquipmentEquipSelection>,
) -> egui::Response {
    assert!(
        item.main_stats.len() + item.sub_stats.len() <= EMPTY_CURTAIN_MAX_STAT_ROWS,
        "validated Console equipment must fit the shared card layout"
    );
    let definition = visuals.catalog.items.get(&item.item_id);
    let language = i18n::current_language();
    let name = definition.map_or(item.item_id.as_str(), |definition| {
        definition.name(language)
    });
    let texture =
        definition.and_then(|definition| visuals.equipment_textures.get(&definition.icon));

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(card_width, EQUIPMENT_CARD_HEIGHT),
        egui::Sense::click(),
    );
    // Keep every card-local overlay inside its already allocated card. Otherwise
    // an inner `allocate_rect` can move the parent row cursor and overlap the next card.
    let clip_rect = ui.clip_rect().intersect(rect);
    let mut card_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    card_ui.set_clip_rect(clip_rect);
    let ui = &mut card_ui;
    let fill = if response.hovered() {
        shadcn_card_hover(visuals.dark_mode)
    } else {
        shadcn_card(visuals.dark_mode)
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(8),
        fill,
        Stroke::new(1.0_f32, shadcn_border(visuals.dark_mode)),
        egui::StrokeKind::Inside,
    );
    let header_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), rect.top() + EQUIPMENT_CARD_HEADER_HEIGHT),
    );
    ui.painter().rect_filled(
        header_rect,
        egui::CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        },
        if response.hovered() {
            shadcn_muted(visuals.dark_mode)
        } else {
            shadcn_card_hover(visuals.dark_mode)
        },
    );
    ui.painter().hline(
        rect.left() + 1.0..=rect.right() - 1.0,
        header_rect.bottom(),
        Stroke::new(1.0_f32, shadcn_border(visuals.dark_mode)),
    );

    let icon_rect = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(10.0, 10.0),
        egui::vec2(EQUIPMENT_ICON_SIZE, EQUIPMENT_ICON_SIZE),
    );
    draw_equipment_icon(ui, icon_rect, texture, name, visuals.dark_mode);
    paint_equipment_status(
        ui,
        rect,
        item.is_equipped(),
        item.locked,
        item.discarded,
        visuals.dark_mode,
    );
    let avatar_rect = item.equipped_character_id.map(|character_id| {
        paint_equipped_character_avatar(
            ui,
            rect,
            character_id,
            visuals.characters,
            visuals.avatar_textures,
            visuals.dark_mode,
        )
    });

    let text_left = icon_rect.right() + 10.0;
    let text_right = avatar_rect.map_or(rect.right() - 10.0, |rect| rect.left() - 6.0);
    let text_width = text_right - text_left;
    let name_font = fitted_font(
        ui,
        name,
        egui::FontFamily::Proportional,
        15.0,
        10.0,
        text_width,
        shadcn_foreground(visuals.dark_mode),
    );
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(text_left, rect.top() + 24.0),
        egui::pos2(text_right, rect.top() + 42.0),
    );
    ui.painter().with_clip_rect(name_rect).text(
        name_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        name,
        name_font,
        shadcn_foreground(visuals.dark_mode),
    );
    let level = tf("Lv.{}", &[&item.level.to_string()]);
    ui.painter().text(
        egui::pos2(text_left, rect.top() + 49.0),
        egui::Align2::LEFT_CENTER,
        level,
        egui::FontId::proportional(11.5),
        ui.visuals().weak_text_color(),
    );

    let stats_top = header_rect.bottom() + 4.0;
    for (row_index, (stat, main_stat)) in item
        .main_stats
        .iter()
        .map(|stat| (stat, true))
        .chain(item.sub_stats.iter().map(|stat| (stat, false)))
        .enumerate()
    {
        let row_rect = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + 10.0,
                stats_top + row_index as f32 * EQUIPMENT_STAT_ROW_HEIGHT,
            ),
            egui::pos2(
                rect.right() - 10.0,
                stats_top + (row_index + 1) as f32 * EQUIPMENT_STAT_ROW_HEIGHT,
            ),
        );
        if row_index > 0 {
            ui.painter().hline(
                row_rect.left()..=row_rect.right(),
                row_rect.top(),
                Stroke::new(
                    1.0_f32,
                    shadcn_border(visuals.dark_mode).gamma_multiply(0.7),
                ),
            );
        }
        draw_equipment_stat_row(
            ui,
            row_rect,
            stat,
            main_stat,
            visuals.catalog,
            visuals.dark_mode,
        );
    }

    let suit = definition
        .filter(|definition| matches!(definition.kind, EquipmentKind::Core))
        .and_then(|definition| definition.suit.as_deref())
        .and_then(|suit_id| visuals.catalog.suits.get(suit_id));
    if let Some(suit) = suit {
        equipment_card_context_menu(
            &response,
            item,
            definition.map(|definition| definition.kind),
            characters,
            request_pending,
            visuals.characters,
            selection,
        );
        response.on_hover_ui(|ui| {
            ui.set_max_width(400.0);
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(
                RichText::new(suit.name(i18n::current_language()))
                    .size(14.0)
                    .strong(),
            );
            ui.separator();
            for effect in &suit.effects {
                ui.label(
                    RichText::new(tf("{}-Piece Set", &[&effect.count.to_string()]))
                        .size(11.5)
                        .strong()
                        .color(ui.visuals().selection.bg_fill),
                );
                ui.label(effect.text(i18n::current_language()));
            }
        })
    } else {
        equipment_card_context_menu(
            &response,
            item,
            definition.map(|definition| definition.kind),
            characters,
            request_pending,
            visuals.characters,
            selection,
        );
        response
    }
}

fn equipment_card_context_menu(
    response: &egui::Response,
    item: &EmptyCurtainItem,
    kind: Option<EquipmentKind>,
    characters: &[EmptyCurtainCharacter],
    request_pending: bool,
    character_definitions: &HashMap<u32, CharacterInfo>,
    selection: &mut Option<EquipmentEquipSelection>,
) {
    response.context_menu(|ui| {
        if request_pending {
            ui.label(t("Waiting for the equipment plugin..."));
            return;
        }
        let locked = !item.locked;
        if ui
            .button(t(if locked { "Lock" } else { "Unlock" }))
            .clicked()
        {
            *selection = Some(EquipmentEquipSelection::Submit {
                character: HtItemNetId::ZERO,
                operation: EquipmentPluginOperation::SetItemLocked {
                    equipment: item.id,
                    locked,
                },
            });
            ui.close();
            return;
        }
        let discarded = !item.discarded;
        if ui
            .button(t(if discarded {
                "Mark as Discarded"
            } else {
                "Clear Discarded Mark"
            }))
            .clicked()
        {
            *selection = Some(EquipmentEquipSelection::Submit {
                character: HtItemNetId::ZERO,
                operation: EquipmentPluginOperation::SetItemDiscarded {
                    equipment: item.id,
                    discarded,
                },
            });
            ui.close();
            return;
        }
        ui.separator();
        let Some(kind) = kind else {
            ui.label(t("Equipment metadata is unavailable"));
            return;
        };
        if let Some(character) = item.character_net_id {
            if ui.button(t("Unequip")).clicked() {
                let operation = match kind {
                    EquipmentKind::Module => {
                        EquipmentPluginOperation::UnequipModule { equipment: item.id }
                    }
                    EquipmentKind::Core => {
                        EquipmentPluginOperation::UnequipCore { equipment: item.id }
                    }
                };
                *selection = Some(EquipmentEquipSelection::Submit {
                    character,
                    operation,
                });
                ui.close();
                return;
            }
            if characters.iter().any(|target| target.net_id != character) {
                ui.menu_button(t("Move to Character"), |ui| {
                    for target in characters
                        .iter()
                        .filter(|target| target.net_id != character)
                    {
                        let fallback = tf("Character ID {}", &[&target.character_id.to_string()]);
                        let name = character_display_name(
                            character_definitions,
                            target.character_id,
                            &fallback,
                        );
                        if ui.button(name).clicked() {
                            *selection = Some(match kind {
                                EquipmentKind::Module => {
                                    EquipmentEquipSelection::Module(PendingModulePlacement {
                                        character: *target,
                                        equipment: item.id,
                                        item_id: item.item_id.clone(),
                                        move_from_other_character: true,
                                    })
                                }
                                EquipmentKind::Core => EquipmentEquipSelection::Submit {
                                    character: target.net_id,
                                    operation: EquipmentPluginOperation::MoveCoreToCharacter {
                                        equipment: item.id,
                                    },
                                },
                            });
                            ui.close();
                        }
                    }
                });
            }
            return;
        }
        if characters.is_empty() {
            ui.label(t("No captured characters available"));
            return;
        }
        ui.menu_button(t("Equip to Character"), |ui| {
            for character in characters {
                let fallback = tf("Character ID {}", &[&character.character_id.to_string()]);
                let name = character_display_name(
                    character_definitions,
                    character.character_id,
                    &fallback,
                );
                if ui.button(name).clicked() {
                    *selection = Some(match kind {
                        EquipmentKind::Module => {
                            EquipmentEquipSelection::Module(PendingModulePlacement {
                                character: *character,
                                equipment: item.id,
                                item_id: item.item_id.clone(),
                                move_from_other_character: false,
                            })
                        }
                        EquipmentKind::Core => EquipmentEquipSelection::Submit {
                            character: character.net_id,
                            operation: EquipmentPluginOperation::EquipCore { equipment: item.id },
                        },
                    });
                    ui.close();
                }
            }
        });
    });
}

fn build_one_key_plan(
    character: EmptyCurtainCharacter,
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Result<EquipmentPluginOperation, OneKeyPlanError> {
    let plan = catalog
        .plans
        .get(&character.character_id)
        .ok_or(OneKeyPlanError::MissingTemplate)?;
    let mut selected = HashSet::new();
    let mut placements = Vec::with_capacity(plan.recommended_modules.len());

    for planned in &plan.recommended_modules {
        let geometry = &catalog
            .items
            .get(&planned.item_id)
            .expect("validated equipment plan must reference a module")
            .geometry;
        let item = items
            .iter()
            .filter(|item| {
                !selected.contains(&item.id)
                    && (item.character_net_id.is_none()
                        || item.character_net_id == Some(character.net_id))
                    && catalog.items.get(&item.item_id).is_some_and(|definition| {
                        definition.kind == EquipmentKind::Module && definition.geometry == *geometry
                    })
            })
            .max_by_key(|item| {
                let definition = catalog
                    .items
                    .get(&item.item_id)
                    .expect("candidate equipment must have catalog metadata");
                (
                    item.item_id == planned.item_id,
                    equipment_quality_rank(&definition.quality),
                    item.level,
                    item.id.solt,
                    item.id.serial,
                )
            })
            .ok_or(OneKeyPlanError::MissingEquipment)?;
        selected.insert(item.id);
        placements.push(EquipmentPluginPlacement {
            equipment: item.id,
            row: planned.row,
            column: planned.column,
        });
    }

    let recommended_core = catalog
        .items
        .get(&plan.recommended_core)
        .expect("validated equipment plan must reference a cassette");
    let core = items
        .iter()
        .filter(|item| {
            (item.character_net_id.is_none() || item.character_net_id == Some(character.net_id))
                && catalog
                    .items
                    .get(&item.item_id)
                    .is_some_and(|definition| definition.kind == EquipmentKind::Core)
        })
        .max_by_key(|item| {
            let definition = catalog
                .items
                .get(&item.item_id)
                .expect("candidate equipment must have catalog metadata");
            (
                item.item_id == plan.recommended_core,
                definition.suit == recommended_core.suit,
                equipment_quality_rank(&definition.quality),
                item.level,
                item.id.solt,
                item.id.serial,
            )
        })
        .ok_or(OneKeyPlanError::MissingEquipment)?;
    Ok(EquipmentPluginOperation::EquipOneKey {
        placements,
        core: core.id,
    })
}

fn equipment_quality_rank(quality: &str) -> u8 {
    match quality {
        "blue" => 0,
        "purple" => 1,
        "orange" => 2,
        _ => unreachable!("equipment quality is validated at resource load"),
    }
}

fn show_module_placement_window(
    ctx: &egui::Context,
    pending: &mut Option<PendingModulePlacement>,
    characters: &HashMap<u32, CharacterInfo>,
    catalog: &EquipmentCatalog,
) -> Option<(PendingModulePlacement, i32, i32)> {
    let placement = pending.as_ref()?.clone();
    let valid_positions = catalog
        .valid_module_positions(placement.character.character_id, &placement.item_id)
        .expect("captured module and character must have validated equipment metadata");
    let mut open = true;
    let mut selected = None;
    egui::Window::new(t("Choose Drive Module Position"))
        .id(egui::Id::new("equipment_plugin_module_position"))
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .show(ctx, |ui| {
            let fallback = tf(
                "Character ID {}",
                &[&placement.character.character_id.to_string()],
            );
            let name =
                character_display_name(characters, placement.character.character_id, &fallback);
            ui.label(tf("Target character: {}", &[&name]));
            ui.label(t(
                "Only positions compatible with the character template and module shape are shown.",
            ));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for position in &valid_positions {
                    if ui
                        .button(format!("{},{}", position.row, position.column))
                        .on_hover_text(tf(
                            "Row {}, Column {}",
                            &[&position.row.to_string(), &position.column.to_string()],
                        ))
                        .clicked()
                    {
                        selected = Some((position.row, position.column));
                    }
                }
            });
        });
    if let Some((row, column)) = selected {
        *pending = None;
        return Some((placement, row, column));
    }
    if !open {
        *pending = None;
    }
    None
}

fn draw_equipment_stat_row(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    stat: &EquipmentStat,
    main_stat: bool,
    catalog: &EquipmentCatalog,
    dark_mode: bool,
) {
    let attribute = catalog.attributes.get(&stat.property);
    let label = attribute.map_or(stat.property.as_str(), |attribute| {
        attribute.name(i18n::current_language())
    });
    let value = format_equipment_stat_value(
        stat.value,
        attribute.is_some_and(|attribute| attribute.percent),
    );
    let value_color = shadcn_foreground(dark_mode);
    let value_font = fitted_font(
        ui,
        &value,
        egui::FontFamily::Monospace,
        13.0,
        10.0,
        rect.width() * 0.4,
        value_color,
    );
    let value_width = ui
        .painter()
        .layout_no_wrap(value.clone(), value_font.clone(), value_color)
        .size()
        .x;
    let label_indent = if main_stat { 6.0 } else { 0.0 };
    let label_width = (rect.width() - value_width - 10.0 - label_indent).max(1.0);
    let label_color = if main_stat {
        shadcn_foreground(dark_mode)
    } else {
        ui.visuals().weak_text_color()
    };
    let label_font = fitted_font(
        ui,
        label,
        egui::FontFamily::Proportional,
        if main_stat { 13.0 } else { 12.5 },
        9.0,
        label_width,
        label_color,
    );
    if main_stat {
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.center().y - 6.0),
                egui::pos2(rect.left() + 2.0, rect.center().y + 6.0),
            ),
            1.0,
            ui.visuals().selection.bg_fill,
        );
    }
    ui.painter().text(
        rect.left_center() + egui::vec2(label_indent, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        label_font,
        label_color,
    );
    ui.painter().text(
        rect.right_center(),
        egui::Align2::RIGHT_CENTER,
        value,
        value_font,
        value_color,
    );
}

fn draw_equipment_icon(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
    fallback_name: &str,
    dark_mode: bool,
) {
    ui.painter().rect_filled(rect, 8.0, shadcn_muted(dark_mode));
    if let Some(texture) = texture {
        egui::Image::new((texture.id(), rect.size()))
            .corner_radius(8)
            .paint_at(ui, rect);
    } else {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            fallback_name.chars().next().unwrap_or('?'),
            egui::FontId::proportional(18.0),
            ui.visuals().weak_text_color(),
        );
    }
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0_f32, shadcn_border(dark_mode)),
        egui::StrokeKind::Inside,
    );
}

fn paint_equipment_status(
    ui: &egui::Ui,
    card_rect: egui::Rect,
    equipped: bool,
    locked: bool,
    discarded: bool,
    dark_mode: bool,
) {
    let mut right = card_rect.right() - 8.0;
    if locked {
        let lock_rect = egui::Rect::from_min_size(
            egui::pos2(right - 15.0, card_rect.top() + 7.0),
            egui::vec2(15.0, 15.0),
        );
        paint_lock_icon(ui.painter(), lock_rect, shadcn_foreground(dark_mode));
        right = lock_rect.left() - 4.0;
    }
    for text in [
        equipped.then(|| t("Equipped")),
        discarded.then(|| t("Discarded")),
    ]
    .into_iter()
    .flatten()
    {
        let font = egui::FontId::proportional(10.0);
        let text_width = ui
            .painter()
            .layout_no_wrap(text.clone(), font.clone(), shadcn_foreground(dark_mode))
            .size()
            .x;
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(right - text_width - 10.0, card_rect.top() + 6.0),
            egui::vec2(text_width + 10.0, 17.0),
        );
        ui.painter()
            .rect_filled(badge_rect, 5.0, shadcn_muted(dark_mode));
        ui.painter().rect_stroke(
            badge_rect,
            5.0,
            Stroke::new(1.0_f32, shadcn_border(dark_mode)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            font,
            shadcn_foreground(dark_mode),
        );
        right = badge_rect.left() - 4.0;
    }
}

fn paint_equipped_character_avatar(
    ui: &mut egui::Ui,
    card_rect: egui::Rect,
    character_id: u32,
    characters: &HashMap<u32, CharacterInfo>,
    avatar_textures: &HashMap<String, egui::TextureHandle>,
    dark_mode: bool,
) -> egui::Rect {
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            card_rect.right() - EQUIPPED_CHARACTER_AVATAR_SIZE - 8.0,
            card_rect.top() + 28.0,
        ),
        egui::vec2(
            EQUIPPED_CHARACTER_AVATAR_SIZE,
            EQUIPPED_CHARACTER_AVATAR_SIZE,
        ),
    );
    let texture = characters
        .get(&character_id)
        .and_then(|character| character.avatar.as_deref())
        .and_then(|avatar| avatar_textures.get(avatar));
    if let Some(texture) = texture {
        egui::Image::new((texture.id(), rect.size()))
            .corner_radius(6.0)
            .paint_at(ui, rect);
    } else {
        let color = character_color(character_id, characters, 0, dark_mode);
        ui.painter()
            .rect_filled(rect, 6.0, color.gamma_multiply(0.85));
        let fallback = character_id.to_string();
        let initial = character_display_name(characters, character_id, &fallback)
            .chars()
            .next()
            .unwrap_or('?');
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            egui::FontId::proportional(14.0),
            contrast_text(color),
        );
    }
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0_f32, shadcn_border(dark_mode)),
        egui::StrokeKind::Inside,
    );
    let fallback = character_id.to_string();
    ui.allocate_rect(rect, egui::Sense::hover())
        .on_hover_text(character_display_name(characters, character_id, &fallback));
    rect
}

fn paint_lock_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32) {
    let body = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 2.0, rect.top() + 7.0),
        egui::pos2(rect.right() - 2.0, rect.bottom() - 1.0),
    );
    painter.rect_filled(body, 2.0, color.gamma_multiply(0.18));
    painter.rect_stroke(
        body,
        2.0,
        Stroke::new(1.2_f32, color),
        egui::StrokeKind::Inside,
    );
    let shackle_left = rect.left() + 4.0;
    let shackle_right = rect.right() - 4.0;
    let shackle_top = rect.top() + 2.0;
    let shackle_bottom = body.top() + 1.0;
    painter.line_segment(
        [
            egui::pos2(shackle_left, shackle_bottom),
            egui::pos2(shackle_left, shackle_top + 2.0),
        ],
        Stroke::new(1.2_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(shackle_left, shackle_top + 2.0),
            egui::pos2(shackle_left + 2.0, shackle_top),
        ],
        Stroke::new(1.2_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(shackle_left + 2.0, shackle_top),
            egui::pos2(shackle_right - 2.0, shackle_top),
        ],
        Stroke::new(1.2_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(shackle_right - 2.0, shackle_top),
            egui::pos2(shackle_right, shackle_top + 2.0),
        ],
        Stroke::new(1.2_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(shackle_right, shackle_top + 2.0),
            egui::pos2(shackle_right, shackle_bottom),
        ],
        Stroke::new(1.2_f32, color),
    );
}

fn fitted_font(
    ui: &egui::Ui,
    text: &str,
    family: egui::FontFamily,
    preferred_size: f32,
    minimum_size: f32,
    max_width: f32,
    color: Color32,
) -> egui::FontId {
    let preferred = egui::FontId::new(preferred_size, family.clone());
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), preferred.clone(), color)
        .size()
        .x;
    if width <= max_width || width <= f32::EPSILON {
        return preferred;
    }
    egui::FontId::new(
        (preferred_size * max_width / width).max(minimum_size),
        family,
    )
}

fn format_equipment_stat_value(value: f32, percent: bool) -> String {
    let scaled = f64::from(value) * if percent { 100.0 } else { 1.0 };
    let mut text = format!("{scaled:.2}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if percent {
        format!("+{text}%")
    } else {
        format!("+{text}")
    }
}

fn equipment_filter_key(
    item: &EmptyCurtainItem,
    catalog: &EquipmentCatalog,
) -> Option<EquipmentFilterKey> {
    let definition = catalog.items.get(&item.item_id)?;
    Some(match definition.kind {
        EquipmentKind::Module => EquipmentFilterKey::Module(definition.geometry.clone()),
        EquipmentKind::Core => EquipmentFilterKey::Core(
            definition
                .suit
                .clone()
                .expect("validated cassette definition must reference a suit"),
        ),
    })
}

fn empty_curtain_filter_matches(
    selected_equipment: &HashSet<EquipmentFilterKey>,
    selected_qualities: &HashSet<String>,
    selected_substats: &HashSet<String>,
    selected_characters: &HashSet<u32>,
    candidate: EquipmentFilterCandidate<'_>,
) -> bool {
    (selected_equipment.is_empty()
        || candidate
            .equipment
            .is_some_and(|key| selected_equipment.contains(key)))
        && (selected_qualities.is_empty()
            || candidate
                .quality
                .is_some_and(|quality| selected_qualities.contains(quality)))
        && (selected_characters.is_empty()
            || candidate
                .character_id
                .is_some_and(|character_id| selected_characters.contains(&character_id)))
        && selected_substats.iter().all(|property| {
            candidate
                .sub_stats
                .iter()
                .any(|stat| stat.property.as_str() == property)
        })
}

fn equipment_filter_options(catalog: &EquipmentCatalog) -> Vec<EquipmentFilterOption> {
    let mut definitions = catalog.items.iter().collect::<Vec<_>>();
    definitions.sort_by(|left, right| left.0.cmp(right.0));
    let mut options = BTreeMap::new();
    for (_, definition) in definitions {
        let (rank, order, group_id, key, names) = match definition.kind {
            EquipmentKind::Module => {
                let grid = definition
                    .grid
                    .expect("validated module definition must declare a grid size");
                (
                    0_u8,
                    grid,
                    definition.geometry.clone(),
                    EquipmentFilterKey::Module(definition.geometry.clone()),
                    (
                        definition.name_zh.clone(),
                        definition.name_en.clone(),
                        definition.name_ja.clone(),
                    ),
                )
            }
            EquipmentKind::Core => {
                let suit_id = definition
                    .suit
                    .as_deref()
                    .expect("validated cassette definition must reference a suit");
                let suit = catalog
                    .suits
                    .get(suit_id)
                    .expect("validated cassette suit must exist in the catalog");
                (
                    1_u8,
                    trailing_number(suit_id),
                    suit_id.to_owned(),
                    EquipmentFilterKey::Core(suit_id.to_owned()),
                    (
                        suit.name_zh.clone(),
                        suit.name_en.clone(),
                        suit.name_ja.clone(),
                    ),
                )
            }
        };
        options
            .entry((rank, order, group_id))
            .or_insert_with(|| EquipmentFilterOption {
                key,
                name_zh: names.0,
                name_en: names.1,
                name_ja: names.2,
                icon: definition.icon.clone(),
            });
    }
    options.into_values().collect()
}

fn trailing_number(value: &str) -> u32 {
    let digits = value
        .chars()
        .rev()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    digits
        .parse()
        .expect("validated equipment group id must end in a number")
}

fn parsed_character_filter_ids(
    items: &[EmptyCurtainItem],
    characters: &HashMap<u32, CharacterInfo>,
) -> Vec<u32> {
    let mut character_ids = items
        .iter()
        .filter_map(|item| item.equipped_character_id)
        .filter(|character_id| characters.contains_key(character_id))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    character_ids.sort_by(|left, right| {
        character_display_name(characters, *left, &left.to_string())
            .cmp(&character_display_name(
                characters,
                *right,
                &right.to_string(),
            ))
            .then_with(|| left.cmp(right))
    });
    character_ids
}

fn show_empty_curtain_filter_window(
    ctx: &egui::Context,
    items: &[EmptyCurtainItem],
    visuals: &EmptyCurtainVisuals<'_>,
    state: &mut KongmuUiState,
) {
    let catalog = visuals.catalog;
    let textures = visuals.equipment_textures;
    let dark_mode = visuals.dark_mode;
    let options = equipment_filter_options(catalog);
    let character_ids = parsed_character_filter_ids(items, visuals.characters);
    let mut substat_properties = items
        .iter()
        .flat_map(|item| item.sub_stats.iter().map(|stat| stat.property.clone()))
        .chain(state.selected_substats.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let language = i18n::current_language();
    substat_properties.sort_by(|left, right| {
        catalog.attributes[left]
            .name(language)
            .cmp(catalog.attributes[right].name(language))
            .then_with(|| left.cmp(right))
    });

    let mut open = state.filter_open;
    let mut done = false;
    egui::Window::new(t("Filter Console Equipment"))
        .id(egui::Id::new("empty_curtain_filter_window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .show(ctx, |ui| {
            ui.set_min_width(600.0);
            egui::ScrollArea::vertical()
                .id_salt("empty_curtain_filter_options")
                .max_height((ctx.content_rect().height() - 180.0).max(260.0))
                .show(ui, |ui| {
                    if !character_ids.is_empty() {
                        ui.label(
                            RichText::new(t("By Character"))
                                .size(14.0)
                                .strong()
                                .color(shadcn_foreground(dark_mode)),
                        );
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            for character_id in character_ids.iter().copied() {
                                let selected = state.selected_characters.contains(&character_id);
                                if draw_character_filter_option(
                                    ui,
                                    character_id,
                                    visuals.characters,
                                    visuals.avatar_textures,
                                    selected,
                                    dark_mode,
                                )
                                .clicked()
                                {
                                    state.toggle_character(character_id);
                                }
                            }
                        });
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(6.0);
                    }
                    ui.label(
                        RichText::new(t("Drive Modules"))
                            .size(14.0)
                            .strong()
                            .color(shadcn_foreground(dark_mode)),
                    );
                    ui.add_space(4.0);
                    draw_filter_option_group(
                        ui,
                        options
                            .iter()
                            .filter(|option| matches!(&option.key, EquipmentFilterKey::Module(_))),
                        textures,
                        state,
                        dark_mode,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(t("Cassettes"))
                            .size(14.0)
                            .strong()
                            .color(shadcn_foreground(dark_mode)),
                    );
                    ui.add_space(4.0);
                    draw_filter_option_group(
                        ui,
                        options
                            .iter()
                            .filter(|option| matches!(&option.key, EquipmentFilterKey::Core(_))),
                        textures,
                        state,
                        dark_mode,
                    );
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(t("Quality"))
                            .size(14.0)
                            .strong()
                            .color(shadcn_foreground(dark_mode)),
                    );
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for quality in EQUIPMENT_QUALITIES {
                            let selected = state.selected_qualities.contains(quality);
                            let label = t(quality_label_key(quality));
                            if draw_quality_chip(ui, quality, &label, selected, dark_mode).clicked()
                            {
                                state.toggle_quality(quality.to_owned());
                            }
                        }
                    });
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(t("Substats (all selected properties must be present)"))
                            .size(14.0)
                            .strong()
                            .color(shadcn_foreground(dark_mode)),
                    );
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for property in substat_properties.iter().cloned() {
                            let selected = state.selected_substats.contains(&property);
                            let name = catalog.attributes[&property].name(language);
                            if ui
                                .add(
                                    egui::Button::selectable(selected, name)
                                        .frame_when_inactive(true),
                                )
                                .clicked()
                            {
                                state.toggle_substat(property);
                            }
                        }
                    });
                });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(t("Clear Filters")).clicked() {
                    state.clear_filters();
                }
                if ui
                    .add(primary_button(t("Done"), ui.visuals().selection.bg_fill))
                    .clicked()
                {
                    done = true;
                }
            });
        });
    if done {
        open = false;
    }
    state.filter_open = open;
}

fn draw_filter_option_group<'a>(
    ui: &mut egui::Ui,
    options: impl Iterator<Item = &'a EquipmentFilterOption>,
    textures: &HashMap<String, egui::TextureHandle>,
    state: &mut KongmuUiState,
    dark_mode: bool,
) {
    ui.horizontal_wrapped(|ui| {
        for option in options {
            let selected = state.selected_equipment.contains(&option.key);
            if draw_filter_option(ui, option, textures, selected, dark_mode).clicked() {
                state.toggle_equipment(option.key.clone());
            }
        }
    });
}

fn draw_filter_option(
    ui: &mut egui::Ui,
    option: &EquipmentFilterOption,
    textures: &HashMap<String, egui::TextureHandle>,
    selected: bool,
    dark_mode: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(FILTER_TILE_WIDTH, FILTER_TILE_HEIGHT),
        egui::Sense::click(),
    );
    let fill = if selected {
        shadcn_muted(dark_mode)
    } else if response.hovered() {
        shadcn_card_hover(dark_mode)
    } else {
        shadcn_card(dark_mode)
    };
    ui.painter().rect(
        rect,
        7.0,
        fill,
        Stroke::new(
            if selected { 1.5_f32 } else { 1.0_f32 },
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                shadcn_border(dark_mode)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let name = match i18n::current_language() {
        Language::English => &option.name_en,
        Language::Japanese => &option.name_ja,
        Language::SimplifiedChinese => &option.name_zh,
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 8.0 + FILTER_ICON_SIZE * 0.5),
        egui::vec2(FILTER_ICON_SIZE, FILTER_ICON_SIZE),
    );
    draw_equipment_icon(ui, icon_rect, textures.get(&option.icon), name, dark_mode);
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 5.0, icon_rect.bottom() + 5.0),
        egui::pos2(rect.right() - 5.0, rect.bottom() - 5.0),
    );
    let mut label_ui = ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
    ));
    label_ui.add_sized(
        label_rect.size(),
        egui::Label::new(
            RichText::new(name)
                .size(10.0)
                .color(shadcn_foreground(dark_mode)),
        )
        .wrap()
        .halign(egui::Align::Center),
    );
    response
}

fn draw_character_filter_option(
    ui: &mut egui::Ui,
    character_id: u32,
    characters: &HashMap<u32, CharacterInfo>,
    avatar_textures: &HashMap<String, egui::TextureHandle>,
    selected: bool,
    dark_mode: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(FILTER_TILE_WIDTH, FILTER_TILE_HEIGHT),
        egui::Sense::click(),
    );
    let fill = if selected {
        shadcn_muted(dark_mode)
    } else if response.hovered() {
        shadcn_card_hover(dark_mode)
    } else {
        shadcn_card(dark_mode)
    };
    ui.painter().rect(
        rect,
        7.0,
        fill,
        Stroke::new(
            if selected { 1.5_f32 } else { 1.0_f32 },
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                shadcn_border(dark_mode)
            },
        ),
        egui::StrokeKind::Inside,
    );

    let fallback = character_id.to_string();
    let name = character_display_name(characters, character_id, &fallback);
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + 8.0 + FILTER_ICON_SIZE * 0.5),
        egui::vec2(FILTER_ICON_SIZE, FILTER_ICON_SIZE),
    );
    let texture = characters
        .get(&character_id)
        .and_then(|character| character.avatar.as_deref())
        .and_then(|avatar| avatar_textures.get(avatar));
    if let Some(texture) = texture {
        egui::Image::new((texture.id(), icon_rect.size()))
            .corner_radius(8.0)
            .paint_at(ui, icon_rect);
    } else {
        let color = character_color(character_id, characters, 0, dark_mode);
        ui.painter()
            .rect_filled(icon_rect, 8.0, color.gamma_multiply(0.85));
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            name.chars().next().unwrap_or('?'),
            egui::FontId::proportional(18.0),
            contrast_text(color),
        );
    }
    ui.painter().rect_stroke(
        icon_rect,
        8.0,
        Stroke::new(1.0_f32, shadcn_border(dark_mode)),
        egui::StrokeKind::Inside,
    );

    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 5.0, icon_rect.bottom() + 5.0),
        egui::pos2(rect.right() - 5.0, rect.bottom() - 5.0),
    );
    let mut label_ui = ui.new_child(egui::UiBuilder::new().max_rect(label_rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
    ));
    label_ui.add_sized(
        label_rect.size(),
        egui::Label::new(
            RichText::new(name)
                .size(10.0)
                .color(shadcn_foreground(dark_mode)),
        )
        .wrap()
        .halign(egui::Align::Center),
    );
    response
}

fn quality_label_key(quality: &str) -> &'static str {
    match quality {
        "blue" => "Blue",
        "purple" => "Purple",
        "orange" => "Orange",
        _ => "Unknown",
    }
}

fn quality_swatch_color(quality: &str, dark_mode: bool) -> Color32 {
    let theme = theme_tokens(dark_mode, AccentColor::Zinc);
    match quality {
        "blue" => theme.dataviz[0],
        "purple" => theme.dataviz[1],
        "orange" => theme.dataviz[3],
        _ => theme.fg_faint,
    }
}

fn draw_quality_chip(
    ui: &mut egui::Ui,
    quality: &str,
    label: &str,
    selected: bool,
    dark_mode: bool,
) -> egui::Response {
    const DOT: f32 = 11.0;
    const HEIGHT: f32 = 30.0;
    let font = egui::FontId::proportional(12.5);
    let text_color = shadcn_foreground(dark_mode);
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), text_color)
        .size()
        .x;
    let width = 10.0 + DOT + 7.0 + text_width + 10.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::click());
    let fill = if selected {
        shadcn_muted(dark_mode)
    } else if response.hovered() {
        shadcn_card_hover(dark_mode)
    } else {
        shadcn_card(dark_mode)
    };
    ui.painter().rect(
        rect,
        7.0,
        fill,
        Stroke::new(
            if selected { 1.5_f32 } else { 1.0_f32 },
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                shadcn_border(dark_mode)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let dot_center = egui::pos2(rect.left() + 10.0 + DOT * 0.5, rect.center().y);
    ui.painter().circle_filled(
        dot_center,
        DOT * 0.5,
        quality_swatch_color(quality, dark_mode),
    );
    ui.painter().circle_stroke(
        dot_center,
        DOT * 0.5,
        Stroke::new(1.0_f32, shadcn_border(dark_mode)),
    );
    ui.painter().text(
        egui::pos2(dot_center.x + DOT * 0.5 + 7.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    response
}

/// Build the list of drive/tape records for the Drive Calculator's
/// `real_inventory.json`. Modules become `drive` pieces (area = grid), cassettes
/// become `tape` pieces (fixed 15-area). Percent stats are scaled to the display
/// magnitude that project stores (e.g. 0.30 -> 30.0), and stat/set/shape names are
/// translated to the exact keys it expects.
fn build_drive_calculator_inventory(
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Vec<serde_json::Value> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let Some(definition) = catalog.items.get(&item.item_id) else {
            continue;
        };
        let uid = format!("{}_{}", item.id.solt, item.id.serial);
        let quality = calculator_quality(&definition.quality);
        let sub_stats = calculator_stat_map(&item.sub_stats, catalog);
        let record = match definition.kind {
            EquipmentKind::Module => {
                let area = definition.grid.unwrap_or(1);
                serde_json::json!({
                    "uid": uid,
                    "item_type": "drive",
                    "quality": quality,
                    "area": area,
                    "shape_id": module_shape_id(&definition.geometry, area),
                    "set_name": "未知套装",
                    "main_stats": calculator_stat_map(&item.main_stats, catalog),
                    "sub_stats": sub_stats,
                })
            }
            EquipmentKind::Core => {
                let set_name = definition
                    .suit
                    .as_deref()
                    .and_then(|suit| catalog.suits.get(suit))
                    .map(|suit| calculator_set_name(&suit.name_zh))
                    .unwrap_or_else(|| "未知套装".to_owned());
                let main_stat = item
                    .main_stats
                    .first()
                    .and_then(|stat| calculator_stat_key(&stat.property, catalog))
                    .unwrap_or_else(|| "未知主词条".to_owned());
                serde_json::json!({
                    "uid": uid,
                    "item_type": "tape",
                    "quality": quality,
                    "area": 15,
                    "shape_id": "TAPE_15",
                    "set_name": set_name,
                    "main_stats": main_stat,
                    "sub_stats": sub_stats,
                })
            }
        };
        records.push(record);
    }
    records
}

fn calculator_quality(quality: &str) -> &'static str {
    match quality {
        "blue" => "Blue",
        "purple" => "Purple",
        // The calculator labels the top rarity "Gold" where this app uses "orange".
        _ => "Gold",
    }
}

/// Map this app's geometry id to the Drive Calculator's fixed shape id. Bars map
/// exactly; the four right-angle (`ZhiJiao`) and two Z (`Z3`/`Z4`) variants map to
/// its L/Trap orientations 1:1. An unknown id falls back to the horizontal bar of
/// the matching area so the piece still occupies the right number of cells.
fn module_shape_id(geometry: &str, area: u32) -> String {
    let mapped = match geometry {
        "Hen2" => "H_2",
        "Hen3" => "H_3",
        "Hen4" => "H_4",
        "Shu2" => "V_2",
        "Shu3" => "V_3",
        "Shu4" => "V_4",
        "Z3" => "Trap_4_H",
        "Z4" => "Trap_4_V",
        "ZhiJiao1" => "L_3_TL",
        "ZhiJiao2" => "L_3_TR",
        "ZhiJiao3" => "L_3_BL",
        "ZhiJiao4" => "L_3_BR",
        _ => match area {
            4 => "H_4",
            3 => "H_3",
            _ => "H_2",
        },
    };
    mapped.to_owned()
}

/// Strip the decorative 「」 brackets to match the calculator's set-name keys, and
/// correct the one set this app spells differently.
fn calculator_set_name(name_zh: &str) -> String {
    let stripped = name_zh.trim_start_matches('「').trim_end_matches('」');
    match stripped {
        "缇娅的夜间酒馆" => "缇娜的夜间酒馆".to_owned(),
        other => other.to_owned(),
    }
}

fn calculator_stat_map(
    stats: &[EquipmentStat],
    catalog: &EquipmentCatalog,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for stat in stats {
        let Some(key) = calculator_stat_key(&stat.property, catalog) else {
            continue;
        };
        let percent = catalog
            .attributes
            .get(&stat.property)
            .is_some_and(|attribute| attribute.percent);
        map.insert(
            key,
            serde_json::json!(calculator_stat_value(stat.value, percent)),
        );
    }
    map
}

/// Scale to the calculator's stored magnitude (percent stats as whole numbers,
/// e.g. 0.125 -> 12.5) and round off f32 noise to two decimals.
fn calculator_stat_value(value: f32, percent: bool) -> f64 {
    let scaled = f64::from(value) * if percent { 100.0 } else { 1.0 };
    (scaled * 100.0).round() / 100.0
}

/// Translate this app's internal attribute id to the exact Chinese stat key the
/// Drive Calculator scores on. An unmapped attribute falls back to its localized
/// name so nothing is silently dropped.
fn calculator_stat_key(property: &str, catalog: &EquipmentCatalog) -> Option<String> {
    let mapped = match property {
        "AtkBase" | "AtkAdd" => "攻击力",
        "AtkUp" => "攻击力%",
        "HPMaxBase" | "HPMaxAdd" => "生命值",
        "HPMaxUp" => "生命值%",
        "DefBase" | "DefAdd" => "防御力",
        "DefUp" => "防御力%",
        "CritBase" | "CritAdd" => "暴击率%",
        "CritDamageBase" | "CritDamageAdd" => "暴击伤害%",
        "DamageUpGeneralBase" | "DamageUpGeneralAdd" => "伤害增加%",
        "Mag" | "MagBase" | "MagAdd" | "MagUp" => "环合强度",
        "UnbalIntensity" | "UnbalIntensityBase" | "UnbalIntensityAdd" | "UnbalIntensityUp" => {
            "倾陷强度"
        }
        "HealUp" => "治疗加成",
        "DamageUpCosmosBase" => "光属性异能伤害增强%",
        "DamageUpNatureBase" => "灵属性异能伤害增强%",
        "DamageUpIncantationBase" => "咒属性异能伤害增强%",
        "DamageUpChaosBase" => "暗属性异能伤害增强%",
        "DamageUpPsycheBase" => "魂属性异能伤害增强%",
        "DamageUpLakshanaBase" => "相属性异能伤害增强%",
        "DamageUpPsychicallyBase" => "心灵伤害增强%",
        // Drive main-stat reaction bonuses the calculator keeps but doesn't score.
        "ReactionGeneralDamageUp" => "环合伤害增强",
        "ReactionGuangLingDamageUp" => "创生伤害增强",
        "ReactionZhouAnDamageUp" => "浊燃伤害增强",
        "ReactionAnHunDamageUp" => "黯星伤害增强",
        _ => {
            return catalog
                .attributes
                .get(property)
                .map(|attribute| attribute.name(Language::SimplifiedChinese).to_owned());
        }
    };
    Some(mapped.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::snapshot::CharacterLoadoutPlacement;

    #[test]
    fn session_reset_invalidates_pending_equipment_work() {
        let character = EmptyCurtainCharacter {
            net_id: HtItemNetId { solt: 1, serial: 2 },
            character_id: 1020,
        };
        let equipment = HtItemNetId { solt: 3, serial: 4 };
        let mut state = KongmuUiState {
            pending_module_placement: Some(PendingModulePlacement {
                character,
                equipment,
                item_id: "module".to_owned(),
                move_from_other_character: false,
            }),
            plugin_request: Some(PendingPluginRequest { request_id: 7 }),
            ..KongmuUiState::default()
        };
        state.filter_cache.valid = true;

        state.reset_session_state();

        assert!(state.pending_module_placement.is_none());
        assert!(state.plugin_request.is_none());
        assert!(!state.filter_cache.valid);
    }

    #[test]
    fn equipment_plugin_risk_dialog_unlocks_only_after_five_seconds() {
        let opened_at = Instant::now();

        assert_eq!(
            plugin_risk_confirmation_remaining(opened_at, opened_at + Duration::from_secs(4)),
            Duration::from_secs(1)
        );
        assert_eq!(
            plugin_risk_confirmation_remaining(opened_at, opened_at + Duration::from_secs(5)),
            Duration::ZERO
        );
    }

    fn stat(property: &str) -> EquipmentStat {
        EquipmentStat {
            property: property.to_owned(),
            value: 1.0,
        }
    }

    fn filter_candidate<'a>(
        equipment: Option<&'a EquipmentFilterKey>,
        quality: Option<&'a str>,
        character_id: Option<u32>,
        sub_stats: &'a [EquipmentStat],
    ) -> EquipmentFilterCandidate<'a> {
        EquipmentFilterCandidate {
            equipment,
            quality,
            character_id,
            sub_stats,
        }
    }

    fn stat_v(property: &str, value: f32) -> EquipmentStat {
        EquipmentStat {
            property: property.to_owned(),
            value,
        }
    }

    #[test]
    fn one_key_plan_uses_the_character_template_and_native_batch_operation() {
        let catalog = crate::engine::parser::load_equipment_catalog(std::path::Path::new(
            crate::engine::parser::EQUIPMENT_CATALOG_PATH,
        ))
        .expect("bundled equipment catalog should load");
        let character = EmptyCurtainCharacter {
            net_id: HtItemNetId {
                solt: 100,
                serial: 101,
            },
            character_id: 1076,
        };
        let plan = &catalog.plans[&character.character_id];
        let mut items = plan
            .recommended_modules
            .iter()
            .enumerate()
            .map(|(index, module)| item(index as u32 + 1, &module.item_id, Vec::new(), Vec::new()))
            .collect::<Vec<_>>();
        items.push(item(1000, &plan.recommended_core, Vec::new(), Vec::new()));

        let operation = build_one_key_plan(character, &items, &catalog)
            .expect("complete inventory must produce a one-key plan");
        let EquipmentPluginOperation::EquipOneKey { placements, core } = operation else {
            panic!("one-key planning must use the native batch operation")
        };
        assert_eq!(placements.len(), plan.recommended_modules.len());
        assert_eq!(core, items.last().expect("test core").id);
    }

    #[test]
    fn imported_loadout_preserves_saved_placements_in_native_batch_operation() {
        let character = EmptyCurtainCharacter {
            net_id: HtItemNetId {
                solt: 100,
                serial: 101,
            },
            character_id: 1076,
        };
        let core = HtItemNetId {
            solt: 200,
            serial: 201,
        };
        let first = HtItemNetId { solt: 1, serial: 2 };
        let second = HtItemNetId { solt: 3, serial: 4 };
        let loadout = ValidatedCharacterLoadout {
            character,
            core,
            placements: vec![
                CharacterLoadoutPlacement {
                    equipment: first,
                    row: 1,
                    column: 2,
                },
                CharacterLoadoutPlacement {
                    equipment: second,
                    row: 4,
                    column: 5,
                },
            ],
        };

        let operation = character_loadout_plugin_operation(&loadout);

        assert_eq!(
            operation,
            EquipmentPluginOperation::EquipOneKey {
                placements: vec![
                    EquipmentPluginPlacement {
                        equipment: first,
                        row: 1,
                        column: 2,
                    },
                    EquipmentPluginPlacement {
                        equipment: second,
                        row: 4,
                        column: 5,
                    },
                ],
                core,
            }
        );
    }

    #[test]
    fn quality_maps_orange_to_gold() {
        assert_eq!(calculator_quality("blue"), "Blue");
        assert_eq!(calculator_quality("purple"), "Purple");
        assert_eq!(calculator_quality("orange"), "Gold");
    }

    #[test]
    fn module_shape_ids_map_by_geometry() {
        assert_eq!(module_shape_id("Hen4", 4), "H_4");
        assert_eq!(module_shape_id("Shu2", 2), "V_2");
        assert_eq!(module_shape_id("Z3", 4), "Trap_4_H");
        assert_eq!(module_shape_id("ZhiJiao1", 3), "L_3_TL");
        // Unknown geometry falls back to the horizontal bar of its area.
        assert_eq!(module_shape_id("Mystery", 3), "H_3");
    }

    #[test]
    fn set_name_strips_brackets_and_corrects_thea() {
        assert_eq!(calculator_set_name("「迪亚波罗斯」"), "迪亚波罗斯");
        assert_eq!(calculator_set_name("「缇娅的夜间酒馆」"), "缇娜的夜间酒馆");
    }

    #[test]
    fn stat_value_scales_percent_to_display_magnitude() {
        assert_eq!(calculator_stat_value(0.125, true), 12.5);
        assert_eq!(calculator_stat_value(0.30, true), 30.0);
        assert_eq!(calculator_stat_value(80.0, false), 80.0);
    }

    fn attr(name_zh: &str, percent: bool) -> crate::engine::parser::EquipmentAttributeDefinition {
        crate::engine::parser::EquipmentAttributeDefinition {
            name_zh: name_zh.to_owned(),
            name_en: String::new(),
            name_ja: String::new(),
            percent,
        }
    }

    fn item_def(
        kind: EquipmentKind,
        quality: &str,
        geometry: &str,
        grid: Option<u32>,
        suit: Option<&str>,
    ) -> crate::engine::parser::EquipmentItemDefinition {
        crate::engine::parser::EquipmentItemDefinition {
            kind,
            name_zh: String::new(),
            name_en: String::new(),
            name_ja: String::new(),
            quality: quality.to_owned(),
            geometry: geometry.to_owned(),
            grid,
            suit: suit.map(str::to_owned),
            icon: String::new(),
            max_level: 20,
            main_count: 2,
            sub_count: 4,
        }
    }

    fn item(
        id: u32,
        item_id: &str,
        main: Vec<EquipmentStat>,
        sub: Vec<EquipmentStat>,
    ) -> EmptyCurtainItem {
        EmptyCurtainItem {
            id: crate::engine::model::HtItemNetId {
                solt: id,
                serial: id + 1,
            },
            item_id: item_id.to_owned(),
            level: 20,
            main_stats: main,
            sub_stats: sub,
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        }
    }

    #[test]
    fn equipped_card_overlay_preserves_next_card_gap() {
        let mut equipped = item(1, "equipped", Vec::new(), Vec::new());
        equipped.locked = true;
        equipped.discarded = true;
        equipped.character_net_id = Some(crate::engine::model::HtItemNetId {
            solt: 10,
            serial: 11,
        });
        equipped.equipped_character_id = Some(1020);
        let following = item(2, "following", Vec::new(), Vec::new());
        let catalog = EquipmentCatalog::default();
        let equipment_textures = HashMap::new();
        let characters = HashMap::from([(
            1020,
            CharacterInfo {
                name_zh: "Haniel".to_owned(),
                name_en: "Haniel".to_owned(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);
        let avatar_textures = HashMap::new();
        let visuals = EmptyCurtainVisuals {
            catalog: &catalog,
            equipment_textures: &equipment_textures,
            characters: &characters,
            avatar_textures: &avatar_textures,
            dark_mode: true,
        };
        let mut card_rects = None;

        egui::__run_test_ui(|ui| {
            ui.spacing_mut().item_spacing.x = EQUIPMENT_CARD_GAP;
            ui.horizontal(|ui| {
                let mut selection = None;
                let equipped_response = draw_empty_curtain_card(
                    ui,
                    &equipped,
                    EQUIPMENT_CARD_MIN_WIDTH,
                    &[],
                    false,
                    &visuals,
                    &mut selection,
                );
                let following_response = draw_empty_curtain_card(
                    ui,
                    &following,
                    EQUIPMENT_CARD_MIN_WIDTH,
                    &[],
                    false,
                    &visuals,
                    &mut selection,
                );
                card_rects = Some((equipped_response.rect, following_response.rect));
            });
        });

        let (equipped_rect, following_rect) =
            card_rects.expect("test UI must render both equipment cards");
        assert_eq!(
            following_rect.left(),
            equipped_rect.right() + EQUIPMENT_CARD_GAP
        );
    }

    #[test]
    fn character_filter_options_only_include_parsed_known_characters() {
        let mut known = item(1, "known", Vec::new(), Vec::new());
        known.equipped_character_id = Some(1020);
        let mut unknown = item(2, "unknown", Vec::new(), Vec::new());
        unknown.equipped_character_id = Some(9999);
        let characters = HashMap::from([(
            1020,
            CharacterInfo {
                name_zh: "Haniel".to_owned(),
                name_en: "Haniel".to_owned(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);

        assert_eq!(
            parsed_character_filter_ids(&[known, unknown], &characters),
            [1020]
        );
    }

    #[test]
    fn inventory_export_maps_drive_and_tape_records() {
        let mut catalog = EquipmentCatalog::default();
        catalog
            .attributes
            .insert("AtkAdd".to_owned(), attr("攻击力", false));
        catalog
            .attributes
            .insert("HPMaxAdd".to_owned(), attr("生命值", false));
        catalog
            .attributes
            .insert("CritBase".to_owned(), attr("暴击率", true));
        catalog.attributes.insert(
            "DamageUpIncantationBase".to_owned(),
            attr("咒属性异能伤害增强", true),
        );
        catalog.items.insert(
            "mod1".to_owned(),
            item_def(EquipmentKind::Module, "orange", "ZhiJiao1", Some(3), None),
        );
        catalog.items.insert(
            "core1".to_owned(),
            item_def(EquipmentKind::Core, "purple", "Core", None, Some("Suit1")),
        );
        catalog.suits.insert(
            "Suit1".to_owned(),
            crate::engine::parser::EquipmentSuitDefinition {
                name_zh: "「迪亚波罗斯」".to_owned(),
                name_en: String::new(),
                name_ja: String::new(),
                effects: Vec::new(),
            },
        );

        let items = vec![
            item(
                10,
                "mod1",
                vec![stat_v("AtkAdd", 84.0), stat_v("HPMaxAdd", 1120.0)],
                vec![stat_v("CritBase", 0.1)],
            ),
            item(
                20,
                "core1",
                vec![stat_v("DamageUpIncantationBase", 0.075)],
                vec![stat_v("AtkAdd", 80.0)],
            ),
        ];
        let out = build_drive_calculator_inventory(&items, &catalog);
        assert_eq!(out.len(), 2);

        let drive = &out[0];
        assert_eq!(drive["item_type"], "drive");
        assert_eq!(drive["quality"], "Gold");
        assert_eq!(drive["area"].as_u64(), Some(3));
        assert_eq!(drive["shape_id"], "L_3_TL");
        assert_eq!(drive["set_name"], "未知套装");
        assert_eq!(drive["uid"], "10_11");
        assert_eq!(drive["main_stats"]["攻击力"].as_f64(), Some(84.0));
        assert_eq!(drive["main_stats"]["生命值"].as_f64(), Some(1120.0));
        assert_eq!(drive["sub_stats"]["暴击率%"].as_f64(), Some(10.0));

        let tape = &out[1];
        assert_eq!(tape["item_type"], "tape");
        assert_eq!(tape["quality"], "Purple");
        assert_eq!(tape["area"].as_u64(), Some(15));
        assert_eq!(tape["shape_id"], "TAPE_15");
        assert_eq!(tape["set_name"], "迪亚波罗斯");
        assert_eq!(tape["main_stats"], "咒属性异能伤害增强%");
        assert_eq!(tape["sub_stats"]["攻击力"].as_f64(), Some(80.0));
    }

    #[test]
    fn substat_filter_requires_every_selected_property() {
        let selected = HashSet::from(["CritRate".to_owned(), "CritDamage".to_owned()]);
        assert!(empty_curtain_filter_matches(
            &HashSet::new(),
            &HashSet::new(),
            &selected,
            &HashSet::new(),
            filter_candidate(
                None,
                None,
                None,
                &[stat("CritRate"), stat("CritDamage"), stat("Attack")],
            ),
        ));
        assert!(!empty_curtain_filter_matches(
            &HashSet::new(),
            &HashSet::new(),
            &selected,
            &HashSet::new(),
            filter_candidate(None, None, None, &[stat("CritRate"), stat("Attack")]),
        ));
    }

    #[test]
    fn equipment_options_are_or_and_cross_dimension_is_and() {
        let module = EquipmentFilterKey::Module("Hen2".to_owned());
        let selected_equipment =
            HashSet::from([module.clone(), EquipmentFilterKey::Core("Suit1".to_owned())]);
        let selected_substats = HashSet::from(["CritRate".to_owned()]);
        assert!(empty_curtain_filter_matches(
            &selected_equipment,
            &HashSet::new(),
            &selected_substats,
            &HashSet::new(),
            filter_candidate(Some(&module), None, None, &[stat("CritRate")]),
        ));
        assert!(!empty_curtain_filter_matches(
            &selected_equipment,
            &HashSet::new(),
            &selected_substats,
            &HashSet::new(),
            filter_candidate(Some(&module), None, None, &[stat("Attack")]),
        ));
        assert!(!empty_curtain_filter_matches(
            &selected_equipment,
            &HashSet::new(),
            &selected_substats,
            &HashSet::new(),
            filter_candidate(
                Some(&EquipmentFilterKey::Module("Shu2".to_owned())),
                None,
                None,
                &[stat("CritRate")],
            ),
        ));
    }

    #[test]
    fn quality_filter_is_or_within_and_across_dimensions() {
        let qualities = HashSet::from(["orange".to_owned(), "purple".to_owned()]);
        // OR within the quality dimension: either selected color matches.
        assert!(empty_curtain_filter_matches(
            &HashSet::new(),
            &qualities,
            &HashSet::new(),
            &HashSet::new(),
            filter_candidate(None, Some("orange"), None, &[]),
        ));
        // A color outside the selected set is filtered out.
        assert!(!empty_curtain_filter_matches(
            &HashSet::new(),
            &qualities,
            &HashSet::new(),
            &HashSet::new(),
            filter_candidate(None, Some("blue"), None, &[]),
        ));
        // AND across dimensions: color matches but a required substat is absent.
        assert!(!empty_curtain_filter_matches(
            &HashSet::new(),
            &qualities,
            &HashSet::from(["CritRate".to_owned()]),
            &HashSet::new(),
            filter_candidate(None, Some("orange"), None, &[stat("Attack")]),
        ));
    }

    #[test]
    fn character_filter_is_or_within_and_across_dimensions() {
        let characters = HashSet::from([1020, 1033]);
        assert!(empty_curtain_filter_matches(
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &characters,
            filter_candidate(None, None, Some(1020), &[]),
        ));
        assert!(!empty_curtain_filter_matches(
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &characters,
            filter_candidate(None, None, Some(1010), &[]),
        ));
        assert!(!empty_curtain_filter_matches(
            &HashSet::new(),
            &HashSet::from(["orange".to_owned()]),
            &HashSet::new(),
            &characters,
            filter_candidate(None, Some("blue"), Some(1033), &[]),
        ));
    }

    #[test]
    fn stat_value_format_scales_percent_and_trims_zeroes() {
        assert_eq!(format_equipment_stat_value(0.038, true), "+3.8%");
        assert_eq!(format_equipment_stat_value(840.0, false), "+840");
        assert_eq!(format_equipment_stat_value(24.5, false), "+24.5");
    }
}
