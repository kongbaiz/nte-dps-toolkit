use super::*;

use semver::Version;

use crate::core::update::{
    AvailableComponentUpdate, MAX_MANIFEST_BYTES, UpdateComponent, UpdateEndpoint, UpdateError,
    verify_manifest,
};
use crate::platform::update_http;
use crate::storage::update::{self as update_storage, PreparedUpdate};

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
    CheckFinished(Result<Vec<AvailableComponentUpdate>, String>),
    DownloadProgress {
        component: UpdateComponent,
        downloaded: u64,
        total: u64,
    },
    DownloadFinished(Result<PreparedUpdate, String>),
    PluginInstallFinished(Result<(Version, Vec<AvailableComponentUpdate>), String>),
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
            if let Err(error) = update_storage::mark_update_healthy_from_environment() {
                eprintln!("Failed to write update health marker: {error}");
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
                UpdateWorkerEvent::CheckFinished(Err(detail)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Check,
                        detail,
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
                UpdateWorkerEvent::DownloadFinished(Err(detail)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Download,
                        detail,
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
                UpdateWorkerEvent::PluginInstallFinished(Err(detail)) => {
                    self.update_client.status = UpdateStatus::Failed {
                        stage: UpdateFailureStage::Install,
                        detail,
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
                    detail: error.to_string(),
                };
                return;
            }
        };
        self.update_client.status = UpdateStatus::Checking;
        let sender = self.update_client.sender.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            let result = check_for_update(&endpoint).map_err(|error| error.to_string());
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
            })
            .map_err(|error| error.to_string());
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
                            detail: error.to_string(),
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
                        .map(|()| (version, remaining))
                        .map_err(|error| error.to_string());
                    let _ = sender.send(UpdateWorkerEvent::PluginInstallFinished(result));
                    repaint.request_repaint();
                });
            }
        }
    }
}

fn check_for_update(
    endpoint: &UpdateEndpoint,
) -> Result<Vec<AvailableComponentUpdate>, Box<dyn std::error::Error + Send + Sync>> {
    let manifest = update_http::get_bytes(&endpoint.manifest_url, MAX_MANIFEST_BYTES)?;
    let current_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("Cargo package version is valid semantic versioning");
    let installed = update_storage::installed_component_versions(current_version)?;
    Ok(verify_manifest(&manifest, endpoint, &installed)?)
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
}
