use nte_dps_tool::core::equipment_runtime::Store;
use std::sync::Mutex;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmptyCurtainOperationState {
    pub status: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}
impl Default for EmptyCurtainOperationState {
    fn default() -> Self {
        Self {
            status: "idle",
            message_key: "No equipment operation is pending",
            message_arguments: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EquipmentOperationSnapshot {
    pub operation: EmptyCurtainOperationState,
    pub revision: u64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum EquipmentOperationError {
    Unavailable,
}
#[derive(Default)]
pub(crate) struct EquipmentOperationService {
    pub inventory: Store,
    operation: Mutex<(EmptyCurtainOperationState, u64)>,
}
impl EquipmentOperationService {
    pub(crate) fn set(
        &self,
        status: &'static str,
        key: &'static str,
    ) -> Result<(), EquipmentOperationError> {
        let mut s = self
            .operation
            .lock()
            .map_err(|_| EquipmentOperationError::Unavailable)?;
        let next = EmptyCurtainOperationState {
            status,
            message_key: key,
            message_arguments: vec![],
        };
        if s.0 != next {
            s.0 = next;
            s.1 = s.1.wrapping_add(1);
        }
        Ok(())
    }
    pub(crate) fn poll_snapshot(
        &self,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError> {
        let s = self
            .operation
            .lock()
            .map_err(|_| EquipmentOperationError::Unavailable)?;
        Ok(EquipmentOperationSnapshot {
            operation: s.0.clone(),
            revision: s.1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_does_not_claim_legacy_failure_and_noop_keeps_revision() {
        let service = EquipmentOperationService::default();
        assert_eq!(service.poll_snapshot().unwrap().operation.status, "idle");
        service
            .set("pending", "Sending equipment request...")
            .unwrap();
        let before = service.poll_snapshot().unwrap();
        service
            .set("pending", "Sending equipment request...")
            .unwrap();
        assert_eq!(before, service.poll_snapshot().unwrap());
        service
            .set(
                "success",
                "Equipment change confirmed by refreshed inventory.",
            )
            .unwrap();
        assert_eq!(
            service.poll_snapshot().unwrap().revision,
            before.revision + 1
        );
    }
}
