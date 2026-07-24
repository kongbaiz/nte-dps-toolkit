use super::*;

use semver::Version;

use crate::core::update::{
    AvailableComponentUpdate, MAX_MANIFEST_BYTES, UpdateComponent, UpdateEndpoint, UpdateError,
    verify_manifest,
};
use crate::platform::equipment_plugin::EquipmentPluginDeploymentError;
use crate::platform::update_http;
use crate::storage::update::{
    self as update_storage, ComponentVersionError, InstallPluginUpdateError, PrepareUpdateError,
    PreparedUpdate,
};

const PROGRESS_REPAINT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateFailureStage {
    Check,
    Download,
    Install,
}

#[derive(Clone, Debug)]
pub(crate) enum UpdateStatus {
    Idle,
    NotConfigured,
    Checking,
    UpToDate,
    Available,
    Downloading {
        component: UpdateComponent,
        downloaded: u64,
        total: u64,
    },
    InstallingPlugin,
    Ready,
    Failed {
        stage: UpdateFailureStage,
        detail: String,
    },
}

pub(crate) struct UpdateClientState {
    pub(crate) auto_check: bool,
    pub(crate) auto_download: bool,
    pub(crate) status: UpdateStatus,
    pub(crate) available: Vec<AvailableComponentUpdate>,
    pub(crate) prepared: Option<PreparedUpdate>,
    next_auto_check: Option<Instant>,
    sender: Sender<UpdateWorkerEvent>,
    receiver: Receiver<UpdateWorkerEvent>,
    health_reported: bool,
}

enum UpdateWorkerEvent {
    CheckFinished(Result<Vec<AvailableComponentUpdate>, CheckUpdateError>),
    DownloadProgress {
        component: UpdateComponent,
        downloaded: u64,
        total: u64,
    },
    DownloadFinished(Result<PreparedUpdate, PrepareUpdateError>),
    PluginInstallFinished(
        Result<(Version, Vec<AvailableComponentUpdate>), InstallPluginUpdateError>,
    ),
}

#[derive(Debug)]
enum CheckUpdateError {
    Download(update_http::HttpError),
    ComponentState(ComponentVersionError),
    Manifest(UpdateError),
}

impl UpdateClientState {
    pub(crate) fn new(config: &UiConfig) -> Self {
        update_storage::cleanup_completed_update_staging();
        let (sender, receiver) = unbounded();
        let next_auto_check = config.auto_check_updates.then(Instant::now);
        Self {
            auto_check: config.auto_check_updates,
            auto_download: config.auto_download_updates,
            status: UpdateStatus::Idle,
            available: Vec::new(),
            prepared: None,
            next_auto_check,
            sender,
            receiver,
            health_reported: false,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::InstallingPlugin
        )
    }

    pub(crate) fn can_check(&self) -> bool {
        !self.busy() && self.prepared.is_none()
    }

    pub(crate) fn schedule_auto_check(&mut self) {
        self.next_auto_check = self.auto_check.then(Instant::now);
    }
}

