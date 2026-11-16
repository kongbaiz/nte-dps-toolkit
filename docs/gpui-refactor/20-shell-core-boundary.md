# 20 外壳/边界测绘：src-tauri 逐文件处置 + 核心可复用性证明

> 任务：为「用 gpui-kit 重构界面」提供外壳/边界侧基线。
> 唯一产出文件：本文件。未修改任何仓库代码；未运行 `cargo build`（仅 `read`/`grep`/`Get-Content`/只读 pwsh 统计）。
> 阅读优先级（Lead 指定）：§4 核心可复用性证明 > §5 Tauri 能力映射 > §2 事件通道 > §3 DTO > §6 打包 > §1 命令清单。
> 配套文档：`10-frontend-inventory.md`（前端处置）、`30-gpui-kit-capability.md`（gpui-kit 能力/许可）。本文只给「需求」，gpui 侧「能否实现」以 30 号文档为准。

---

## 0. 口径对账（先说数字怎么来的）

| 口径 | src-tauri/src | src/（核心） | 说明 |
| --- | --- | --- | --- |
| 全行（含空行、注释、测试） | **28,529** 行 / 80 个 `.rs` | **90,407** 行 / 83 个 `.rs` | `(Get-Content $f).Count` 逐文件求和 |
| 非空行 | **26,546** | **85,048** | 过滤 `$_.Trim() -eq ''`；Lead 基线 26,546 / 85,036 即此口径 |
| 测试行 | **7,994**（`state/tests.rs` 4,247 + 文件内 `#[cfg(test)] mod tests` 3,747） | 未单列 | 见附录 B 每文件 `测试` 列 |
| 生产行（全行−测试） | **20,535** | — | — |

- Lead 基线的 `commands(17)/channels(11)/windows(8)` 是「排除 `mod.rs`/`tests.rs`」的计数；按目录文件数分别为 **20 / 12 / 9**（含 `commands/mod.rs`、`channels/mod.rs`、`windows/mod.rs`），另 `state` 8（含 `tests.rs`），`contract` 17，根文件 14。
- 版本修正（以 `src-tauri/Cargo.lock` 为准）：`tauri` 锁定 **2.11.5**（`Cargo.toml` 写 `2.11.3` 为 caret 下限）、`wry 0.55.1`、`webview2-com 0.38.2`、`tauri-plugin-log 2.9.0`、`ts-rs 12.0.1`、`tokio 1.53.1`；工具链 `rust-toolchain.toml` = **1.98.0**。
- 复现命令见附录 A。

**一句话结论**：核心 crate **已经是 UI 无关的**（`src/` 90,407 行里 0 处 tauri/webview/egui/winit 依赖），gpui 单进程 app 可把 `src/` 当普通库依赖、零改动直接调用。重构面全在 `src-tauri/src`（80 文件 / 28,529 行）：**可直接复用 16,966 行（59.5%）、需改写成 gpui 壳 10,185 行（35.7%）、直接删除 1,378 行（4.8%）**（三分类账见 §8.1）。

---

## 1. 命令清单（144 条）

### 1.1 分布（`#[tauri::command]` 实际计数）

| 目录/文件 | 命令数 | 说明 |
| --- | --- | --- |
| `channels/*` | **21** | 10 组 subscribe/unsubscribe + `acknowledge_stream_delivery` |
| `commands/*`（19 个文件） | **122** | 每文件 1–32 条，见下表 |
| `windows/main_dps.rs` | **1** | `show_main_dps_when_ready`（首帧后显示主窗） |
| **合计** | **144** | 与 `lib.rs:161-306` 的 `generate_handler!` 清单逐条一致 |

`commands/` 逐文件：`main_dps` 32、`settings` 25、`technical` 9、`toolkit` 9、`history` 9、`empty_curtain` 7、`diagnostics` 6、`abyss_values` 5、`encrypted_ini` 5、`island` 3、`mod_studio` 2、`plugin_form` 2、`timeline` 2、`character_data` 1、`console_control` 1、`desktop_window` 1、`packets` 1、`skills` 1、`user_characters` 1。

**统一形状**：所有 144 条命令都额外带 `state: State<'_, AppState>` 与 `window: WebviewWindow`（其中 50 条还带 `app: AppHandle`）——下表已省略这三个框架参数，凡返回 `Result<X, CommandError>` 的只列 `X`；`()` 记为 `—`。共 **145 处 `validate_window(&window)?`** 窗口白名单校验、**400 行**框架参数签名、**57 处 `spawn_blocking`**、**62 行 `async_runtime`**。
gpui 单进程下：`State<'_, AppState>` → `&AppState` 字段、`WebviewWindow` → `Window`/`Entity` 句柄、`CommandError` → 直接返回 `Result` 或就地弹提示。

### 1.2 全量清单

