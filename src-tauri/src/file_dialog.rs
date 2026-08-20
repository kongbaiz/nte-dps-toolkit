use std::{fmt, io, path::PathBuf, process::Command};

#[cfg(windows)]
use nte_dps_tool::storage::i18n;
#[cfg(windows)]
use tauri::WebviewWindow;
#[cfg(windows)]
use tauri_plugin_dialog::{DialogExt, FilePath};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    Selected(PathBuf),
    Cancelled,
}

#[derive(Debug)]
pub(crate) enum DialogError {
    OwnerUnavailable,
    InvalidSelection,
    WorkerUnavailable,
    OpenDirectory(io::ErrorKind),
}

impl fmt::Display for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerUnavailable => formatter.write_str("dialog owner is unavailable"),
            Self::InvalidSelection => formatter.write_str("dialog returned an invalid file path"),
            Self::WorkerUnavailable => formatter.write_str("dialog worker did not complete"),
            Self::OpenDirectory(kind) => write!(formatter, "open directory failed ({kind:?})"),
        }
    }
}

impl std::error::Error for DialogError {}

#[cfg(windows)]
#[derive(Clone, Copy)]
enum DialogOperation {
    PickFile,
    SaveFile,
    PickFolder,
}

#[cfg(windows)]
struct DialogFilter {
    label: String,
    extensions: &'static [&'static str],
}

#[cfg(windows)]
struct DialogRequest {
    title: String,
    filters: Vec<DialogFilter>,
    default_file_name: Option<String>,
    operation: DialogOperation,
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
fn map_selection(selection: Option<FilePath>) -> Result<DialogOutcome, DialogError> {
    selection
        .map(|path| {
            path.into_path()
                .map(DialogOutcome::Selected)
                .map_err(|_| DialogError::InvalidSelection)
        })
        .unwrap_or(Ok(DialogOutcome::Cancelled))
}

#[cfg(windows)]
async fn run_dialog(
    window: &WebviewWindow,
    request: DialogRequest,
) -> Result<DialogOutcome, DialogError> {
    let owner_window = window.clone();
    run_blocking(move || {
        // Resolve the WebView handle immediately before building the native dialog.
        // This rejects a destroyed owner instead of silently opening an unowned picker.
        owner_window
            .hwnd()
            .map_err(|_| DialogError::OwnerUnavailable)?;

        let mut builder = owner_window
            .dialog()
            .file()
            .set_parent(&owner_window)
            .set_title(request.title);
        for filter in request.filters {
            builder = builder.add_filter(filter.label, filter.extensions);
        }
        if let Some(default_file_name) = request.default_file_name {
            builder = builder.set_file_name(default_file_name);
        }

        let selection = match request.operation {
            DialogOperation::PickFile => builder.blocking_pick_file(),
            DialogOperation::SaveFile => builder.blocking_save_file(),
            DialogOperation::PickFolder => builder.blocking_pick_folder(),
        };
        map_selection(selection)
    })
    .await?
}

#[cfg(windows)]
fn file_filters(label_key: &str, extension: &'static str) -> Vec<DialogFilter> {
    vec![
        DialogFilter {
            label: i18n::t(label_key),
            extensions: match extension {
                "json" => &["json"],
                "ini" => &["ini"],
                "pcapng" => &["pcapng"],
                _ => &[],
            },
        },
        DialogFilter {
            label: i18n::t("All files (*.*)"),
            extensions: &["*"],
        },
    ]
}

#[cfg(windows)]
async fn choose_open_path(
    window: &WebviewWindow,
    title: String,
    label_key: &str,
    extension: &'static str,
) -> Result<DialogOutcome, DialogError> {
    run_dialog(
        window,
        DialogRequest {
            title,
            filters: file_filters(label_key, extension),
            default_file_name: None,
            operation: DialogOperation::PickFile,
        },
    )
    .await
}

#[cfg(windows)]
async fn choose_save_path(
    window: &WebviewWindow,
    title: String,
    default_file_name: String,
    label_key: &str,
    extension: &'static str,
) -> Result<DialogOutcome, DialogError> {
    run_dialog(
        window,
        DialogRequest {
            title,
            filters: file_filters(label_key, extension),
            default_file_name: Some(default_file_name),
            operation: DialogOperation::SaveFile,
        },
    )
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_json_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    choose_open_path(window, title, "JSON files (*.json)", "json").await
}

#[cfg(windows)]
pub(crate) async fn choose_pcapng_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    choose_open_path(window, title, "PCAPNG files (*.pcapng)", "pcapng").await
}

#[cfg(windows)]
pub(crate) async fn choose_ini_open_path(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    choose_open_path(window, title, "INI files (*.ini)", "ini").await
}

#[cfg(windows)]
pub(crate) async fn choose_json_save_path(
    window: &WebviewWindow,
    title: String,
    default_file_name: String,
) -> Result<DialogOutcome, DialogError> {
    choose_save_path(
        window,
        title,
        default_file_name,
        "JSON files (*.json)",
        "json",
    )
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_pcapng_save_path(
    window: &WebviewWindow,
    title: String,
    default_file_name: String,
) -> Result<DialogOutcome, DialogError> {
    choose_save_path(
        window,
        title,
        default_file_name,
        "PCAPNG files (*.pcapng)",
        "pcapng",
    )
    .await
}

#[cfg(windows)]
pub(crate) async fn choose_folder(
    window: &WebviewWindow,
    title: String,
) -> Result<DialogOutcome, DialogError> {
    run_dialog(
        window,
        DialogRequest {
            title,
            filters: Vec::new(),
            default_file_name: None,
            operation: DialogOperation::PickFolder,
        },
    )
    .await
}

#[cfg(windows)]
pub(crate) async fn open_directory(path: PathBuf) -> Result<(), DialogError> {
    run_blocking(move || {
        Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| DialogError::OpenDirectory(error.kind()))
    })
    .await?
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    #[cfg(windows)]
    #[test]
    fn official_plugin_selection_keeps_selected_cancelled_and_invalid_distinct() {
        let selected = PathBuf::from(format!(r"C:\capture\{}.json", "a".repeat(300)));
        assert_eq!(
            map_selection(Some(FilePath::Path(selected.clone()))).expect("selection"),
            DialogOutcome::Selected(selected)
        );
        assert_eq!(
            map_selection(None).expect("cancellation"),
            DialogOutcome::Cancelled
        );
        assert!(matches!(
            map_selection(Some(FilePath::Url(
                "https://example.invalid/not-a-local-file"
                    .parse()
                    .expect("test URL")
            ))),
            Err(DialogError::InvalidSelection)
        ));
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
            DialogError::InvalidSelection.to_string(),
            "dialog returned an invalid file path"
        );
    }
}
