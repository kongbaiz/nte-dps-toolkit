use crate::equipment_operation_service::{EmptyCurtainOperationState, EquipmentOperationSnapshot};
use nte_dps_tool::core::snapshot::{InventorySnapshot, inventory_snapshot};

use super::*;

impl AppState {
    pub(crate) fn uses_plugin_equipment(&self) -> Result<bool, CoreError> {
        Ok(
            self.data_mode() == nte_dps_tool::storage::config::DataMode::Plugin
                && !self.main_processing_paused()
                && !matches!(
                    self.0.live_capture.quality_source()?,
                    CaptureQualitySource::PcapngReplay | CaptureQualitySource::JsonReplay
                ),
        )
    }

    pub(crate) fn equipment_service(&self) -> &EquipmentOperationService {
        &self.0.equipment_operation
    }

    pub(crate) fn equipment_rpc(&self) -> Arc<nte_dps_tool::core::equipment_rpc::Router> {
        self.0.live_capture.equipment_rpc()
    }

    fn plugin_inventory(
        &self,
    ) -> Result<
        (
            Option<Arc<nte_dps_tool::core::user_equipment::Inventory>>,
            u64,
        ),
        CoreError,
    > {
        self.0.equipment_operation.inventory.get().map_err(|_| {
            CoreError::new(
                CoreErrorCode::CaptureStateUnavailable,
                "Equipment inventory is unavailable",
            )
        })
    }

    pub(crate) fn empty_curtain_snapshot(&self) -> Result<InventorySnapshot, CoreError> {
        if self.uses_plugin_equipment()? {
            let (data, revision) = self.plugin_inventory()?;
            let resources = self.0.live_capture.resources();
            return Ok(match data {
                Some(data) => inventory_snapshot(
                    &data.items,
                    &data.characters,
                    &self.0.equipment_catalog,
                    &resources.characters,
                    revision,
                    data.observed_us / 1000,
                ),
                None => InventorySnapshot {
                    generation: revision,
                    observed_at_unix_ms: 0,
                    complete: false,
                    items: vec![],
                    characters: vec![],
                },
            });
        }

        let resources = self.0.live_capture.resources();
        let observed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.with_main_presented_state(|state| {
            inventory_snapshot(
                &state.empty_curtain,
                &state.empty_curtain_characters,
                &self.0.equipment_catalog,
                &resources.characters,
                state.empty_curtain_generation,
                observed_at_unix_ms,
            )
        })
    }

    pub(crate) fn empty_curtain_data_snapshot(
        &self,
    ) -> Result<EmptyCurtainDataSnapshot, CoreError> {
        if self.uses_plugin_equipment()? {
            let (data, _) = self.plugin_inventory()?;
            let data = data.ok_or_else(|| {
                CoreError::new(
                    CoreErrorCode::CaptureStateUnavailable,
                    "Refresh equipment inventory first",
                )
            })?;
            return Ok((
                data.items.clone(),
                data.characters.clone(),
                self.equipment_catalog(),
            ));
        }

        let (items, characters) = self.with_main_presented_state(|state| {
            (
                state.empty_curtain.clone(),
                state.empty_curtain_characters.clone(),
            )
        })?;
        Ok((items, characters, Arc::clone(&self.0.equipment_catalog)))
    }

    pub(crate) fn equipment_catalog(&self) -> Arc<EquipmentCatalog> {
        Arc::clone(&self.0.equipment_catalog)
    }

    pub(crate) fn empty_curtain_operation_snapshot(
        &self,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError> {
        self.0.equipment_operation.poll_snapshot()
    }

    pub(crate) fn empty_curtain_revision_and_operation(
        &self,
    ) -> Result<((u64, u64, u64), EmptyCurtainOperationState), EmptyCurtainRuntimeError> {
        let operation = self.empty_curtain_operation_snapshot()?;
        if self.uses_plugin_equipment()? {
            let (_, revision) = self.plugin_inventory()?;
            return Ok((
                (revision, revision, operation.revision),
                operation.operation,
            ));
        }
        let (inventory, characters) = if self.main_processing_paused() {
            self.with_main_presented_state(|state| {
                (
                    state.empty_curtain_generation,
                    state.empty_curtain_characters_generation,
                )
            })?
        } else {
            self.0.live_capture.inventory_revision()?
        };
        Ok((
            (inventory, characters, operation.revision),
            operation.operation,
        ))
    }
}
