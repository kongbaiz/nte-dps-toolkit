#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EmptyCurtainOperationState {
    pub status: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}
impl Default for EmptyCurtainOperationState {
    fn default() -> Self {
        Self {
            status: "error",
            message_key: "Legacy equipment changes are not supported by UE Tools.",
            message_arguments: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EquipmentOperationSnapshot {
    pub operation: EmptyCurtainOperationState,
    pub revision: u64,
}
pub(crate) type EquipmentOperationError = std::convert::Infallible;
#[derive(Default)]
pub(crate) struct EquipmentOperationService;
impl EquipmentOperationService {
    pub(crate) fn poll_snapshot(
        &self,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError> {
        Ok(EquipmentOperationSnapshot {
            operation: EmptyCurtainOperationState::default(),
            revision: 0,
        })
    }
}