| 命令 | 参数（去框架参数） | 返回 | 定义位置 | 前端调用方 |
| --- | --- | --- | --- | --- |
| `subscribe_diagnostics` | subscription_id: String, on_event: Channel<DiagnosticsEvent> | SubscriptionReceipt | channels/diagnostics.rs | diagnostics-client.ts |
| `unsubscribe_diagnostics` | subscription_id: String | — | channels/diagnostics.rs | diagnostics-client.ts |
| `subscribe_empty_curtain` | subscription_id: String, on_event: Channel<EmptyCurtainEvent> | SubscriptionReceipt | channels/empty_curtain.rs | empty-curtain-client.ts |
| `unsubscribe_empty_curtain` | subscription_id: String | — | channels/empty_curtain.rs | empty-curtain-client.ts |
| `subscribe_history` | subscription_id: String, on_event: Channel<HistoryEvent> | SubscriptionReceipt | channels/history.rs | history-client.ts |
| `unsubscribe_history` | subscription_id: String | — | channels/history.rs | history-client.ts |
| `subscribe_main_dps_detail` | subscription_id: String, on_event: Channel<MainDpsDetailSnapshot> | SubscriptionReceipt | channels/main_dps_detail.rs | main-dps-detail-client.ts |
| `unsubscribe_main_dps_detail` | subscription_id: String | — | channels/main_dps_detail.rs | main-dps-detail-client.ts |
| `subscribe_main_dps` | subscription_id: String, on_event: Channel<MainDpsEvent> | SubscriptionReceipt | channels/main_dps.rs | main-dps-client.ts |
| `unsubscribe_main_dps` | subscription_id: String | — | channels/main_dps.rs | main-dps-client.ts |
| `subscribe_packets` | subscription_id: String, on_event: Channel<PacketsEvent> | SubscriptionReceipt | channels/packets.rs | packets-client.ts |
| `unsubscribe_packets` | subscription_id: String | — | channels/packets.rs | packets-client.ts |
| `subscribe_settings` | subscription_id: String, on_event: Channel<SettingsEvent> | SubscriptionReceipt | channels/settings.rs | settings-client.ts |
| `unsubscribe_settings` | subscription_id: String | — | channels/settings.rs | settings-client.ts |
| `subscribe_skills` | subscription_id: String, scope: TimelineScope, on_event: Channel<SkillsEvent> | SubscriptionReceipt | channels/skills.rs | skills-client.ts |
| `unsubscribe_skills` | subscription_id: String | — | channels/skills.rs | skills-client.ts |
| `acknowledge_stream_delivery` | subscription_id: String, stream_kind: StreamKind, stream_generation: String, delivery_sequence: String | — | channels/stream_runtime.rs | stream-client.ts |
| `subscribe_technical_state` | subscription_id: String, on_event: Channel<TechnicalEvent> | SubscriptionReceipt | channels/technical.rs | technical-client.ts |
| `unsubscribe_technical_state` | subscription_id: String | — | channels/technical.rs | technical-client.ts |
| `subscribe_timeline` | subscription_id: String, scope: TimelineScope, on_event: Channel<TimelineEvent> | SubscriptionReceipt | channels/timeline.rs | timeline-client.ts |
| `unsubscribe_timeline` | subscription_id: String | — | channels/timeline.rs | timeline-client.ts |
| `get_abyss_values_snapshot` | — | AbyssValuesSnapshot | commands/abyss_values.rs | abyss-values-client.ts |
| `import_abyss_prediction_team` | half: AbyssHalf, json: String | AbyssPredictionTeamsSnapshot | commands/abyss_values.rs | abyss-values-client.ts |
| `import_current_abyss_prediction_team` | half: AbyssHalf | AbyssPredictionTeamsSnapshot | commands/abyss_values.rs | abyss-values-client.ts |
| `clear_abyss_prediction_team` | half: AbyssHalf | AbyssPredictionTeamsSnapshot | commands/abyss_values.rs | abyss-values-client.ts |
| `swap_abyss_prediction_teams` | — | AbyssPredictionTeamsSnapshot | commands/abyss_values.rs | abyss-values-client.ts |
| `get_character_data_snapshot` | — | CharacterDataSnapshot | commands/character_data.rs | character-data-client.ts |
| `execute_console_control` | action: ConsoleControlAction | — | commands/console_control.rs | console-control-client.ts |
| `set_desktop_window_always_on_top` | enabled: bool | — | commands/desktop_window.rs | desktop-window-client.ts |
| `get_diagnostics_snapshot` | — | DiagnosticsSnapshot | commands/diagnostics.rs | diagnostics-client.ts |
| `run_diagnostics` | — | DiagnosticsSnapshot | commands/diagnostics.rs | diagnostics-client.ts |
| `import_diagnostics_pcapng` | — | DiagnosticsActionResult | commands/diagnostics.rs | diagnostics-client.ts |
| `import_diagnostics_json` | — | DiagnosticsActionResult | commands/diagnostics.rs | diagnostics-client.ts |
| `export_diagnostics_json` | — | DiagnosticsActionResult | commands/diagnostics.rs | diagnostics-client.ts |
| `export_diagnostics_pcapng` | — | DiagnosticsActionResult | commands/diagnostics.rs | diagnostics-client.ts |
| `get_empty_curtain_snapshot` | — | EmptyCurtainSnapshot | commands/empty_curtain.rs | empty-curtain-client.ts |
| `get_empty_curtain_positions` | item: ItemUidInput, character: ItemUidInput | Vec<EmptyCurtainPositionSnapshot> | commands/empty_curtain.rs | empty-curtain-client.ts |
| `manage_empty_curtain_item` | item: ItemUidInput, action: EquipmentAction, character: Option<ItemUidInput>, row: Option<i32>, column: Option<i32> | EmptyCurtainSnapshot | commands/empty_curtain.rs | empty-curtain-client.ts |
| `apply_empty_curtain_character_action` | character: ItemUidInput, action: CharacterEquipmentAction | EmptyCurtainSnapshot | commands/empty_curtain.rs | empty-curtain-client.ts |
| `export_empty_curtain_inventory` | — | EmptyCurtainFileResult | commands/empty_curtain.rs | empty-curtain-client.ts |
| `export_empty_curtain_loadout` | character: ItemUidInput | EmptyCurtainFileResult | commands/empty_curtain.rs | empty-curtain-client.ts |
| `import_empty_curtain_loadout` | — | EmptyCurtainFileResult | commands/empty_curtain.rs | empty-curtain-client.ts |
| `get_encrypted_ini_snapshot` | — | EncryptedIniSnapshot | commands/encrypted_ini.rs | encrypted-ini-client.ts |
| `open_encrypted_ini` | — | OpenEncryptedIniResult | commands/encrypted_ini.rs | encrypted-ini-client.ts |
| `reload_encrypted_ini` | — | EncryptedIniSnapshot | commands/encrypted_ini.rs | encrypted-ini-client.ts |
| `save_encrypted_ini` | request: SaveEncryptedIniRequest | SaveEncryptedIniResult | commands/encrypted_ini.rs | encrypted-ini-client.ts |
| `clear_encrypted_ini` | — | EncryptedIniSnapshot | commands/encrypted_ini.rs | encrypted-ini-client.ts |
| `get_history_snapshot` | — | HistorySnapshot | commands/history.rs | history-client.ts |
| `import_history_record_file` | — | HistoryImportFileSnapshot | commands/history.rs | history-client.ts |
| `save_current_history_summary` | — | HistorySnapshot | commands/history.rs | history-client.ts |
| `import_history_record_json` | json: String | HistorySnapshot | commands/history.rs | history-client.ts |
| `delete_history_record` | record_id: String | HistoryDeleteSnapshot | commands/history.rs | history-client.ts |
| `restore_deleted_history_record` | undo_token: String | HistorySnapshot | commands/history.rs | history-client.ts |
| `export_history_record_file` | record_id: String | HistoryFileActionSnapshot | commands/history.rs | history-client.ts |
| `compare_history_records` | left_id: String, right_id: String | HistoryComparisonSnapshot | commands/history.rs | history-client.ts |
| `set_history_prediction_team` | record_id: String, line: AbyssHalf | HistorySnapshot | commands/history.rs | history-client.ts |
| `get_island_snapshot` | — | IslandSnapshot | commands/island.rs | island-client.ts |
| `dismiss_island_notice` | notice_id: String | IslandSnapshot | commands/island.rs | island-client.ts |
| `undo_island_notice` | notice_id: String | IslandSnapshot | commands/island.rs | island-client.ts |
| `get_main_dps_snapshot` | — | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `get_main_dps_update_prompt` | — | UpdatePromptSnapshot | commands/main_dps.rs | update-prompt-client.ts |
| `download_main_dps_update` | component: UpdateComponent | UpdatePromptSnapshot | commands/main_dps.rs | update-prompt-client.ts |
| `install_main_dps_update` | — | UpdatePromptSnapshot | commands/main_dps.rs | update-prompt-client.ts |
| `start_main_dps_capture` | replace_current: bool | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts, main-dps-detail-client.ts |
| `stop_main_dps_capture` | — | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `reset_main_dps_session` | confirmed: bool | MainDpsResetResult | commands/main_dps.rs | main-dps-client.ts |
| `undo_main_dps_reset` | undo_token: String | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `start_main_dps_new_round` | — | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `set_main_dps_paused` | paused: bool | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `select_main_dps_round` | record_id: Option<String> | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `set_main_dps_onboarding_step` | step: usize | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `finish_main_dps_onboarding` | hud_preset: HudPreset | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `select_main_dps_abyss_half` | half: Option<AbyssHalf> | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `import_main_dps_replay` | kind: CaptureReplayKind, replace_current: bool | MainDpsActionResult | commands/main_dps.rs | main-dps-client.ts, main-dps-detail-client.ts |
| `import_main_dps_replay_path` | path: String, replace_current: bool | MainDpsActionResult | commands/main_dps.rs | main-dps-client.ts |
| `open_main_dps_console_shortcut` | target: ConsoleShortcutTarget | — | commands/main_dps.rs | main-dps-client.ts |
| `set_main_dps_always_on_top` | enabled: bool | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `set_main_dps_appearance` | dark_mode: bool, opacity: f32 | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `open_main_dps_console` | — | — | commands/main_dps.rs | main-dps-client.ts |
| `open_main_dps_hud` | — | — | commands/main_dps.rs | main-dps-client.ts |
| `open_main_dps_character_details` | character_id: u32 | — | commands/main_dps.rs | main-dps-client.ts |
| `open_main_dps_team_details` | filter: MainDpsDetailFilter | — | commands/main_dps.rs | main-dps-client.ts |
| `get_main_dps_hit_snapshot` | hit_id: String, snapshot_key: String | HitSnapshotReply | commands/main_dps.rs | main-dps-detail-client.ts |
| `get_main_dps_detail_snapshot` | offset: usize, limit: Option<usize> | MainDpsDetailSnapshot | commands/main_dps.rs | main-dps-detail-client.ts |
| `set_main_dps_detail_view` | filter: MainDpsDetailFilter, qte_type: Option<String>, skill_filter: Option<String> | MainDpsDetailSnapshot | commands/main_dps.rs | main-dps-detail-client.ts |
| `set_main_dps_detail_columns` | columns: MainDpsDetailColumns | MainDpsDetailSnapshot | commands/main_dps.rs | main-dps-detail-client.ts |
| `set_main_dps_passthrough` | enabled: bool | MainDpsSnapshot | commands/main_dps.rs | main-dps-client.ts |
| `show_main_dps_from_hud` | — | — | commands/main_dps.rs | hud-window-client.ts |
| `minimize_main_dps_window` | — | — | commands/main_dps.rs | main-dps-client.ts |
| `toggle_main_dps_maximized` | — | — | commands/main_dps.rs | main-dps-client.ts |
| `close_main_dps_window` | — | — | commands/main_dps.rs | main-dps-client.ts |
| `get_mod_market_catalog` | — | ModMarketCatalogSnapshot | commands/mod_studio.rs | mod-studio-client.ts |
| `install_mod_market_item` | id: String | bool | commands/mod_studio.rs | mod-studio-client.ts |
| `get_packets_snapshot` | — | PacketsSnapshot | commands/packets.rs | packets-client.ts |
| `get_plugin_form` | file: String, expected_identity: String | PluginFormSnapshot | commands/plugin_form.rs | plugin-form-catalog.ts |
| `invoke_plugin_form_action` | request: PluginFormActionRequest | ReleaseResult | commands/plugin_form.rs | plugin-form-panel.tsx |
| `set_settings_interface` | settings: InterfaceSettingsInput | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_update_preferences` | settings: UpdateSettingsInput | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `check_settings_updates` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `download_settings_update` | component: UpdateComponent | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `install_settings_update` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_capture` | settings: CaptureSettingsInput | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `refresh_settings_capture_devices` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hotkeys_enabled` | enabled: bool | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hotkey_binding` | action: GlobalHotkeyAction, binding: Option<HotkeyBindingSnapshot> | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_main_dps_display` | settings: MainDpsDisplayConfig | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `apply_settings_layout_profile` | profile: LayoutProfile | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `import_settings_team_data` | json: String | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `import_settings_team_data_file` | — | TeamDataImportFileResult | commands/settings.rs | settings-client.ts |
| `export_settings_team_data` | — | TeamDataExportResult | commands/settings.rs | settings-client.ts |
| `refresh_settings_capture_files` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `clear_settings_capture_files` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `open_settings_abyss_values` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `get_settings_snapshot` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hud_option` | option: HudSettingOption, enabled: bool | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `apply_settings_hud_preset` | preset: HudPreset | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hud_module_visibility` | module: HudModule, visible: bool | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `move_settings_hud_module` | dragged: HudModule, target: HudModule, insert_after: bool | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hud_width` | width: u16 | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `set_settings_hud_always_on_top` | enabled: bool | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `open_settings_hud_editor` | — | SettingsSnapshot | commands/settings.rs | settings-client.ts |
| `get_skills_snapshot` | scope: TimelineScope | SkillsSnapshot | commands/skills.rs | skills-client.ts |
| `get_technical_snapshot` | — | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `set_hud_passthrough` | enabled: bool | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `set_hud_always_on_top` | enabled: bool | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `move_hud_module` | dragged: HudModule, target: HudModule, insert_after: bool | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `set_hud_module_visibility` | module: HudModule, visible: bool | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `set_hud_width` | width: u16 | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `start_hud_capture` | — | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `stop_hud_capture` | — | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `reset_hud_session` | — | TechnicalSnapshot | commands/technical.rs | technical-client.ts |
| `get_timeline_snapshot` | scope: TimelineScope | TimelineSnapshot | commands/timeline.rs | timeline-client.ts |
| `set_timeline_preferences` | scope: TimelineScope, bucket_seconds: f32, view_mode: TimelineDpsViewMode | TimelineSnapshot | commands/timeline.rs | timeline-client.ts |
| `get_plugin_panel` | — | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `set_data_mode` | mode: DataMode, acknowledge_risk: bool | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `control_plugin` | action: PluginControlAction, file: String, expected_identity: String | — | commands/toolkit.rs | toolkit-client.ts |
| `host_action` | action: HostAction, expected_identity: String, confirmed: bool | ReleaseResult | commands/toolkit.rs | host-runtime-controls.tsx |
| `set_host_loading_method` | method: ModStudioLoadingMethod | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `select_host_game_directory` | — | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `clear_host_game_directory` | — | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `uninstall_proxy_host` | confirmed: bool | PluginPanel | commands/toolkit.rs | toolkit-client.ts |
| `launch_plugin_host` | force_overwrite: bool | HostLaunchResult | commands/toolkit.rs | toolkit-client.ts |
| `get_user_characters` | request: CharacterRequest | CharacterResponse | commands/user_characters.rs | user-characters-client.ts |
| `show_main_dps_when_ready` | — | — | windows/main_dps.rs | window-ready.ts |

### 1.3 结构结论

1. **没有死命令**：144 条全部能在 `frontend/src` 找到调用点（19 个 `lib/tauri/*-client.ts` + `features/mod-studio/*` 2 处 + `window-ready.ts`/`stream-client.ts`）。
2. **没有业务规则**：命令体一律是 `validate_window(&window)?` → `state.xxx(...)?` → 可选 `island::publish_notice(...)` → `snapshot()` 返回。122 条 `commands/*` 中，真正需要重写的是：窗口白名单（145 处调用）、通知岛发布、文件对话框、`spawn_blocking` 调度；其余是参数解包与状态调用。
3. **144 → 收敛为 ~30 个 action**：同域命令可合并（如 `main_dps` 32 条中 15 条都返回 `MainDpsSnapshot`，在 gpui 里就是「改状态 + 重投影」一件事）。
4. **21 条通道命令整体删除**（§2.4），`show_main_dps_when_ready` 删除（gpui 首帧渲染后才显示，无需回调）。

---

## 4. 核心可复用性证明（最高优先级）

### 4.1 判定摘要

| 判定 | 结论 | 证据 |
| --- | --- | --- |
| 核心是否已 UI 无关 | **是**（结构性证明，非推测） | `src/` 内 `grep -E 'tauri|webview|egui|eframe|wgpu|winit'` 命中 69 处，**全部**是 `#[cfg_attr(feature = "tauri_frontend", derive(ts_rs::TS))]` 形式的 TypeScript 绑定属性（54 处 `ts_rs::TS` + 4 处 `ts(export)`），**无一处 UI 框架调用** |
| 核心对外 API 形态 | **「快照 + revision + 命令式请求」，不是回调、不是 UI 事件循环** | `LiveCaptureService` 提供 `revision()/with_state()/state_and_source_snapshot()/request_start()/request_stop()/cut_round()/reset_session()`；`src-tauri` 侧统一用「每 N ms 比对 revision，变了才投影快照」拉取（见 §2） |
| core 里有没有 crossbeam 事件推给 UI | **没有**。crossbeam 只在 `engine/capture.rs`（抓包线程→解析）、`core/live_capture.rs`（帧队列）、`cli/stdio.rs`（JSON-RPC 队列）内部使用 | `core/equipment_rpc.rs` 用 `std::mpsc::sync_channel(1)` 做 RPC 请求/应答（同步语义），也不是 UI 事件总线 |
| 窗口/系统能力是否已在 core | **部分已下沉，且入参是裸 `HWND: isize`，与 UI 框架无关** | `platform/window_style.rs`：`apply_persistent_acrylic(hwnd: isize, bool)`、`apply_borderless_rounding(hwnd: isize)`、`apply_uniform_opacity(hwnd: isize, f32)`；`platform/passthrough_hotkey.rs`：`PassthroughHotkeyHandle::start(HotkeyBinding, GlobalHotkeys) -> (Self, Receiver<PassthroughHotkeyEvent>)` |
| 桌面配置是否已在 core | **是数据，不是行为** | `storage/config.rs`（2,197 行）持有 `HudConfig`/`HudModule`/`GlobalHotkeys`/窗口几何（`main_window_geometry`、`console_window_geometry`、`hud_window_position`、`island_offset_x` 等），全部是 serde 持久化结构，不含窗口调用 |
| 结论 | **新 gpui app crate 只需 `nte-dps-tool = { default-features = false, features = ["desktop"] }`**；`tauri_frontend` feature 删除即可（其唯一作用是给 54 处类型加 `ts-rs` 派生，删掉后行为不变） | `Cargo.toml:10-20`；`src-tauri/Cargo.toml:39` 当前正是这种用法 |