impl DpsApp {
    pub(crate) fn poll_update_client(&mut self, ctx: &egui::Context) {
        if !self.update_client.health_reported {
            match update_storage::mark_update_healthy_from_environment() {
                Ok(Some(marker)) => {
                    thread::spawn(move || {
                        update_storage::cleanup_completed_app_update(marker);
                    });
                }
                Ok(None) => {}
                Err(error) => eprintln!("Failed to write update health marker: {error}"),
            }
            self.update_client.health_reported = true;
        }

        let mut auto_download = None;
        while let Ok(event) = self.update_client.receiver.try_recv() {
            match event {
                UpdateWorkerEvent::CheckFinished(Ok(updates)) if !updates.is_empty() => {
                    if let Some(app) = updates
                        .iter()
                        .find(|update| update.component == UpdateComponent::App)
                    {
                        self.notifications.status =
                            tf("Version {} is available", &[&app.version.to_string()]);
                    } else if let Some(plugin) = updates.first() {
                        self.notifications.status = tf(
                            "Equipment plugin version {} is available",
                            &[&plugin.version.to_string()],
                        );
                    }
                    auto_download = self
                        .update_client
                        .auto_download
                        .then(|| updates[0].component);
                    self.update_client.available = updates;
                    self.update_client.prepared = None;
                    self.update_client.status = UpdateStatus::Available;
                }
                UpdateWorkerEvent::CheckFinished(Ok(_)) => {
                    self.update_client.available.clear();
                    self.update_client.prepared = None;
                    self.update_client.status = UpdateStatus::UpToDate;
                }
                UpdateWorkerEvent::CheckFinished(Err(error)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Check,
                        detail: update_check_error_detail(&error),
                    };
                }
                UpdateWorkerEvent::DownloadProgress {
                    component,
                    downloaded,
                    total,
                } => {
                    self.update_client.status = UpdateStatus::Downloading {
                        component,
                        downloaded,
                        total,
                    };
                }
                UpdateWorkerEvent::DownloadFinished(Ok(prepared)) => {
                    self.notifications.status = match prepared.component() {
                        UpdateComponent::App => tf(
                            "Version {} is ready to install",
                            &[&prepared.version().to_string()],
                        ),
                        UpdateComponent::EquipmentPlugin => tf(
                            "Equipment plugin {} is ready to install",
                            &[&prepared.version().to_string()],
                        ),
                    };
                    self.update_client.prepared = Some(prepared);
                    self.update_client.status = UpdateStatus::Ready;
                }
                UpdateWorkerEvent::DownloadFinished(Err(error)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Download,
                        detail: update_download_error_detail(&error),
                    };
                }
                UpdateWorkerEvent::PluginInstallFinished(Ok((version, remaining))) => {
                    self.notifications.status =
                        tf("Equipment plugin {} was installed", &[&version.to_string()]);
                    self.update_client.available = remaining;
                    self.update_client.prepared = None;
                    self.update_client.status = if self.update_client.available.is_empty() {
                        UpdateStatus::UpToDate
                    } else {
                        UpdateStatus::Available
                    };
                }
                UpdateWorkerEvent::PluginInstallFinished(Err(error)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Install,
                        detail: plugin_update_error_detail(&error),
                    };
                }
            }
        }

        if let Some(component) = auto_download {
            self.start_component_update_download(ctx, component);
        }
        if self
            .update_client
            .next_auto_check
            .is_some_and(|deadline| deadline <= Instant::now())
            && self.update_client.can_check()
        {
            self.update_client.next_auto_check = None;
            self.start_update_check(ctx);
        }
        if let Some(deadline) = self.update_client.next_auto_check {
            ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
        }
    }

    pub(crate) fn set_auto_check_updates(&mut self, enabled: bool) {
        self.update_client.auto_check = enabled;
        self.update_client.schedule_auto_check();
    }

    pub(crate) fn start_update_check(&mut self, ctx: &egui::Context) {
        if !self.update_client.can_check() {
            return;
        }
        let endpoint = match UpdateEndpoint::official() {
            Ok(endpoint) => endpoint,
            Err(UpdateError::ClientNotConfigured) => {
                self.update_client.status = UpdateStatus::NotConfigured;
                return;
            }
            Err(error) => {
                self.update_client.status = UpdateStatus::Failed {
                    stage: UpdateFailureStage::Check,
                    detail: update_manifest_error_detail(&error),
                };
                return;
            }
        };
        self.update_client.status = UpdateStatus::Checking;
        let sender = self.update_client.sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = check_for_update(&endpoint);
            let _ = sender.send(UpdateWorkerEvent::CheckFinished(result));
            repaint.request_repaint();
        });
    }

    pub(crate) fn start_component_update_download(
        &mut self,
        ctx: &egui::Context,
        component: UpdateComponent,
    ) {
        if self.update_client.busy() || self.update_client.prepared.is_some() {
            return;
        }
        let Some(update) = self
            .update_client
            .available
            .iter()
            .find(|update| update.component == component)
            .cloned()
        else {
            return;
        };
        self.update_client.status = UpdateStatus::Downloading {
            component,
            downloaded: 0,
            total: update.artifact_size,
        };
        let sender = self.update_client.sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let mut last_progress = Instant::now() - PROGRESS_REPAINT_INTERVAL;
            let result = update_storage::prepare_update(&update, |downloaded, total| {
                let now = Instant::now();
                if downloaded == total
                    || now.saturating_duration_since(last_progress) >= PROGRESS_REPAINT_INTERVAL
                {
                    last_progress = now;
                    let _ = sender.send(UpdateWorkerEvent::DownloadProgress {
                        component,
                        downloaded,
                        total,
                    });
                    repaint.request_repaint();
                }
            });
            let _ = sender.send(UpdateWorkerEvent::DownloadFinished(result));
            repaint.request_repaint();
        });
    }

    pub(crate) fn install_prepared_update(&mut self, ctx: &egui::Context) {
        let Some(prepared) = self.update_client.prepared.as_ref() else {
            return;
        };
        match prepared.component() {
            UpdateComponent::App => {
                if self.capture.is_some() || self.replay_thread.is_some() {
                    return;
                }
                match update_storage::launch_prepared_app_update(prepared) {
                    Ok(_) => {
                        self.notifications.status = t("Restarting to install the update...");
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Close,
                        );
                    }
                    Err(error) => {
                        self.update_client.status = UpdateStatus::Failed {
                            stage: UpdateFailureStage::Install,
                            detail: updater_launch_error_detail(&error),
                        };
                    }
                }
            }
            UpdateComponent::EquipmentPlugin => {
                let prepared = prepared.clone();
                let remaining: Vec<_> = self
                    .update_client
                    .available
                    .iter()
                    .filter(|update| update.component != UpdateComponent::EquipmentPlugin)
                    .cloned()
                    .collect();
                let version = prepared.version().clone();
                self.update_client.status = UpdateStatus::InstallingPlugin;
                let sender = self.update_client.sender.clone();
                let repaint = ctx.clone();
                thread::spawn(move || {
                    let result = update_storage::install_prepared_plugin_update(&prepared)
                        .map(|()| (version, remaining));
                    let _ = sender.send(UpdateWorkerEvent::PluginInstallFinished(result));
                    repaint.request_repaint();
                });
            }
        }
    }
}

