//! Shared crate root for the two binaries: the egui GUI (`nte-dps-tool`, feature
//! `gui`) and the headless stdio sidecar (`nte-core`, feature `cli`). Module
//! feature gating happens here; everything below `engine`/`storage`/`platform`
//! stays UI-free so both frontends reuse the same capture and parsing core.

#[cfg(feature = "cli")]
pub mod api;
#[cfg(feature = "gui")]
pub mod app;
#[cfg(feature = "cli")]
pub mod cli;
pub mod core;
pub mod engine;
pub mod platform;
pub mod storage;
// Debug/maintenance helpers (character editor, encrypted INI, diagnostics,
// resource audit) back the GUI console pages only; nothing in the CLI path
// uses them, so they stay out of the headless build.
#[cfg(feature = "gui")]
pub mod support;

#[cfg(feature = "gui")]
mod gui_entry {
    use crate::app::{self, DpsApp};
    use crate::storage;
    use anyhow::Result;
    use eframe::egui;
    use std::path::Path;
    use std::sync::Arc;

    pub fn run_gui() -> Result<()> {
        env_logger::init();
        install_panic_log();
        let (ui_config, config_warning) = storage::config::load();
        // Load the active locale before the first frame so the UI never flashes English keys.
        storage::i18n::set_language(ui_config.language);
        storage::ability_names::init(ui_config.language);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("NTE DPS TOOL")
                // Reopen at the last dragged size (native edge-resize via BeginResize grips), falling
                // back to the base size on first run. The window stays borderless; resize is driven by
                // the custom grips in `window_resize_grips`.
                .with_inner_size(
                    ui_config
                        .main_window_size
                        .map(egui::Vec2::from)
                        .unwrap_or(app::MAIN_WINDOW_BASE_SIZE),
                )
                .with_min_inner_size(egui::Vec2::from(storage::config::MAIN_WINDOW_MIN_SIZE))
                .with_decorations(false)
                .with_resizable(true)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_icon(Arc::new(app_icon()))
                .with_window_level(if ui_config.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }),
            // Render through wgpu, not glow/OpenGL. On this transparent, borderless
            // window the NVIDIA OpenGL driver loses the GL context ("GPU has been
            // disconnected", error 10) during the native corner-resize modal loop,
            // killing the process with no Rust panic — the diagonal-resize flash-crash
            // (egui #4061 / #5460). wgpu avoids that driver path entirely. Default::default
            // already resolves to Wgpu once glow is off, but pin it so a re-added glow
            // feature can't silently switch the backend back.
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: wgpu_options_with_transparent_dx12(),
            ..Default::default()
        };

        eframe::run_native(
            "NTE DPS TOOL",
            options,
            Box::new(move |cc| Ok(Box::new(DpsApp::new(cc, ui_config, config_warning)))),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    /// wgpu's normal DX12 swapchain is created directly from the window's HWND,
    /// and that kind of swapchain never reports a `CompositeAlphaMode` with real
    /// transparency on Windows regardless of window flags — the HUD's transparent
    /// viewport renders as solid black instead of see-through (wgpu #1375,
    /// #7108). Since wgpu 27 there's a builtin fix: `Dx12SwapchainKind::DxgiFromVisual`
    /// makes wgpu-hal wrap the swapchain in a `DirectComposition` visual it creates
    /// and manages internally, which *does* support alpha compositing with the
    /// desktop (wgpu PR #7550). This needs the DX12 backend specifically — Vulkan
    /// has no equivalent option and was tried first; it also reports Opaque-only.
    fn wgpu_options_with_transparent_dx12() -> eframe::egui_wgpu::WgpuConfiguration {
        let mut options = eframe::egui_wgpu::WgpuConfiguration::default();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut options.wgpu_setup {
            create_new.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
            create_new
                .instance_descriptor
                .backend_options
                .dx12
                .presentation_system = eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        options
    }

    fn app_icon() -> egui::IconData {
        let bytes = storage::resource::read_resource_bytes(Path::new("res/icons/app-icon.png"))
            .expect("application icon resource must be available");
        let image = image::load_from_memory(bytes.as_ref())
            .expect("embedded application icon must be valid")
            .into_rgba8();
        let (width, height) = image.dimensions();
        egui::IconData {
            rgba: image.into_raw(),
            width,
            height,
        }
    }

    fn install_panic_log() {
        std::panic::set_hook(Box::new(|info| {
            let directory = storage::paths::capture_log_dir();
            let _ = std::fs::create_dir_all(&directory);
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let path = directory.join(format!("nte_panic_{timestamp}.log"));
            let backtrace = std::backtrace::Backtrace::force_capture();
            let _ = std::fs::write(path, format!("{info}\n\n{backtrace}\n"));
        }));
    }
}

#[cfg(feature = "gui")]
pub use gui_entry::run_gui;