### 4.2 src/ 逐模块「零改动可用」清单

| 模块 | 文件数 | 行数 | 对 gpui app 的可用性 | 备注 |
| --- | --- | --- | --- | --- |
| `engine/` | 19 | 43,141 | **100% 零改动**（其中 `engine/capture/tests.rs` 8,562 行、`engine/settlement/tests.rs` 537 行为测试） | 解析/模型/协议/结算，与 UI 无关；只有 `--features cli` 和桌面共用 |
| `storage/` | 14 | 14,148 | **100% 零改动** | `config.rs`(2,197)、`history.rs`(5,631)、`mod_scripts.rs`(2,296)、`update.rs`(1,125) 等；含 i18n 字典与资源嵌入消费 |
| `platform/` | 11 | 6,611 | **100% 零改动**；`window_style`/`passthrough_hotkey` 直接拿 `isize` HWND，gpui 窗口也能拿到 HWND | `mods_plugin.rs`(3,153)、`toolkit.rs`(573)、`update_http.rs`(883)、`update_install.rs`(460) |
| `core/` | 27 | 21,166 | **100% 零改动** | 已含纯投影层：`hud.rs`(1,027)、`timeline.rs`(517)、`skills.rs`(454)、`packets.rs`(412)、`combat_details.rs`(266)、`history.rs`(269)、`diagnostics.rs`(766) |
| `api/` + `cli/` + `bin/` | 11 | 5,328 | 只服务于 `--features cli` 的 `nte-core` 旁路，桌面二进制不编入 | 与 gpui 重构无关，保留 |
| `lib.rs` | 1 | 13 | 零改动 | 模块声明 |

**核心零改动可用的具体形态（可直接写进新 app 的调用）：**

| 能力 | 现成接口（core 侧，签名有删节） | gpui app 里怎么用 |
| --- | --- | --- |
| 启动/刷新 HUD 视图 | `core::hud::project_hud(state: &CombatState, config: &HudConfig, hidden: &HashSet<u32>, options: HudProjectionOptions) -> HudSnapshot` | 在 `Entity::update` 里调用，`HudSnapshot` 直接喂给 HUD 组件树 |
| 主面板读数 | `LiveCaptureService::with_state(\|s\| ...)` + `state::combat::main_dps_readout()`（该函数在 src-tauri，但只依赖 core） | 同左；`MainDpsReadout` 是纯结构 |
| 抓包控制 | `LiveCaptureService::request_start(CaptureControllerOptions)` / `request_stop()` / `status()` / `replay_running()` | 直接调用，无需窗口参数 |
| 回放/导入 | `core::live_capture::request_replay()`、`engine::capture::prepare_capture_json_replay()` | 直接调用 |
| 历史归档 | `core::live_capture::cut_round()`、`storage::history::*` | 直接调用 |
| 全局热键 | `platform::passthrough_hotkey::PassthroughHotkeyHandle::start(...) -> (Handle, Receiver<PassthroughHotkeyEvent>)` | `cx.spawn` 起一个任务 `recv_timeout` 轮询，收到事件后 `entity.update` |
| 透明/Acrylic/圆角/整体不透明度 | `platform::window_style::{apply_persistent_acrylic, apply_borderless_rounding, apply_uniform_opacity}(hwnd: isize, ...)` | gpui 窗口 `window.hwnd()` 取出 HWND 传入即可 |
| 更新链路 | `core::update`、`platform::update_http`、`platform::update_install`、独立 `nte-updater.exe` | 直接调用；唯一 Tauri 点是「启动 updater 后 `app.exit(0)`」（见 §5.10） |

### 4.3 src-tauri 目前做了哪些「适配」（这些就是 gpui 侧要重写的部分）

| 适配类别 | 现状实现位置 | 是否 UI 框架强绑定 | gpui 侧做法 |
| --- | --- | --- | --- |
| **线程/异步** | `tauri::async_runtime::{spawn, spawn_blocking}`（`async_runtime` 62 行、`spawn_blocking` 57 处，Tauri 内置 tokio 运行时） | 是（API 形式） | 用 gpui `cx.background_executor().spawn()` / 自建 tokio 运行时；逻辑体原样搬运 |
| **节流/轮询** | `channels/stream_runtime.rs`：`tokio::time::interval` + `MissedTickBehavior::Skip`，间隔 100–500 ms | 是（tokio 定时器） | `cx.spawn` + `Timer`（或保留 tokio，单进程无冲突） |
| **生命周期** | `lib.rs:56-160` setup 里取 7 个窗口句柄、`on_page_load` 首次显示、`on_window_event(Destroyed)` 清流、`RunEvent::Exit` 关闭流 | 是 | gpui `Application::run` 内创建窗口 + `on_window_closed`/`Drop` |
| **窗口联动** | 关闭主窗销毁 6 个从窗（`windows/main_dps.rs:137-153`）；Console/Details 关闭改为隐藏（`windows/console.rs:53-61`）；island 关闭改隐藏（`windows/island.rs:173-181`） | 是（窗口 API） | gpui 窗口关闭回调 + 手动 `drop`/隐藏语义 |
| **窗口几何持久化** | 3 套几乎相同的「debounce 180–350 ms 写盘线程」：`windows/hud.rs:88-157`、`windows/console.rs:36-137`、`windows/combat_details.rs:72-145`，+ `windows/main_dps.rs:200-226` | 线程部分无关，读取窗口状态部分绑定 | 保留 debounce 逻辑，替换「读 `outer_position/inner_size/scale_factor`」为 gpui window bounds |
| **窗口位置可达性** | `windows/window_position.rs`（159 行，纯函数 `restored_window_position`） | 只依赖 monitor 列表，**可搬运** | 用 gpui 显示器信息重建入参 |
| **HUD 原生外观** | `windows/hud.rs:55-64` → core `window_style`（HWND） | 仅窗口句柄绑定 | 直接复用 core |
| **热键派发** | `windows/passthrough_hotkey.rs`（274 行）：50 ms `recv_timeout` 派发 `Toggle`/`GlobalAction`，失败回退编辑态 | 事件源在 core，只有窗口操作用 Tauri | **逻辑可搬运**，`app.get_webview_window(...)` 换成 `app.windows()` 查表 |
| **更新安装后退出** | `commands/settings.rs:165-171`：`update_storage::launch_prepared_app_update()` 成功后 `std::thread::spawn(|| { sleep(200ms); app.exit(0) })` | 仅 `app.exit` | `cx.quit()` / `std::process::exit(0)` |
| **单实例/托盘/自启动** | **不存在**（全仓 grep `tray`/`autostart`/`single_instance` 无匹配） | — | 无需能力，也无历史包袱 |

### 4.4 src/ 中「隐含 UI 假设」的完整清单（只有 4 处，且都是数据/文案）

| # | 位置 | 假设内容 | 对 gpui 的影响 |
| --- | --- | --- | --- |
| 1 | `storage/config.rs` | `UiConfig` 里含 7 个窗口的几何、`hud_width`、`hud_window_position`、`island_offset_x`、`island_notifications`、`opacity`、`hud_always_on_top`、`windows_always_on_top` | 数据兼容：gpui app 继续读写同一份配置文件即可，无需改 core（新窗口尺寸字段可共用/扩展） |
| 2 | `core/hud.rs:73,90` | `HudProjectionOptions`/`HudConfig.show_passthrough_state` 决定 HUD 是否显示「穿透中」提示 | 纯展示字段，gpui 直接消费 |
| 3 | `storage/i18n.rs` + `res/languages/zh-CN.json` | 所有用户文案由 Rust 侧 `t()`/`tf()` 生成（含 HUD 文案、island 通知文案） | 好消息：gpui 无需重做 i18n；坏消息：文案里硬编码了「按 F12/Home 键」等文本，与热键绑定联动（`state.passthrough_hotkey().label()`） |
| 4 | `core/diagnostics.rs`、`core/update.rs` 等 | 输出 `&'static str` code + `message_key`（供前端 i18n 查表） | gpui 可继续沿用「code + message_key」错误模型，无需改动 |

**结论**：核心不存在需要在 gpui 里「重写」的 UI 假设；`src/` 可执行文件零改动复用，唯一建议的 crate 变更见 4.5。

### 4.5 建议的 crate 边界

| 方案 | 内容 | 评价 |
| --- | --- | --- |
| **保留根 crate `nte-dps-tool`（`src/`）** | 删除 `tauri_frontend` feature（`Cargo.toml:20`）、删除 4 处 `ts(export)`/54 处 `cfg_attr(ts_rs::TS)`、`build.rs` 去掉 `CARGO_FEATURE_TAURI_FRONTEND` 分支（`build.rs:34,65,169`） | **推荐**。副作用仅 1 个：前端图片资源重新由 Rust 嵌入（当前 `tauri_frontend` 把它排除，交给 Vite 打包，`build.rs:169 is_frontend_image_resource`） |
| 新增 `crates/app-desktop`（bin = `nte-dps-tool`） | 依赖 `nte-dps-tool{desktop}` + gpui/gpui-kit + log（见下）；吸收 `src-tauri/src/state/`、8 个 service/runtime、`contract/` 改造的 view-model、窗口几何/位置纯函数、热键派发逻辑 | 推荐。见 §8.2 详表 |
| 删除 `src-tauri/` 与 `frontend/` | 28,529 + 前端全量 | 目标态；迁移期可保留 `src-tauri` 作对照构建 |
| 不动 `nte-updater` / `nte-core` | 两个 bin 已与 Tauri 解耦（`src/bin/nte-updater.rs` 24 行只调 `platform::update_install::run_from_args()`） | 必须保留 |

> 搬 `state/` 的唯一依赖缺口：`state/` 与 8 个 service 使用 `log::*` 宏（如 `state.rs:53`、`history_runtime.rs:29`），而根 `Cargo.toml` 目前**没有** `log` 依赖 → 新 app crate 需加 `log` + 一个 logger（当前 `tauri-plugin-log` 只在 `debug_assertions` 下初始化，`lib.rs:152-158`）。

---

## 5. Tauri 能力逐项映射（次高优先级）

### 5.1 tauri.conf.json 的 7 个窗口 → gpui 窗口

所有窗口 `decorations: false`、`visible: false`（首帧后由前端/Rust 显示）、`resizable` 见列。`backgroundColor` 仅 4 个不透明窗口声明，`hud-spike`/`notification-island` 透明。