fn check_for_update(
    endpoint: &UpdateEndpoint,
) -> Result<Vec<AvailableComponentUpdate>, CheckUpdateError> {
    let manifest = update_http::get_bytes(&endpoint.manifest_url, MAX_MANIFEST_BYTES)
        .map_err(CheckUpdateError::Download)?;
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("Cargo package version is valid semantic versioning");
    let installed = update_storage::installed_component_versions(current_version)
        .map_err(CheckUpdateError::ComponentState)?;
    verify_manifest(&manifest, endpoint, &installed).map_err(CheckUpdateError::Manifest)
}

fn update_check_error_detail(error: &CheckUpdateError) -> String {
    match error {
        CheckUpdateError::Download(error) => update_http_error_detail(error),
        CheckUpdateError::ComponentState(error) => component_version_error_detail(error),
        CheckUpdateError::Manifest(error) => update_manifest_error_detail(error),
    }
}

fn update_manifest_error_detail(error: &UpdateError) -> String {
    match error {
        UpdateError::ClientNotConfigured => {
            t("The official update channel is not configured in this build")
        }
        UpdateError::InvalidCompiledPublicKey => {
            t("This build contains an invalid update verification key")
        }
        UpdateError::ManifestTooLarge
        | UpdateError::InvalidEnvelope
        | UpdateError::MissingTrustedSignature
        | UpdateError::InvalidSignature
        | UpdateError::InvalidPayload
        | UpdateError::UnsupportedSchema(_)
        | UpdateError::WrongProduct
        | UpdateError::WrongChannel
        | UpdateError::InvalidReleaseId
        | UpdateError::UnsupportedUpdaterProtocol(_)
        | UpdateError::InvalidVersion
        | UpdateError::InvalidVersionRequirement
        | UpdateError::DuplicateComponent(_)
        | UpdateError::InvalidArtifactUrl
        | UpdateError::InvalidArtifactSize
        | UpdateError::InvalidArtifactHash => {
            t("The official update manifest is invalid or incompatible with this build")
        }
    }
}

