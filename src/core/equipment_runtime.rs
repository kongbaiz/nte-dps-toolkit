//! Finite equipment transactions. A dispatch receipt is not confirmation: only
//! a fresh, identity-matched inventory readback confirms the requested change.
use super::{
    equipment_rpc::{Client, Router, request_id},
    user_equipment::{self, Inventory},
};
use crate::{
    engine::{model::HtItemNetId, parser::EquipmentCatalog},
    platform::{
        mods_plugin::ModsPluginOperation,
        toolkit::{ToolkitClient, ToolkitError},
    },
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Transport(ToolkitError),
    Snapshot(user_equipment::Error),
    Unconfirmed,
    ConfirmationPending(ReadbackFailure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadbackFailure {
    Transport(ToolkitError),
    Snapshot(user_equipment::Error),
}
fn confirmation_error(error: Error) -> Error {
    match error {
        Error::Transport(e) => Error::ConfirmationPending(ReadbackFailure::Transport(e)),
        Error::Snapshot(e) => Error::ConfirmationPending(ReadbackFailure::Snapshot(e)),
        other => other,
    }
}
struct Confirmation {
    identity: String,
    provider: String,
    domain: String,
    character: HtItemNetId,
    operation: ModsPluginOperation,
    deadline: std::time::Instant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmationState {
    Waiting,
    Confirmed,
    Expired,
    SourceChanged,
}
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub provider_id: String,
    pub process_id: u32,
    pub process_created_file_time: String,
    pub equipment_watch_supported: bool,
    pub sdk_compatible: bool,
    pub equipment_revision: String,
    pub equipment_full_revision: String,
    pub equipment_flags: Vec<FlagChange>,
}
#[derive(Clone, Debug, serde::Deserialize, PartialEq)]
pub struct FlagChange {
    pub solt: u32,
    pub serial: u32,
    pub revision: String,
}
impl Watch {
    pub fn validate(&self, identity: &str) -> Result<(), Error> {
        if !self.equipment_watch_supported
            || !self.sdk_compatible
            || self.provider_id.is_empty()
            || format!("{}:{}", self.process_id, self.process_created_file_time) != identity
            || self.provider_id.len() > 128
            || self.equipment_flags.len() > 256
        {
            return Err(ToolkitError::InvalidProtocol.into());
        }
        for n in std::iter::once(&self.equipment_revision)
            .chain(std::iter::once(&self.equipment_full_revision))
            .chain(self.equipment_flags.iter().map(|f| &f.revision))
        {
            if n.is_empty() || n.len() > 20 || n.parse::<u64>().is_err() {
                return Err(ToolkitError::InvalidProtocol.into());
            }
        }
        let revision = self
            .equipment_revision
            .parse::<u64>()
            .map_err(|_| ToolkitError::InvalidProtocol)?;
        if self
            .equipment_full_revision
            .parse::<u64>()
            .map_err(|_| ToolkitError::InvalidProtocol)?
            > revision
        {
            return Err(ToolkitError::InvalidProtocol.into());
        }
        let mut seen = std::collections::HashSet::new();
        for item in &self.equipment_flags {
            if (item.solt == 0 && item.serial == 0)
                || !seen.insert((item.solt, item.serial))
                || item
                    .revision
                    .parse::<u64>()
                    .map_err(|_| ToolkitError::InvalidProtocol)?
                    > revision
            {
                return Err(ToolkitError::InvalidProtocol.into());
            }
        }
        Ok(())
    }
}
pub fn watch(host: &ToolkitClient, stop: &std::sync::atomic::AtomicBool) -> Result<Watch, Error> {
    let value: Watch = host.json(300, 1, "", &|| {
        stop.load(std::sync::atomic::Ordering::Acquire)
    })?;
    value.validate(&host.identity())?;
    Ok(value)
}
fn inspect(client: &mut Client, before: &Inventory, item: HtItemNetId) -> Result<Value, Error> {
    let value = client.call("equipment.inspect", json!({"equipment":id(item)}))?;
    let key = value["domainKey"]
        .as_str()
        .ok_or(ToolkitError::InvalidProtocol)?;
    if value["providerId"] != before.provider
        || !key.starts_with(&(before.domain.clone() + "/user-"))
    {
        return Err(ToolkitError::SessionChanged.into());
    }
    if value["item"]["UniqueID"]["solt"] != item.solt
        || value["item"]["UniqueID"]["serial"] != item.serial
    {
        return Err(ToolkitError::InvalidProtocol.into());
    }
    Ok(value)
}
fn patch_flags(data: &mut Inventory, item: HtItemNetId, value: &Value) -> Result<(), Error> {
    let row = data
        .items
        .iter_mut()
        .find(|r| r.id == item)
        .ok_or(user_equipment::Error::Changed)?;
    let owner = row.character_net_id.unwrap_or(HtItemNetId::ZERO);
    if value["CharacterNetID"]["solt"] != owner.solt
        || value["CharacterNetID"]["serial"] != owner.serial
    {
        return Err(user_equipment::Error::Changed.into());
    }
    row.locked = value["IsLocked"]
        .as_bool()
        .ok_or(ToolkitError::InvalidProtocol)?;
    row.discarded = value["IsDiscarded"]
        .as_bool()
        .ok_or(ToolkitError::InvalidProtocol)?;
    Ok(())
}
pub fn update_flags(
    client: &mut Client,
    before: &Inventory,
    previous: &Watch,
    next: &Watch,
) -> Result<Inventory, Error> {
    let old = previous
        .equipment_revision
        .parse::<u64>()
        .map_err(|_| ToolkitError::InvalidProtocol)?;
    if next.provider_id != previous.provider_id
        || next
            .equipment_full_revision
            .parse::<u64>()
            .map_err(|_| ToolkitError::InvalidProtocol)?
            > old
    {
        return Err(user_equipment::Error::Changed.into());
    }
    let mut after = before.clone();
    for change in &next.equipment_flags {
        if change
            .revision
            .parse::<u64>()
            .map_err(|_| ToolkitError::InvalidProtocol)?
            > old
        {
            let item = HtItemNetId {
                solt: change.solt,
                serial: change.serial,
            };
            let value = inspect(client, before, item)?;
            patch_flags(&mut after, item, &value["item"])?;
        }
    }
    Ok(after)
}
impl From<ToolkitError> for Error {
    fn from(v: ToolkitError) -> Self {
        Self::Transport(v)
    }
}
impl From<user_equipment::Error> for Error {
    fn from(v: user_equipment::Error) -> Self {
        Self::Snapshot(v)
    }
}
#[derive(Default)]
struct State {
    inventory: Option<Arc<Inventory>>,
    identity: String,
    watch: Option<Watch>,
    revision: u64,
    confirmation: Option<Confirmation>,
}
#[derive(Default)]
pub struct Store {
    state: Mutex<State>,
}
impl Store {
    pub fn defer_confirmation(
        &self,
        identity: String,
        before: &Inventory,
        character: HtItemNetId,
        operation: ModsPluginOperation,
    ) -> Result<(), Error> {
        let mut s = self.state.lock().map_err(|_| ToolkitError::Failed)?;
        if s.confirmation.is_some() {
            return Err(ToolkitError::Busy.into());
        }
        s.confirmation = Some(Confirmation {
            identity,
            provider: before.provider.clone(),
            domain: before.domain.clone(),
            character,
            operation,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(10),
        });
        Ok(())
    }
    pub fn confirming(&self) -> Result<bool, Error> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ToolkitError::Failed)?
            .confirmation
            .is_some())
    }
    pub fn settle_confirmation(
        &self,
        identity: &str,
        data: Option<&Inventory>,
    ) -> Result<Option<ConfirmationState>, Error> {
        let mut s = self.state.lock().map_err(|_| ToolkitError::Failed)?;
        let Some(pending) = &s.confirmation else {
            return Ok(None);
        };
        let result = if identity != pending.identity
            || data.is_some_and(|d| d.provider != pending.provider || d.domain != pending.domain)
        {
            ConfirmationState::SourceChanged
        } else if data.is_some_and(|d| confirmed(d, pending.character, &pending.operation)) {
            ConfirmationState::Confirmed
        } else if std::time::Instant::now() >= pending.deadline {
            ConfirmationState::Expired
        } else {
            ConfirmationState::Waiting
        };
        if result != ConfirmationState::Waiting {
            s.confirmation = None;
        }
        Ok(Some(result))
    }
    pub fn watch(&self) -> Result<Option<Watch>, Error> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ToolkitError::Failed)?
            .watch
            .clone())
    }
    pub fn set_watch(&self, value: Watch) -> Result<(), Error> {
        self.state.lock().map_err(|_| ToolkitError::Failed)?.watch = Some(value);
        Ok(())
    }
    pub fn get(&self) -> Result<(Option<Arc<Inventory>>, u64), Error> {
        let s = self.state.lock().map_err(|_| ToolkitError::Failed)?;
        Ok((s.inventory.clone(), s.revision))
    }
    pub fn identity(&self) -> Result<String, Error> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ToolkitError::Failed)?
            .identity
            .clone())
    }
    pub fn publish(&self, identity: String, inventory: Inventory) -> Result<(), Error> {
        let inventory = Arc::new(inventory);
        let mut s = self.state.lock().map_err(|_| ToolkitError::Failed)?;
        if s.identity != identity || s.inventory.as_deref() != Some(inventory.as_ref()) {
            s.revision = s.revision.wrapping_add(1);
            s.identity = identity;
            s.inventory = Some(inventory);
        }
        Ok(())
    }
}
pub fn collect(client: &mut Client, catalog: &EquipmentCatalog) -> Result<Inventory, Error> {
    // Server inventory notifications can invalidate a read racing the just-
    // dispatched change. Retry only acquisition, never the equipment RPC.
    for attempt in 0..3 {
        match collect_once(client, catalog) {
            Err(
                Error::Transport(ToolkitError::SessionChanged | ToolkitError::Busy)
                | Error::Snapshot(user_equipment::Error::Changed),
            ) if attempt < 2 => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            result => return result,
        }
    }
    Err(user_equipment::Error::Changed.into())
}
fn collect_once(client: &mut Client, catalog: &EquipmentCatalog) -> Result<Inventory, Error> {
    let meta = client.call("snapshot.refresh", json!({"domain":"inventory"}))?;
    let identity = user_equipment::metadata(&meta)?;
    let mut records = Vec::with_capacity(identity.3);
    let mut bytes = 0usize;
    while records.len() < identity.3 {
        let page = client.call(
            "snapshot.page",
            json!({"domain":"inventory","snapshotId":identity.2,"offset":records.len(),"limit":64}),
        )?;
        if user_equipment::metadata(&page)? != identity
            || page["revision"] != meta["revision"]
            || page["characterRefs"] != meta["characterRefs"]
        {
            return Err(user_equipment::Error::Changed.into());
        }
        bytes = bytes.saturating_add(
            serde_json::to_vec(&page)
                .map_err(|_| ToolkitError::InvalidProtocol)?
                .len(),
        );
        if bytes > 32 * 1024 * 1024 {
            return Err(ToolkitError::TooLarge.into());
        }
        let rows = page["records"]
            .as_array()
            .filter(|r| !r.is_empty() && r.len() <= 64)
            .ok_or(ToolkitError::InvalidProtocol)?;
        let next = records.len() + rows.len();
        if next > identity.3
            || (next < identity.3 && page["nextOffset"].as_u64() != Some(next as u64))
            || (next == identity.3 && !page["nextOffset"].is_null())
        {
            return Err(ToolkitError::InvalidProtocol.into());
        }
        records.extend(rows.iter().cloned());
    }
    let after = client.call("snapshot.status", json!({}))?;
    let domains = after["domains"]
        .as_array()
        .filter(|d| d.len() == 4)
        .ok_or(ToolkitError::InvalidProtocol)?;
    let current = domains
        .iter()
        .find(|d| d["domain"] == "inventory")
        .ok_or(ToolkitError::InvalidProtocol)?;
    // snapshot.status intentionally strips characterRefs from its lightweight
    // metadata. Completeness was verified on every full page; fence only the
    // still-current source and revision here.
    if current["ready"] != true
        || current["dirty"] != false
        || current["snapshotId"] != meta["snapshotId"]
        || current["domainKey"] != meta["domainKey"]
        || current["providerId"] != meta["providerId"]
        || current["revision"] != meta["revision"]
    {
        return Err(user_equipment::Error::Changed.into());
    }
    user_equipment::project(&meta, &records, catalog).map_err(Into::into)
}
fn id(v: HtItemNetId) -> Value {
    json!({"solt":v.solt,"serial":v.serial})
}
pub fn command(character: HtItemNetId, op: &ModsPluginOperation) -> (&'static str, Value) {
    use ModsPluginOperation::*;
    let mut p = json!({"character":id(character)});
    let method = match op {
        SetItemLocked { equipment, locked } => {
            p = json!({"equipment":id(*equipment),"locked":locked});
            "equipment.set_item_locked"
        }
        SetItemDiscarded {
            equipment,
            discarded,
        } => {
            p = json!({"equipment":id(*equipment),"discarded":discarded});
            "equipment.set_item_discarded"
        }
        EquipModule {
            equipment,
            row,
            column,
        }
        | MoveModuleToCharacter {
            equipment,
            row,
            column,
        } => {
            p["equipment"] = id(*equipment);
            p["row"] = json!(row);
            p["column"] = json!(column);
            if matches!(op, EquipModule { .. }) {
                "equipment.equip_module"
            } else {
                "equipment.move_module_to_character"
            }
        }
        EquipCore { equipment } | MoveCoreToCharacter { equipment } => {
            p["equipment"] = id(*equipment);
            if matches!(op, EquipCore { .. }) {
                "equipment.equip_core"
            } else {
                "equipment.move_core_to_character"
            }
        }
        UnequipModule { equipment } | UnequipCore { equipment } => {
            p["equipment"] = id(*equipment);
            if matches!(op, UnequipModule { .. }) {
                "equipment.unequip_module"
            } else {
                "equipment.unequip_core"
            }
        }
        UnequipAll => "equipment.unequip_all",
        EquipOneKey { placements, core } => {
            p["core"] = id(*core);
            p["placements"] = json!(
                placements
                    .iter()
                    .map(|p| json!({"equipment":id(p.equipment),"row":p.row,"column":p.column}))
                    .collect::<Vec<_>>()
            );
            "equipment.equip_one_key"
        }
    };
    (method, p)
}
pub fn confirmed(data: &Inventory, character: HtItemNetId, op: &ModsPluginOperation) -> bool {
    use ModsPluginOperation::*;
    let item = |id| data.items.iter().find(|i| i.id == id);
    let equipped = |id| item(id).is_some_and(|i| i.character_net_id == Some(character));
    let placed = |id, row, column| {
        item(id).is_some_and(|i| {
            i.character_net_id == Some(character)
                && i.equipped_placement
                    .is_some_and(|p| p.row == row && p.column == column)
        })
    };
    match op {
        SetItemLocked { equipment, locked } => {
            item(*equipment).is_some_and(|i| i.locked == *locked)
        }
        SetItemDiscarded {
            equipment,
            discarded,
        } => item(*equipment).is_some_and(|i| i.discarded == *discarded),
        EquipModule {
            equipment,
            row,
            column,
        }
        | MoveModuleToCharacter {
            equipment,
            row,
            column,
        } => placed(*equipment, *row, *column),
        EquipCore { equipment } | MoveCoreToCharacter { equipment } => equipped(*equipment),
        UnequipCore { equipment } | UnequipModule { equipment } => {
            item(*equipment).is_some_and(|i| i.character_net_id.is_none())
        }
        UnequipAll => data
            .items
            .iter()
            .all(|i| i.character_net_id != Some(character)),
        EquipOneKey { placements, core } => {
            equipped(*core)
                && placements
                    .iter()
                    .all(|p| placed(p.equipment, p.row, p.column))
                && data
                    .items
                    .iter()
                    .filter(|i| i.character_net_id == Some(character))
                    .count()
                    == placements.len() + 1
        }
    }
}
fn confirm_readback(
    before: &Inventory,
    character: HtItemNetId,
    op: &ModsPluginOperation,
    readback: Result<Inventory, Error>,
) -> Result<Inventory, Error> {
    let after = readback.map_err(confirmation_error)?;
    if after.provider != before.provider || after.domain != before.domain {
        return Err(confirmation_error(ToolkitError::SessionChanged.into()));
    }
    if !confirmed(&after, character, op) {
        return Err(Error::Unconfirmed);
    }
    Ok(after)
}
pub fn execute(
    client: &mut Client,
    before: &Inventory,
    character: HtItemNetId,
    op: &ModsPluginOperation,
    catalog: &EquipmentCatalog,
) -> Result<Inventory, Error> {
    if let ModsPluginOperation::SetItemLocked { equipment, .. }
    | ModsPluginOperation::SetItemDiscarded { equipment, .. } = op
    {
        let observed = inspect(client, before, *equipment)?;
        let mut after = before.clone();
        patch_flags(&mut after, *equipment, &observed["item"])?;
        let request = request_id();
        let (method, mut params) = command(character, op);
        params["requestId"] = json!(request);
        params["domainKey"] = observed["domainKey"].clone();
        params["epoch"] = observed["epoch"].clone();
        let ack = client.call_with_id(&request, method, params)?;
        if ack["requestId"] != request
            || ack["domainKey"] != observed["domainKey"]
            || ack["providerId"] != before.provider
            || ack["confirmed"] != false
            || !matches!(
                ack["status"].as_str(),
                Some("rpc_dispatched" | "outcome_unknown")
            )
        {
            return Err(ToolkitError::InvalidProtocol.into());
        }
        // Read only this item while the server acknowledgement is pending.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let observed = inspect(client, before, *equipment).map_err(confirmation_error)?;
            patch_flags(&mut after, *equipment, &observed["item"])?;
            if confirmed(&after, character, op) {
                return Ok(after);
            }
            if std::time::Instant::now() >= deadline {
                return Err(Error::Unconfirmed);
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    // Freeze the selected loadout, not a second account-wide scan. Validate the
    // concrete item identities and owners before sending the native operation.
    let current = before;
    let selected = match op {
        ModsPluginOperation::EquipOneKey { placements, core } => placements
            .iter()
            .map(|p| p.equipment)
            .chain(std::iter::once(*core))
            .collect::<Vec<_>>(),
        ModsPluginOperation::UnequipAll => before
            .items
            .iter()
            .filter(|i| i.character_net_id == Some(character))
            .map(|i| i.id)
            .collect(),
        ModsPluginOperation::EquipModule { equipment, .. }
        | ModsPluginOperation::MoveModuleToCharacter { equipment, .. }
        | ModsPluginOperation::EquipCore { equipment }
        | ModsPluginOperation::MoveCoreToCharacter { equipment }
        | ModsPluginOperation::UnequipModule { equipment }
        | ModsPluginOperation::UnequipCore { equipment } => vec![*equipment],
        _ => vec![],
    };
    for selected in selected {
        let observed = inspect(client, before, selected)?;
        let expected = before
            .items
            .iter()
            .find(|i| i.id == selected)
            .ok_or(user_equipment::Error::Changed)?
            .character_net_id
            .unwrap_or(HtItemNetId::ZERO);
        if observed["item"]["CharacterNetID"]["solt"] != expected.solt
            || observed["item"]["CharacterNetID"]["serial"] != expected.serial
        {
            return Err(user_equipment::Error::Changed.into());
        }
    }
    let status = client.call("equipment.status", json!({}))?;
    let domain = status["domainKey"]
        .as_str()
        .filter(|s| s.len() <= 1024)
        .ok_or(ToolkitError::InvalidProtocol)?;
    if status["ready"] != true
        || status["providerId"] != current.provider
        || !domain.starts_with(&(current.domain.clone() + "/user-"))
    {
        return Err(ToolkitError::SessionChanged.into());
    }
    let epoch = status["epoch"]
        .as_str()
        .filter(|s| s.parse::<u64>().is_ok())
        .ok_or(ToolkitError::InvalidProtocol)?;
    let (method, mut params) = command(character, op);
    let request = request_id();
    params["requestId"] = json!(request);
    params["domainKey"] = json!(domain);
    params["epoch"] = json!(epoch);
    let ack = client.call_with_id(&request, method, params)?;
    if ack["requestId"] != request
        || ack["domainKey"] != domain
        || ack["epoch"] != epoch
        || ack["providerId"] != current.provider
        || ack["confirmed"] != false
        || !matches!(
            ack["status"].as_str(),
            Some("rpc_dispatched" | "outcome_unknown")
        )
    {
        return Err(ToolkitError::InvalidProtocol.into());
    }
    confirm_readback(current, character, op, collect(client, catalog))
}
pub fn connect(router: &Router, host: &ToolkitClient, pid: u32) -> Result<Client, Error> {
    router.connect(host, pid).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::model::{EmptyCurtainCharacter, EmptyCurtainItem, EmptyCurtainPlacement},
        platform::mods_plugin::ModsPluginPlacement,
    };
    fn uid(n: u32) -> HtItemNetId {
        HtItemNetId { solt: n, serial: 1 }
    }
    fn inventory() -> Inventory {
        let item = |n, placement| EmptyCurtainItem {
            id: uid(n),
            item_id: format!("fixture-{n}"),
            level: 0,
            main_stats: vec![],
            sub_stats: vec![],
            locked: false,
            discarded: false,
            character_net_id: Some(uid(3)),
            equipped_character_id: Some(1004),
            equipped_placement: placement,
        };
        Inventory {
            provider: "fixture".into(),
            domain: "fixture-session".into(),
            snapshot: "1".into(),
            observed_us: 1,
            characters: vec![EmptyCurtainCharacter {
                net_id: uid(3),
                character_id: 1004,
            }],
            items: vec![
                item(1, Some(EmptyCurtainPlacement { row: 2, column: 3 })),
                item(2, None),
            ],
        }
    }
    #[test]
    fn all_ten_legacy_equipment_actions_map_to_native_requests_and_require_readback() {
        use ModsPluginOperation::*;
        let ops = vec![
            (
                EquipModule {
                    equipment: uid(1),
                    row: 2,
                    column: 3,
                },
                "equipment.equip_module",
                true,
            ),
            (
                MoveModuleToCharacter {
                    equipment: uid(1),
                    row: 2,
                    column: 3,
                },
                "equipment.move_module_to_character",
                true,
            ),
            (
                EquipCore { equipment: uid(2) },
                "equipment.equip_core",
                true,
            ),
            (
                MoveCoreToCharacter { equipment: uid(2) },
                "equipment.move_core_to_character",
                true,
            ),
            (
                UnequipModule { equipment: uid(1) },
                "equipment.unequip_module",
                false,
            ),
            (
                UnequipCore { equipment: uid(2) },
                "equipment.unequip_core",
                false,
            ),
            (UnequipAll, "equipment.unequip_all", false),
            (
                EquipOneKey {
                    placements: vec![ModsPluginPlacement {
                        equipment: uid(1),
                        row: 2,
                        column: 3,
                    }],
                    core: uid(2),
                },
                "equipment.equip_one_key",
                true,
            ),
            (
                SetItemLocked {
                    equipment: uid(1),
                    locked: true,
                },
                "equipment.set_item_locked",
                false,
            ),
            (
                SetItemDiscarded {
                    equipment: uid(1),
                    discarded: true,
                },
                "equipment.set_item_discarded",
                false,
            ),
        ];
        let data = inventory();
        for (op, method, expected) in ops {
            let (actual, params) = command(uid(3), &op);
            assert_eq!(actual, method);
            assert!(params.is_object());
            assert_eq!(confirmed(&data, uid(3), &op), expected);
        }
        assert!(!confirmed(
            &data,
            uid(4),
            &EquipModule {
                equipment: uid(1),
                row: 2,
                column: 3
            }
        ));
        assert!(!confirmed(
            &data,
            uid(3),
            &EquipModule {
                equipment: uid(1),
                row: 3,
                column: 2
            }
        ));
        let mut changed = data.clone();
        changed.items[0].locked = true;
        assert!(confirmed(
            &changed,
            uid(3),
            &SetItemLocked {
                equipment: uid(1),
                locked: true
            }
        ));
        changed.items[0].character_net_id = None;
        assert!(confirmed(
            &changed,
            uid(3),
            &UnequipModule { equipment: uid(1) }
        ));
    }
    #[test]
    fn published_inventory_is_immutable_and_noop_does_not_bump_revision() {
        let store = Store::default();
        let data = inventory();
        store.publish("p".into(), data.clone()).unwrap();
        let (old, rev) = store.get().unwrap();
        store.publish("p".into(), data.clone()).unwrap();
        assert_eq!(store.get().unwrap().1, rev);
        let mut next = data;
        next.items[0].locked = true;
        store.publish("p".into(), next).unwrap();
        assert_eq!(store.get().unwrap().1, rev + 1);
        assert!(!old.unwrap().items[0].locked);
    }
    #[test]
    fn post_dispatch_read_errors_are_pending_not_failed_operations() {
        let before = inventory();
        for op in [
            ModsPluginOperation::UnequipAll,
            ModsPluginOperation::EquipModule {
                equipment: uid(1),
                row: 2,
                column: 3,
            },
            ModsPluginOperation::EquipOneKey {
                placements: vec![ModsPluginPlacement {
                    equipment: uid(1),
                    row: 2,
                    column: 3,
                }],
                core: uid(2),
            },
        ] {
            assert_eq!(
                confirm_readback(&before, uid(3), &op, Err(ToolkitError::Busy.into())),
                Err(Error::ConfirmationPending(ReadbackFailure::Transport(
                    ToolkitError::Busy
                )))
            );
            assert_eq!(
                confirm_readback(
                    &before,
                    uid(3),
                    &op,
                    Err(user_equipment::Error::Changed.into())
                ),
                Err(Error::ConfirmationPending(ReadbackFailure::Snapshot(
                    user_equipment::Error::Changed
                )))
            );
        }
        let mut after = before.clone();
        for row in &mut after.items {
            row.character_net_id = None;
            row.equipped_placement = None;
        }
        assert!(
            confirm_readback(&before, uid(3), &ModsPluginOperation::UnequipAll, Ok(after)).is_ok()
        );
        assert_eq!(
            confirm_readback(
                &before,
                uid(3),
                &ModsPluginOperation::UnequipAll,
                Ok(before.clone())
            ),
            Err(Error::Unconfirmed)
        );
    }
    #[test]
    fn deferred_confirmation_requires_matching_source_and_expected_state() {
        let store = Store::default();
        let before = inventory();
        store
            .defer_confirmation(
                "host".into(),
                &before,
                uid(3),
                ModsPluginOperation::UnequipAll,
            )
            .unwrap();
        assert!(
            store
                .defer_confirmation(
                    "host".into(),
                    &before,
                    uid(3),
                    ModsPluginOperation::UnequipAll
                )
                .is_err()
        );
        assert_eq!(
            store.settle_confirmation("host", None).unwrap(),
            Some(ConfirmationState::Waiting)
        );
        assert_eq!(
            store.settle_confirmation("host", Some(&before)).unwrap(),
            Some(ConfirmationState::Waiting)
        );
        let mut after = before.clone();
        for item in &mut after.items {
            item.character_net_id = None;
        }
        assert_eq!(
            store.settle_confirmation("host", Some(&after)).unwrap(),
            Some(ConfirmationState::Confirmed)
        );
        assert!(!store.confirming().unwrap());
        store
            .defer_confirmation(
                "host".into(),
                &before,
                uid(3),
                ModsPluginOperation::UnequipAll,
            )
            .unwrap();
        assert_eq!(
            store
                .settle_confirmation("different-host", Some(&after))
                .unwrap(),
            Some(ConfirmationState::SourceChanged)
        );
        store
            .defer_confirmation(
                "host".into(),
                &before,
                uid(3),
                ModsPluginOperation::UnequipAll,
            )
            .unwrap();
        store
            .state
            .lock()
            .unwrap()
            .confirmation
            .as_mut()
            .unwrap()
            .deadline = std::time::Instant::now() - std::time::Duration::from_secs(1);
        assert_eq!(
            store.settle_confirmation("host", None).unwrap(),
            Some(ConfirmationState::Expired)
        );
        assert!(!store.confirming().unwrap());
    }
    #[test]
    fn watch_rejects_future_duplicate_foreign_and_unsupported_changes() {
        let watch = Watch {
            provider_id: "fixture".into(),
            process_id: 7,
            process_created_file_time: "8".into(),
            equipment_watch_supported: true,
            sdk_compatible: true,
            equipment_revision: "3".into(),
            equipment_full_revision: "1".into(),
            equipment_flags: vec![FlagChange {
                solt: 1,
                serial: 2,
                revision: "3".into(),
            }],
        };
        assert!(watch.validate("7:8").is_ok());
        assert!(watch.validate("7:9").is_err());
        let mut bad = watch.clone();
        bad.equipment_flags.push(bad.equipment_flags[0].clone());
        assert!(bad.validate("7:8").is_err());
        let mut bad = watch.clone();
        bad.equipment_full_revision = "4".into();
        assert!(bad.validate("7:8").is_err());
        let mut bad = watch.clone();
        bad.equipment_watch_supported = false;
        assert!(bad.validate("7:8").is_err());
        let mut bad = watch;
        bad.equipment_flags[0].revision = "18446744073709551616".into();
        assert!(bad.validate("7:8").is_err());
    }
    #[test]
    fn targeted_flags_patch_preserves_other_rows_and_rejects_owner_changes() {
        let mut data = inventory();
        let untouched = data.items[1].clone();
        patch_flags(
            &mut data,
            uid(1),
            &json!({"CharacterNetID":{"solt":3,"serial":1},"IsLocked":true,"IsDiscarded":false}),
        )
        .unwrap();
        assert!(data.items[0].locked);
        assert_eq!(data.items[1], untouched);
        assert!(patch_flags(&mut data,uid(1),&json!({"CharacterNetID":{"solt":8,"serial":1},"IsLocked":false,"IsDiscarded":false})).is_err());
        assert!(data.items[0].locked);
    }
}