| # | label | title | url | 尺寸 (w×h) / min / max | transparent | alwaysOnTop | skipTaskbar | shadow | backgroundColor | 语义 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | `main-dps` | NTE DPS TOOL | main-dps.html | 670×560 / 420×360 | false | false（可切） | false | true | `#f6f7f8` | 主窗口，**拥有整个桌面生命周期** |
| 2 | `hud-spike` | NTE DPS TOOL - HUD Spike | hud.html | 380×238 / 280×180 | **true** | **true**（可切） | **true** | false | — | 游戏内 HUD 浮层，Acrylic + 圆角 + 鼠标穿透 |
| 3 | `notification-island` | NTE DPS TOOL - Notification | notification.html | 440×76 / 320×64 / 560×120 | **true** | **true** | **true** | false | — | 顶部通知岛，5 s 自动消失、不可聚焦 |
| 4 | `console` | NTE Console | console.html | 1180×760 / 820×560 | false | false（可切） | false | true | `#f6f7f8` | 控制台（F12），关闭=隐藏 |
| 5 | `abyss-values` | NTE Abyss Values | abyss-values.html | 1100×720 / 760×480 | false | false（可切） | false | true | —（未声明） | 深渊数值表 |
| 6 | `character-details` | …Character Combat Details | combat-details.html | 880×620 / 620×420 | false | false（可切） | false | true | `#f6f7f8` | 角色详情，关闭=隐藏 |
| 7 | `team-details` | …Team Combat Details | combat-details.html | 980×680 / 620×420 | false | false（可切） | false | true | `#f6f7f8` | 队伍详情，关闭=隐藏（与 6 同一 HTML，用 label 区分 kind） |

gpui 侧要点：
- 6 与 7 共用同一视图实现 + 一个 `enum DetailKind`，gpui 里是「同一 `Entity<DetailView>` 两个窗口」还是「两个实例」，由 30 号文档的窗口能力决定；`MainDpsDetailKind`（`state.rs:807`）已是现成枚举。
- 1–7 全部 `decorations:false` → 自绘标题栏，前端 `components/nte/desktop-titlebar.tsx` 用 `onPointerDown → getCurrentWindow().startDragging()`，并有 pin/最小化/最大化/关闭四个按钮（`lib/tauri/desktop-window-client.ts:6-12`）。
- 窗口初始 `visible:false` + `on_page_load(Finished)` 才显示（`lib.rs:34-45`）→ gpui 里「首帧渲染完成再 show」是默认行为，这段适配**可以整段删掉**。

### 5.2 capabilities 的 7 个 JSON → **全部删除**

| 文件 | 授权窗口 | 权限 | gpui 处理 |
| --- | --- | --- | --- |
| `default.json` | hud-spike | `core:default`、`allow-start-dragging` | 删除（无 IPC 权限模型） |
| `main-dps.json` | main-dps | `core:default`、start-dragging、minimize、toggle-maximize、close、window set-background-color、webview set-background-color | 删除；对应能力改为 Rust 直接调窗口 API |
| `console.json` | console | 同上 | 删除 |
| `abyss-values.json` | abyss-values | 同上 | 删除 |
| `combat-details.json` | character-details + team-details | 同上 | 删除 |
| `desktop-window-topmost.json` | 6 个桌面窗口 | `allow-set-always-on-top` | 删除（`set_always_on_top` 变成 Rust 调用） |
| `notification-island.json` | notification-island | `core:default`、window/webview set-background-color | 删除 |

合计 92 行 JSON + `tauri.conf.json:153-156` 的两条 CSP（含 `unsafe-eval`）→ 全部随 Tauri 消失。**这是净收益：攻击面与「权限文案」一起消失。**

### 5.3 无边框自绘标题栏 / 最小化 / 关闭 / 置顶

| 能力 | 现实现 | 证据 | gpui 侧需求（待 30 号文档核实实现方式） |
| --- | --- | --- | --- |
| 拖动窗口 | 前端 `startDragging()`（`desktop-window-client.ts:6`、`technical-hud-page.tsx:105`、`desktop-titlebar.tsx:65`） | capability `allow-start-dragging` | 需要「按住标题栏拖动」API（gpui 有 `Window::drag_window`，需确认 gpui-kit 暴露） |
| 最小化 | `getCurrentWindow().minimize()`；HUD 走 `window.minimize()` | `lib.rs:39` 等 | `Window::minimize` |
| 最大化切换 | `toggleMaximized()`（`desktop-window-client.ts:8`） | — | `Window::zoom`/等价 |
| 关闭按钮 | 主窗真关闭；其余窗口 `CloseRequested → prevent_close + hide` | `windows/console.rs:53-61`、`windows/combat_details.rs:133-138`、`windows/island.rs:173-181` | gpui 窗口关闭回调里 `window.remove_window()` 或仅隐藏 |
| 置顶 | 每窗口独立偏好（`state.set_window_always_on_top(kind, bool)`，`state/settings.rs`），启动时 6 窗口逐个 `set_always_on_top`（`lib.rs:101-113`），Console 的 pin 按钮调用 `execute_console_control(TogglePin)`（`commands/console_control.rs:126-136`） | `desktop-window-topmost.json` | `Window::set_always_on_top`（Win32 `SetWindowPos HWND_TOPMOST` 兜底） |
| 系统主题跟随 | 前端 `prefers-color-scheme` + `syncConsoleWindowBackground()` 给不透明窗口设 `#09090b`/`#f6f7f8`（`console-window-background.ts:18-20`） | capability `set-background-color` | gpui：窗口背景由根视图 `div().bg(...)` 绘制，**天然可做**，无需窗口 API |

### 5.4 透明 HUD + 鼠标穿透 + Acrylic + 圆角（最高风险项）

| 环节 | 现实现（可复用性） | 证据 |
| --- | --- | --- |
| 窗口透明 | `tauri.conf.json` `transparent:true`（WebView 内容 alpha 合成） | hud / island |
| Acrylic 背景 | **core** `platform::window_style::apply_persistent_acrylic(hwnd, bool)`：`user32!SetWindowCompositionAttribute` + `ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND`，注释明确解释了为何不用 Tauri 的 `Acrylic`/`Blur`（W11 失焦淡出、WebView2 黑闪） | `window_style.rs:55-94`；调用点 `windows/hud.rs:55-58`、`lib.rs:114` |
| 原生圆角 + 去系统边框 | **core** `apply_borderless_rounding(hwnd)`：`DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE=ROUND)` + `DWMWA_BORDER_COLOR=0xFFFFFFFE` | `window_style.rs:96-128` |
| 整体不透明度（主窗） | **core** `apply_uniform_opacity(hwnd, f32)`：`WS_EX_LAYERED`+`SetLayeredWindowAttributes`，alpha clamp 0.35–1.0 | `window_style.rs:130-154`；调用点 `windows/main_dps.rs:65-89` |
| 鼠标穿透 | Tauri `window.set_ignore_cursor_events(bool)`；开关前必须 `state.passthrough_hotkey_ready()`，否则报 `passthrough_hotkey_unavailable`；切换后要重新 Acrylic/圆角/高度（`windows/hud.rs:304-348`）；主窗还有「穿透后再重设不透明度」的补偿（`windows/main_dps.rs:41-56`） | `windows/hud.rs:325-348` |
| 穿透/编辑互斥事务 | `state.lock_passthrough_transaction()`（`settings_service.rs` 的 `TransactionState`） | `windows/hud.rs:309-323` |
| 穿透热键 | **core** 低级键盘钩子 `WH_KEYBOARD_LL`（`platform/passthrough_hotkey.rs`，472 行），事件 `Toggle`/`GlobalAction`/`HookInstalled`/`HookInstallFailed`，派发器 50 ms 轮询（`windows/passthrough_hotkey.rs:22,58-107`），钩子失败时**回退到编辑态**（`restore_editing_after_hook_failure`） | 已完全在 core + 可搬运派发器 |
| 高度锁定 | HUD 高度按模块内容锁定（`state.hud_initial_height()`），改高度时按「先 max 后 min / 先 min 后 max」顺序设约束避免 Windows 拒绝（`windows/hud.rs:244-302`） | **纯算法 + 窗口尺寸 API**，可搬运 |

**判别**：除「`set_ignore_cursor_events`」和「透明窗口创建」两个窗口 API 需要 gpui/win32 等价物外，HUD 的全部特效代码已在 core 且与 UI 框架无关。风险集中在 gpui 是否能创建 **transparent + click-through + always-on-top + skip-taskbar 且不抢焦点** 的窗口（`skipTaskbar` 在 Win32 是 `WS_EX_TOOLWINDOW`，gpui 若不暴露需自行 `SetWindowLongPtr`）。→ 见 §7 R1。

### 5.5 notification island（通知岛）行为清单

| 行为 | 现实现 | 证据 |
| --- | --- | --- |
| 显示条件 | `UiConfig.island_notifications` 为真才显示 | `windows/island.rs:40-42` |
| 屏幕定位 | 取「最后聚焦的源窗口」所在显示器 → 水平居中 + `island_offset_x` 偏移 + 12 px 边距 clamp，垂直固定 10 px | `windows/island.rs:46-61,119-141` |
| 每次唤醒重设 | 重新 `set_always_on_top(true)`、`set_skip_taskbar(true)`、`unminimize`、`show`，并 `emit_to(island, "notification-island-changed")` 让前端重渲染 | `windows/island.rs:62-79` |
| 不可聚焦 | `lib.rs:97 set_focusable(false)` | — |
| 关闭=隐藏 | `bind_close_to_hide` | `windows/island.rs:173-181` |
| 5 s 自动过期 + 撤销 | `ISLAND_NOTICE_WINDOW = 5s`（`state.rs:1273`），`publish_island_notice`/`dismiss`/`undo`（`commands/island.rs`），`IslandNoticeRuntime`（`state.rs:666-768`） | 逻辑在「A 类可搬运」部分 |
| 文案来源 | Rust `&'static str` + `IslandTone`，前端只渲染 | `channels/main_dps.rs:97-137` 里甚至在通道轮询里发布通知 |

> 单进程 gpui 下，island 可降级为「主窗顶部的浮层 view」或独立小窗。独立小窗方案下「取源窗口显示器」可换成「取当前聚焦 gpui 窗口显示器」，逻辑等价。

### 5.6 窗口位置/几何持久化

| 机制 | 位置 | 可搬运部分 | 需替换部分 |
| --- | --- | --- | --- |
| 多显示器可达性判定（标题条 32 px、最小 64 px 重叠、否则回落主屏居中） | `windows/window_position.rs:61-105` | **纯函数整段可搬运**（入参 `saved_position/window_size/&[PhysicalBounds]/fallback`） | `available_monitors/primary_monitor/outer_size/set_position` |
| 主窗几何 | `windows/main_dps.rs:157-230` | `migrate_legacy_default_size`、`restorable_main_window_position`、`should_persist_geometry`（均有测试） | `scale_factor/inner_size/outer_position/set_size/set_position` |
| Console 几何 | `windows/console.rs:70-145` | `restorable_position`、debounce 循环 `persist_geometry_loop` | 同上 |
| 角色/队伍详情几何 | `windows/combat_details.rs:27-145` | `should_persist_geometry`、debounce | 同上 |
| HUD 宽/位置 | `windows/hud.rs:88-157` | `logical_hud_width`（含 clamp/NaN 处理）、`run_debounced_persistence` | `on_window_event(Resized/Moved/ScaleFactorChanged)` → gpui 窗口事件 |

> 4 套 debounce（180/180/180/350 ms）应合并成 1 个通用 helper（AGENTS.md §5「重复 3 次以上提取 helper」）；这是重写时的顺手收益，不属于本次方案范围。

### 5.7 Console 背景色

`lib/tauri/console-window-background.ts` 对 5 个不透明窗口调 `getCurrentWebviewWindow().setBackgroundColor(...)`：浅色 `[246,247,248,255]`、深色 `[9,9,11,255]`。有 `console-window-background.test.ts` 锁定行为。gpui 里改为根视图背景色，**无需窗口 API**，且这个「窗口底色/内容色不同步导致闪白」问题会顺带消失。

### 5.8 文件对话框（7 个入口，必须重写）

