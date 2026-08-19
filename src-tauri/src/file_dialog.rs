use std::{fmt, io, path::PathBuf};

#[cfg(windows)]
use nte_dps_tool::{platform::file_dialog as native, storage::i18n};
#[cfg(windows)]
use tauri::WebviewWindow;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Selected(PathBuf),
    Cancelled,
}

#[derive(Debug)]
pub(crate) enum DialogError {
    OwnerUnavailable,
    NativeHresult(i32),
    WorkerUnavailable,
    OpenDirectory(io::ErrorKind),
}

impl fmt::Display for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerUnavailable => formatter.write_str("dialog owner is unavailable"),
            Self::NativeHresult(hresult) => write!(
                formatter,
                "native dialog failed with HRESULT {:#010x}",
                *hresult as u32
            ),
            Self::WorkerUnavailable => formatter.write_str("dialog worker did not complete"),
            Self::OpenDirectory(kind) => write!(formatter, "open directory failed ({kind:?})"),
        }
    }
}

impl std::error::Error for DialogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DialogOwner(isize);

#[cfg(windows)]
fn dialog_owner(window: &WebviewWindow) -> Result<DialogOwner, DialogError> {
    map_dialog_owner(window.hwnd().map(|owner| owner.0 as isize))
}

#[cfg(windows)]
fn map_dialog_owner<E>(owner: Result<isize, E>) -> Result<DialogOwner, DialogError> {
    owner
        .map(DialogOwner)
        .map_err(|_| DialogError::OwnerUnavailable)
}

#[cfg(windows)]
fn validate_dialog_owner(owner: DialogOwner) -> Result<(), DialogError> {
    if !native::is_current_process_window(owner.0) {
        return Err(DialogError::OwnerUnavailable);
    }
    Ok(())
}

#[cfg(windows)]
fn localized_file_dialog_filter_labels() -> native::FileDialogFilterLabels {
    file_dialog_filter_labels(i18n::t)
}

#[cfg(windows)]
fn file_dialog_filter_labels(translate: impl Fn(&str) -> String) -> native::FileDialogFilterLabels {
    native::FileDialogFilterLabels {
        all_files: translate("All files (*.*)"),
        json_files: translate("JSON files (*.json)"),
        ini_files: translate("INI files (*.ini)"),
        pcapng_files: translate("PCAPNG files (*.pcapng)"),
    }
}

#[cfg(windows)]
fn execute_owned_dialog<T, Validate, Operation>(
    owner: DialogOwner,
    validate: Validate,
    operation: Operation,
) -> Result<T, DialogError>
where
    Validate: FnOnce(DialogOwner) -> Result<(), DialogError>,
    Operation: FnOnce(isize) -> Result<T, DialogError>,
{
    validate(owner)?;
    operation(owner.0)
}

async fn run_blocking<T, F>(operation: F) -> Result<T, DialogError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| DialogError::WorkerUnavailable)
}

#[cfg(windows)]
async fn run_dialog<F>(window: &WebviewWindow, operation: F) -> Result<DialogOutcome, DialogError>
where
    F: FnOnce(isize) -> Result<DialogOutcome, DialogError> + Send + 'static,
{
    let owner_window = window.clone();
    run_blocking(move || {
        // Resolve and validate the native owner on the blocking worker immediately
        // before opening the modal dialog. The clone remains live until it returns.
        let owner = dialog_owner(&owner_window)?;
        execute_owned_dialog(owner, validate_dialog_owner, operation)
    })
    .await?
}

#[cfg(windows)]
fn map_open_outcome(
    outcome: Result<native::OpenFileDialogOutcome, native::FileDialogError>,
) -> Result<DialogOutcome, DialogError> {
    match outcome {
        Ok(native::OpenFileDialogOutcome::Selected(path)) => Ok(DialogOutcome::Selected(path)),
        Ok(native::OpenFileDialogOutcome::Cancelled) => Ok(DialogOutcome::Cancelled),
        Err(error) => Err(native_dialog_error(error)),
    }
}