fn update_http_error_detail(error: &update_http::HttpError) -> String {
    match error {
        update_http::HttpError::InvalidUrl(_) => t("The official update URL is invalid"),
        update_http::HttpError::Transport { source, .. } => tf(
            "The update server connection failed (system error {})",
            &[&system_error_code(source)],
        ),
        update_http::HttpError::Status(status) => {
            tf("The update server returned HTTP {}", &[&status.to_string()])
        }
        update_http::HttpError::ResponseTooLarge {
            maximum,
            received_at_least,
        } => tf(
            "The update response exceeds the allowed size: maximum {} bytes, received at least {} bytes",
            &[&maximum.to_string(), &received_at_least.to_string()],
        ),
        update_http::HttpError::PackageLargerThanManifest {
            expected,
            received_at_least,
        } => tf(
            "The official update package is inconsistent with its signed manifest: the manifest declares {} bytes, but the server returned at least {} bytes. Please retry later or report this release.",
            &[&expected.to_string(), &received_at_least.to_string()],
        ),
        update_http::HttpError::SizeMismatch { expected, actual } => tf(
            "The official update package is incomplete: the signed manifest declares {} bytes, but the server returned {} bytes. Please retry later or report this release.",
            &[&expected.to_string(), &actual.to_string()],
        ),
        update_http::HttpError::File(error) => tf(
            "The update file operation failed (system error {})",
            &[&system_error_code(error)],
        ),
    }
}

fn component_version_error_detail(error: &ComponentVersionError) -> String {
    match error {
        ComponentVersionError::File(error) => tf(
            "The installed component state could not be read (system error {})",
            &[&system_error_code(error)],
        ),
        ComponentVersionError::TooLarge | ComponentVersionError::Invalid(_) => {
            t("The installed component version state is invalid")
        }
    }
}

fn update_download_error_detail(error: &PrepareUpdateError) -> String {
    match error {
        PrepareUpdateError::Download(error) => update_http_error_detail(error),
        PrepareUpdateError::HashMismatch => t(
            "The official update package SHA-256 does not match its signed manifest. Download it again; if the problem continues, report this release.",
        ),
        PrepareUpdateError::Archive(_)
        | PrepareUpdateError::UnsafeArchivePath(_)
        | PrepareUpdateError::UnsupportedArchivePath(_)
        | PrepareUpdateError::TooManyArchiveEntries
        | PrepareUpdateError::ArchiveTooLarge
        | PrepareUpdateError::MissingApplication
        | PrepareUpdateError::MissingUpdater
        | PrepareUpdateError::MissingEquipmentPlugin
        | PrepareUpdateError::UnexpectedPluginArchiveContents
        | PrepareUpdateError::InvalidEquipmentPluginSize => {
            t("The official update package has an invalid structure")
        }
        PrepareUpdateError::File(error) => tf(
            "The update files could not be prepared (system error {})",
            &[&system_error_code(error)],
        ),
        PrepareUpdateError::Transaction(_) => t("The update transaction could not be prepared"),
    }
}

fn updater_launch_error_detail(error: &std::io::Error) -> String {
    tf(
        "The updater process could not be started (system error {})",
        &[&system_error_code(error)],
    )
}