`src-tauri/src/file_dialog.rs`（304 行）全部走 `tauri_plugin_dialog`，`blocking_pick_file/pick_folder/blocking_save_file`，且每次弹窗前先 `window.hwnd()` 校验 owner 未销毁（`file_dialog.rs:85-113`）。

| 入口 | 扩展名过滤 | 调用方 |
| --- | --- | --- |
| `choose_game_directory` | 目录 | `commands/toolkit.rs`（选择游戏目录） |
| `choose_json_open_path` | json + 全部 | 回放/队伍/诊断导入 |
| `choose_pcapng_open_path` | pcapng + 全部 | 诊断导入/回放 |
| `choose_ini_open_path` | ini + 全部 | 加密 ini 打开 |
| `choose_json_save_path` | json | 导出 |
| `choose_pcapng_save_path` | pcapng | 导出 |
| `open_directory` | 资源管理器 | 打开抓包日志目录 |

gpui 侧：若 gpui-kit 无原生对话框，退化为 `rfd`（Windows 走 IFileOpenDialog）或直接手写 `IFileOpenDialog`；`DialogOutcome`/`DialogError`/`file_filters()`/i18n 标题映射可保留，`run_blocking` 改 `background_executor`。

### 5.9 日志

`tauri-plugin-log` 仅在 `cfg!(debug_assertions)` 时初始化 Info 级（`lib.rs:152-158`）；release 版本没有任何 logger 初始化，`log::*` 宏落到 no-op。gpui 侧换成 `env_logger`/`tracing-subscriber` 即可，**release 行为不变、debug 更可控**。

### 5.10 updater 链路（已与 Tauri 解耦，只需 1 行改动）

| 环节 | 实现 | 框架耦合 |
| --- | --- | --- |
| 检查/下载/校验 | `core::update`(692) + `platform::update_http`(883) + `storage::update`(1,125)，`UpdateRuntimeService`（`update_runtime.rs` 275 行，**0 框架引用**） | 无 |
| 安装（App） | `update_storage::launch_prepared_app_update(&prepared)` 启动**独立** `nte-updater.exe`；随后 `std::thread::spawn(|| { sleep(200ms); app.exit(0) })` | **仅 `app.exit(0)`**（`commands/settings.rs:169`） |
| 安装（Mod loader） | `update_storage::install_prepared_plugin_update()`（`spawn_blocking`） | 仅调度 API |
| 健康标记/清理 | `mark_update_healthy_from_environment` / `cleanup_completed_app_update`（独立线程） | 无 |
| `nte-updater` 本体 | `src/bin/nte-updater.rs` 24 行，只调 `platform::update_install::run_from_args()` | 无 |

### 5.11 跨窗口事件（6 个自定义事件 → 全部变成进程内调用）

| 事件名 | 发出位置 | 接收方 | gpui 替代 |
| --- | --- | --- | --- |
| `main-dps-confirmation-requested` | `windows/passthrough_hotkey.rs:231`（热键要求确认时弹回主窗） | main-dps 前端 | 直接 `main_view.update(...)` |
| `main-dps-detail-requested` | `windows/combat_details.rs:59` | 详情窗前端 | 同上 |
| `update-available` | `commands/settings.rs:222`（自动检查发现更新） | main-dps 前端 | 同上 |
| `console-navigate` | `commands/main_dps.rs:479,519,536`（跳到 packets/timeline 页） | console 前端 | 同上（改 `ConsoleView` 的选中页状态） |
| `console-command-palette-open` | `commands/main_dps.rs:476` | console 前端 | 同上 |
| `notification-island-changed` | `windows/island.rs:78` | island 前端 | 同上 |
| `desktop-window-motion-enter` | `windows/mod.rs:15`（仅常量，前端用于动效边界） | 前端 | 删除（gpui 无 WebView 动画边界概念） |

### 5.12 不存在的能力（无需评估）

全仓 grep 确认**没有**托盘（`tray`）、开机自启（`autostart`）、单实例（`single_instance`）实现；也没有 macOS/Linux 分支（全部 Windows 专用代码，`#[cfg(windows)]` 集中在 `platform/`、`windows/passthrough_hotkey.rs`、`file_dialog.rs`）。

---

## 2. 事件通道（10 路流 + 1 条 ack 命令）

### 2.1 机制（读代码确认，不是猜测）

```
前端 subscribeStream()                         Rust
  crypto.randomUUID() ──invoke(subscribe_*)──▶ state.reserve_stream(窗口label, "kind:uuid")
  channel.onmessage ◀── Tauri Channel<T> ◀── spawn_polling_stream：tokio interval(100..500ms)
        │                                      每 tick: 读 revision → 变了才投影 snapshot → send
        └── onEvent() 后 invoke("acknowledge_stream_delivery", {generation, sequence})
                                               → registry 计算 sent==acked 才允许下一次投递
```

关键点（全部有代码依据）：
1. **推送源是轮询，不是 core 回调**：`spawn_polling_stream`（`channels/stream_runtime.rs:100-171`）每 `interval_ms` 唤醒一次，闭包返回 `NoChange | Event(T) | Stop`；`NoChange` 时**不发事件**。
2. **变更检测靠 revision**：每个通道读自己那组 revision（`stream_revision()`、`main_dps_stream_revision()`、`settings_revision()`、`history_revision()`、`diagnostics_revision()`、`empty_curtain_revision_and_operation()`、`packet_stream_revision()`），与上次比较相等就跳过；`settings` 通道额外比较「更新安装阻塞项」与「战斗时钟健康度」两个非常量投影（`channels/settings.rs:40-60`）。
3. **慢消费者靠 ack 背压**：`begin_delivery()` 用 `sent/acked` 的 CAS 保证同一时刻只有 1 条在途（`state.rs:328-341`）；投递失败/通道断开 → `break` → `finish_stream` 清理（`stream_runtime.rs:159-167`）。
4. **有 1 路是阻塞轮询**：`empty_curtain` 用 `spawn_blocking_polling_stream`（`stream_runtime.rs:181-233`），因为每轮要查插件状态（`poll_plugin_changes`），不能占 tokio 执行线程；它的休眠用 `25ms × ceil(interval/25)` 分片以便及时响应取消。
5. **容量上限**：`MAX_ACTIVE_STREAMS = 128`、`MAX_PENDING_STREAMS`、单条事件 `MAX_STREAM_DELIVERY_BYTES = 14 MB`（`contract/stream.rs:4`），超限即断开。
6. **窗口销毁自动清流**：`lib.rs:46-55` 的 `WindowEvent::Destroyed → stop_streams_for_window(label)`，注册表以 `(window_label, stream_key)` 为身份。

### 2.2 逐流清单

| 流 | subscribe/unsubscribe 命令 | 间隔 | revision 来源 | 事件 DTO | 生产/发送 | 前端消费 |
| --- | --- | --- | --- | --- | --- | --- |
| technical | `subscribe_technical_state` | **100 ms**（`state.rs:175`） | `AppState::stream_revision() -> {capture, presentation}` | `TechnicalEvent::Snapshot(TechnicalSnapshot)` | `state.snapshot()`（HUD 投影 + 采集状态 + 设置快照） | `technical-client.ts`（HUD 窗） |
| mainDps | `subscribe_main_dps` | **100 ms** | `main_dps_stream_revision()` | `MainDpsEvent::Snapshot(MainDpsSnapshot)` | `commands::main_dps::snapshot()`；**且在轮询里发布 island 通知**（采集相位跃迁：开始/停止/失败/回放完成） | `main-dps-client.ts`（主窗） |
| mainDpsDetail | `subscribe_main_dps_detail` | **250 ms** | `main_dps_stream_revision()` | `MainDpsDetailSnapshot` | `MainDpsDetailSnapshot::from_state(...)`；**窗口不可见时跳过投影**（`channels/main_dps_detail.rs:44-50`，`window.is_visible()`） | `main-dps-detail-client.ts`（2 个详情窗） |
| packets | `subscribe_packets` | **100 ms** | `packet_stream_revision()` | `PacketsEvent` | 增量快照（`snapshot_since(last_revision)`） | `packets-client.ts`（console 内页） |
| timeline | `subscribe_timeline` | **100 ms** | `main_dps_stream_revision()` | `TimelineEvent` | `state.timeline_projection(scope)` | `timeline-client.ts`（console） |
| skills | `subscribe_skills` | **100 ms** | `main_dps_stream_revision()` | `SkillsEvent` | `state.skills_projection(scope)` | `skills-client.ts`（console） |
| diagnostics | `subscribe_diagnostics` | **500 ms** | `diagnostics_revision() -> (3 个 u64)` | `DiagnosticsEvent` | `commands::diagnostics::snapshot()` | `diagnostics-client.ts`（console） |
| history | `subscribe_history` | **250 ms**（`history_runtime.rs:25`） | `history_revision()` + 语言 + 角色目录指针（`history_projection_key`） | `HistoryEvent::Snapshot(HistorySnapshot)` | 走 `HistoryRuntime`：**独立投影缓存**（同一 key 复用 `Arc<HistorySnapshot>`），磁盘扫描在 `spawn_blocking`，扫描期间不持有 history 事务锁 | `history-client.ts`（console） |
| emptyCurtain | `subscribe_empty_curtain` | **500 ms** | `empty_curtain_revision_and_operation()` | `EmptyCurtainEvent` | **阻塞轮询**：每轮查插件（`poll_plugin_changes`）→ 投影空幕快照 | `empty-curtain-client.ts`（console） |
| settings | `subscribe_settings` | **200 ms** | `settings_revision()` + 安装阻塞项 + 战斗时钟健康度 | `SettingsEvent::Snapshot(SettingsSnapshot)` | `state.settings_snapshot()` | `settings-client.ts`（console） |

> 合计 10 路 subscribe + 10 路 unsubscribe + 1 路 `acknowledge_stream_delivery` = 21 个通道相关命令。

### 2.3 前端消费方式（`frontend/src/lib/tauri/stream-client.ts`，94 行）

- `subscribeStream<Event>()` 用 `crypto.randomUUID()` 生成订阅 id；`Channel.onmessage` 收到事件后**先** `onEvent(event)`，**再** `invoke(acknowledge, {streamGeneration, deliverySequence++})`；
- Rust 可能在 `receipt` 返回前就投递第一条（`pendingEvent` 暂存，拿到 generation 再投递），说明**生成号与投递序号是 IPC 有序性的显式契约**；
- `close()` 采用 `receipt.then(unsubscribe)`，并对 `subscribe` 失败做兜底（unsubscribe 幂等）。

### 2.4 gpui 单进程下的替身方案

| 现状机制 | gpui 替代 | 可原样保留的代码 |
| --- | --- | --- |
| 每订阅 1 个 tokio 任务 + `interval` | 全局 1 个 ticker（如 16 ms/33 ms 或按需 100 ms）驱动所有视图，或每视图 `cx.spawn(async move { loop { Timer::after(100ms).await; ... } })` | interval 数值本身（100/200/250/500 ms）|
| revision 比较决定是否重绘 | 保留：`fn needs_render(&self, rev) -> bool`，revision 已由 core 提供 | **全部** revision 结构与比较逻辑 |
| 投影函数（snapshot） | 保留：直接产出 `HudSnapshot`/`MainDpsSnapshot`/…，作为 `Entity` 的 view-model 字段 | 投影调用（`state.snapshot()`、`project_hud`、`from_state`） |
| `Channel<T>` 序列化投递 | 删除：同进程直接 `entity.update(cx, |v, cx| { v.model = next; cx.notify() })` | — |
| ack/生成号/序号/`SubscriptionReceipt` | **删除**：无 IPC 边界即无「投递确认」需求（约 230 行 + `contract/stream.rs` 41 行） | — |
| 订阅注册表（按窗口 label 管理、容量上限、窗口销毁清流） | 保留语义但简化：窗口关闭 → `drop Entity`；不需要「按 label 字符串索引」与 128 容量上限 | `StreamRegistry` 的并发/poison 处理（373 行状态机）在单窗口 Entity 生命周期下**不必保留** |
| `spawn_blocking_polling_stream`（empty_curtain） | `cx.background_executor().spawn(...)`，同样把阻塞查询放后台 | 25 ms 分片取消检查 |

