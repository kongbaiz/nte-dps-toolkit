use super::*;

fn key_pressed_without_repeat(events: &[egui::Event], key: egui::Key) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: event_key,
                pressed: true,
                repeat: false,
                ..
            } if *event_key == key
        )
    })
}

impl DpsApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        ui_config: UiConfig,
        config_warning: Option<String>,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        configure_style(
            &cc.egui_ctx,
            ui_config.dark_mode,
            ui_config.accent,
            ui_config.density,
            ui_config.reduce_motion,
        );
        let ui_config = ui_config.sanitized();
        let (hotkey, hotkey_receiver) = HotkeyHandle::start(
            cc.egui_ctx.clone(),
            ui_config.passthrough_hotkey,
            ui_config.global_hotkeys,
        );
        let (sender, receiver) = unbounded();
        let (resource_audit_sender, resource_audit_receiver) = unbounded();
        let (diagnostics_sender, diagnostics_receiver) = unbounded();
        let data_root = data_root();
        let characters_path = data_root.join(CHARACTER_DATA_PATH);
        let (mut characters, character_load_error) =
            match load_characters(characters_path.as_path()) {
                Ok(characters) => (characters, None),
                Err(error) => (
                    HashMap::new(),
                    Some(tf(
                        "Failed to load character data ({}): {}",
                        &[&characters_path.display().to_string(), &error.to_string()],
                    )),
                ),
            };
        fill_missing_character_colors_from_avatars(&mut characters, &data_root);
        let characters = Arc::new(characters);
        let equipment_catalog_path = data_root.join(EQUIPMENT_CATALOG_PATH);
        let (equipment_catalog, equipment_load_error) =
            match load_equipment_catalog(&equipment_catalog_path) {
                Ok(catalog) => (catalog, None),
                Err(error) => {
                    eprintln!("Failed to load Console equipment data: {error:#}");
                    (
                        EquipmentCatalog::default(),
                        Some(t(
                            "Failed to load Console equipment data; cards will use placeholders",
                        )),
                    )
                }
            };
        let equipment_catalog = Arc::new(equipment_catalog);
        let abyss_overview = AbyssOverviewState::load();
        let history = HistoryState::load();
        // Decode the texture sets (avatars, attribute icons, damage digits,
        // reaction glyphs, monster portraits) on a background thread so the window
        // appears immediately instead of blocking on ~6 MB of PNG decode. The maps
        // start empty; every texture lookup in the draw code already falls back when
        // a key is missing, so rows show their color/initial placeholder until the
        // sets stream in. The loader repaints after each set so an idle UI wakes to
        // pick them up.
        let (texture_load_sender, texture_load_receiver) = unbounded();
        {
            let ctx = cc.egui_ctx.clone();
            let root = data_root.clone();
            let avatar_characters = Arc::clone(&characters);
            let equipment_catalog = Arc::clone(&equipment_catalog);
            let monster_ids = abyss_overview.monster_ids();
            thread::spawn(move || {
                let send = |load: TextureLoad| {
                    if texture_load_sender.send(load).is_ok() {
                        ctx.request_repaint();
                    }
                };
                send(TextureLoad::Avatars(load_character_avatars(
                    &ctx,
                    &root,
                    &avatar_characters,
                )));
                send(TextureLoad::Attributes(load_attribute_icons(&ctx, &root)));
                send(TextureLoad::DamageDigits(load_damage_digit_textures(
                    &ctx, &root,
                )));
                send(TextureLoad::Reactions(load_reaction_text_textures(
                    &ctx, &root,
                )));
                send(TextureLoad::Monsters(load_monster_textures(
                    &ctx,
                    &root,
                    &monster_ids,
                )));
                send(TextureLoad::Equipment(load_equipment_textures(
                    &ctx,
                    &root,
                    &equipment_catalog,
                )));
            });
        }
        let character_editor =
            CharacterEditorState::load(&characters_path).unwrap_or_else(|error| {
                CharacterEditorState {
                    document: serde_json::json!({"version": 2, "characters": {}}),
                    selected_id: None,
                    form: CharacterEditForm::default(),
                    search: String::new(),
                    new_id: String::new(),
                    dirty: false,
                    message: error,
                    cancel_selection: None,
                }
            });
        // Probe the capture environment (Npcap device list + HTGame.exe NIC) on a
        // background thread so the window appears immediately instead of blocking on
        // device enumeration. `start_live` re-runs `refresh_game_network` on every
        // capture start, so this startup probe only seeds the initial status and
        // device dropdown and can never gate capturing. Results arrive via
        // `drain_device_detection`, guarded so a late result never clobbers a live
        // capture or a user-initiated refresh.
        let manual_capture_device = ui_config.manual_capture_device.clone();
        let devices: Vec<CaptureDevice> = Vec::new();
        let selected_device: usize = 0;
        let game_network: Option<GameNetwork> = None;
        let game_process_detected = false;
        let local_ip = String::new();
        let status = t("Detecting the capture environment...");
        let diagnostic: Option<String> = None;
        let (device_detection_sender, device_detection_receiver) = unbounded();
        {
            let ctx = cc.egui_ctx.clone();
            let manual = manual_capture_device.clone();
            let character_error = character_load_error.clone();
            thread::spawn(move || {
                let detection =
                    detect_capture_environment(manual.as_deref(), character_error.as_deref());
                if device_detection_sender.send(detection).is_ok() {
                    ctx.request_repaint();
                }
            });
        }
        let (game_process_monitor_sender, game_process_monitor_receiver) = unbounded();
        let (game_process_monitor_stop, game_process_monitor_stop_receiver) = unbounded();
        let game_process_monitor_thread = {
            let ctx = cc.egui_ctx.clone();
            thread::spawn(move || {
                let mut previous = None;
                loop {
                    let result = game_process_is_running();
                    if previous.as_ref() != Some(&result) {
                        previous = Some(result.clone());
                        if game_process_monitor_sender.send(result).is_ok() {
                            ctx.request_repaint();
                        }
                    }
                    match game_process_monitor_stop_receiver.recv_timeout(Duration::from_secs(2)) {
                        Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    }
                }
            })
        };
        let startup_errors = [config_warning, character_load_error, equipment_load_error]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let startup_error = (!startup_errors.is_empty()).then(|| startup_errors.join("\n"));
        let last_status_toast = status.clone();
        Self {
            characters,
            avatar_textures: HashMap::new(),
            attribute_textures: HashMap::new(),
            monster_textures: HashMap::new(),
            damage_digit_textures: HashMap::new(),
            reaction_textures: HashMap::new(),
            equipment_catalog,
            equipment_textures: HashMap::new(),
            kongmu_ui: KongmuUiState::default(),
            state: CombatState::default(),
            combat_active: false,
            last_combat_timestamp: None,
            last_combat_activity: None,
            combat_start_generation: 0,
            combat_end_generation: 0,
            hidden_character_ids: HashSet::new(),
            selected_abyss_half: AbyssHalf::First,
            abyss_compact_mode: false,
            hud_mode: false,
            hud_size_key: None,
            hud_config: ui_config.hud.clone(),
            abyss_overview,
            history,
            resource_audit: ResourceAuditState::default(),
            abyss_overview_open: false,
            abyss_overview_corner_applied: false,
            hit_detail_char_id: None,
            hit_detail_filter: HitDetailFilter::All,
            hit_detail_skill_filter: String::new(),
            hit_detail_corner_applied: false,
            team_hit_detail_open: false,
            team_hit_detail_filter: HitDetailFilter::All,
            team_hit_detail_corner_applied: false,
            character_hit_cache: HitDetailCache::default(),
            team_hit_cache: HitDetailCache::default(),
            skill_summary_cache: SkillSummaryCache::default(),
            timeline_cache: TimelineCache::default(),
            timeline_view: TimelineViewState::default(),
            skill_breakdown_cache: SkillBreakdownCache::default(),
            selected_timeline_char: None,
            selected_skill_breakdown_char: None,
            detail_last_scroll_activity: None,
            devices,
            selected_device,
            manual_capture_device,
            local_ip,
            game_process_detected,
            game_network,
            filter: "udp".to_owned(),
            active_capture_filter: None,
            capture_quality_source: CaptureQualitySource::Unknown,
            include_incoming: true,
            server_damage_calibration: ui_config.server_damage_calibration,
            dps_time_mode: ui_config.dps_time_mode,
            timeline_bucket_seconds: ui_config.timeline_bucket_seconds,
            timeline_dps_view_mode: ui_config.timeline_dps_view_mode,
            capture: None,
            raw_capture: None,
            replay_stop: None,
            replay_thread: None,
            sender,
            receiver,
            resource_audit_sender,
            resource_audit_receiver,
            resource_audit_thread: None,
            diagnostics_sender,
            diagnostics_receiver,
            diagnostics_thread: None,
            diagnostics_report: None,
            diagnostics_running: false,
            texture_load_receiver,
            device_detection_receiver,
            awaiting_device_detection: true,
            game_process_monitor_receiver,
            game_process_monitor_stop,
            game_process_monitor_thread: Some(game_process_monitor_thread),
            game_process_monitor_error: None,
            capture_log_stats: None,
            paused_events: VecDeque::new(),
            dropped_debug_packets: 0,
            status,
            last_status_toast,
            status_toasts: VecDeque::new(),
            undo_states: HashMap::new(),
            next_toast_id: 1,
            diagnostic,
            last_error: startup_error,
            last_error_action: None,
            last_error_viewport: egui::ViewportId::ROOT,
            console_open: false,
            console_corner_applied: false,
            console_sidebar_manually_collapsed: false,
            console_tab: ConsoleTab::default(),
            command_palette: CommandPaletteState::default(),
            debug_only_hits: false,
            debug_search: String::new(),
            character_editor,
            encrypted_ini_editor: EncryptedIniEditorState::default(),
            paused: false,
            language: ui_config.language,
            dark_mode: ui_config.dark_mode,
            accent: ui_config.accent,
            density: ui_config.density,
            reduce_motion: ui_config.reduce_motion,
            always_on_top: ui_config.always_on_top,
            mouse_passthrough: false,
            passthrough_notice: None,
            passthrough_hotkey: ui_config.passthrough_hotkey,
            global_hotkeys: ui_config.global_hotkeys,
            recording_hotkey: None,
            hotkey_hook_available: false,
            onboarding_done: ui_config.onboarding_done,
            onboarding_step: 0,
            onboarding_hotkey_preview_generation: 0,
            console_sidebar_migration_seen: ui_config.console_sidebar_migration_seen,
            opacity: ui_config.opacity,
            applied_opacity: None,
            corner_applied_hwnd: None,
            main_window_size: ui_config
                .main_window_size
                .map(egui::Vec2::from)
                .unwrap_or(MAIN_WINDOW_BASE_SIZE),
            abyss_window_size: ui_config
                .abyss_window_size
                .map(egui::Vec2::from)
                .unwrap_or(ABYSS_WINDOW_BASE_SIZE),
            hit_detail_window_size: ui_config
                .hit_detail_window_size
                .map(egui::Vec2::from)
                .unwrap_or(HIT_DETAIL_WINDOW_BASE_SIZE),
            team_hit_detail_window_size: ui_config
                .team_hit_detail_window_size
                .map(egui::Vec2::from)
                .unwrap_or(TEAM_HIT_DETAIL_WINDOW_BASE_SIZE),
            console_window_size: ui_config
                .console_window_size
                .map(egui::Vec2::from)
                .unwrap_or(CONSOLE_WINDOW_BASE_SIZE),
            main_size_restore_frames: 0,
            applied_main_min_size: egui::Vec2::ZERO,
            // eframe may replace the context style after app construction.
            style_key_applied: None,
            session_epoch: 0,
            opacity_reapply_frames: 4,
            theme_transition_from: None,
            pending_file_dialog: None,
            active_import: None,
            engine_task_viewport: None,
            pending_confirmation: None,
            pending_confirmation_viewport: egui::ViewportId::ROOT,
            saved_ui_config: ui_config,
            pending_ui_config: None,
            ui_config_path: config::config_path(),
            native_file_drop: NativeFileDrop::new(),
            last_dropped_file: None,
            hotkey_receiver,
            hotkey,
        }
    }

    pub(crate) fn stop_engine(&mut self) {
        if let Some(mut capture) = self.capture.take() {
            capture.stop();
        }
        if let Some(stop) = self.replay_stop.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(thread) = self.replay_thread.take() {
            let _ = thread.join();
        }
        // All producers are joined, so every queued event belongs to the stopped task.
        // Apply them now to prevent a delayed CaptureStopped from affecting the next task.
        self.drain_pending_events();
        self.active_import = None;
        self.engine_task_viewport = None;
    }

    pub(crate) fn reset_combat_session(&mut self) {
        self.state.clear();
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.reset_combat_view_state();
    }

    pub(crate) fn reset_combat_view_state(&mut self) {
        self.combat_active = false;
        self.last_combat_timestamp = None;
        self.last_combat_activity = None;
        self.hidden_character_ids.clear();
        self.kongmu_ui.invalidate_inventory();
        self.selected_abyss_half = AbyssHalf::First;
        self.abyss_compact_mode = false;
        self.hit_detail_char_id = None;
        self.hit_detail_filter = HitDetailFilter::All;
        self.hit_detail_skill_filter.clear();
        self.hit_detail_corner_applied = false;
        self.team_hit_detail_open = false;
        self.team_hit_detail_filter = HitDetailFilter::All;
        self.team_hit_detail_corner_applied = false;
        self.character_hit_cache = HitDetailCache::default();
        self.team_hit_cache = HitDetailCache::default();
        self.skill_summary_cache = SkillSummaryCache::default();
        self.timeline_cache = TimelineCache::default();
        self.timeline_view = TimelineViewState::default();
        self.skill_breakdown_cache = SkillBreakdownCache::default();
        self.selected_timeline_char = None;
        self.selected_skill_breakdown_char = None;
        self.detail_last_scroll_activity = None;
        self.paused = false;
        self.paused_events.clear();
        self.dropped_debug_packets = 0;
        self.capture_quality_source = CaptureQualitySource::Unknown;
    }

    pub(crate) fn has_session_data(&self) -> bool {
        !self.state.hits.is_empty()
            || !self.state.packets.is_empty()
            || !self.state.stats.is_empty()
            || !self.state.empty_curtain.is_empty()
            || self.state.abyss.is_active()
    }

    pub(crate) fn request_reset_combat_session(&mut self, ctx: &egui::Context) {
        if self.capture.is_none()
            && self.replay_thread.is_none()
            && !self.has_session_data()
            && let Some(id) = self.latest_combat_undo_id()
        {
            self.apply_undo(id, ctx.viewport_id());
            return;
        }
        if self.capture.is_some() || self.replay_thread.is_some() {
            self.stop_engine();
        }
        if !self.has_session_data() {
            self.reset_combat_session();
            self.status = t("Stats reset");
            return;
        }
        let previous = CombatUndoSnapshot {
            state: std::mem::take(&mut self.state),
            capture_quality_source: self.capture_quality_source,
            timeline_view: std::mem::take(&mut self.timeline_view),
            hidden_character_ids: std::mem::take(&mut self.hidden_character_ids),
            selected_abyss_half: self.selected_abyss_half,
            abyss_compact_mode: self.abyss_compact_mode,
        };
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.reset_combat_view_state();
        self.status = t("Stats reset");
        let reset_message = if self.global_hotkeys.enabled {
            self.global_hotkeys
                .binding(GlobalHotkeyAction::ResetSession)
                .map(|binding| {
                    tf(
                        "Session reset · press {} again or use Undo within 5 seconds",
                        &[&binding.label()],
                    )
                })
                .unwrap_or_else(|| t("Session reset · use Undo within 5 seconds"))
        } else {
            t("Session reset · use Undo within 5 seconds")
        };
        let toast_viewport = self.interactive_viewport_for(ctx);
        self.push_undo_toast(
            toast_viewport,
            reset_message,
            UndoState::CombatSession(Box::new(previous)),
        );
    }

    pub(crate) fn preferred_interactive_viewport(&self, ctx: &egui::Context) -> egui::ViewportId {
        if ctx.viewport_id() == egui::ViewportId::ROOT && (self.hud_mode || self.mouse_passthrough)
        {
            console_viewport_id()
        } else {
            ctx.viewport_id()
        }
    }

    pub(crate) fn interactive_viewport_for(&mut self, ctx: &egui::Context) -> egui::ViewportId {
        let viewport = self.preferred_interactive_viewport(ctx);
        if viewport == console_viewport_id() {
            self.console_open = true;
            self.console_corner_applied = false;
        }
        viewport
    }

    pub(crate) fn request_start_live(&mut self, ctx: &egui::Context) {
        if self.has_session_data() {
            let viewport = self.interactive_viewport_for(ctx);
            self.request_confirmation_for(viewport, ConfirmationAction::StartLive);
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
        } else {
            let viewport = self.preferred_interactive_viewport(ctx);
            if !self.start_live_for(viewport) {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            }
        }
    }

    pub(crate) fn request_import_file(&mut self, kind: DebugImportKind, path: PathBuf) {
        self.request_import_file_for(kind, path, egui::ViewportId::ROOT);
    }

    pub(crate) fn request_import_file_for(
        &mut self,
        kind: DebugImportKind,
        path: PathBuf,
        viewport: egui::ViewportId,
    ) {
        let action = match kind {
            DebugImportKind::Pcapng => ConfirmationAction::ImportPcapng(path),
            DebugImportKind::CaptureJson => ConfirmationAction::ImportCaptureJson(path),
            DebugImportKind::EncryptedIni => {
                self.load_encrypted_ini_for(path, viewport);
                return;
            }
        };
        if self.has_session_data() || self.capture.is_some() || self.replay_thread.is_some() {
            self.request_confirmation_for(viewport, action);
        } else {
            self.run_confirmation_action_for(action, viewport);
        }
    }

    pub(crate) fn run_confirmation_action_for(
        &mut self,
        action: ConfirmationAction,
        viewport: egui::ViewportId,
    ) {
        match action {
            ConfirmationAction::StartLive => {
                self.start_live_for(viewport);
            }
            ConfirmationAction::ImportPcapng(path) => self.start_pcapng_import_for(path, viewport),
            ConfirmationAction::ImportCaptureJson(path) => {
                self.start_capture_json_import_for(path, viewport);
            }
            ConfirmationAction::ClearEncryptedIni => {
                self.encrypted_ini_editor = EncryptedIniEditorState::default();
                self.status = t("Encrypted INI editor cleared");
            }
            ConfirmationAction::ReloadEncryptedIni(path) => {
                self.load_encrypted_ini_for(path, viewport)
            }
            ConfirmationAction::ClearCaptureLogs => self.clear_capture_logs_now(),
        }
    }

    /// Lazily (re)scan `logs/` for raw capture files so the settings panel can show
    /// disk usage without doing file I/O every frame.
    pub(crate) fn refresh_capture_log_stats(&mut self) {
        self.capture_log_stats = Some(capture_logs::scan_capture_logs(Path::new(
            capture_logs::CAPTURE_LOG_DIR,
        )));
    }

    /// Delete the raw capture logs. The active capture's file is held open by the
    /// OS, so it fails to delete and is reported as "占用中" rather than removed.
    fn clear_capture_logs_now(&mut self) {
        let outcome = capture_logs::clear_capture_logs(Path::new(capture_logs::CAPTURE_LOG_DIR));
        self.refresh_capture_log_stats();
        self.status = if outcome.deleted == 0 && outcome.failed == 0 {
            t("No capture files to clear")
        } else if outcome.failed > 0 {
            tf(
                "Cleared {} capture files (freed {}); {} in use and not deleted",
                &[
                    &outcome.deleted.to_string(),
                    &capture_logs::format_bytes(outcome.freed_bytes),
                    &outcome.failed.to_string(),
                ],
            )
        } else {
            tf(
                "Cleared {} capture files, freed {}",
                &[
                    &outcome.deleted.to_string(),
                    &capture_logs::format_bytes(outcome.freed_bytes),
                ],
            )
        };
    }

    pub(crate) fn request_confirmation_for(
        &mut self,
        viewport: egui::ViewportId,
        action: ConfirmationAction,
    ) {
        self.close_command_palette_for(viewport);
        self.pending_confirmation = Some(action);
        self.pending_confirmation_viewport = viewport;
    }

    pub(crate) fn set_last_error(
        &mut self,
        message: impl Into<String>,
        action: Option<ErrorAction>,
    ) {
        self.set_last_error_for(egui::ViewportId::ROOT, message, action);
    }

    pub(crate) fn set_last_error_for(
        &mut self,
        viewport: egui::ViewportId,
        message: impl Into<String>,
        action: Option<ErrorAction>,
    ) {
        self.close_command_palette_for(viewport);
        self.last_error = Some(message.into());
        self.last_error_action = action;
        self.last_error_viewport = viewport;
    }

    pub(crate) fn set_last_error_in(
        &mut self,
        ctx: &egui::Context,
        message: impl Into<String>,
        action: Option<ErrorAction>,
    ) {
        self.set_last_error_for(ctx.viewport_id(), message, action);
    }

    pub(crate) fn clear_last_error(&mut self) {
        self.last_error = None;
        self.last_error_action = None;
    }

    pub(crate) fn set_passthrough_hotkey(&mut self, hotkey: PassthroughHotkey) {
        if self.passthrough_hotkey == hotkey {
            return;
        }
        self.passthrough_hotkey = hotkey;
        self.hotkey.set_passthrough_hotkey(hotkey);
        self.status = tf("Mouse passthrough hotkey switched to {}", &[hotkey.label()]);
    }

    pub(crate) fn set_global_hotkeys(&mut self, hotkeys: GlobalHotkeys) {
        self.global_hotkeys = hotkeys.sanitized();
        self.hotkey.set_global_hotkeys(self.global_hotkeys);
    }

    pub(crate) fn set_recording_hotkey(&mut self, action: Option<GlobalHotkeyAction>) {
        self.recording_hotkey = action;
        self.hotkey.set_recording(action.is_some());
    }

    pub(crate) fn drain_hotkeys(&mut self, ctx: &egui::Context) {
        self.handle_local_hotkeys(ctx);
        while let Ok(event) = self.hotkey_receiver.try_recv() {
            match event {
                HotkeyEvent::TogglePassthrough => self.toggle_mouse_passthrough(ctx),
                HotkeyEvent::GlobalAction(action) => self.execute_global_hotkey(ctx, action),
                HotkeyEvent::ToggleCommandPalette => self.toggle_command_palette(ctx),
                #[cfg(not(feature = "no_debug"))]
                HotkeyEvent::ToggleDebug => self.toggle_debug_console(),
                HotkeyEvent::HookInstalled => self.hotkey_hook_available = true,
                HotkeyEvent::HookInstallFailed { error } => {
                    self.hotkey_hook_available = false;
                    if self.mouse_passthrough {
                        self.set_mouse_passthrough(ctx, false);
                    }
                    let message = tf(
                        "Could not install the global keyboard hook (error {})",
                        &[&error.to_string()],
                    );
                    self.diagnostic = Some(message.clone());
                    self.push_status_toast(
                        egui::ViewportId::ROOT,
                        message,
                        ToastTone::Danger,
                        STATUS_TOAST_DURATION,
                        None,
                    );
                }
            }
        }
    }

    pub(crate) fn handle_local_hotkeys(&mut self, ctx: &egui::Context) {
        if self.recording_hotkey.is_some() {
            return;
        }
        let (modifiers, pressed_keys) = ctx.input(|input| {
            (
                input.modifiers,
                HotkeyKey::all()
                    .iter()
                    .copied()
                    .filter(|key| key_pressed_without_repeat(&input.events, key.egui_key()))
                    .collect::<Vec<_>>(),
            )
        });
        let passthrough_key = self.passthrough_hotkey.egui_key();
        if self
            .passthrough_hotkey
            .matches_egui(modifiers, passthrough_key)
            && ctx.input(|input| key_pressed_without_repeat(&input.events, passthrough_key))
        {
            self.toggle_mouse_passthrough(ctx);
        }
        if self.global_hotkeys.enabled
            && let Some(action) = GlobalHotkeyAction::all().iter().copied().find(|action| {
                self.global_hotkeys.binding(*action).is_some_and(|binding| {
                    pressed_keys
                        .iter()
                        .any(|key| binding.matches_egui(modifiers, key.egui_key()))
                })
            })
        {
            self.execute_global_hotkey(ctx, action);
        }
        let command_palette_pressed = ctx.input(|input| {
            input.modifiers.ctrl
                && !input.modifiers.alt
                && !input.modifiers.shift
                && key_pressed_without_repeat(&input.events, egui::Key::K)
        });
        if command_palette_pressed {
            self.toggle_command_palette(ctx);
        }
        let undo_pressed = ctx.input(|input| {
            input.modifiers.ctrl
                && !input.modifiers.alt
                && !input.modifiers.shift
                && key_pressed_without_repeat(&input.events, egui::Key::Z)
        }) && !ctx.egui_wants_keyboard_input();
        if undo_pressed {
            self.undo_latest(ctx.viewport_id());
        }
        let import_pressed = ctx.input(|input| {
            input.modifiers.command
                && !input.modifiers.alt
                && !input.modifiers.shift
                && key_pressed_without_repeat(&input.events, egui::Key::O)
        });
        if import_pressed {
            self.request_debug_import(ctx, DebugImportKind::Pcapng);
        }
        #[cfg(not(feature = "no_debug"))]
        if modifiers == egui::Modifiers::NONE && pressed_keys.contains(&HotkeyKey::F12) {
            self.toggle_debug_console();
        }
        if ctx.viewport_id() == egui::ViewportId::ROOT
            && self.state.abyss.is_active()
            && !ctx.egui_wants_keyboard_input()
            && modifiers == egui::Modifiers::NONE
            && ctx.input(|input| key_pressed_without_repeat(&input.events, egui::Key::Tab))
        {
            self.selected_abyss_half = match self.selected_abyss_half {
                AbyssHalf::First => AbyssHalf::Second,
                AbyssHalf::Second => AbyssHalf::First,
            };
        }
    }

    fn execute_global_hotkey(&mut self, ctx: &egui::Context, action: GlobalHotkeyAction) {
        let action = match action {
            GlobalHotkeyAction::ToggleCapture => AppAction::ToggleCapture,
            GlobalHotkeyAction::ResetSession => AppAction::ResetSession,
            GlobalHotkeyAction::ToggleHud => AppAction::ToggleHud,
        };
        self.execute_action(ctx, action);
    }

    #[cfg(not(feature = "no_debug"))]
    fn toggle_debug_console(&mut self) {
        self.console_open = !self.console_open;
        if self.console_open {
            self.console_corner_applied = false;
            self.console_tab = ConsoleTab::Packets;
        }
    }

    pub(crate) fn set_mouse_passthrough(&mut self, ctx: &egui::Context, enabled: bool) {
        if self.mouse_passthrough == enabled {
            return;
        }
        if enabled && !self.hotkey_hook_available {
            let message = t("Global hotkeys are not ready; mouse passthrough was not enabled");
            self.status = message.clone();
            self.set_last_error_in(ctx, message, None);
            return;
        }
        self.mouse_passthrough = enabled;
        let now = Instant::now();
        motion::seed_bool_for_viewport(
            ctx,
            egui::ViewportId::ROOT,
            "passthrough_notice_visibility",
            false,
        );
        self.passthrough_notice = Some(PassthroughNotice {
            enabled,
            shown_until: now + Duration::from_millis(1200),
        });
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::MousePassthrough(enabled),
        );
        self.opacity_reapply_frames = 2;
        let hotkey = self.passthrough_hotkey.label();
        self.status = if self.mouse_passthrough {
            if self.hud_mode {
                tf("HUD passthrough on; press {} to enter edit mode", &[hotkey])
            } else {
                tf("Mouse passthrough on; press {} to turn off", &[hotkey])
            }
        } else if self.hud_mode {
            tf(
                "HUD edit mode on; press {} to return to game passthrough",
                &[hotkey],
            )
        } else {
            t("Mouse passthrough off")
        };
    }

    pub(crate) fn toggle_mouse_passthrough(&mut self, ctx: &egui::Context) {
        self.set_mouse_passthrough(ctx, !self.mouse_passthrough);
    }

    pub(crate) fn set_hud_mode(&mut self, ctx: &egui::Context, enabled: bool) {
        if self.hud_mode == enabled {
            return;
        }
        motion::seed_bool_for_viewport(
            ctx,
            egui::ViewportId::ROOT,
            "hud_mode_transition",
            !enabled,
        );
        self.hud_mode = enabled;
        if enabled {
            if !self.always_on_top {
                self.always_on_top = true;
                ctx.send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop),
                );
            }
            self.set_mouse_passthrough(ctx, true);
            self.status = if self.mouse_passthrough {
                tf(
                    "Combat HUD on: always-on-top with mouse passthrough by default; press {} to edit",
                    &[self.passthrough_hotkey.label()],
                )
            } else {
                t("Combat HUD opened in edit mode because global hotkeys are unavailable")
            };
        } else {
            self.set_mouse_passthrough(ctx, false);
            self.status = t("Exited combat HUD");
        }
    }

    pub(crate) fn toggle_always_on_top(&mut self, ctx: &egui::Context) {
        self.always_on_top = !self.always_on_top;
        let level = if self.always_on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        };
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::WindowLevel(level),
        );
        self.opacity_reapply_frames = 2;
        self.status = if self.always_on_top {
            t("Always-on-top enabled")
        } else {
            t("Always-on-top disabled")
        };
    }

    pub(crate) fn title_bar(&mut self, ui: &mut egui::Ui) {
        let title_height = ui.available_height().max(28.0);
        // The whole title bar is the drag-to-move zone: allocate it first with a
        // drag sense, then draw the dot/title/buttons on top. Interactive widgets
        // (added later) win the pointer where they are, so dragging still works on
        // any empty area — the centered title included.
        let (full_rect, title_drag) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), title_height),
            egui::Sense::click_and_drag(),
        );
        if title_drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let title_status = if self.paused {
            tf(
                "Paused · pending {} · dropped debug packets {}",
                &[
                    &self.paused_events.len().to_string(),
                    &self.dropped_debug_packets.to_string(),
                ],
            )
        } else {
            self.status.clone()
        };

        // Native window controls, right-aligned: minimize · maximize/restore ·
        // close. Drawn first so their extent is known before the title centers
        // itself. The overlay toggles (pin/passthrough/appearance) moved off the
        // title bar onto the live toolbar — see `control_buttons`.
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        controls.set_clip_rect(full_rect);
        {
            let ui = &mut controls;
            ui.spacing_mut().item_spacing.x = 2.0;
            if window_control_button(ui, WindowControlIcon::Close, &t("Close")).clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let maximized = ui
                .input(|input| input.viewport().maximized)
                .unwrap_or(false);
            let (icon, tooltip) = if maximized {
                (WindowControlIcon::Restore, t("Restore"))
            } else {
                (WindowControlIcon::Maximize, t("Maximize"))
            };
            if window_control_button(ui, icon, &tooltip).clicked() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            }
            if window_control_button(ui, WindowControlIcon::Minimize, &t("Minimize")).clicked() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        let controls_left = controls.min_rect().left();

        // Status dot pinned to the far left; hovering it shows the current status.
        let dot_center = egui::pos2(full_rect.left() + 5.0, full_rect.center().y);
        let dot_rect =
            egui::Rect::from_center_size(dot_center, egui::vec2(12.0, full_rect.height()));
        let dot_response = ui.interact(
            dot_rect,
            ui.id().with("title_status_dot"),
            egui::Sense::hover(),
        );
        ui.painter().circle_filled(
            dot_center,
            3.5,
            status_color(&self.status, self.paused, self.dark_mode),
        );
        dot_response.on_hover_text(title_status);

        // Centered branding, clipped to the gap between the dot and the window
        // controls so a too-narrow window elides it against the buttons instead of
        // painting over them.
        let title_clip = egui::Rect::from_min_max(
            egui::pos2(dot_rect.right(), full_rect.top()),
            egui::pos2(
                (controls_left - 6.0).max(dot_rect.right()),
                full_rect.bottom(),
            ),
        );
        ui.painter().with_clip_rect(title_clip).text(
            full_rect.center(),
            egui::Align2::CENTER_CENTER,
            "NTE DPS TOOL",
            egui::FontId::proportional(13.0),
            self.theme().accent,
        );
    }

    /// Keeps the main window at the configured floor. The live toolbar adapts before
    /// this width, so it must not grow the minimum and make the compact layout
    /// unreachable. If the window is meaningfully narrower than the floor it is
    /// nudged back up — this heals a stale persisted size or a HUD restore.
    ///
    /// The caller must skip this while a programmatic resize is still settling (see
    /// `main_size_restore_frames`), otherwise it would clamp the in-flight restore to
    /// the minimum instead of the user's larger saved size.
    pub(crate) fn enforce_main_min_size(&mut self, ctx: &egui::Context, maximized: bool) {
        // Deadband: after the OS rounds a requested logical size to physical pixels
        // and back, the reported width can sit a hair under the minimum. Only correct
        // a shortfall larger than this, so rounding noise can't set up a per-frame
        // resize oscillation (edge "jitter").
        const UNDERSIZE_DEADBAND: f32 = 6.0;
        let min_size = egui::Vec2::from(config::MAIN_WINDOW_MIN_SIZE);
        if (min_size - self.applied_main_min_size).length() > 0.5 {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min_size));
            self.applied_main_min_size = min_size;
        }
        // winit clamps only *future* user resizes to the minimum, so a window that is
        // already too small (startup default, stale saved size, HUD-exit restore, or a
        // language switch that grew the minimum) must be nudged wider here. Skipped
        // while maximized (would drop that state) and until the viewport reports a real
        // size (an early degenerate rect must not shrink it).
        if !maximized {
            let current = ctx.content_rect().size();
            if current.x >= 1.0 && current.y >= 1.0 && current.x < min_size.x - UNDERSIZE_DEADBAND {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    min_size.x,
                    current.y.max(min_size.y),
                )));
            }
        }
    }

    /// Compact title strip for HUD mode: a drag zone plus the two controls that
    /// matter while positioning the overlay. It is hidden completely while
    /// click-through is active so the combat readout sits directly on the game.
    pub(crate) fn hud_title_bar(&mut self, ui: &mut egui::Ui) {
        if self.mouse_passthrough {
            return;
        }
        let (full_rect, drag) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ui.available_height().max(24.0)),
            egui::Sense::click_and_drag(),
        );
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        // A solid rail makes the edit strip easy to grab after the pass-through
        // hotkey disables viewport mouse pass-through.
        let hud_theme = self.theme().hud;
        let painter = ui.painter();
        painter.rect_filled(
            full_rect,
            egui::CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            },
            hud_theme.edit_bg,
        );
        painter.hline(
            full_rect.x_range(),
            full_rect.bottom() - 0.5,
            Stroke::new(1.0_f32, hud_theme.edit_border),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full_rect.shrink2(egui::vec2(8.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.label(
            RichText::new("NTE DPS")
                .size(10.5)
                .strong()
                .color(hud_theme.edit_text),
        );
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let passthrough_hint = tf(
                "{} toggles anytime; while passthrough is on you can't click buttons, so press {} to turn it off before exiting",
                &[
                    self.passthrough_hotkey.label(),
                    self.passthrough_hotkey.label(),
                ],
            );
            if ui
                .small_button(t("Exit"))
                .on_hover_text(t("Return to the normal window"))
                .clicked()
            {
                self.set_hud_mode(ui.ctx(), false);
            }
            // A plain small button to match Exit: this strip only renders while
            // passthrough is off (it returns early otherwise), so a selectable
            // toggle's active state would never show here — it just read as a
            // heavier, out-of-place control on the compact rail.
            if ui
                .small_button(t("Passthrough"))
                .on_hover_ui(|ui| {
                    // The HUD is its own small OS window (HUD_WINDOW_WIDTH), not
                    // a panel inside a larger one — a tooltip can't spill past
                    // its edges the way it could in the normal window, so wrap
                    // well short of that width instead of relying on the
                    // default single-line-then-clip tooltip sizing.
                    ui.set_max_width(HUD_WINDOW_WIDTH - 100.0);
                    ui.label(passthrough_hint);
                })
                .clicked()
            {
                self.toggle_mouse_passthrough(ui.ctx());
            }
        });
    }

    pub(crate) fn start_live_for(&mut self, viewport: egui::ViewportId) -> bool {
        self.stop_engine();
        self.active_capture_filter = None;
        if let Err(error) = self.refresh_game_network() {
            if viewport == console_viewport_id() {
                self.console_open = true;
                self.console_corner_applied = false;
            }
            self.set_last_error_for(viewport, error, Some(ErrorAction::RefreshNetwork));
            return false;
        }
        let Some(device) = self.devices.get(self.selected_device).cloned() else {
            if viewport == console_viewport_id() {
                self.console_open = true;
                self.console_corner_applied = false;
            }
            self.set_last_error_for(
                viewport,
                t("No usable capture device; confirm Npcap is installed"),
                Some(ErrorAction::RefreshNetwork),
            );
            return false;
        };
        let local_ip = self.game_network.as_ref().map(|network| network.local_ip);
        // The base filter (`self.filter`, "udp") keeps all UDP, which covers the game-world
        // server that carries combat/GAS replication and equipment (e.g. :30196). The game's
        // account / life-sim service talks TCP :30031 to a *different* server IP, so a UDP-only
        // BPF drops it before it can even reach the raw pcapng. Widen the filter to also keep
        // everything to/from that detected host. The live parser only decodes UDP
        // (`parse_udp_ipv4` rejects non-UDP), so the extra TCP frames are retained for offline
        // analysis without affecting live parsing. Falls back to UDP-only if the game endpoint
        // was not detected.
        let capture_filter = match self.game_network.as_ref() {
            Some(network) => format!("{} or host {}", self.filter, network.remote_ip),
            None => self.filter.clone(),
        };
        self.reset_combat_session();
        self.capture_quality_source = CaptureQualitySource::Live;
        let capture = start_capture(
            device,
            local_ip,
            capture_filter.clone(),
            self.include_incoming,
            self.server_damage_calibration,
            self.characters.clone(),
            self.sender.clone(),
        );
        self.active_capture_filter = Some(capture_filter);
        self.raw_capture = Some(capture.raw_capture());
        self.capture = Some(capture);
        self.engine_task_viewport = Some(viewport);
        self.status = t("Starting live capture...");
        true
    }

    pub(crate) fn refresh_game_network(&mut self) -> Result<(), String> {
        // A user-initiated refresh owns the device state from here on; drop any
        // still-pending startup probe so it can't clobber this result.
        self.awaiting_device_detection = false;
        self.game_process_detected = false;
        self.game_network = None;
        self.local_ip.clear();
        self.game_process_detected = game_process_is_running().inspect_err(|error| {
            self.diagnostic = Some(error.clone());
        })?;
        self.devices = list_devices().inspect_err(|error| {
            self.diagnostic = Some(error.clone());
        })?;
        if let Some(name) = self.manual_capture_device.clone() {
            return self.apply_manual_capture_device(&name);
        }
        let (index, network) = detect_game_device(&self.devices).inspect_err(|error| {
            self.diagnostic = Some(error.clone());
        })?;
        self.selected_device = index;
        self.local_ip = network.local_ip.to_string();
        self.status = t("Game detected, ready");
        self.diagnostic = None;
        self.game_network = Some(network);
        Ok(())
    }

    /// Manual capture mode: pin capture to the chosen NIC and best-effort resolve the game's local
    /// IP for direction inference. A missing game connection is non-fatal — capture still proceeds
    /// and `infer_outgoing` falls back to its public/private heuristic. Only a vanished NIC aborts.
    pub(crate) fn apply_manual_capture_device(&mut self, name: &str) -> Result<(), String> {
        let Some(index) = self.devices.iter().position(|device| device.name == name) else {
            let message = tf(
                "The manually selected NIC ({}) is currently unavailable; reselect in settings or switch back to auto",
                &[name],
            );
            self.diagnostic = Some(message.clone());
            self.game_network = None;
            self.local_ip.clear();
            self.status = t("Manual NIC unavailable");
            return Err(message);
        };
        self.selected_device = index;
        match detect_game_network() {
            Ok(network) => {
                self.local_ip = network.local_ip.to_string();
                self.game_network = Some(network);
                self.status = t("Ready (manual NIC)");
                self.diagnostic = None;
            }
            Err(error) => {
                self.local_ip.clear();
                self.game_network = None;
                self.status = t("Manual NIC selected (no game connection detected)");
                self.diagnostic = Some(error);
            }
        }
        Ok(())
    }

    pub(crate) fn start_pcapng_import_for(&mut self, path: PathBuf, viewport: egui::ViewportId) {
        self.stop_engine();
        self.raw_capture = None;
        self.active_capture_filter = None;
        self.reset_combat_session();
        self.capture_quality_source = CaptureQualitySource::PcapngReplay;
        let local_ip_hint = self
            .game_network
            .as_ref()
            .map(|network| network.local_ip)
            .or_else(|| self.local_ip.parse::<Ipv4Addr>().ok());
        let stop = Arc::new(AtomicBool::new(false));
        self.active_import = Some(ActiveImport {
            kind: DebugImportKind::Pcapng,
            path: path.clone(),
            started_at: Instant::now(),
            viewport,
        });
        self.engine_task_viewport = Some(viewport);
        self.replay_thread = Some(import_pcapng(
            path,
            self.characters.clone(),
            local_ip_hint,
            self.include_incoming,
            self.server_damage_calibration,
            self.sender.clone(),
            stop.clone(),
        ));
        self.replay_stop = Some(stop);
        self.status = local_ip_hint.map_or_else(
            || t("Importing and parsing pcapng (heuristic direction)..."),
            |ip| {
                tf(
                    "Importing and parsing pcapng (local IP {} filter/direction)...",
                    &[&ip.to_string()],
                )
            },
        );
    }

    pub(crate) fn start_capture_json_import_for(
        &mut self,
        path: PathBuf,
        viewport: egui::ViewportId,
    ) {
        self.stop_engine();
        self.raw_capture = None;
        self.active_capture_filter = None;
        self.reset_combat_session();
        self.capture_quality_source = CaptureQualitySource::JsonReplay;
        let stop = Arc::new(AtomicBool::new(false));
        self.active_import = Some(ActiveImport {
            kind: DebugImportKind::CaptureJson,
            path: path.clone(),
            started_at: Instant::now(),
            viewport,
        });
        self.engine_task_viewport = Some(viewport);
        self.replay_thread = Some(import_capture_json(path, self.sender.clone(), stop.clone()));
        self.replay_stop = Some(stop);
        self.status = t("Importing capture JSON...");
    }

    pub(crate) fn process_file_drops(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        self.native_file_drop.install(frame);
        let mut paths = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect::<Vec<_>>()
        });
        paths.extend(self.native_file_drop.try_iter());
        for path in paths {
            self.import_dropped_file(path);
        }
    }

    pub(crate) fn import_dropped_file(&mut self, path: PathBuf) {
        if self
            .last_dropped_file
            .as_ref()
            .is_some_and(|(previous, at)| {
                previous == &path && at.elapsed() < Duration::from_secs(1)
            })
        {
            return;
        }
        self.last_dropped_file = Some((path.clone(), Instant::now()));
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase());
        match extension.as_deref() {
            Some("pcapng") => self.request_import_file(DebugImportKind::Pcapng, path),
            Some("json") => self.request_import_file(DebugImportKind::CaptureJson, path),
            _ => {
                let name = file_display_name(&path);
                self.set_last_error(
                    tf(
                        "Unsupported dropped file: {}\nCurrently .pcapng and .json are supported",
                        &[&name],
                    ),
                    Some(ErrorAction::OpenPcapng),
                );
            }
        }
    }

    /// Switch the live UI language, reload its locale map, refresh localized
    /// reaction glyph textures, and reload the localized ability/skill name
    /// table so already-captured hits display the new language too (see
    /// [`crate::storage::ability_names`]). `current_ui_config` includes the
    /// language so the debounced save persists the choice to the config file.
    pub(crate) fn set_language(&mut self, ctx: &egui::Context, language: Language) {
        self.language = language;
        i18n::set_language(language);
        crate::storage::ability_names::reload(language);
        self.reaction_textures = load_reaction_text_textures(ctx, &data_root());
        ctx.request_repaint();
    }

    pub(crate) fn current_ui_config(&self) -> UiConfig {
        UiConfig {
            language: self.language,
            opacity: self.opacity,
            dark_mode: self.dark_mode,
            accent: self.accent,
            density: self.density,
            reduce_motion: self.reduce_motion,
            always_on_top: self.always_on_top,
            server_damage_calibration: self.server_damage_calibration,
            manual_capture_device: self.manual_capture_device.clone(),
            dps_time_mode: self.dps_time_mode,
            timeline_bucket_seconds: self.timeline_bucket_seconds,
            timeline_dps_view_mode: self.timeline_dps_view_mode,
            hud: self.hud_config.clone(),
            passthrough_hotkey: self.passthrough_hotkey,
            global_hotkeys: self.global_hotkeys,
            onboarding_done: self.onboarding_done,
            console_sidebar_migration_seen: self.console_sidebar_migration_seen,
            main_window_size: Some([self.main_window_size.x, self.main_window_size.y]),
            abyss_window_size: Some([self.abyss_window_size.x, self.abyss_window_size.y]),
            hit_detail_window_size: Some([
                self.hit_detail_window_size.x,
                self.hit_detail_window_size.y,
            ]),
            team_hit_detail_window_size: Some([
                self.team_hit_detail_window_size.x,
                self.team_hit_detail_window_size.y,
            ]),
            console_window_size: Some([self.console_window_size.x, self.console_window_size.y]),
        }
        .sanitized()
    }

    pub(crate) fn ui_config_save_plan(
        current: &UiConfig,
        saved_ui_config: &UiConfig,
        pending_ui_config: Option<&(UiConfig, Instant)>,
        now: Instant,
    ) -> UiConfigSavePlan {
        if current == saved_ui_config {
            UiConfigSavePlan::NoChange
        } else if let Some((pending, save_at)) = pending_ui_config {
            if pending == current {
                if *save_at <= now {
                    UiConfigSavePlan::Save(pending.clone())
                } else {
                    UiConfigSavePlan::KeepPending((pending.clone(), *save_at))
                }
            } else {
                UiConfigSavePlan::SetPending((current.clone(), now + UI_CONFIG_SAVE_DELAY))
            }
        } else {
            UiConfigSavePlan::SetPending((current.clone(), now + UI_CONFIG_SAVE_DELAY))
        }
    }

    pub(crate) fn persist_ui_config(&mut self) {
        let current = self.current_ui_config();
        let now = Instant::now();
        match Self::ui_config_save_plan(
            &current,
            &self.saved_ui_config,
            self.pending_ui_config.as_ref(),
            now,
        ) {
            UiConfigSavePlan::NoChange => {
                self.pending_ui_config = None;
            }
            UiConfigSavePlan::SetPending((pending, save_at))
            | UiConfigSavePlan::KeepPending((pending, save_at)) => {
                self.pending_ui_config = Some((pending, save_at));
            }
            UiConfigSavePlan::Save(pending) => match config::save(&self.ui_config_path, &pending) {
                Ok(()) => {
                    self.saved_ui_config = pending;
                    self.pending_ui_config = None;
                }
                Err(error) => {
                    self.set_last_error(
                        tf(
                            "Failed to save UI config; check permissions or disk space: {}\n{}",
                            &[&error, &self.ui_config_path.display().to_string()],
                        ),
                        Some(ErrorAction::OpenConsole),
                    );
                    self.pending_ui_config = Some((pending, now + UI_CONFIG_SAVE_RETRY_DELAY));
                }
            },
        }
    }

    pub(crate) fn persist_ui_config_on_shutdown(&mut self) {
        let current = self.current_ui_config();
        if let Some((pending, _)) = self.pending_ui_config.take() {
            let _ = config::save(&self.ui_config_path, &pending);
            return;
        }
        if current != self.saved_ui_config {
            let _ = config::save(&self.ui_config_path, &current);
        }
    }

    pub(crate) fn request_debug_import(&mut self, ctx: &egui::Context, kind: DebugImportKind) {
        let purpose = FileDialogPurpose::DebugImport { kind };
        match kind {
            DebugImportKind::Pcapng => {
                let filter = t("Wireshark capture");
                self.spawn_file_dialog(ctx, purpose, move |owner| {
                    with_owner(
                        rfd::FileDialog::new().add_filter(filter, &["pcapng"]),
                        owner,
                    )
                    .pick_file()
                });
            }
            DebugImportKind::CaptureJson => {
                let filter = t("NTE exported capture");
                self.spawn_file_dialog(ctx, purpose, move |owner| {
                    with_owner(rfd::FileDialog::new().add_filter(filter, &["json"]), owner)
                        .pick_file()
                });
            }
            DebugImportKind::EncryptedIni => {
                let ini_filter = t("NTE encrypted INI");
                let all_filter = t("All files");
                self.spawn_file_dialog(ctx, purpose, move |owner| {
                    with_owner(
                        rfd::FileDialog::new()
                            .add_filter(ini_filter, &["ini"])
                            .add_filter(all_filter, &["*"]),
                        owner,
                    )
                    .pick_file()
                });
            }
        }
    }

    /// Run a native file dialog on a worker thread and remember what to do with
    /// the picked path (see [`FileDialogPurpose`]); [`Self::poll_file_dialog`]
    /// picks up the result. Only one dialog may be open at a time — further
    /// requests are ignored until it closes.
    ///
    /// `dialog` receives the root window as a [`DialogOwner`] so it can call
    /// `.set_parent(owner)`: an owned window always renders above its owner
    /// regardless of topmost/z-order, which is what keeps the dialog from
    /// appearing hidden behind an always-on-top window. This deliberately avoids
    /// `clear_process_windows_topmost`/`SetWindowPos`-based approaches — those
    /// deadlock on this app's wgpu backend when run on the UI thread (a same-
    /// thread `SetWindowPos` synchronously re-enters `logic()` via WndProc) and
    /// still block forever when moved to a worker thread (the cross-thread call
    /// waits on a UI-thread message that never gets drained).
    pub(crate) fn spawn_file_dialog(
        &mut self,
        ctx: &egui::Context,
        purpose: FileDialogPurpose,
        dialog: impl FnOnce(Option<DialogOwner>) -> Option<PathBuf> + Send + 'static,
    ) {
        if self.pending_file_dialog.is_some() {
            return;
        }
        let owner = DialogOwner::from_hwnd(self.corner_applied_hwnd);
        let (sender, receiver) = unbounded();
        let waker = ctx.clone();
        thread::spawn(move || {
            let picked = dialog(owner);
            let _ = sender.send(picked);
            // Wake an idle UI so poll_file_dialog sees the result promptly.
            waker.request_repaint();
        });
        self.pending_file_dialog = Some(PendingFileDialog {
            purpose,
            viewport: ctx.viewport_id(),
            receiver,
        });
        ctx.request_repaint();
    }

    pub(crate) fn poll_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_file_dialog else {
            return;
        };
        let result = match pending.receiver.try_recv() {
            Ok(result) => result,
            // Fallback wake in case the worker's repaint races this frame.
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(200));
                return;
            }
            Err(TryRecvError::Disconnected) => None,
        };
        let Some(PendingFileDialog {
            purpose, viewport, ..
        }) = self.pending_file_dialog.take()
        else {
            return;
        };
        // Nudge opacity to reapply in case focus moved while the dialog was open.
        self.opacity_reapply_frames = 2;
        ctx.request_repaint();
        let Some(path) = result else {
            return;
        };
        match purpose {
            FileDialogPurpose::DebugImport { kind } => match kind {
                DebugImportKind::Pcapng | DebugImportKind::CaptureJson => {
                    self.request_import_file_for(kind, path, viewport);
                }
                DebugImportKind::EncryptedIni => self.load_encrypted_ini_for(path, viewport),
            },
            FileDialogPurpose::TeamDpsImportAll => self.finish_team_dps_import(viewport, &path),
            FileDialogPurpose::TeamDpsImportLine { upper } => {
                self.finish_team_dps_line_import(viewport, upper, &path);
            }
            FileDialogPurpose::TeamDpsExport { json } => {
                self.finish_team_dps_export(viewport, &path, &json);
            }
            FileDialogPurpose::HistoryExport { json } => {
                self.finish_history_record_export(viewport, &path, &json);
            }
            FileDialogPurpose::EmptyCurtainExport { json } => {
                self.finish_empty_curtain_export(viewport, &path, &json);
            }
            FileDialogPurpose::CaptureInfoExport => {
                self.finish_capture_info_export(viewport, &path);
            }
            FileDialogPurpose::RawCaptureExport => self.finish_raw_capture_export(viewport, &path),
        }
    }

    pub(crate) fn drain_events(&mut self) {
        let started = Instant::now();
        let scrolling = self.detail_scroll_active();
        let event_limit = if scrolling {
            MAX_UI_EVENTS_WHILE_SCROLLING
        } else {
            MAX_UI_EVENTS_PER_FRAME
        };
        if self.paused {
            for _ in 0..event_limit {
                if started.elapsed() >= UI_EVENT_BUDGET {
                    break;
                }
                let Ok(event) = self.receiver.try_recv() else {
                    break;
                };
                self.buffer_paused_event(event);
            }
            // Bound the queue even if inflow outpaces the per-frame budget while paused.
            while self.receiver.len() > MAX_ENGINE_QUEUE_HARD_CAP {
                let Ok(event) = self.receiver.try_recv() else {
                    break;
                };
                self.buffer_paused_event(event);
            }
            return;
        }
        for _ in 0..event_limit {
            if started.elapsed() >= UI_EVENT_BUDGET {
                break;
            }
            let event = if let Some(event) = self.paused_events.pop_front() {
                event
            } else if let Ok(event) = self.receiver.try_recv() {
                event
            } else {
                break;
            };
            self.apply_engine_event(event);
        }
        if !scrolling && started.elapsed() < UI_EVENT_BUDGET {
            self.shed_event_backlog(started);
        }
        self.enforce_engine_queue_hard_cap();
    }

    /// Routes one event while paused: debug packets are dropped, hit-like events are buffered
    /// (oldest dropped past the cap) for replay on resume, and lifecycle events apply immediately.
    pub(crate) fn buffer_paused_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Packet(_) => {
                self.dropped_debug_packets = self.dropped_debug_packets.saturating_add(1);
            }
            EngineEvent::Hit(_)
            | EngineEvent::HitFollowUp(_)
            | EngineEvent::HitDamageCorrection(_)
            | EngineEvent::Abyss(_)
            | EngineEvent::TimeStop(_)
            | EngineEvent::EmptyCurtain(_) => {
                if self.paused_events.len() == MAX_PAUSED_EVENTS {
                    self.paused_events.pop_front();
                }
                self.paused_events.push_back(event);
            }
            EngineEvent::Status(_)
            | EngineEvent::Warning(_)
            | EngineEvent::Error(_)
            | EngineEvent::CaptureStopped => self.apply_engine_event(event),
        }
    }

    /// Absolute ceiling on the engine→UI queue so it can never grow without bound — e.g. a sustained
    /// packet flood while the user keeps a detail list scrolling (which otherwise skips shedding).
    /// Dropping debug packets is O(1); the rare non-packet events are applied so stats stay correct.
    pub(crate) fn enforce_engine_queue_hard_cap(&mut self) {
        while self.receiver.len() > MAX_ENGINE_QUEUE_HARD_CAP {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            if matches!(event, EngineEvent::Packet(_)) {
                self.dropped_debug_packets = self.dropped_debug_packets.saturating_add(1);
            } else {
                self.apply_engine_event(event);
            }
        }
    }

    pub(crate) fn shed_event_backlog(&mut self, started: Instant) {
        while self.receiver.len() > MAX_ENGINE_QUEUE_BACKLOG && started.elapsed() < UI_EVENT_BUDGET
        {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            if matches!(event, EngineEvent::Packet(_)) {
                self.dropped_debug_packets = self.dropped_debug_packets.saturating_add(1);
            } else {
                self.apply_engine_event(event);
            }
        }
    }

    pub(crate) fn drain_pending_events(&mut self) {
        while let Some(event) = self.paused_events.pop_front() {
            self.apply_engine_event(event);
        }
        while let Ok(event) = self.receiver.try_recv() {
            self.apply_engine_event(event);
        }
    }

    pub(crate) fn apply_engine_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Hit(hit) => {
                self.note_combat_hit(&hit);
                self.state.push_hit(*hit);
            }
            EngineEvent::HitFollowUp(follow_up) => self.state.apply_follow_up(follow_up),
            EngineEvent::HitDamageCorrection(correction) => {
                self.state.apply_damage_correction(correction)
            }
            EngineEvent::Packet(packet) => self.state.push_packet(*packet),
            EngineEvent::Abyss(event) => {
                self.character_hit_cache = HitDetailCache::default();
                self.team_hit_cache = HitDetailCache::default();
                self.skill_summary_cache = SkillSummaryCache::default();
                self.timeline_cache = TimelineCache::default();
                self.skill_breakdown_cache = SkillBreakdownCache::default();
                if let AbyssEvent::Stage { half, .. } = &event {
                    self.selected_abyss_half = *half;
                    self.abyss_compact_mode = true;
                } else if matches!(&event, AbyssEvent::Success { .. } | AbyssEvent::Exit { .. }) {
                    self.abyss_compact_mode = false;
                    self.finish_combat_visual();
                }
                self.state.apply_abyss_event(event);
            }
            EngineEvent::TimeStop(event) => {
                self.timeline_cache = TimelineCache::default();
                self.state.apply_time_stop_event(event);
            }
            EngineEvent::EmptyCurtain(items) => self.state.replace_empty_curtain(items),
            EngineEvent::Status(status) => self.status = status,
            EngineEvent::Warning(warning) => {
                self.diagnostic = Some(tf(
                    "Some resources failed to load; features degraded: {}",
                    &[&warning],
                ));
            }
            EngineEvent::Error(error) => {
                self.status = t("Run failed");
                let action = import_error_action(&error);
                let mut viewport = self
                    .active_import
                    .as_ref()
                    .map(|task| task.viewport)
                    .or(self.engine_task_viewport)
                    .unwrap_or(egui::ViewportId::ROOT);
                if viewport == egui::ViewportId::ROOT && (self.hud_mode || self.mouse_passthrough) {
                    viewport = console_viewport_id();
                }
                if viewport == console_viewport_id() {
                    self.console_open = true;
                    self.console_corner_applied = false;
                }
                self.set_last_error_for(viewport, humanize_engine_error(&error), action);
            }
            EngineEvent::CaptureStopped => {
                self.finish_combat_visual();
                let import_finished = self.replay_thread.is_some();
                self.capture.take();
                self.replay_stop = None;
                if let Some(thread) = self.replay_thread.take() {
                    let _ = thread.join();
                }
                if import_finished {
                    self.selected_abyss_half = AbyssHalf::First;
                    self.abyss_compact_mode = false;
                    self.active_import = None;
                    self.status = t("Import complete; see parse quality on the diagnostics page");
                } else {
                    self.status = t("Stopped");
                }
            }
        }
    }

    pub(crate) fn update_status_toast(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.last_status_toast != self.status {
            self.last_status_toast = self.status.clone();
            if !self.status.trim().is_empty() {
                self.push_status_toast(
                    egui::ViewportId::ROOT,
                    self.status.clone(),
                    ToastTone::Status,
                    STATUS_TOAST_DURATION,
                    None,
                );
            }
        }

        let mut expired = Vec::new();
        for toast in &mut self.status_toasts {
            let elapsed = now.saturating_duration_since(toast.last_tick);
            if toast.hovered {
                toast.shown_until += elapsed;
            }
            toast.last_tick = now;
            toast.hovered = false;
            if toast.shown_until <= now {
                expired.push(toast.id);
            } else {
                ctx.request_repaint_after(toast.shown_until.saturating_duration_since(now));
            }
        }
        for id in expired {
            self.dismiss_toast(id);
        }
    }

    fn note_combat_hit(&mut self, hit: &crate::engine::model::Hit) {
        if hit.direction == "incoming" {
            return;
        }
        if self.combat_active
            && self
                .last_combat_timestamp
                .is_some_and(|last| hit.timestamp - last > COMBAT_SEGMENT_GAP_SECONDS)
        {
            self.finish_combat_visual();
        }
        if !self.combat_active {
            self.combat_active = true;
            self.combat_start_generation = self.combat_start_generation.wrapping_add(1);
        }
        self.last_combat_timestamp = Some(hit.timestamp);
        self.last_combat_activity = Some(Instant::now());
    }

    fn finish_combat_visual(&mut self) {
        if self.combat_active {
            self.combat_active = false;
            self.combat_end_generation = self.combat_end_generation.wrapping_add(1);
        }
    }

    pub(crate) fn update_combat_visual(&mut self) {
        if !self.paused
            && self.combat_active
            && self.last_combat_activity.is_some_and(|activity| {
                activity.elapsed().as_secs_f64() >= COMBAT_SEGMENT_GAP_SECONDS
            })
        {
            self.finish_combat_visual();
        }
    }

    pub(crate) fn show_status_toast(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let viewport = ctx.viewport_id();
        let ids = self
            .status_toasts
            .iter()
            .rev()
            .filter(|toast| toast.viewport == viewport && toast.shown_until > now)
            .map(|toast| toast.id)
            .collect::<Vec<_>>();
        let mut stack_y = 0.0;
        let mut dismiss = Vec::new();
        let mut undo = None;
        for id in ids {
            let seed_entrance = self
                .status_toasts
                .iter_mut()
                .find(|toast| toast.id == id)
                .is_some_and(|toast| {
                    let seed = !toast.animation_seeded;
                    toast.animation_seeded = true;
                    seed
                });
            if seed_entrance {
                motion::seed_bool(ctx, ("toast_entrance", id), false);
            }
            let Some(toast) = self.status_toasts.iter().find(|toast| toast.id == id) else {
                continue;
            };
            let text = toast.text.clone();
            let tone = toast.tone;
            let undo_id = toast.undo_id;
            let color = match tone {
                ToastTone::Status => status_color(&text, self.paused, self.dark_mode),
                ToastTone::Success => self.theme().success,
                ToastTone::Warning => self.theme().warning,
                ToastTone::Danger => self.theme().danger,
            };
            let progress = motion::animate_bool(
                ctx,
                ("toast_entrance", id),
                true,
                motion::dur::BASE,
                self.reduce_motion,
                motion::ease::entrance,
            );
            let animated_stack = motion::animate_value(
                ctx,
                ("toast_stack", id),
                stack_y,
                motion::dur::BASE,
                self.reduce_motion,
            );
            let fill = self.theme().floating;
            let response = egui::Area::new(egui::Id::new(("status_toast", id)))
                .order(egui::Order::Foreground)
                .interactable(true)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-14.0 + (1.0 - progress) * 12.0, -14.0 - animated_stack),
                )
                .show(ctx, |ui| {
                    ui.set_opacity(progress);
                    egui::Frame::new()
                        .fill(fill)
                        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.85)))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            let max_width = if self.hud_mode { 330.0 } else { 420.0 };
                            ui.set_max_width(max_width);
                            ui.horizontal(|ui| {
                                let (dot_rect, _) = ui.allocate_exact_size(
                                    egui::vec2(9.0, 9.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(dot_rect.center(), 4.0, color);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(text).size(11.5).color(self.theme().fg),
                                    )
                                    .wrap(),
                                );
                                if let Some(undo_id) = undo_id
                                    && ui.button(t("Undo")).clicked()
                                {
                                    undo = Some(undo_id);
                                }
                            });
                        });
                })
                .response;
            if let Some(toast) = self.status_toasts.iter_mut().find(|toast| toast.id == id) {
                toast.hovered = response.hovered();
            }
            if response.clicked() && undo != undo_id {
                dismiss.push(id);
            }
            stack_y += response.rect.height() + 8.0;
        }
        for id in dismiss {
            self.dismiss_toast(id);
        }
        if let Some(id) = undo {
            self.apply_undo(id, viewport);
        }
    }

    pub(crate) fn show_onboarding(&mut self, ctx: &egui::Context) {
        if self.onboarding_done || ctx.viewport_id() != egui::ViewportId::ROOT {
            return;
        }

        let step = self.onboarding_step.min(3);
        let theme = self.theme();
        let awaiting_detection = self.awaiting_device_detection;
        let device_count = self.devices.len();
        let game_connection_detected = self.game_network.is_some();
        let game_process_error = self.game_process_monitor_error.clone();
        let capture_active = self.capture.is_some();
        let passthrough_hotkey = self.passthrough_hotkey.label();
        let current_hud = self.hud_config.clone();
        let hotkey_preview = (step == 2).then(|| {
            motion::animate_generation(
                ctx,
                "onboarding_hotkey_preview",
                self.onboarding_hotkey_preview_generation,
                motion::dur::TREND,
                self.reduce_motion,
            )
        });
        let available_width = (ctx.content_rect().width() - 48.0).clamp(320.0, 460.0);
        let mut go_back = false;
        let mut go_next = false;
        let mut finish = false;
        let mut retry_detection = false;
        let mut selected_hud = None;
        let mut preview_hotkey = false;

        egui::Modal::new(egui::Id::new("first_run_onboarding"))
            .backdrop_color(theme.modal_backdrop)
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(theme.bg_elevated)
                    .stroke(Stroke::new(1.0, theme.border_strong))
                    .corner_radius(12)
                    .inner_margin(egui::Margin::symmetric(22, 18)),
            )
            .show(ctx, |ui| {
                ui.set_width(available_width);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(t("Welcome to NTE DPS Tool"))
                            .size(20.0)
                            .strong()
                            .color(theme.fg),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.weak(tf(
                            "Step {} of 4",
                            &[&(usize::from(step) + 1).to_string()],
                        ));
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    for index in 0..4_u8 {
                        let color = if index <= step {
                            theme.accent
                        } else {
                            theme.border_strong
                        };
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2((available_width - 24.0) / 4.0, 3.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 2.0, color);
                    }
                });
                ui.add_space(14.0);

                match step {
                    0 => {
                        ui.heading(t("Check the capture environment"));
                        ui.label(t(
                            "Npcap provides the network packets used for live damage statistics.",
                        ));
                        ui.add_space(12.0);
                        egui::Frame::new()
                            .fill(theme.card)
                            .stroke(Stroke::new(1.0, theme.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                if awaiting_detection {
                                    ui.horizontal(|ui| {
                                        ui.add(egui::Spinner::new().size(16.0));
                                        ui.label(t("Checking Npcap and available NICs..."));
                                    });
                                } else if device_count > 0 {
                                    ui.colored_label(
                                        theme.success,
                                        tf(
                                            "Npcap is ready · {} NICs found",
                                            &[&device_count.to_string()],
                                        ),
                                    );
                                } else {
                                    ui.colored_label(
                                        theme.danger,
                                        t("Npcap is unavailable or no usable NIC was found"),
                                    );
                                }
                            });
                        if !awaiting_detection && ui.button(t("Check again")).clicked() {
                            retry_detection = true;
                        }
                    }
                    1 => {
                        ui.heading(t("Automatic NIC selection"));
                        ui.label(t(
                            "The tool detects the HTGame.exe connection and selects its NIC automatically. You can pin a NIC later in Settings when using a VPN.",
                        ));
                        ui.add_space(12.0);
                        let (color, label) = if game_connection_detected {
                            (theme.success, t("Game connection detected"))
                        } else {
                            (theme.warning, t("Game connection not detected yet"))
                        };
                        ui.colored_label(color, label);
                        if let Some(error) = &game_process_error {
                            ui.small(
                                RichText::new(tf("Game process check failed: {}", &[error]))
                                    .color(theme.danger),
                            );
                        }
                        if ui.button(t("Re-detect")).clicked() {
                            retry_detection = true;
                        }
                    }
                    2 => {
                        ui.heading(t("Keep control while playing"));
                        ui.label(t(
                            "Mouse passthrough lets clicks reach the game while the HUD stays visible.",
                        ));
                        ui.add_space(12.0);
                        egui::Frame::new()
                            .fill(theme.card)
                            .stroke(Stroke::new(1.0, theme.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.label(t("Mouse passthrough shortcut"));
                                ui.label(
                                    RichText::new(passthrough_hotkey)
                                        .size(18.0)
                                        .strong()
                                        .color(theme.accent),
                                );
                                ui.weak(t(
                                    "The shortcut always restores edit mode, even while passthrough is active.",
                                ));
                                ui.add_space(10.0);
                                let phase = if self.onboarding_hotkey_preview_generation == 0 {
                                    None
                                } else {
                                    Some(
                                        (((1.0 - hotkey_preview.expect("step 3 owns the preview"))
                                            * 3.0)
                                            .floor() as usize)
                                            .min(2),
                                    )
                                };
                                ui.horizontal_wrapped(|ui| {
                                    for (index, label) in [
                                        t("Edit mode"),
                                        t("Passthrough mode"),
                                        t("Edit mode restored"),
                                    ]
                                    .into_iter()
                                    .enumerate()
                                    {
                                        if index > 0 {
                                            ui.label(RichText::new("→").color(theme.fg_faint));
                                        }
                                        ui.label(
                                            RichText::new(label).strong().color(
                                                if phase == Some(index) {
                                                    theme.accent
                                                } else {
                                                    theme.fg_muted
                                                },
                                            ),
                                        );
                                    }
                                });
                                if ui.button(t("Preview shortcut flow")).clicked() {
                                    preview_hotkey = true;
                                }
                            });
                    }
                    _ => {
                        ui.heading(t("Choose a Combat HUD preset"));
                        ui.label(t(
                            "Presets only choose which readouts are visible; every item remains adjustable in Settings.",
                        ));
                        ui.add_space(12.0);
                        ui.columns(3, |columns| {
                            let presets = [
                                (t("Minimal"), HudConfig::minimal()),
                                (t("Standard"), HudConfig::default()),
                                (t("Detailed"), HudConfig::detailed()),
                            ];
                            for (column, (label, preset)) in columns.iter_mut().zip(presets) {
                                if column
                                    .selectable_label(current_hud == preset, label)
                                    .clicked()
                                {
                                    selected_hud = Some(preset);
                                }
                            }
                        });
                        ui.add_space(10.0);
                        ui.colored_label(
                            if capture_active {
                                theme.success
                            } else {
                                theme.fg_muted
                            },
                            if capture_active {
                                t("Live capture is already running")
                            } else {
                                t("You can start capture from the main window or press Ctrl+F9")
                            },
                        );
                    }
                }

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t("Skip setup")).clicked() {
                        finish = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = if step == 3 { t("Finish") } else { t("Next") };
                        if ui.add(primary_button(label, theme.accent)).clicked() {
                            if step == 3 {
                                finish = true;
                            } else {
                                go_next = true;
                            }
                        }
                        if step > 0 && ui.button(t("Back")).clicked() {
                            go_back = true;
                        }
                    });
                });
            });

        if retry_detection && let Err(error) = self.refresh_game_network() {
            self.set_last_error(error, Some(ErrorAction::RefreshNetwork));
        }
        if let Some(preset) = selected_hud {
            self.hud_config = preset;
        }
        if preview_hotkey {
            self.onboarding_hotkey_preview_generation =
                self.onboarding_hotkey_preview_generation.wrapping_add(1);
        }
        if go_back {
            self.onboarding_step -= 1;
        } else if go_next {
            self.onboarding_step += 1;
        }
        if finish {
            self.onboarding_done = true;
            self.status = t("Setup complete");
        }
    }

    pub(crate) fn show_passthrough_notice(&mut self, ctx: &egui::Context) {
        let Some(notice) = &self.passthrough_notice else {
            return;
        };
        let now = Instant::now();
        if notice.shown_until <= now {
            self.passthrough_notice = None;
            return;
        }
        let enabled = notice.enabled;
        let shown_until = notice.shown_until;
        let exit_duration =
            Duration::from_secs_f32(motion::duration(self.reduce_motion, motion::dur::BASE));
        let fade_out_at = shown_until.checked_sub(exit_duration).unwrap_or(now);
        let fading_out = now >= fade_out_at;
        let opacity = if fading_out {
            motion::animate_bool(
                ctx,
                "passthrough_notice_visibility",
                false,
                motion::dur::BASE,
                self.reduce_motion,
                motion::ease::exit,
            )
        } else {
            motion::animate_bool(
                ctx,
                "passthrough_notice_visibility",
                true,
                motion::dur::FAST,
                self.reduce_motion,
                motion::ease::entrance,
            )
        };
        let text = if enabled {
            tf(
                "Passthrough enabled · press {} to restore control",
                &[self.passthrough_hotkey.label()],
            )
        } else {
            tf(
                "Edit mode enabled · press {} to return to passthrough",
                &[self.passthrough_hotkey.label()],
            )
        };
        egui::Area::new(egui::Id::new("passthrough_notice"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                egui::Frame::new()
                    .fill(self.theme().notice_bg)
                    .stroke(Stroke::new(1.0_f32, self.theme().accent))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(18, 12))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(text)
                                .size(18.0)
                                .strong()
                                .color(contrast_text(self.theme().notice_bg)),
                        );
                    });
            });
        if fading_out {
            ctx.request_repaint_after(shown_until.saturating_duration_since(now));
        } else {
            ctx.request_repaint_after(fade_out_at.saturating_duration_since(now));
        }
    }

    fn push_status_toast(
        &mut self,
        viewport: egui::ViewportId,
        text: String,
        tone: ToastTone,
        duration: Duration,
        undo: Option<UndoState>,
    ) {
        while self.status_toasts.len() >= 5 {
            if let Some(toast) = self.status_toasts.pop_front() {
                self.undo_states.remove(&toast.id);
            }
        }
        let now = Instant::now();
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.wrapping_add(1).max(1);
        if let Some(undo) = undo {
            self.undo_states.insert(id, undo);
        }
        self.status_toasts.push_back(StatusToast {
            id,
            text,
            tone,
            viewport,
            shown_until: now + duration,
            last_tick: now,
            hovered: false,
            animation_seeded: false,
            undo_id: self.undo_states.contains_key(&id).then_some(id),
        });
    }

    fn dismiss_toast(&mut self, id: u64) {
        self.status_toasts.retain(|toast| toast.id != id);
        self.undo_states.remove(&id);
    }

    pub(crate) fn push_undo_toast(
        &mut self,
        viewport: egui::ViewportId,
        text: String,
        undo: UndoState,
    ) {
        self.last_status_toast = self.status.clone();
        self.push_status_toast(
            viewport,
            text,
            ToastTone::Success,
            UNDO_TOAST_DURATION,
            Some(undo),
        );
    }

    pub(crate) fn undo_latest(&mut self, viewport: egui::ViewportId) {
        let Some(id) = self
            .status_toasts
            .iter()
            .rev()
            .find_map(|toast| toast.undo_id)
        else {
            self.status = t("Nothing to undo");
            return;
        };
        self.apply_undo(id, viewport);
    }

    fn latest_combat_undo_id(&self) -> Option<u64> {
        self.status_toasts
            .iter()
            .rev()
            .filter_map(|toast| toast.undo_id)
            .find(|id| matches!(self.undo_states.get(id), Some(UndoState::CombatSession(_))))
    }

    fn apply_undo(&mut self, id: u64, viewport: egui::ViewportId) {
        let Some(undo) = self.undo_states.remove(&id) else {
            return;
        };
        match undo {
            UndoState::CombatSession(snapshot) => {
                self.status_toasts.retain(|toast| toast.id != id);
                if self.has_session_data() || self.capture.is_some() || self.replay_thread.is_some()
                {
                    self.status = t("Cannot restore the previous session after new data arrives");
                    self.last_status_toast = self.status.clone();
                    self.push_status_toast(
                        viewport,
                        self.status.clone(),
                        ToastTone::Warning,
                        STATUS_TOAST_DURATION,
                        None,
                    );
                    return;
                }
                let snapshot = *snapshot;
                self.state = snapshot.state;
                self.session_epoch = self.session_epoch.wrapping_add(1);
                self.reset_combat_view_state();
                self.capture_quality_source = snapshot.capture_quality_source;
                self.timeline_view = snapshot.timeline_view;
                self.hidden_character_ids = snapshot.hidden_character_ids;
                self.selected_abyss_half = snapshot.selected_abyss_half;
                self.abyss_compact_mode = snapshot.abyss_compact_mode;
                self.status = t("Session reset undone");
            }
            UndoState::HistoryRecord(record) => match history::restore_record(&record) {
                Ok(()) => {
                    self.status_toasts.retain(|toast| toast.id != id);
                    let record_id = record.id.clone();
                    self.history.reload();
                    self.history.selected_id = Some(record_id);
                    self.history.ensure_selection();
                    self.status = t("History deletion undone");
                }
                Err(error) => {
                    self.undo_states
                        .insert(id, UndoState::HistoryRecord(record));
                    self.set_last_error_for(
                        viewport,
                        tf("Failed to restore history summary: {}", &[&error]),
                        None,
                    );
                }
            },
        }
    }

    pub(crate) fn export_capture_info(&mut self, ctx: &egui::Context) {
        self.drain_pending_events();
        if self.state.hits.is_empty()
            && self.state.packets.is_empty()
            && self.state.empty_curtain.is_empty()
        {
            self.set_last_error_in(
                ctx,
                t("No capture info to export"),
                Some(ErrorAction::OpenConsole),
            );
            return;
        }
        if self.capture.is_some() || self.replay_thread.is_some() {
            self.set_last_error_in(
                ctx,
                t("Stop capture or replay first, then export this capture info"),
                None,
            );
            return;
        }

        let filter = t("Capture info JSON");
        self.spawn_file_dialog(ctx, FileDialogPurpose::CaptureInfoExport, move |owner| {
            with_owner(
                rfd::FileDialog::new()
                    .add_filter(filter, &["json"])
                    .set_file_name(default_export_filename()),
                owner,
            )
            .save_file()
        });
    }

    fn finish_capture_info_export(&mut self, viewport: egui::ViewportId, path: &Path) {
        // The UI stayed live while the dialog was open, so re-check that no
        // capture or replay started in the meantime.
        if self.capture.is_some() || self.replay_thread.is_some() {
            self.set_last_error_for(
                viewport,
                t("Stop capture or replay first, then export this capture info"),
                None,
            );
            return;
        }
        match atomic_write_file(path, |writer| {
            let mut out = IoFmtWriter::new(writer);
            self.write_capture_export_json(&mut out);
            out.finish()
        }) {
            Ok(()) => {
                self.status = t("Capture info exported");
                self.clear_last_error();
            }
            Err(error) => {
                self.set_last_error_for(
                    viewport,
                    tf("Failed to export capture info: {}", &[&error.to_string()]),
                    None,
                );
            }
        }
    }

    pub(crate) fn export_raw_capture(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() {
            self.set_last_error_in(
                ctx,
                t("Stop capture first, then save the full PCAPNG"),
                None,
            );
            return;
        }
        if self.raw_capture.is_none() {
            self.set_last_error_in(ctx, t("No full PCAPNG to save"), None);
            return;
        }
        let default_file_name = format!("nte_raw_{}.pcapng", Local::now().format("%Y%m%d_%H%M%S"));
        let filter = t("Full raw capture");
        self.spawn_file_dialog(ctx, FileDialogPurpose::RawCaptureExport, move |owner| {
            with_owner(
                rfd::FileDialog::new()
                    .add_filter(filter, &["pcapng"])
                    .set_file_name(default_file_name),
                owner,
            )
            .save_file()
        });
    }

    fn finish_raw_capture_export(&mut self, viewport: egui::ViewportId, destination: &Path) {
        // The UI stayed live while the dialog was open; the buffer may have been
        // cleared (or a capture restarted) in the meantime.
        if self.capture.is_some() {
            self.set_last_error_for(
                viewport,
                t("Stop capture first, then save the full PCAPNG"),
                None,
            );
            return;
        }
        let Some(raw_capture) = self.raw_capture.as_ref() else {
            self.set_last_error_for(viewport, t("No full PCAPNG to save"), None);
            return;
        };
        match raw_capture.save(destination) {
            Ok((packet_count, captured_bytes)) => {
                let file_name = destination
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.to_owned())
                    .unwrap_or_else(|| t("PCAPNG file"));
                self.status = tf(
                    "Saved the full capture to {} ({} packets, {} bytes)",
                    &[
                        &file_name,
                        &packet_count.to_string(),
                        &captured_bytes.to_string(),
                    ],
                );
                self.clear_last_error();
            }
            Err(error) => {
                self.set_last_error_for(
                    viewport,
                    tf("Failed to save the full capture: {}", &[&error.to_string()]),
                    None,
                );
            }
        }
    }

    pub(crate) fn write_capture_export_json(&self, mut out: &mut dyn std::fmt::Write) {
        let subtract_time_stop = self.subtract_time_stop_for_dps();
        let duration = self.state_duration_for_current_mode().max(0.001);
        let packet_count = self.state.packets.len();
        let hit_count = self.state.hits.len();
        let started_at = self.state.started_at;
        let ended_at = self
            .state
            .hits
            .iter()
            .map(|hit| hit.timestamp)
            .chain(self.state.packets.iter().map(|packet| packet.timestamp))
            .max_by(|left, right| left.total_cmp(right));

        let mut rows: Vec<_> = self.state.stats.values().collect();
        rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));

        writeln!(&mut out, "{{").ok();
        writeln!(
            &mut out,
            "  \"exported_at\": {},",
            json_string(&Local::now().format("%Y-%m-%d %H:%M:%S").to_string())
        )
        .ok();
        writeln!(&mut out, "  \"filter\": {},", json_string(&self.filter)).ok();
        writeln!(
            &mut out,
            "  \"include_incoming\": {},",
            self.include_incoming
        )
        .ok();
        if let Some(network) = &self.game_network {
            writeln!(&mut out, "  \"game_network\": {{").ok();
            writeln!(&mut out, "    \"pid\": {},", network.pid).ok();
            writeln!(
                &mut out,
                "    \"local_ip\": {},",
                json_string(&network.local_ip.to_string())
            )
            .ok();
            writeln!(
                &mut out,
                "    \"remote_ip\": {},",
                json_string(&network.remote_ip.to_string())
            )
            .ok();
            writeln!(&mut out, "    \"remote_port\": {}", network.remote_port).ok();
            writeln!(&mut out, "  }},").ok();
        } else {
            writeln!(&mut out, "  \"game_network\": null,").ok();
        }
        writeln!(&mut out, "  \"summary\": {{").ok();
        writeln!(&mut out, "    \"hits\": {},", hit_count).ok();
        writeln!(&mut out, "    \"packets\": {},", packet_count).ok();
        writeln!(
            &mut out,
            "    \"total_damage\": {},",
            json_f64(self.state.total_damage)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"dps\": {},",
            json_f64(self.state_dps_for_current_mode())
        )
        .ok();
        writeln!(
            &mut out,
            "    \"duration_seconds\": {},",
            json_f64(duration)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"dps_time_mode\": {},",
            json_string(self.dps_time_mode.label())
        )
        .ok();
        writeln!(
            &mut out,
            "    \"started_at_unix\": {},",
            json_option_f64(started_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"started_at_local\": {},",
            json_option_time(started_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"ended_at_unix\": {},",
            json_option_f64(ended_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"ended_at_local\": {}",
            json_option_time(ended_at)
        )
        .ok();
        writeln!(&mut out, "  }},").ok();

        writeln!(&mut out, "  \"party\": [").ok();
        for (index, row) in rows.iter().enumerate() {
            let share = if self.state.total_damage > 0.0 {
                row.damage / self.state.total_damage * 100.0
            } else {
                0.0
            };
            let row_duration = self
                .state
                .character_duration_with_time_stop(row, subtract_time_stop);
            let row_dps = self
                .state
                .character_dps_with_time_stop(row, subtract_time_stop);
            writeln!(&mut out, "    {{").ok();
            writeln!(&mut out, "      \"char_id\": {},", row.char_id).ok();
            writeln!(&mut out, "      \"name\": {},", json_string(&row.name)).ok();
            writeln!(&mut out, "      \"hits\": {},", row.hits).ok();
            writeln!(&mut out, "      \"damage\": {},", json_f64(row.damage)).ok();
            writeln!(&mut out, "      \"dps\": {},", json_f64(row_dps)).ok();
            writeln!(
                &mut out,
                "      \"duration_seconds\": {},",
                json_f64(row_duration)
            )
            .ok();
            writeln!(&mut out, "      \"share_percent\": {}", json_f64(share)).ok();
            writeln!(
                &mut out,
                "    }}{}",
                if index + 1 == rows.len() { "" } else { "," }
            )
            .ok();
        }
        writeln!(&mut out, "  ],").ok();

        writeln!(&mut out, "  \"abyss\": {{").ok();
        writeln!(
            &mut out,
            "    \"detected\": {},",
            self.state.abyss.is_active()
        )
        .ok();
        writeln!(
            &mut out,
            "    \"floor\": {},",
            self.state
                .abyss
                .floor
                .map_or_else(|| "null".to_owned(), |floor| floor.to_string())
        )
        .ok();
        writeln!(
            &mut out,
            "    \"active_half\": {},",
            self.state
                .abyss
                .active_half
                .map(|half| json_string(half.label()))
                .unwrap_or_else(|| "null".to_owned())
        )
        .ok();
        writeln!(
            &mut out,
            "    \"success_at_unix\": {},",
            json_option_f64(self.state.abyss.success_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"first_half_at_unix\": {},",
            json_option_f64(self.state.abyss.first_half_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"second_half_at_unix\": {},",
            json_option_f64(self.state.abyss.second_half_at)
        )
        .ok();
        writeln!(
            &mut out,
            "    \"exited_at_unix\": {},",
            json_option_f64(self.state.abyss.exited_at)
        )
        .ok();
        write_abyss_half_json(
            &mut out,
            "first_half",
            &self.state.abyss.first_half,
            subtract_time_stop,
            true,
        );
        write_abyss_half_json(
            &mut out,
            "second_half",
            &self.state.abyss.second_half,
            subtract_time_stop,
            false,
        );
        writeln!(&mut out, "  }},").ok();

        let empty_curtain = serde_json::to_string(&self.state.empty_curtain)
            .expect("validated Console equipment snapshot must serialize");
        writeln!(&mut out, "  \"empty_curtain\": {empty_curtain},").ok();

        writeln!(&mut out, "  \"hits\": [").ok();
        for (index, hit) in self.state.hits.iter().enumerate() {
            writeln!(&mut out, "    {{").ok();
            writeln!(
                &mut out,
                "      \"timestamp_unix\": {},",
                json_f64(hit.timestamp)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"time_local\": {},",
                json_string(&format_time(hit.timestamp))
            )
            .ok();
            writeln!(&mut out, "      \"char_id\": {},", hit.char_id).ok();
            writeln!(
                &mut out,
                "      \"char_name\": {},",
                json_string(&hit.char_name)
            )
            .ok();
            writeln!(&mut out, "      \"damage\": {},", json_f64(hit.damage)).ok();
            writeln!(
                &mut out,
                "      \"attack_type\": {},",
                hit.attack_type
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"gameplay_effect_index\": {},",
                hit.gameplay_effect_index
                    .map_or_else(|| "null".to_owned(), |value| value.to_string())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"gameplay_effect_name\": {},",
                hit.gameplay_effect_name
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"ability_name\": {},",
                hit.ability_name
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"damage_name\": {},",
                hit.damage_name
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"damage_attribute\": {},",
                hit.damage_attribute
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"follow_up_damage\": {},",
                json_f64(hit.follow_up_damage)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"follow_up_timestamp\": {},",
                hit.follow_up_timestamp
                    .map_or_else(|| "null".to_owned(), json_f64)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"follow_up_damage_name\": {},",
                hit.follow_up_damage_name
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"follow_up_attack_type\": {},",
                hit.follow_up_attack_type
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"follow_up_damage_attribute\": {},",
                hit.follow_up_damage_attribute
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"direction\": {},",
                json_string(&hit.direction)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_hp_before\": {},",
                json_f64(hit.target_hp_before)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_hp_after\": {},",
                json_f64(hit.target_hp_after)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_max_hp\": {},",
                json_f64(hit.target_max_hp)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_hp_percent\": {},",
                json_f64(hit.target_hp_percent)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_id\": {},",
                hit.target_id
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"target_name\": {},",
                hit.target_name
                    .as_deref()
                    .map(json_string)
                    .unwrap_or_else(|| "null".to_owned())
            )
            .ok();
            writeln!(&mut out, "      \"target_context\": [").ok();
            for (context_index, value) in hit.target_context.iter().enumerate() {
                writeln!(
                    &mut out,
                    "        {}{}",
                    json_string(value),
                    if context_index + 1 == hit.target_context.len() {
                        ""
                    } else {
                        ","
                    }
                )
                .ok();
            }
            writeln!(&mut out, "      ]").ok();
            writeln!(
                &mut out,
                "    }}{}",
                if index + 1 == hit_count { "" } else { "," }
            )
            .ok();
        }
        writeln!(&mut out, "  ],").ok();

        writeln!(&mut out, "  \"packets\": [").ok();
        for (index, packet) in self.state.packets.iter().enumerate() {
            writeln!(&mut out, "    {{").ok();
            writeln!(
                &mut out,
                "      \"timestamp_unix\": {},",
                json_f64(packet.timestamp)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"time_local\": {},",
                json_string(&format_time(packet.timestamp))
            )
            .ok();
            writeln!(
                &mut out,
                "      \"source\": {},",
                json_string(&packet.source.to_string())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"destination\": {},",
                json_string(&packet.destination.to_string())
            )
            .ok();
            writeln!(
                &mut out,
                "      \"direction\": {},",
                json_string(&packet.direction)
            )
            .ok();
            writeln!(&mut out, "      \"payload_len\": {},", packet.payload_len).ok();
            writeln!(
                &mut out,
                "      \"declared_ids\": {},",
                serde_json::to_string(&packet.declared_ids).unwrap_or_else(|_| "[]".to_owned())
            )
            .ok();
            writeln!(&mut out, "      \"parsed_hits\": {},", packet.parsed_hits).ok();
            writeln!(&mut out, "      \"note\": {},", json_string(&packet.note)).ok();
            writeln!(
                &mut out,
                "      \"payload_preview\": {},",
                json_string(&packet.payload_preview)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"payload_hex\": {},",
                json_string(&packet.payload_hex)
            )
            .ok();
            writeln!(
                &mut out,
                "      \"decoded_text\": {}",
                json_string(&packet.decoded_text)
            )
            .ok();
            writeln!(
                &mut out,
                "    }}{}",
                if index + 1 == packet_count { "" } else { "," }
            )
            .ok();
        }
        writeln!(&mut out, "  ]").ok();
        writeln!(&mut out, "}}").ok();
    }

    pub(crate) fn selected_party_state(&self) -> Option<&PartyCombatState> {
        self.state
            .abyss
            .is_active()
            .then(|| self.state.abyss.half(self.selected_abyss_half))
    }

    pub(crate) fn subtract_time_stop_for_dps(&self) -> bool {
        matches!(self.dps_time_mode, DpsTimeMode::TimeStopAdjusted)
    }

    pub(crate) fn party_duration_for_current_mode(&self, party: &PartyCombatState) -> f64 {
        party.duration_with_time_stop(self.subtract_time_stop_for_dps())
    }

    pub(crate) fn party_dps_for_current_mode(&self, party: &PartyCombatState) -> f64 {
        party.dps_with_time_stop(self.subtract_time_stop_for_dps())
    }

    pub(crate) fn state_duration_for_current_mode(&self) -> f64 {
        self.state
            .duration_with_time_stop(self.subtract_time_stop_for_dps())
    }

    pub(crate) fn state_dps_for_current_mode(&self) -> f64 {
        self.state
            .dps_with_time_stop(self.subtract_time_stop_for_dps())
    }

    pub(crate) fn character_duration_for_current_source(&self, row: &CharacterStats) -> f64 {
        if let Some(party) = self.selected_party_state() {
            party.character_duration_with_time_stop(row, self.subtract_time_stop_for_dps())
        } else {
            self.state
                .character_duration_with_time_stop(row, self.subtract_time_stop_for_dps())
        }
    }

    pub(crate) fn character_dps_for_current_source(&self, row: &CharacterStats) -> f64 {
        if let Some(party) = self.selected_party_state() {
            party.character_dps_with_time_stop(row, self.subtract_time_stop_for_dps())
        } else {
            self.state
                .character_dps_with_time_stop(row, self.subtract_time_stop_for_dps())
        }
    }

    pub(crate) fn detail_source(&self) -> (HitDetailSource, u64) {
        if self.state.abyss.is_active() {
            let party = self.state.abyss.half(self.selected_abyss_half);
            let source = match self.selected_abyss_half {
                AbyssHalf::First => HitDetailSource::AbyssFirst,
                AbyssHalf::Second => HitDetailSource::AbyssSecond,
            };
            (source, party.hits_generation)
        } else {
            (HitDetailSource::Global, self.state.hits_generation)
        }
    }

    pub(crate) fn note_detail_scroll_activity(&mut self, ctx: &egui::Context) {
        let scrolling = ctx.input(|input| {
            input.is_scrolling()
                || input.smooth_scroll_delta() != egui::Vec2::ZERO
                || ((self.hit_detail_char_id.is_some() || self.team_hit_detail_open)
                    && input.pointer.primary_down())
        });
        if scrolling {
            self.detail_last_scroll_activity = Some(Instant::now());
        }
    }

    pub(crate) fn detail_scroll_active(&self) -> bool {
        self.detail_last_scroll_activity
            .is_some_and(|last| last.elapsed() < DETAIL_CACHE_REFRESH_DELAY)
    }

    pub(crate) fn cached_skill_summaries(&mut self, char_id: u32) -> Vec<SkillDamageSummary> {
        let (source, generation) = self.detail_source();
        let key = SkillSummaryCacheKey {
            source,
            generation,
            char_id,
        };
        let structural_change = self
            .skill_summary_cache
            .key
            .as_ref()
            .is_none_or(|current| current.source != source || current.char_id != char_id);
        let generation_changed = self.skill_summary_cache.key.as_ref() != Some(&key);
        if generation_changed && self.skill_summary_cache.dirty_since.is_none() {
            self.skill_summary_cache.dirty_since = Some(Instant::now());
        }
        let refresh_due = structural_change
            || (generation_changed
                && !self.detail_scroll_active()
                && self
                    .skill_summary_cache
                    .dirty_since
                    .is_some_and(|dirty| dirty.elapsed() >= DETAIL_CACHE_REFRESH_DELAY));
        if refresh_due {
            let rows = aggregate_character_skill_damage(
                detail_hits_for_source(&self.state, source),
                char_id,
            );
            self.skill_summary_cache = SkillSummaryCache {
                key: Some(key),
                rows,
                dirty_since: None,
            };
        }
        self.skill_summary_cache.rows.clone()
    }

    pub(crate) fn cached_timeline_series(&mut self) -> TimelineSeries {
        let (source, generation) = self.detail_source();
        let subtract_time_stop = self.subtract_time_stop_for_dps();
        let bucket_seconds = config::sanitize_timeline_bucket_seconds(self.timeline_bucket_seconds);
        if (bucket_seconds - self.timeline_bucket_seconds).abs() > f32::EPSILON {
            self.timeline_bucket_seconds = bucket_seconds;
        }
        let key = TimelineCacheKey {
            source,
            generation,
            subtract_time_stop,
            bucket_millis: timeline_bucket_millis(bucket_seconds),
        };
        if self.timeline_cache.key.as_ref() != Some(&key) {
            let series = match source {
                HitDetailSource::Global => self
                    .state
                    .timeline(bucket_seconds as f64, subtract_time_stop),
                HitDetailSource::AbyssFirst => self.abyss_half_timeline_series(
                    AbyssHalf::First,
                    bucket_seconds as f64,
                    subtract_time_stop,
                ),
                HitDetailSource::AbyssSecond => self.abyss_half_timeline_series(
                    AbyssHalf::Second,
                    bucket_seconds as f64,
                    subtract_time_stop,
                ),
            };
            self.timeline_cache = TimelineCache {
                key: Some(key),
                series,
            };
        }
        self.timeline_cache.series.clone()
    }

    pub(crate) fn abyss_half_timeline_series(
        &self,
        half: AbyssHalf,
        bucket_seconds: f64,
        subtract_time_stop: bool,
    ) -> TimelineSeries {
        let mut series = self
            .state
            .abyss
            .half(half)
            .timeline(bucket_seconds, subtract_time_stop);
        if let (Some(start), Some(end)) = (series.start_timestamp, series.end_timestamp) {
            series.markers = self.state.abyss.timeline_markers_for_half(half, start, end);
        }
        series
    }

    pub(crate) fn cached_skill_breakdown(&mut self, char_id: Option<u32>) -> SkillBreakdown {
        let (source, generation) = self.detail_source();
        let key = SkillBreakdownCacheKey {
            source,
            generation,
            char_id,
        };
        if self.skill_breakdown_cache.key.as_ref() != Some(&key) {
            let breakdown = crate::engine::model::summarize_skill_breakdown(
                detail_hits_for_source(&self.state, source),
                char_id,
            );
            self.skill_breakdown_cache = SkillBreakdownCache {
                key: Some(key),
                breakdown,
            };
        }
        self.skill_breakdown_cache.breakdown.clone()
    }

    pub(crate) fn current_quality_summary(&self) -> CaptureQualitySummary {
        self.state
            .capture_quality_summary(self.capture_quality_source)
    }

    pub(crate) fn request_resource_audit(&mut self) {
        if self.resource_audit.loading {
            return;
        }
        self.resource_audit.loading = true;
        self.resource_audit.message = t("Checking runtime resources...");
        let sender = self.resource_audit_sender.clone();
        self.resource_audit_thread = Some(thread::spawn(move || {
            let summary = audit_runtime_resources();
            let _ = sender.send(summary);
        }));
    }

    /// Pick up texture sets decoded by the background loader thread and swap them
    /// into the live maps. Until a set arrives its map stays empty and draw-sites
    /// fall back gracefully, so this never blocks the first paint.
    pub(crate) fn drain_texture_loads(&mut self) {
        while let Ok(load) = self.texture_load_receiver.try_recv() {
            match load {
                TextureLoad::Avatars(map) => self.avatar_textures = map,
                TextureLoad::Attributes(map) => self.attribute_textures = map,
                TextureLoad::DamageDigits(map) => self.damage_digit_textures = map,
                TextureLoad::Reactions(map) => self.reaction_textures = map,
                TextureLoad::Monsters(map) => self.monster_textures = map,
                TextureLoad::Equipment(map) => self.equipment_textures = map,
            }
        }
    }

    /// Apply the startup capture-environment probe once it completes on its
    /// background thread. Guarded so a late result never overwrites a capture/replay
    /// already in flight or a device list a user-initiated refresh has populated.
    pub(crate) fn drain_device_detection(&mut self) {
        if !self.awaiting_device_detection {
            return;
        }
        let Ok(detection) = self.device_detection_receiver.try_recv() else {
            return;
        };
        self.awaiting_device_detection = false;
        if self.capture.is_some() || self.replay_thread.is_some() {
            return;
        }
        self.devices = detection.devices;
        self.selected_device = detection.selected_device;
        self.game_process_detected = detection.game_process_detected;
        self.game_network = detection.game_network;
        self.local_ip = detection.local_ip;
        self.status = detection.status;
        self.diagnostic = detection.diagnostic;
    }

    pub(crate) fn drain_game_process_monitor(&mut self) {
        while let Ok(result) = self.game_process_monitor_receiver.try_recv() {
            match result {
                Ok(detected) => {
                    self.game_process_detected = detected;
                    self.game_process_monitor_error = None;
                }
                Err(error) => {
                    self.game_process_monitor_error = Some(error);
                }
            }
        }
    }

    pub(crate) fn drain_resource_audit(&mut self) {
        while let Ok(summary) = self.resource_audit_receiver.try_recv() {
            let error_count = summary.error_count();
            let warning_count = summary.warning_count();
            self.resource_audit.summary = Some(summary);
            self.resource_audit.loading = false;
            self.resource_audit.message = tf(
                "Resource check complete: {} errors, {} warnings",
                &[&error_count.to_string(), &warning_count.to_string()],
            );
            if let Some(thread) = self.resource_audit_thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub(crate) fn request_capture_diagnostics(&mut self) {
        if self.diagnostics_running {
            return;
        }
        self.diagnostics_running = true;
        let sender = self.diagnostics_sender.clone();
        let snapshot = self.diagnostic_snapshot();
        self.diagnostics_thread = Some(thread::spawn(move || {
            let report = run_capture_diagnostics(snapshot);
            let _ = sender.send(report);
        }));
    }

    pub(crate) fn drain_capture_diagnostics(&mut self) {
        while let Ok(report) = self.diagnostics_receiver.try_recv() {
            let failed = report.failed_count();
            let warnings = report.warning_count();
            self.diagnostics_report = Some(report);
            self.diagnostics_running = false;
            self.status = tf(
                "Diagnostics complete: {} failed, {} warnings",
                &[&failed.to_string(), &warnings.to_string()],
            );
            if let Some(thread) = self.diagnostics_thread.take() {
                let _ = thread.join();
            }
        }
    }

    pub(crate) fn diagnostic_snapshot(&self) -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            capture_running: self.capture.is_some(),
            replay_running: self.replay_thread.is_some(),
            active_capture_filter: self.active_capture_filter.clone(),
            raw_packet_count: self
                .raw_capture
                .as_ref()
                .map_or(0, RawCaptureBuffer::packet_count),
            parsed_packet_count: self.state.packets.len(),
            hit_count: self.state.hits.len(),
            include_incoming: self.include_incoming,
            server_damage_calibration: self.server_damage_calibration,
            last_diagnostic: self.diagnostic.clone(),
        }
    }
}

/// Probe the capture environment: enumerate Npcap devices, then either honor the
/// persisted manual NIC override or auto-detect the HTGame.exe NIC. Folds in any
/// character-load error so the seeded status diagnostic matches the previous
/// synchronous startup behavior exactly. Pure aside from the OS queries, so it is
/// safe to run on the startup background thread.
fn detect_capture_environment(
    manual_capture_device: Option<&str>,
    character_load_error: Option<&str>,
) -> DeviceDetection {
    let game_process_probe = game_process_is_running();
    let (devices, device_error) = match list_devices() {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let (mut selected_device, mut game_network, mut status, mut diagnostic) = match device_error {
        Some(error) => (0, None, t("Capture environment unavailable"), Some(error)),
        None => match detect_game_device(&devices) {
            Ok((index, network)) => (index, Some(network), t("Ready"), None),
            Err(error) => (0, None, t("Game not detected"), Some(error)),
        },
    };
    // Apply the persisted manual NIC override (VPN fallback). The saved choice is kept even when
    // the interface is momentarily absent, so it re-engages once the adapter is back.
    if let Some(name) = manual_capture_device.filter(|_| !devices.is_empty()) {
        match devices.iter().position(|device| device.name == name) {
            Some(index) => {
                selected_device = index;
                match detect_game_network() {
                    Ok(network) => {
                        game_network = Some(network);
                        status = t("Ready (manual NIC)");
                        diagnostic = None;
                    }
                    Err(error) => {
                        game_network = None;
                        status = t("Manual NIC selected (no game connection detected)");
                        diagnostic = Some(error);
                    }
                }
            }
            None => {
                game_network = None;
                status = t("Manual NIC unavailable");
                diagnostic = Some(tf(
                    "The manually selected NIC ({}) is currently unavailable; reselect in settings or switch back to auto",
                    &[name],
                ));
            }
        }
    }
    if let Some(error) = character_load_error {
        diagnostic = Some(match diagnostic {
            Some(existing) => format!("{error}\n{existing}"),
            None => error.to_owned(),
        });
    }
    let game_process_detected = match game_process_probe {
        Ok(detected) => detected,
        Err(error) => {
            diagnostic = Some(match diagnostic {
                Some(existing) => format!("{existing}\n{error}"),
                None => error,
            });
            false
        }
    };
    let local_ip = game_network
        .as_ref()
        .map(|network| network.local_ip.to_string())
        .unwrap_or_default();
    DeviceDetection {
        devices,
        selected_device,
        game_process_detected,
        game_network,
        local_ip,
        status,
        diagnostic,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hotkeys_ignore_repeated_key_events() {
        let event = |repeat| egui::Event::Key {
            key: egui::Key::F9,
            physical_key: Some(egui::Key::F9),
            pressed: true,
            repeat,
            modifiers: egui::Modifiers::CTRL,
        };

        assert!(key_pressed_without_repeat(&[event(false)], egui::Key::F9));
        assert!(!key_pressed_without_repeat(&[event(true)], egui::Key::F9));
    }
}