fn plugin_update_error_detail(error: &InstallPluginUpdateError) -> String {
    match error {
        InstallPluginUpdateError::WrongComponent => {
            t("The prepared update component does not match")
        }
        InstallPluginUpdateError::HashMismatch => {
            t("The prepared equipment plugin hash does not match")
        }
        InstallPluginUpdateError::File(error) => tf(
            "The equipment plugin update file operation failed (system error {})",
            &[&system_error_code(error)],
        ),
        InstallPluginUpdateError::State(_) | InstallPluginUpdateError::Rollback(_) => {
            t("The equipment plugin update could not be completed")
        }
        InstallPluginUpdateError::Deployment(error) => equipment_plugin_update_error_detail(error),
    }
}

fn equipment_plugin_update_error_detail(error: &EquipmentPluginDeploymentError) -> String {
    match error {
        EquipmentPluginDeploymentError::GameRunning => {
            t("Close HTGame.exe before changing the equipment plugin.")
        }
        EquipmentPluginDeploymentError::GameProcessProbe(_) => {
            t("Equipment plugin status check failed")
        }
        EquipmentPluginDeploymentError::GameInstallationNotFound => {
            t("Game installation not detected")
        }
        EquipmentPluginDeploymentError::Registry(_) => t("Game installation not detected"),
        EquipmentPluginDeploymentError::PluginSourceNotFound => {
            t("Equipment plugin file plugins/dwmapi.dll was not found")
        }
        EquipmentPluginDeploymentError::ConflictingDwmapi => t(
            "The game directory already contains a dwmapi.dll that is not managed by this tool. Remove the conflicting mod manually before enabling this plugin.",
        ),
        EquipmentPluginDeploymentError::InstalledPluginChanged => t(
            "The installed dwmapi.dll or its ownership marker changed outside this tool. Check the game directory manually before trying again.",
        ),
        EquipmentPluginDeploymentError::FileSystem(_) => {
            t("The equipment plugin files could not be updated")
        }
    }
}

fn system_error_code(error: &std::io::Error) -> String {
    error
        .raw_os_error()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_update_check_is_due_at_startup() {
        let before = Instant::now();
        let state = UpdateClientState::new(&UiConfig::default());
        let deadline = state
            .next_auto_check
            .expect("automatic update checks are enabled by default");

        assert!(deadline >= before);
        assert!(deadline <= Instant::now());
    }

    #[test]
    fn disabled_automatic_update_check_has_no_startup_deadline() {
        let config = UiConfig {
            auto_check_updates: false,
            ..UiConfig::default()
        };

        assert!(UpdateClientState::new(&config).next_auto_check.is_none());
    }

    #[test]
    fn prepared_update_blocks_a_new_check_after_an_install_failure() {
        let mut state = UpdateClientState::new(&UiConfig::default());
        state.status = UpdateStatus::Failed {
            stage: UpdateFailureStage::Install,
            detail: "test failure".to_owned(),
        };
        state.prepared = Some(PreparedUpdate::EquipmentPlugin {
            version: Version::parse("0.3.6").unwrap(),
            transaction_id: "plugin-0.3.6-test".to_owned(),
            staging_dir: PathBuf::from("staging"),
            plugin_path: PathBuf::from("staging/plugins/dwmapi.dll"),
            plugin_sha256: [7; 32],
        });

        assert!(!state.can_check());
    }

    #[test]
    fn update_error_details_do_not_expose_internal_error_text() {
        let manifest = update_manifest_error_detail(&UpdateError::DuplicateComponent(
            "private-component".to_owned(),
        ));
        let archive = update_download_error_detail(&PrepareUpdateError::Archive(
            "private-archive-detail".to_owned(),
        ));
        let plugin = plugin_update_error_detail(&InstallPluginUpdateError::State(
            "private-state-detail".to_owned(),
        ));

        assert!(!manifest.contains("private-component"));
        assert!(!archive.contains("private-archive-detail"));
        assert!(!plugin.contains("private-state-detail"));
    }
}