**建议**：把「revision → 投影」抽成 `trait Projection { type Model; fn project(&AppState) -> Self::Model; fn revision(&AppState) -> ... }`，10 路流各实现一次；ticker 只做「遍历活跃视图 → revision 变了就 project + notify」。这样 10 个 `channels/*.rs`（1,365 行）的主体逻辑会收敛到 ~300 行，且**通知岛、采集相位跃迁仍能在 ticker 里发布**（保留 `channels/main_dps.rs:97-137` 的跃迁判定函数 `capture_transition_notice`，它有测试）。

---

## 3. DTO / 契约层

### 3.1 规模

| 指标 | 数值 | 复现 |
| --- | --- | --- |
| `contract/` 文件 | 17（`contract.rs` + 16 子模块） | — |
| `contract/` 行数 | **6,262**（`contract.rs` 504 在根目录，另 16 文件 5,758） | 附录 A |
| `contract/` 中 `#[derive(...TS)]` 类型 | **133** | grep `derive\(.*TS` |
| 全 `src-tauri` 中带 `#[derive(TS)]` 的类型 | **155**（另 22 个在 `commands/*.rs` 的入参枚举 + `state.rs` 的 `HudPreset`/`HudSettingOption` 等） | 同上 |
| `#[ts(export)]` 标记 | **155**（与 derive 数一致） | 同上 |
| 核心 `src/` 中条件导出 | **4 处 `ts(export)`**（`platform/toolkit_form.rs:9,26,64`、`core/live_capture.rs:148`）+ 54 处 `derive(ts_rs::TS)`，全部包在 `#[cfg_attr(feature = "tauri_frontend", ...)]` | grep |
| 生成的 TS 文件 | **209** 个（`frontend/src/lib/tauri/generated/`），含 ts-rs 自动补齐的嵌套依赖类型 | 目录计数 |
| 生成机制 | `.cargo/config.toml` 设 `TS_RS_EXPORT_DIR=frontend/src/lib/tauri/generated`、`TS_RS_LARGE_INT=number`；ts-rs 在 `cargo test` 时由自动生成的 `export_bindings_*` 测试写盘；CI 步骤：删目录 → `cargo test --manifest-path src-tauri/Cargo.toml export_bindings` + `cargo test --features tauri_frontend --lib export_bindings`（`.github/workflows/build.yml:137-140`） | 仓库内**无**手写 `export_bindings` 测试函数 |
| serde/ts 属性行 | **406**（`#[derive(` 138 + `#[ts(` 134 + `#[serde(` 134） | 附录 A |

### 3.2 contract 层与 core 的关系（「打包」还是「直出」）

- **不是纯镜像**：`contract/*.rs` 直接 `use nte_dps_tool::...` 并把 core 类型放进 DTO 字段。例：`contract.rs:31-42` `TechnicalSnapshot.hud: HudSnapshot` 是 core 类型直出；`contract.rs:352-357` 有 `impl From<CoreError> for CommandError`；`contract/island.rs` 定义 `IslandTone`（纯展示枚举，core 没有）。
- **每个 `contract/*.rs` 都至少有 1 条 `use nte_dps_tool::...`**（22 条 import 语句，见 `grep '^use nte_dps_tool'`），即 contract 是「core 类型 + 少量重命名/展平 + TS 导出」。
- `contract` 里的测试（1,558 行，如 `contract/main_dps_detail.rs` 647 行、`contract/empty_curtain.rs` 250 行）测的是**投影/展平逻辑**（分页、列裁剪、camelCase 数值字符串化），这些测试在 gpui 里仍然有意义。

### 3.3 保留/删除判定

| 子层 | 行数 | 判定 | 理由 |
| --- | --- | --- | --- |
| 结构体/枚举定义 + `From`/投影 | ≈5,856 | **保留为 view-model（推荐）** | 单进程后虽不需要「跨 IPC 契约」，但这些结构正好是 gpui 视图要绑定的只读模型；改造成本最低，且保住 1,558 行投影测试 |
| `#[derive(...)]`/`#[ts(export)]`/`#[serde(rename_all)]` | 406 | **删除** | 无序列化边界；camelCase 只服务于 TS |
| `SubscriptionReceipt`/`StreamKind`/`MAX_STREAM_DELIVERY_BYTES` | 41+ | **删除** | 无 Channel 即无订阅回执/交付上限 |
| `CommandError`（`contract.rs:123-350`，约 230 行，60 个构造器） | 230 | **删除或降级** | 现在承担「core `CoreError` → 稳定 code + i18n key」的翻译，且测试断言「不泄露 panic/私有路径」（`contract.rs:489-503`）。gpui 里仍需「错误 → 用户文案」，建议保留**翻译表**、删除「错误类型转换样板」；若保留 UI 错误模型，可整段复用 |
| `contract/update.rs`(17)、`contract/stream.rs`(41)、`contract/island.rs`(57) | 115 | 前者保留（更新提示模型），后两者删除 | — |

> 若选激进方案（viewport 直接用 core 类型、不保留 DTO），可再删 ~4,000 行，但需要重写 1,558 行测试、并接受 core 类型直接进视图层（`HudSnapshot` 等本来就是投影结果，可行）。**建议不要**：省下的行数不抵改造风险。

---

## 6. 打包 / 构建

### 6.1 现状

| 项 | 内容 |
| --- | --- |
| `src-tauri/build.rs` | **3 行**：`fn main() { tauri_build::build() }`（生成 context/权限/图标资源） |
| `tauri.conf.json` | 163 行：`build.frontendDist=../frontend/dist`、`devUrl=http://localhost:5173`、`beforeDevCommand=pnpm dev`、`beforeBuildCommand=pnpm build`、7 窗口（§5.1）、CSP/devCsp、`bundle: { active: true, targets: "all", icon: ["icons/icon.ico"] }` |
| 根 `build.rs` | 336 行：把 `res/` 嵌入二进制（JSON 压缩、图片按 `tauri_frontend` 排除/记录 `frontend_resource_exists`）、`winresource` 写 exe 图标/版本信息 |
| 前端构建 | `pnpm build` = `tsc -b && vite build`；`vite.config.ts` 6 个 HTML 入口（main-dps/console/hud/notification/combat-details/abyss-values），`@res` 别名把 `../res` 打进 dist |
| CI 关键步骤 | `.github/workflows/build.yml`：`pnpm --dir frontend tauri:build --no-bundle --ci` → `cargo build --release --bin nte-updater` → 复制 `src-tauri/target/release/nte-dps-tool.exe` + `target/release/nte-updater.exe` 到 `dist/`；`--features external_resources` 再跑一遍（复制 `res/`）；UPX 压缩 + 冒烟测试；打 3 个 zip（tool / core / external-resources） |
| 安装包 | **CI 实际不产出 NSIS/MSI**：`--no-bundle` 跳过打包，`bundle.targets="all"` 只在开发者本机 `pnpm tauri build` 时生效。发布物 = 裸 exe + zip |
| 签名/更新产物 | 更新产物由 `nte-updater` + `platform/update_install` 处理（签名在 core 的 ed25519 校验里），与 Tauri bundler 无关 |

### 6.2 换 gpui 单 exe 后要重建的流水线

| 步骤 | 现状 | gpui 后 |
| --- | --- | --- |
| 前端依赖安装/格式/类型/lint/test | `pnpm install --frozen-lockfile`、`format:check`、`lint`、`typecheck`、`test` | **整段删除**（前端消失） |
| 前端构建 | `pnpm build` / `tauri build` 触发 | **删除** |
| `tauri build` | `pnpm tauri:build --no-bundle` | 换成 `cargo build --release -p nte-dps-tool-desktop`（或 `-p nte-dps-tool --bin nte-dps-tool`） |
| TS 绑定生成/校验 | 删 `generated/` → 两次 `cargo test export_bindings` → 校验 git 无 diff（CI:137-140） | **删除**（无 ts-rs） |
| 资源嵌入 | `res/` 嵌入 + 前端图片交给 Vite | 恢复「全量嵌入」（`build.rs` 去掉 `tauri_frontend` 排除分支）；`external_resources` 变体逻辑不变 |
| 产物 | `src-tauri/target/release/nte-dps-tool.exe` + `target/release/nte-updater.exe` | 同一目录约定，改 1 行路径；zip/UPX/冒烟步骤**原样复用** |
| 审计 | `cargo audit` 根 + `--file src-tauri/Cargo.lock`（GTK3 告警注释） | **只审计根 lock**；`src-tauri/Cargo.lock` 与其中的 GTK/WebKit 依赖链消失（体积/供应链收益） |
| 体积 | WebView2 运行时系统自带（非打包），exe 由 UPX 压 | gpui 静态链入，体积会明显变大（wgpu/dx 后端），UPX 收益变化需要实测 → 记入 §7 R6 |

---

## 7. 风险清单（按「必须在 gpui 里重写且无现成等价物」排序）

| # | 风险 | 现状实现 | 为什么危险 | 缓解 |
| --- | --- | --- | --- | --- |
| **R1** | **透明 + 置顶 + 跳过任务栏 + 鼠标穿透 + 不抢焦点的 HUD 浮层** | `transparent`/`alwaysOnTop`/`skipTaskbar` 三合一是 Tauri 声明式配置；穿透是 `set_ignore_cursor_events`；`skipTaskbar` 在 Win32 需 `WS_EX_TOOLWINDOW`，穿透需 `WS_EX_TRANSPARENT` 或 `WM_NCHITTEST` 返回 `HTTRANSPARENT` | 任一属性缺失 → 要么 HUD 抢焦点导致游戏掉帧/最小化，要么鼠标被吞掉无法操作游戏，要么出现在任务栏。**这是产品核心卖点，无等价就得自己写 Win32 扩展样式** | core `window_style.rs` 已有 3 个 HWND API；再补 `WS_EX_TRANSPARENT`/`WS_EX_TOOLWINDOW` 两个 `SetWindowLongPtr` 封装即可兜底（风险可控，但必须真机验收）。**待 30 号文档确认 gpui 窗口创建是否支持 transparent/always-on-top** |
| **R2** | **中文输入法（IME）** | 现在由 WebView2 提供完整 IME（含候选窗、拼音、组合态） | 应用内有多个文本输入：Console 过滤/搜索、队伍导入、加密 ini 编辑、Mod 表单（`commands/plugin_form.rs`）。gpui 的 IME 支持有限，候选窗定位/组合态处理是自研工作量 | 优先用「只读展示 + 少量输入」设计、把复杂编辑降到最少；详见 30 号文档。**风险等级取决于 30 号结论** |
| **R3** | **原生文件对话框** | `tauri-plugin-dialog` 7 个入口，且依赖 owner HWND 判活 | gpui 无内置对话框；`rfd` 增加依赖与许可证审计（AGENTS.md §9 要求说明用途/许可/体积） | 建议 `rfd`（MIT/Apache，Windows 走 IFileOpenDialog），保留 `DialogOutcome`/filters/i18n；需在方案里补许可说明 |
| **R4** | **无边框窗口的拖动/双击最大化/边缘 resize** | `startDragging()` + capability；resize 由 Tauri 无边框实现（`resizable:true` + `shadow:true`） | 自绘标题栏后，「边缘 4 px 拖拽缩放」「双击标题栏最大化」「Aero Snap」都要自己补；HUD 还要「按内容锁高度、宽度可拖」 | gpui 若暴露 drag/resize 区域即可；否则用 `WM_NCHITTEST` 自绘 hit-test（工作量中等）。真机验收项 |
| **R5** | **通知岛的「不抢焦点 + 全部窗口之上 + 5 s 生命周期 + 撤销」** | `set_focusable(false)` + 每次 show 重设 topmost/skipTaskbar + `ISLAND_NOTICE_WINDOW=5s` 撤销令牌 | 若 gpui 无法创建「不可聚焦」窗口，从游戏切出/切回时通知岛会抢走焦点 | 退化为「主窗内浮层」可规避；撤销/过期逻辑已在 `IslandNoticeRuntime`（可搬运） |
| **R6** | **单 exe 体积 / 二进制膨胀** | WebView2 复用系统运行时，exe 小（UPX 后更小） | gpui 静态链入渲染栈，体积可能翻倍；CI 的 UPX + 冒烟测试要重测 | 接受；在 CI 里加体积对比门限（现有 UPX 步骤可直接复用） |
| **R7** | **多窗口同步/焦点语义** | 7 个独立 WebView，各自有 JS 事件循环；跨窗用 emit_to | 单进程后共享一个 UI 线程，**一个慢视图会卡住所有窗口**；现有「每视图独立 100 ms 轮询」在单线程下变成串行 | ticker 分帧（每帧最多投影 1–2 个视图）、重投影（history/diagnostics 500 ms）；R1 的 HUD 高频刷新（100 ms）要移到轻量 view 或在后台线程准备好模型 |
| **R8** | **未验证项：HUD/HUD 与游戏共存、置顶被系统降级、DPI 缩放** | `tauri.conf.json` 的 `transparent`+`alwaysOnTop` 由 wry/Win32 保证；HUD 位置/宽度有 DPI 换算（`logical_hud_width`） | 只能真机验收（AGENTS.md §11：窗口/HUD 行为需用户验收） | 明确标注「未验证」，交付前由用户跑真机场景 |