#[cfg(windows)]
fn map_save_outcome(
    outcome: Result<native::SaveFileDialogOutcome, native::FileDialogError>,
) -> Result<DialogOutcome, DialogError> {
    match outcome {
        Ok(native::SaveFileDialogOutcome::Selected(path)) => Ok(DialogOutcome::Selected(path)),
        Ok(native::SaveFileDialogOutcome::Cancelled) => Ok(DialogOutcome::Cancelled),
        Err(error) => Err(native_dialog_error(error)),
    }
}

#[cfg(windows)]
fn map_folder_outcome(
    outcome: Result<native::FolderDialogOutcome, native::FolderDialogError>,
) -> Result<DialogOutcome, DialogError> {
    match outcome {
        Ok(native::FolderDialogOutcome::Selected(path)) => Ok(DialogOutcome::Selected(path)),
        Ok(native::FolderDialogOutcome::Cancelled) => Ok(DialogOutcome::Cancelled),
        Err(error) => Err(native_dialog_error(error)),
    }
}

#[cfg(windows)]
fn native_dialog_error(error: native::FileDialogError) -> DialogError {
    DialogError::NativeHresult(error.hresult())
}

#[cfg(windows)]
pub(crate) async fn choose_json_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    let labels = localized_file_dialog_filter_labels();
    run_dialog(window, move |owner| {
        map_open_outcome(native::choose_json_open_path(owner, &title, &labels))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_pcapng_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    let labels = localized_file_dialog_filter_labels();
    run_dialog(window, move |owner| {
        map_open_outcome(native::choose_pcapng_open_path(owner, &title, &labels))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_ini_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    let labels = localized_file_dialog_filter_labels();
    run_dialog(window, move |owner| {
        map_open_outcome(native::choose_ini_open_path(owner, &title, &labels))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_json_save_path(
    window: &WebviewWindow,
    title: String,
    default_file_name: String,
) -> Result<DialogOutcome, DialogError> {
    let labels = localized_file_dialog_filter_labels();
    run_dialog(window, move |owner| {
        map_save_outcome(native::choose_json_save_path(
            owner,
            &title,
            &default_file_name,
            &labels,
        ))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_pcapng_save_path(
    window: &WebviewWindow,
    title: String,
    default_file_name: String,
) -> Result<DialogOutcome, DialogError> {
    let labels = localized_file_dialog_filter_labels();
    run_dialog(window, move |owner| {
        map_save_outcome(native::choose_pcapng_save_path(
            owner,
            &title,
            &default_file_name,
            &labels,
        ))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_folder(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    run_dialog(window, move |owner| {
        map_folder_outcome(native::choose_folder(owner, &title))
    })
    .await
}

#[cfg(windows)]
pub(crate) async fn open_directory(path: PathBuf) -> Result<(), DialogError> {
    run_blocking(move || {
        native::open_directory(&path).map_err(|error| DialogError::OpenDirectory(error.kind()))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, thread};

    use super::*;

    #[test]
    fn commands_use_the_tauri_file_dialog_adapter() {
        let commands = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/commands");
        let mut violations = Vec::new();
        for entry in fs::read_dir(&commands).expect("commands directory must be readable") {
            let entry = entry.expect("commands directory entry must be readable");
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).expect("command source must be readable");
            if source.contains("platform::file_dialog") {
                violations.push(
                    path.file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or("<unknown>")
                        .to_owned(),
                );
            }
        }
        violations.sort();
        assert!(
            violations.is_empty(),
            "Tauri commands must use crate::file_dialog instead of the shared platform module: {}",
            violations.join(", ")
        );
    }

    #[cfg(windows)]
    #[test]
    fn native_outcomes_keep_selection_cancellation_and_error_distinct() {
        let selected = PathBuf::from(format!(r"C:\capture\{}.json", "a".repeat(300)));
        assert_eq!(
            map_open_outcome(Ok(native::OpenFileDialogOutcome::Selected(
                selected.clone()
            )))
            .expect("open selection"),
            DialogOutcome::Selected(selected.clone())
        );
        assert_eq!(
            map_save_outcome(Ok(native::SaveFileDialogOutcome::Cancelled))
                .expect("save cancellation"),
            DialogOutcome::Cancelled
        );
        assert_eq!(
            map_folder_outcome(Ok(native::FolderDialogOutcome::Selected(selected.clone())))
                .expect("folder selection"),
            DialogOutcome::Selected(selected)
        );
        assert!(matches!(
            map_open_outcome(Err(native::FileDialogError::NativeHresult(
                0x8000_3002_u32 as i32
            ))),
            Err(DialogError::NativeHresult(value)) if value == 0x8000_3002_u32 as i32
        ));
        assert!(matches!(
            map_save_outcome(Err(native::FileDialogError::NativeHresult(
                0x8000_3003_u32 as i32
            ))),
            Err(DialogError::NativeHresult(value)) if value == 0x8000_3003_u32 as i32
        ));
        assert!(matches!(
            map_folder_outcome(Err(native::FolderDialogError::NativeHresult(
                0x8000_3004_u32 as i32
            ))),
            Err(DialogError::NativeHresult(value)) if value == 0x8000_3004_u32 as i32
        ));
    }

    #[cfg(windows)]
    #[test]
    fn owner_mapping_does_not_fabricate_an_unowned_dialog() {
        assert_eq!(
            map_dialog_owner::<()>(Ok(123)).expect("owner"),
            DialogOwner(123)
        );
        assert!(matches!(
            map_dialog_owner::<()>(Err(())),
            Err(DialogError::OwnerUnavailable)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn adapter_supplies_every_user_visible_filter_label() {
        let labels = file_dialog_filter_labels(|key| format!("localized:{key}"));

        assert_eq!(labels.all_files, "localized:All files (*.*)");
        assert_eq!(labels.json_files, "localized:JSON files (*.json)");
        assert_eq!(labels.ini_files, "localized:INI files (*.ini)");
        assert_eq!(labels.pcapng_files, "localized:PCAPNG files (*.pcapng)");
    }

    #[test]
    fn owner_is_resolved_and_validated_inside_the_blocking_operation() {
        let source = include_str!("file_dialog.rs");

        assert!(source.contains("validate_dialog_owner"));
        assert!(source.contains("is_current_process_window"));
        let run_dialog = source
            .split("async fn run_dialog")
            .nth(1)
            .expect("run_dialog source");
        let blocking_body = run_dialog
            .split("run_blocking(move ||")
            .nth(1)
            .expect("blocking dialog body");
        assert!(blocking_body.contains("dialog_owner(&owner_window)"));
        assert!(blocking_body.contains("validate_dialog_owner"));
    }

    #[test]
    fn every_dialog_and_directory_operation_uses_the_blocking_pool() {
        let source = include_str!("file_dialog.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source");

        assert_eq!(production.matches("run_dialog(window").count(), 6);
        let open_directory = production
            .split("async fn open_directory")
            .nth(1)
            .expect("open_directory source");
        assert!(open_directory.contains("run_blocking(move ||"));
    }

    #[cfg(windows)]
    #[test]
    fn invalid_owner_prevents_the_native_operation() {
        let called = std::sync::atomic::AtomicBool::new(false);

        let result = execute_owned_dialog(DialogOwner(0), validate_dialog_owner, |_| {
            called.store(true, std::sync::atomic::Ordering::Release);
            Ok(DialogOutcome::Cancelled)
        });

        assert!(matches!(result, Err(DialogError::OwnerUnavailable)));
        assert!(!called.load(std::sync::atomic::Ordering::Acquire));
    }

    #[test]
    fn blocking_runner_moves_work_off_the_calling_thread() {
        let calling_thread = thread::current().id();
        let worker_thread = tauri::async_runtime::block_on(run_blocking(|| thread::current().id()))
            .expect("blocking worker");
        assert_ne!(calling_thread, worker_thread);
    }

    #[test]
    fn dialog_error_display_never_contains_a_selected_path() {
        assert_eq!(
            DialogError::OpenDirectory(io::ErrorKind::PermissionDenied).to_string(),
            "open directory failed (PermissionDenied)"
        );
        assert_eq!(
            DialogError::NativeHresult(0x8007_04c7_u32 as i32).to_string(),
            "native dialog failed with HRESULT 0x800704c7"
        );
    }
}