---

## 8. 结论：三分类账 + crate 布局建议

### 8.1 三分类账（`src-tauri/src` 28,529 行 = 100%）

**第一步：按文件整分（可复现，见附录 B）**

| 文件类别 | 文件数 | 行数 |
| --- | --- | --- |
| 零框架引用（A 类源集合） | 37 | **17,452** |
| 至少 1 处框架引用（B 类源集合） | 43 | **11,077** |
| 合计 | **80** | **28,529** |

**第二步：从 A/B 中划出「随 Tauri 消失而删除」的行（C 类，均为可 grep 定位的机制代码）**

| C 组成 | 行数 | 位置 |
| --- | --- | --- |
| serde/ts/TS 派生属性 | 406 | `contract.rs` + `contract/*`（`#[derive(` 138、`#[ts(` 134、`#[serde(` 134） |
| 命令签名里的框架参数（`State<'_,`/`WebviewWindow`/`AppHandle`） | 400 | `commands/*` + `channels/*` |
| Tauri Builder/插件/`generate_handler`/`RunEvent`/窗口装配 | 280 | `lib.rs:21-319` |
| Channel 投递/ack/交付计数/14MB 上限/`SubscriptionReceipt` | 150 | `channels/stream_runtime.rs`（302 行内）+ `contract/stream.rs` |
| `StreamDeliveryProgress` 在 `AppState` 内的生成号/ack 状态 | 80 | `state.rs:275-300,321-358,1578-1595` |
| `async_runtime` 调度行（调度本身被 gpui executor 取代） | 62 | 全目录 |
| **C 合计** | **1,378** | — |

**第三步：最终台账**

| 分类 | 行数 | 占比 | 非空行口径 | 构成 |
| --- | --- | --- | --- | --- |
| **A. 可直接复用**（框架无关，整文件搬运） | **16,966** | **59.5%** | 15,670 | `state/` 8,730（含 `state/tests.rs` 4,247）、`contract/` 6,766、8 个 service/runtime 1,926、2 个 `mod.rs` 30，减去 C 内的 406+80 |
| **B. 需改写成 gpui app 壳**（逻辑保留、Tauri API 换实现） | **10,185** | **35.7%** | 9,528 | `commands/` 6,300、`channels/` 1,215、`windows/` 1,939、`history_runtime` 383、`lib.rs` 39、`file_dialog` 304、`main.rs` 5；见下逐目录 |
| **C. 直接删除** | **1,378** | **4.8%** | 1,348 | 见上表 |
| 合计 | **28,529** | 100% | 26,546 | — |

> 校验：`16,966 + 10,185 + 1,378 = 28,529`；`15,670 + 9,528 + 1,348 = 26,546`。
> 其中**测试 7,994 行**：A 内 6,520（`state/tests.rs` 4,247 + state/contract 文件内联 2,273）可随逻辑平移；B 内 1,474 行测试多数要随命令/通道改写重写。

**补充判断（给 Lead 决策用）：**

1. `state/`（8,730 行，含 4,247 行测试）是**这次重构最大的资产**：`AppState` 及其 7 个领域模块已经把「采集编排、暂停、轮次选择、撤销（reset 5s / history delete 5s）、历史归档重试队列、HUD 配置事务、队伍导入、设备目录、更新状态机」全部写成框架无关代码，只依赖 `nte_dps_tool::* + std + log`。**建议原样搬进新 app crate（第一步就搬，几乎零改动）。**
2. `commands/` 的 122 条命令里**没有业务规则**，一律是「`validate_window()` → `state.*()` → `island::publish_notice()` → `snapshot()`」；其中体量最大的是 `empty_curtain`(971)、`settings`(1,229)、`main_dps`(1,059)、`toolkit`(1,012)、`history`(611)，这些文件里**真正需要改写的是窗口校验/通知/文件对话框/异步调度**，其余（参数反序列化之外的编排）可直接变成 `AppState` 上的方法或 view 的 action 处理函数。
3. `channels/`（1,365 行）的**投影与 revision 比较逻辑必须保留**（§2.4），只有 Channel/ack/订阅注册表要删。
4. `windows/`（1,939 行）里约 **350 行是纯算法/常量/测试**（`window_position` 159 行中 100+ 是纯函数与测试；`hud.rs` 的 `logical_hud_width`/`height_constraint_order`/`run_debounced_persistence`；`main_dps` 的 `migrate_legacy_default_size`/`restorable_main_window_position`/`should_persist_geometry`；`console`/`combat_details` 的 `restorable_position`/`should_persist_geometry`），可直接搬走。
5. **必须删除但容易漏的**：`WINDOW_MOTION_ENTER_EVENT`（`windows/mod.rs:15`）、`on_page_load` 首帧显示逻辑、`frontend/src/lib/tauri/*`（前端 209 个 generated 文件）、7 个 capability JSON、`tauri.conf.json` 的 CSP、`.cargo/config.toml` 的 `TS_RS_*` 环境变量。

### 8.2 建议的新 crate 布局

```
D:\NTE_DPS_TOOL\
├─ Cargo.toml                 # 根 crate nte-dps-tool（保留 src/）
│                             #   删除 feature tauri_frontend；features: default=["desktop"], cli, external_resources
│                             #   [[bin]] nte-core（cli）、nte-updater（desktop）不动
├─ build.rs                   # 去掉 CARGO_FEATURE_TAURI_FRONTEND 分支 → res/ 全量嵌入
├─ src/                       # 90,407 行，零改动
├─ crates/
│  └─ app-desktop/            # 新 gpui 单进程 app（package = nte-dps-tool-desktop）
│     ├─ Cargo.toml           #   nte-dps-tool{default-features=false, features=["desktop"]},
│     │                       #   gpui, gpui-kit, log, serde/serde_json（仅持久化需要）
│     └─ src/
│        ├─ main.rs           #   Application::run，装配 6-7 个窗口 + 热键 runtime + 全局 ticker
│        ├─ app_state/        #   ← src-tauri/src/state/*（8,730 行）：0 改动，仅 use 路径
│        ├─ services/         #   ← settings/character_data/encrypted_ini/equipment/team_import
│        │                    #      + update_runtime/desktop_runtime/diagnostics_runtime（1,926 行）
│        ├─ viewmodel/        #   ← src-tauri/src/contract/*（406 行属性删除；保留投影与测试）
│        ├─ views/            #   6 个页面：main_dps/console/hud/notification/combat_details/abyss
│        ├─ shell/            #   ← src-tauri/src/windows/* 的可搬运部分：
│        │                    #     窗口几何/位置持久化、HUD 原生外观、island 定位、
│        │                    #     热键派发器（50ms 轮询）、跨窗动作
│        └─ dialogs.rs        #   rfd 或 IFileDialog（← file_dialog.rs 的 filters/outcome 保留）
└─ (删除) src-tauri/          # 28,529 行：A 搬走、B 重写、C 删除
   (删除) frontend/           # 全部 TS/React（另见 10-frontend-inventory.md）
```

**迁移顺序建议（可与本次测绘并行）：**
1. 搬 `state/` + 8 services → 新 crate，跑通现有 4,247 行测试（只改 `use crate::` 路径与 `AppState::new` 的构造参数）——**这一步就能证明 A 类 16,966 行（59.5%）的复用是真实的**。
2. 搬 `contract/`（删属性）→ `viewmodel/`，投影测试同步搬。
3. 写 gpui 主窗 + console 两个窗口，用「ticker → revision → project → notify」跑通 main_dps/settings 两路数据。
4. 用 core `window_style` + Win32 扩展样式做 HUD 透明/穿透/置顶，真机验收（R1/R4/R8）。
5. 最后再做 island、详情窗、文件对话框、IME 输入面（R2/R3/R5）。

---

## 附录 A：复现命令

```powershell
# A1 行数/文件数（全行 与 非空 两种口径）
$root='D:\NTE_DPS_TOOL\src-tauri\src'
Get-ChildItem $root -Recurse -File -Filter *.rs | ForEach-Object {
  $c=Get-Content $_.FullName
  [pscustomobject]@{ file=$_.FullName.Substring($root.Length+1); all=$c.Count
    nonblank=($c | Where-Object { $_.Trim() -ne '' }).Count }
} | Measure-Object all,nonblank -Sum

# A2 命令总数与逐文件分布
$re=[regex]::new('#\[tauri::command[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*pub(?:\(crate\))?\s+(?:async\s+)?fn\s+(\w+)')
Get-ChildItem $root -Recurse -File -Filter *.rs | ForEach-Object {
  $t=[System.IO.File]::ReadAllText($_.FullName); ($re.Matches($t)).Count
} | Measure-Object -Sum          # => 144

# A3 核心是否引用 UI 框架（期望：只有 tauri_frontend/ts_rs 属性）
Select-String -Path 'D:\NTE_DPS_TOOL\src\**\*.rs' -Pattern 'tauri|webview|egui|eframe|wgpu|winit'

# A4 contract 的 serde/ts 属性行
$c = @(Get-ChildItem "$root\contract" -File -Filter *.rs) + @(Get-Item "$root\contract.rs")
($c | ForEach-Object { Select-String -Path $_.FullName -Pattern '#\[(derive|ts|serde)\(' -AllMatches }).Matches.Count   # => 406

# A5 框架耦合行（tauri:: 与 async_runtime）
Get-ChildItem $root -Recurse -File -Filter *.rs | ForEach-Object {
  Select-String -Path $_.FullName -Pattern 'tauri::' -AllMatches
} | Measure-Object   # => 263 行

# A6 前端命令调用方（命令名 → client 文件）
#   见本文 §1 表格；生成方式：对 144 个命令名在 frontend/src 中查 `"<name>"` 字面量

# A7 TS 生成物与派生属性
(Get-ChildItem 'D:\NTE_DPS_TOOL\frontend\src\lib\tauri\generated' -File).Count   # => 209
Select-String -Path "$root\**\*.rs" -Pattern '#\[ts\(export\)\]' | Measure-Object  # src-tauri => 155

# A8 依赖锁定版本
Select-String -Path 'D:\NTE_DPS_TOOL\src-tauri\Cargo.lock' -Pattern '^name = "tauri"$' -Context 0,1
```

## 附录 B：80 个文件逐文件处置（行数=全行；框架引用行=含 `tauri::|State<|AppHandle|WebviewWindow|Channel<|async_runtime` 等）

| 文件 | 行数 | 非空 | 测试 | 框架引用行 | 分类 | 处置 |
| --- | --- | --- | --- | --- | --- | --- |
| channels/diagnostics.rs | 74 | 70 | 0 | 8 | B | 订阅+轮询+Channel 投递；换成 gpui 定时器+Entity |
| channels/empty_curtain.rs | 94 | 92 | 0 | 8 | B | 订阅+轮询+**阻塞**投递；同上 |
| channels/history.rs | 62 | 58 | 0 | 10 | B | 订阅；投影走 HistoryRuntime |
| channels/main_dps_detail.rs | 80 | 78 | 0 | 8 | B | 订阅；含 `window.is_visible()` 门控 → 改 gpui 可见性 |
| channels/main_dps.rs | 178 | 171 | 25 | 10 | B | 订阅 + **island 采集相位通知**（判定函数可搬运） |
| channels/mod.rs | 11 | 11 | 0 | 0 | A | 模块声明 |
| channels/packets.rs | 77 | 73 | 0 | 8 | B | 订阅 + 增量快照 |
| channels/settings.rs | 171 | 155 | 61 | 8 | B | 订阅 + 两个非常量投影比较（可搬运） |
| channels/skills.rs | 74 | 70 | 0 | 8 | B | 订阅 |
| channels/stream_runtime.rs | 302 | 278 | 68 | 8 | B | 轮询调度器（保留）+ ack/回执/交付上限（删除，约 150 行） |
| channels/technical.rs | 96 | 86 | 19 | 8 | B | 订阅 |
| channels/timeline.rs | 157 | 148 | 81 | 8 | B | 订阅 |
| character_data_service.rs | 78 | 66 | 22 | 0 | A | 纯状态服务 |
| commands/abyss_values.rs | 113 | 105 | 0 | 17 | B | 5 条命令 → AppState 方法 |
| commands/character_data.rs | 78 | 71 | 21 | 4 | B | 1 条命令 |
| commands/console_control.rs | 156 | 147 | 0 | 9 | B | 8 个控制动作枚举（可搬运）+ 窗口操作（重写） |
| commands/desktop_window.rs | 103 | 94 | 36 | 6 | B | 置顶切换 + opacity 补偿（可搬运判断逻辑） |
| commands/diagnostics.rs | 332 | 303 | 39 | 25 | B | 6 条命令 + 导出/导入 |
| commands/empty_curtain.rs | 971 | 934 | 71 | 30 | B | 7 条命令（最大单文件） |
| commands/encrypted_ini.rs | 271 | 253 | 28 | 23 | B | 5 条命令 + 文件对话框 |
| commands/history.rs | 611 | 575 | 108 | 43 | B | 9 条命令 + 导入导出对话框 |
| commands/island.rs | 64 | 60 | 0 | 11 | B | 3 条通知岛命令 |
| commands/main_dps.rs | 1059 | 997 | 86 | 121 | B | 32 条命令（框架耦合最重） |
| commands/mod_studio.rs | 102 | 99 | 0 | 9 | B | 2 条 |
| commands/mod.rs | 19 | 19 | 0 | 0 | A | 模块声明 |
| commands/packets.rs | 37 | 33 | 0 | 4 | B | 1 条 |
| commands/plugin_form.rs | 103 | 100 | 0 | 9 | B | 2 条（插件表单） |
| commands/settings.rs | 1229 | 1146 | 205 | 101 | B | 25 条命令 + **更新安装/退出（唯一 `app.exit`）** |
| commands/skills.rs | 29 | 26 | 0 | 4 | B | 1 条 |
| commands/technical.rs | 204 | 193 | 0 | 34 | B | 9 条 HUD 命令（含穿透/置顶/宽度/复位） |
| commands/timeline.rs | 53 | 49 | 0 | 7 | B | 2 条 |
| commands/toolkit.rs | 1012 | 989 | 129 | 38 | B | 9 条 + 代理宿主部署（`tools/NTE-Loader.exe` 启动） |
| commands/user_characters.rs | 235 | 232 | 20 | 5 | B | 1 条（分页请求） |
| contract.rs | 504 | 457 | 69 | 0 | A | `CommandError` + 3 个快照；删 ~110 行属性 |
| contract/abyss_values.rs | 273 | 253 | 0 | 0 | A | 删属性 |
| contract/character_data.rs | 48 | 44 | 0 | 0 | A | 删属性 |
| contract/diagnostics.rs | 334 | 309 | 33 | 0 | A | 删属性 |
| contract/dps_time.rs | 159 | 147 | 74 | 0 | A | 删属性 |
| contract/empty_curtain.rs | 705 | 669 | 250 | 0 | A | 删属性 |
| contract/encrypted_ini.rs | 77 | 68 | 10 | 0 | A | 删属性 |
| contract/history.rs | 670 | 621 | 94 | 0 | A | 删属性 |
| contract/island.rs | 57 | 51 | 0 | 0 | A | 保留 `IslandTone`；删属性 |
| contract/main_dps_detail.rs | 1722 | 1615 | 647 | 0 | A | 最大 DTO 文件；删属性 |
| contract/main_dps.rs | 845 | 790 | 274 | 0 | A | 删属性 |
| contract/mod_studio.rs | 194 | 184 | 0 | 0 | A | 删属性 |
| contract/packets.rs | 166 | 157 | 69 | 0 | A | 删属性 |
| contract/settings.rs | 491 | 453 | 36 | 0 | A | 删属性 |
| contract/skills.rs | 272 | 255 | 71 | 0 | A | 删属性 |
| contract/stream.rs | 41 | 37 | 0 | 0 | A→C | `StreamKind`/`SubscriptionReceipt`/14MB 上限 → 删除 |
| contract/timeline.rs | 191 | 180 | 0 | 0 | A | 删属性 |
| contract/update.rs | 17 | 14 | 0 | 0 | A | 保留 |
| desktop_runtime.rs | 100 | 84 | 19 | 0 | A | 进程级标量（uptime/sequence/passthrough 就绪/置顶偏好/onboarding） |
| diagnostics_runtime.rs | 113 | 96 | 53 | 0 | A | 诊断运行/报告缓存 |
| encrypted_ini_service.rs | 433 | 391 | 140 | 0 | A | 加密 ini 编辑会话 |
| equipment_operation_service.rs | 93 | 92 | 27 | 0 | A | 装备操作串行化 |
| file_dialog.rs | 304 | 276 | 48 | 13 | B | 换 rfd/IFileDialog；保留 outcome/filters |
| history_runtime.rs | 383 | 346 | 97 | 6 | B | 历史投影缓存 + 250ms 维护线程；仅 async_runtime/Channel 需换 |
| lib.rs | 319 | 315 | 0 | 9 | B→C | 7 窗口装配 + 144 命令注册（约 280 行删除） |
| main.rs | 5 | 4 | 0 | 0 | B | 换成 `Application::run` |
| settings_service.rs | 526 | 474 | 287 | 0 | A | 配置事务/设备目录/穿透锁 |
| state.rs | 1615 | 1465 | 0 | 0 | A | `AppState` 组合门面；订阅注册表在此（约 400 行可简化） |
| state/combat.rs | 912 | 859 | 0 | 3 | A | 主面板读数/暂停/轮次/深渊/时间线投影 |
| state/diagnostics.rs | 235 | 220 | 0 | 0 | A | 诊断状态 |
| state/equipment.rs | 147 | 135 | 0 | 0 | A | 装备目录/空幕状态 |
| state/history.rs | 453 | 428 | 0 | 0 | A | 归档/重试队列/删除墓碑/撤销 |
| state/session.rs | 252 | 235 | 0 | 0 | A | 采集生命周期/撤销/插件控制预留 |
| state/settings.rs | 755 | 681 | 0 | 0 | A | 设置投影/HUD 配置/窗口几何/更新状态 |
| state/teams.rs | 114 | 105 | 0 | 0 | A | 队伍导入/深渊两路 |
| state/tests.rs | 4247 | 3947 | **4247** | 0 | A | 全部随 AppState 平移（本次复用的最强证据） |
| team_import_service.rs | 308 | 280 | 98 | 0 | A | 队伍数据导入服务 |
| update_runtime.rs | 275 | 254 | 0 | 0 | A | 更新状态机（check/download/install 阶段） |
| windows/abyss_values.rs | 137 | 126 | 11 | 5 | B | 窗口几何持久化 |
| windows/combat_details.rs | 166 | 154 | 11 | 6 | B | 两窗口 kind + 几何持久化 + 关闭隐藏 |
| windows/console.rs | 160 | 145 | 14 | 5 | B | 几何恢复/持久化 + 关闭隐藏 |
| windows/hud.rs | 454 | 412 | 81 | 20 | B | Acrylic/圆角/宽度高度锁定/位置持久化（核心算法可搬运） |
| windows/island.rs | 232 | 210 | 50 | 9 | B | 定位算法（可搬运）+ 显示/隐藏/emit |
| windows/main_dps.rs | 300 | 277 | 69 | 15 | B | 主窗几何/不透明度补偿/关闭销毁从窗 |
| windows/mod.rs | 57 | 50 | 21 | 2 | B | 窗口 label 白名单 + 事件常量 |
| windows/passthrough_hotkey.rs | 274 | 258 | 22 | 6 | B | 热键派发器（逻辑可搬运） |
| windows/window_position.rs | 159 | 142 | 53 | 3 | B | 可达性纯函数（100+ 行可搬运） |

## 附录 C：关键代码位置索引

| 主题 | 位置 |
| --- | --- |
| 应用装配 / 7 窗口 / 144 命令注册 | `src-tauri/src/lib.rs:21-319` |
| 流调度器（轮询+节流+背压+断开） | `src-tauri/src/channels/stream_runtime.rs:100-233` |
| 订阅注册表（按窗口 label、容量、generation、poison 修复） | `src-tauri/src/state.rs:263-660` |
| AppState 组合与构造 | `src-tauri/src/state.rs:1373-1615` |
| HUD 原生外观（Acrylic/圆角/整体不透明度） | `src/platform/window_style.rs:63-154` |
| 全局低级键盘钩子 | `src/platform/passthrough_hotkey.rs:277-472` |
| HUD 投影（gpui 视图可直接消费） | `src/core/hud.rs:152-269` |
| 抓包服务对外 API | `src/core/live_capture.rs:60-3802` |
| 历史投影缓存 | `src-tauri/src/history_runtime.rs:37-285` |
| 更新状态机 | `src-tauri/src/update_runtime.rs`（全文件） |
| 更新安装 + 退出 | `src-tauri/src/commands/settings.rs:158-242` |
| 文件对话框 | `src-tauri/src/file_dialog.rs`（全文件） |
| 窗口可达性算法 | `src-tauri/src/windows/window_position.rs:61-105` |
| 前端流消费（ack 契约） | `frontend/src/lib/tauri/stream-client.ts:1-94` |
| TS 生成配置 | `.cargo/config.toml`（`TS_RS_EXPORT_DIR`） |
| CSP / 窗口声明 / bundle | `src-tauri/tauri.conf.json:18-163` |
| CI 构建/产物/UPX | `.github/workflows/build.yml:160-295` |
