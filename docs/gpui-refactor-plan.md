# NTE DPS TOOL · GPUI-Kit 界面重构方案

> 目标：把 Tauri 2 + React 的 WebView 渲染层整体替换为 Rust 原生渲染层（[GPUI](https://gpui.rs) + [gpui-kit](https://gpui-kit.com) 0.9.1）。界面不要求与现状逐像素一致，允许重排信息架构。
> 版本：v1 · 2026-11-10 · 依据本仓库实测（branch `master` @ `b6f0908`）与 3 份专项测绘（见附录 A）。
> 口径：本文行数默认「全行（含空行/注释）」；与「非空行」口径并列时单独标注。

---

## 0. 一页纸结论

**1. 这不是"换一个 UI 库"，而是"删掉一整层可抛弃的渲染进程"。**
本仓库经过多轮架构收敛后，`src/` 的 90,407 行 Rust 核心**已经是 UI 无关的**（`src/` 内对 `tauri|webview|egui|winit` 的 69 处命中全部是 `ts-rs` 条件派生属性，0 处框架调用）。React 层只做渲染与 UI-only 状态，`src-tauri` 只做薄适配。因此这次重构的账非常清楚：

| 处置 | 行数 | 占比 | 内容 |
|---|---:|---:|---|
| **原封不动** | **90,407** | **60.2%** | 整个 `src/`（engine 43,141 / core 21,166 / storage 14,148 / platform 6,611 / api+cli+bin 5,328 / lib 13） |
| **原样搬运** | 16,966 | 11.3% | `src-tauri/src/state/` 8,730（含 4,247 测试）、`contract/` 6,766、8 个 service/runtime 1,926 |
| **拆分改写** | 14,738 | 9.8% | `src-tauri` 的 commands 6,300 + channels 1,365 + windows 1,939 + 启动 39 + 文件对话框 304（合计 10,185）；前端展示逻辑 A 2,518 + UI-only 状态 B 2,035 |
| **重写视图** | 18,061 | 12.0% | 前端 44 个纯视图文件（17 个页面/浮层 + 通用组件） |
| **删除** | 9,996 | 6.7% | 前端 8,618（测试 3,536、ts-rs 生成 643、shadcn/base-ui 适配 1,071、CSS 1,574、Tauri/React 胶水、core 已有等价逻辑 326）+ `src-tauri` Tauri 机制代码 1,378 |
| 合计 | 150,168 | 100% | `src/` + `src-tauri/` + `frontend/` |

→ **71.5% 的代码（107,373 行）不需要重写，只需搬家或原地不动**；真正的新写量是 18,061 行视图 + 14,738 行壳/逻辑改写。

**2. 必须移除 Tauri 运行时，不能"同进程并存"。** GPUI 要求主线程消息泵 + 进程级 DPI/COM 单例 + 自带 Windows manifest，与 Tauri 的运行时（主线程、WebView2、`tauri-winres` manifest）冲突且零收益（Tauri 唯一价值是给 React 做 IPC，UI 变成 Rust 后这一层整层消失）。过渡只能按**两个 exe 并存**做。
**3. gpui-kit 判定：有条件可行。** 组件覆盖率很高（DataTable 行列虚拟化 / VirtualList / Chart 7 类 / TextView Markdown / TitleBar / 38 token 主题 / rust-i18n / AccessKit / 1,830 内置图标），但有 **4 个硬缺口必须自建**：① 窗口置顶 ② 鼠标穿透 ③ JSON Schema 动态表单 ④ 技能时间轴自绘。另有 **2 个未验证项可能推翻方案**：**Windows 中文 IME**、**透明窗口 + DirectComposition + 穿透标志组合** —— 因此 **P0 先做 3 天真机 spike（含环境验证共 6–8 天），不通过不启动全面迁移**。
**4. 工期：** P0 spike 6–8 天；主窗/控制台/11 业务页/详情/收尾按阶段估算合计 **133–196 人日**（单人 6–9 个月，双人 13–20 周），详见 §6。
**5. 数据与行为向后兼容：** 配置文件格式（`UiConfig`）、历史库、i18n JSON（`res/languages/*.json`）、Mod ABI、CLI JSON-RPC、更新签名协议**全部不变**，用户升级后现有设置/历史可继续读。

---

## 1. 现状基线（实测）

### 1.1 三层代码账

| 层 | 目录 | 文件 | 行数 | 角色 | 处置 |
|---|---|---:|---:|---|---|
| Rust 核心 | `src/` | 83 | 90,407 | 抓包/解析/reducer/history/存储/更新/Mod/工具箱/Win32 原生能力 | **原封不动** |
| Tauri 薄壳 | `src-tauri/` | 80 | 28,529 | 144 条命令、10 路流、7 窗口、DTO（209 个 ts-rs 生成物） | 搬运 59.5% / 改写 35.7% / 删除 4.8% |
| React 前端 | `frontend/src/` | 405 | 31,232 | 17 个页面与全部视图 | 重写 57.8% / 逻辑 14.6% / 删除 27.6%（详见 10 号报告） |
| 根测试 | `tests/` | 1 | 69 | 集成测试 | 保留 |

### 1.2 桌面形态

7 个独立窗口，全部 `decorations:false`（自绘标题栏）：

| # | 窗口 | 尺寸 | 特殊能力 | 承载页面 |
|---|---|---|---|---|
| 1 | `main-dps` | 670×560 | 透明否 / 主窗，拥有桌面生命周期 | 实时 DPS 主面板、开场引导、更新提示、命中快照弹窗 |
| 2 | `hud-spike` | 380×238 | **透明 + 置顶 + 跳任务栏 + Acrylic + 圆角 + 鼠标穿透** | HUD 浮层（6 个模块，可排序/可调宽/锁高） |
| 3 | `notification-island` | 440×76 | **透明 + 置顶 + 跳任务栏 + 不抢焦点** | 通知岛（5s 自动消失 + 撤销） |
| 4 | `console` | 1180×760 | 关闭=隐藏 | **11 个子页**：角色数据、技能、时间轴、历史、空幕、加密 ini、Mod 工坊、数据包、诊断、设置、快捷键 |
| 5 | `abyss-values` | 1100×720 | 关闭=隐藏 | 深渊数值与预测队伍 |
| 6/7 | `character-details` / `team-details` | 880×620 / 980×680 | 关闭=隐藏，共用同一 HTML | 战斗明细（逐击表 + 列宽配置） |

### 1.3 现有数据通路（重构要替换的部分）

```
抓包线程 ──crossbeam──> LiveCaptureService(核心, revision 计数) ──> AppState(src-tauri, 领域状态)
                                                                       │
        10 个 tokio 轮询任务(100–500ms)：比对 revision → 投影快照 ───────┘
                    │ tauri::ipc::Channel（含 ack、generation、交付序号、14MB 上限）
                    ▼
        React：stream-client.ts → zustand/useState → 视图
        React：invoke("command") ──> 144 条 #[tauri::command]
```

单进程化后，**Channel/ack/序号/订阅注册表/ts-rs 生成物整体消失**，换成一个 UI ticker：`比对 revision → 调 core 投影函数 → 更新 View Entity → cx.notify()`。

### 1.4 为什么这个仓库的迁移成本低于一般项目

1. **core 里已经有投影层**：`core/hud.rs`(1,027)、`timeline.rs`(517)、`skills.rs`(454)、`packets.rs`(412)、`character_data.rs`、`combat_details.rs`、`history.rs`、`diagnostics.rs` 的注释明确写着 "frontend-neutral projection"。一般项目迁移 UI 要先补的 "PageState/ViewModel" 这一层，本仓库已经做完了。
2. **配置词汇表已在 Rust**：`storage/config.rs`(2,197) 用 serde enum 定义了 `ThemePreset`/`AccentColor`/`UiDensity`/`HudModule`/`HitDetailColumn`/`MainDpsMetric`/`MainDpsAttribution`/`TimelineDpsViewMode`/`HotkeyKey`/`GlobalHotkeyAction`，以及 7 个窗口的几何字段。UI 只需消费，不许重新定义。
3. **i18n 已在 Rust**：`storage/i18n.rs` 的 `t()/tf()` + `res/languages/{zh-CN,ja}.json`（2,468 键）是唯一事实源。新 UI 直接 `t()`，不需要 2–3 人日的 i18n 迁移（30 号文档按通用项目估的这部分在本仓库可省）。
4. **Win32 已下沉 core**：`platform/window_style.rs`（Acrylic/圆角/整体不透明度，入参是裸 `HWND: isize`）、`platform/passthrough_hotkey.rs`（`WH_KEYBOARD_LL` 钩子 + 热键表）不依赖任何 UI 框架，GPUI 窗口拿到 HWND 后可直接复用。
5. **无历史包袱**：全仓无托盘、无开机自启、无单实例、无路由库、无状态持久化中间件。

---

## 2. 目标架构

### 2.1 crate 布局

```
D:\NTE_DPS_TOOL\
├─ Cargo.toml                    # 保留根 crate `nte-dps-tool`（= 现在的 src/）
│                                #   features: default=["desktop"], cli, external_resources
│                                #   删除 feature `tauri_frontend`（唯一作用是 54 处 ts-rs 派生）
│                                #   [[bin]] nte-core(cli)、nte-updater(desktop) 不动
├─ src/                          # 90,407 行，零改动
├─ build.rs                      # 去掉 CARGO_FEATURE_TAURI_FRONTEND 分支 → res/ 全量嵌入
├─ crates/
│  └─ desktop/                   # 新增：gpui 单进程 app（package = nte-dps-tool-desktop，bin = nte-dps-tool）
│     ├─ Cargo.toml              #   nte-dps-tool{default-features=false, features=["desktop"]}
│     │                          #   + gpui-kit 0.9.1（Apache-2.0）
│     │                          #   + log/env_logger（现有 debug 日志用 tauri-plugin-log，替换）
│     └─ src/
│        ├─ main.rs              #   gpui application().run：init(cx) → 注册主题/字体 → 开 7 个窗口（6 类视图）
│        ├─ session/             #   ← src-tauri/src/state/*（8,730 行，0 逻辑改动）
│        ├─ services/            #   ← src-tauri/src/{settings,encrypted_ini,character_data,
│        │                       #      equipment_operation,team_import}_service.rs、*_runtime.rs（1,926 行）
│        ├─ viewmodel/           #   ← src-tauri/src/contract/*（6,766 行，删 TS/serde 属性）
│        ├─ presentation/        #   前端 models 中 core 未覆盖的纯逻辑（约 1,000–1,500 行新写）
│        ├─ shell/               #   ← src-tauri/src/windows/* 可搬运部分 + 新窗口编排
│        │   ├─ windows.rs       #     窗口注册表：enum WindowKind + HashMap<WindowKind, AnyWindowHandle>
│        │   ├─ geometry.rs      #     ← window_position.rs + 各窗 debounce 持久化（180–350ms）
│        │   ├─ hud_native.rs    #     HWND → core::platform::window_style + 新增 WS_EX 扩展
│        │   ├─ hotkeys.rs       #     ← passthrough_hotkey.rs 派发器（50ms 轮询）
│        │   └─ island.rs        #     ← island 通知运行时（5s 生命周期/撤销令牌）
│        ├─ ticker.rs            #   全局 UI ticker：revision → project → notify（替换 10 路 Channel）
│        ├─ theme.rs             #   现有语义 token → gpui-kit Theme（38 token + 4 语义扩展）
│        ├─ dialogs.rs           #   文件对话框（gpui prompt_for_paths，← file_dialog.rs 的 filters/outcome 保留）
│        └─ views/               #   17 个页面/浮层（新写，18,061 行前端视图的对应物）
└─ (删除) src-tauri/             # 28,529 行：A 搬走 / B 改写 / C 删除
   (删除) frontend/              # 31,232 行
```

依赖方向（硬约束）：`crates/desktop → nte-dps-tool(desktop)`。**反向禁止**：`src/` 不得依赖 gpui/gpui-kit/窗口库；`crates/desktop` 不得内含业务规则（解析、reducer、评分、持久化规则），只能读 core 快照并渲染。

### 2.2 单进程内的数据流

```
LiveCaptureService (core) ──revision──> ticker (crates/desktop, 单个 gpui Task)
                                          │  每个流保留自己的"投影闭包"（从 src-tauri/channels/* 原样搬）
                                          │  HUD 100ms / 主窗 100ms / settings 200ms / history·diagnostics 500ms
                                          ▼
                                   AppSession (Entity, 持有 Arc<AppState>)
                                          │ observe / event
                        ┌─────────────────┼──────────────────┐
                        ▼                 ▼                  ▼
                  MainDpsView        HudView             ConsoleView(11 子页)
                  实体各自保存上一帧快照 + UI-only 状态；只调用 session 的 action 方法
```

- **命令 → 方法**：144 条 `#[tauri::command]` 没有业务规则，一律是 `validate_window → state.*() → publish_notice → snapshot()`。改写后 = `AppSession` 上的 pub 方法（参数直接用 Rust 类型，不再 serde 反序列化），视图通过 `cx.listener` 调用。
- **通道 → 投影闭包**：`channels/*.rs` 的 `FnMut(&AppState) -> PollingStreamOutput<T>` **原样保留**，只换投递端点（Channel → `WeakEntity.update`）。
- **窗口事件 → 进程内调用**：现在 6 个自定义跨窗事件（`emit_to`/`listen`）改成直接调用目标窗口 View 的 handle。

### 2.3 分层职责（"谁不许做什么"）

| 职责 | core `src/` | `crates/desktop` | 禁止 |
|---|---|---|---|
| 抓包/解析/reducer/history/SQLite/更新/Mod | ✅ | ✗ | — |
| revision 与快照投影 | ✅（已存在） | ✗ | UI 层重算业务规则 |
| 窗口创建/显隐/置顶/穿透/几何持久化 | ✗ | ✅ | core 引入窗口库 |
| UI-only 状态（选中页、展开态、输入框、滚动位置） | ✗ | ✅ | 塞进 core / 配置文件 |
| 主题 token、字号、间距、动效 | ✗ | ✅（`theme.rs`） | 视图里硬编码色值 |
| 文案 | ✅（`t()` + res/languages） | ✗ | 视图里硬编码中文 |
| 文件对话框、剪贴板、打开链接 | ✗ | ✅ | core 依赖 rfd 等 |

---

## 3. 处置总账

### 3.1 原封不动（90,407 行，60.2%）

| 模块 | 行数 | 说明 |
|---|---:|---|
| `src/engine/` | 43,141 | 协议解析、模型、结算；19 个文件，含 8,562 行测试 |
| `src/core/` | 21,166 | reducer、live_capture、history、**已有 8 个 frontend-neutral 投影**、mod/toolkit 编排 |
| `src/storage/` | 14,148 | config/i18n/history/mod_scripts/update/resource（含 i18n 字典 2,468 键） |
| `src/platform/` | 6,611 | mods_plugin、toolkit、update_http/install、**window_style（HWND）**、**passthrough_hotkey** |
| `src/api` + `src/cli` + `src/bin` | 5,328 | CLI JSON-RPC 旁路与 nte-core/nte-updater 两个 bin |
| `src/lib.rs` | 13 | 模块声明 |

**唯一改动**（不改逻辑，只删 UI 相关属性）：删 54 处 `#[cfg_attr(feature="tauri_frontend", derive(ts_rs::TS))]`、4 处 `#[ts(export)]`、`Cargo.toml` 的 `tauri_frontend` feature 与 `ts-rs` 依赖、`build.rs` 的 `CARGO_FEATURE_TAURI_FRONTEND` 分支、`.cargo/config.toml` 的 `TS_RS_*` 环境变量。

**同样原封不动**：`res/`（2,468 键 i18n、518 个图片/图标、data、mod-runtime-schema）、`docs/CLI_PROTOCOL*.md`、`docs/examples/`、`tests/`、`scripts/`（仅改扫描路径）、`nte-core`/`nte-updater` 两个产物、更新签名/清单协议、历史库格式、Mod ABI、`UiConfig` 配置文件格式。

### 3.2 原样搬运（16,966 行，11.3%）——"拆分"的第一类：改路径即可

| 来源 | 行数 | 迁往 | 需要的改动 |
|---|---:|---|---|
| `state/`（combat/session/settings/history/teams/equipment/diagnostics + tests） | 8,730 | `crates/desktop/src/session/` | 只改 `use crate::` 路径与 `AppState::new` 构造参数；4,247 行测试随迁 |
| `contract/*`（17 个文件） | 6,766 | `crates/desktop/src/viewmodel/` | 删 138 处 `#[derive(...TS...)]`、134 处 `#[ts(...)]`、`serde(rename_all)` 可保留可删；投影测试随迁 |
| 8 个 service/runtime（settings_service 474、history_runtime 383、encrypted_ini 391、update_runtime 254、team_import 280、desktop_runtime 100、diagnostics_runtime 96、character_data 66、equipment_operation 92） | 1,926 | `crates/desktop/src/services/` | 去掉 `tauri::State/AppHandle` 参数，改为直接持有 `AppState` |
| `windows/` 中纯算法/常量/测试 | ≈350 | `crates/desktop/src/shell/` | `window_position.rs` 的位置可达性、HUD 的 `logical_hud_width`/`height_constraint_order`/`run_debounced_persistence`、几何迁移函数 |

> 关键判断：`state/` 是本次重构**最大的资产**。采集编排、暂停、轮次、撤销（reset 5s / history delete 5s）、归档重试队列、HUD 配置事务、队伍导入、设备目录、更新状态机都已经是框架无关代码。**P1 第一步就搬它并跑通 4,247 行测试**，就能证明"59.5% 可复用"是真实的。

### 3.3 需要拆分（14,738 行，9.8%）——"拆分"的第二类：逻辑留下、外壳换掉

**A. `src-tauri/commands/` 6,300 行 → 14 组方法/动作**

| 组 | 文件 | 行数 | 拆分去向 |
|---|---|---:|---|
| 设置 | `settings.rs` | 1,146 | `AppSession::settings_*`（约 30 个方法）+ 设置页 view action |
| 插件/工具箱 | `toolkit.rs` | 989 | `AppSession::toolkit_*` + 文件对话框调用点 |
| 主窗 | `main_dps.rs` | 997 | 一半是窗口控制（→ `shell/windows.rs`），一半是采集/会话动作（→ `AppSession`） |
| 空幕 | `empty_curtain.rs` | 934 | 装备网格动作（下移为 `AppSession` 方法）+ 画布视图 |
| 历史 | `history.rs` | 575 | `AppSession::history_*` + `DataTable` 视图 |
| 诊断 | `diagnostics.rs` | 303 | 导入/导出（文件对话框）+ 只读视图 |
| 加密 ini | `encrypted_ini.rs` | 253 | 打开/保存（对话框）+ `Textarea`/`Editor` 视图 |
| 角色/队伍 | `user_characters/team_import/character_data/abyss_values/island/skills/packets/timeline/console_control/mod_studio/plugin_form/desktop_window` | 1,103 | 按域拆：窗口控制 → shell；数据动作 → session；导航 → 视图 |
| 备注 | 144 条命令 **0 条死命令**（每条都有前端调用方），改写时按"窗口校验/通知/文件对话框/异步调度"四类逐条替换 |

**B. `src-tauri/channels/` 1,365 行 → 保留投影、删注册表**

| 保留 | 删除 |
|---|---|
| 10 路流的**投影闭包与 revision 比较**（`technical/main_dps/main_dps_detail/history/settings/skills/timeline/packets/diagnostics/empty_curtain`） | `stream_runtime.rs` 全部（Channel/ack/generation/交付序号/14MB 上限）、`contract/stream.rs`、`state.rs` 内的订阅注册表、21 条 subscribe/unsubscribe 命令、前端 `stream-client.ts`(84) |

**C. `src-tauri/windows/` 1,939 行 → `shell/`**

| 文件 | 行数 | 处置 |
|---|---:|---|
| `hud.rs` | 412 | 窗口内容尺寸/高度锁/宽度持久化（搬运）+ 原生外观调用改为 `HWND`（复用 core） |
| `main_dps.rs` | 277 | 窗口创建/首帧显示可删；关闭主窗销毁其余窗口的逻辑重写 |
| `island.rs` | 210 | 通知生命周期/撤销/定位搬运；`set_focusable(false)` 改为 Win32 扩展 |
| `passthrough_hotkey.rs` | 258 | 事件源在 core；派发器改写为查窗口注册表 |
| `window_position.rs` | 142 | 纯函数搬运 |
| `console.rs` / `combat_details.rs` / `abyss_values.rs` | 425 | 几何持久化（debounce 线程可留）重写为 gpui bounds |
| `mod.rs` | 50 | 删除（窗口标签校验随 IPC 消失） |

**D. 启动与对话框**

- `lib.rs` 315 行：7 个窗口装配 + 首帧显示 + 流清理 + 插件初始化 + 热键运行时 + 3 个后台调度 → 重写为 `main.rs` + `session/boot.rs`（约 200 行）。
- `file_dialog.rs` 304 行（7 个入口）：保留 filters/结果枚举与 i18n，对话框本身换 gpui `App::prompt_for_paths`（不新增 `rfd` 依赖）。
- `main.rs` 4 行：改 bin 入口。

**E. 前端 4,553 行非视图文件 → 两类拆分**（逐文件和逐 feature 明细见 [10 号报告](gpui-refactor/10-frontend-inventory.md) §4）

| 类 | 文件/行数 | 去向 |
|---|---:|---|
| **A 展示逻辑**（无 React/DOM 依赖：格式化、几何刻度、命令目录、导航、过滤排序、颜色兜底） | 23 / **2,518** | 约 0.4k 在 core 已有等价实现 → 删；其余约 2.1k 进 `crates/desktop/src/presentation/`（Rust 纯逻辑 + `#[test]`） |
| **B UI-only 状态**（`use-*.ts` 13 个、`hooks/` 3 个、zustand store） | 20 / **2,035** | 各 View Entity 字段：选中页/展开项/搜索词/列宽/排序/滚动锚点/拖拽中/加载与错误态；订阅与清理由 GPUI `Subscription`/`Task` 生命周期取代 |

> 典型例子：`console-command-palette-model.ts`(327)、`timeline-ui-model.ts`(247)、`equipment-canvas-layout.ts`(238)、`main-dps-detail-model.ts`(185)、`abyss-values-model.ts`(182)、`technical-view-model.ts`(174) 属 A；`use-history.ts`(278)、`use-settings.ts`(240)、`use-empty-curtain.ts`(207)、`use-encrypted-ini.ts`(163) 属 B。

### 3.4 重写（18,061 行，12.0%）——17 个页面

详见 §5 组件映射；逐页清单：

| 页面/浮层 | 视图行数（C 类） | gpui-kit 组合 | 难度 |
|---|---:|---|---|
| 主窗 DPS 面板（含工具栏、数据源、队伍行、开场引导、更新提示、命中快照弹窗） | 3,815 | 自建排行行 + `Chart` + `Progress` + `Toolbar` + `Dialog` | **高** |
| 空幕装备（**整卡 canvas 自绘 680 行**：品质/套装/锁定角标、文本截断、命中测试、右键意图） | 1,968 | 自绘卡片 + `Canvas`/`PathBuilder` + `Menu`/`Dialog` + 拖放 | **高（勿按普通列表估）** |
| Mod 工坊（JSONForms 459 行真代码 + Ajv 校验 + 数组增删/失败回滚 + 市场 + 宿主控制） | 1,458 | 自写 JSON Schema 渲染层（10–14 人日）+ `List`/`Dialog` | **最高** |
| 设置（**`settings-catalog.tsx` 1,159 行：14 卡片、约 90 控件、5 份表单草稿 keepMounted**、HUD 模块排序、热键） | 1,684 | `Settings` + `Form` + `List` 拖拽排序 + 热键捕获；"切 Tab 不丢未保存输入"需 View Entity 显式持有草稿 | **高** |
| HUD 浮层（6 模块） | 1,405 | 自建模块行 + 迷你 `Chart`/自绘 | 中高（宿主层最难） |
| 时间轴（**`timeline-chart.tsx` 742 行 canvas**：双模式/累积线/标记/hover lens/框选缩放/右键/DPR） | 1,153 | `Chart` + 自绘色条/刻度/缩放平移/命中测试 | **高（勿按 Chart 估）** |
| 控制台外壳（导航/侧栏/命令面板） | 981 | `Sidebar` + 自建 `enum Page` + `Command` | 中 |
| 历史 | 1,046 | `DataTable` + 对比视图 + `Dialog` | 中 |
| 深渊数值 | 938 | `DataTable` + `Dialog` | 中 |
| 技能 | 690 | `DataTable` + 诊断卡片 | 中 |
| 诊断 | 653 | `List` + 状态卡片 | 低中 |
| 角色数据/配装表 | 606 | `DataTable` + `Tree`（分组） | 中 |
| 加密 ini | 410 | `Textarea`/`Editor` + IME 风险 | 中（IME） |
| 数据包 | 357 | `VirtualList`（1 万行） | 低中 |
| 快捷键 | 302 | 自建按键捕获 + `Kbd` | 中 |
| 通知岛 | 186 | 轻量视图 + 窗口行为 | 中（窗口行为） |
| 通用组件（标题栏/表单行/空状态/提示/动画数字/窗口动效边界/markdown） | 409 | `TitleBar`/`Form`/`Empty`/`Alert`/`Skeleton`/`TextView` + 自建动画数字 | 低中 |

### 3.5 删除（9,996 行，6.7%）

| 项 | 行数 |
|---|---:|
| 前端测试（43 个文件：33 个 ts + 10 个 tsx；其中 2,085 行纯逻辑测试迁成 Rust `#[test]`，其余 1,451 行视图/胶水测试直接删） | 3,536 |
| `frontend/src/lib/tauri/generated/`（209 个 ts-rs 生成物） | 643 |
| shadcn/@base-ui 适配层（`components/ui` 15 个非测试文件） | 1,071 |
| motion / dnd-kit 适配（`window-motion-boundary`、`hud-module-sortable`、`use-hud-module-sortable` 等） | 196 |
| `@jsonforms` 适配（`plugin-form-renderers.tsx`） | 314 |
| feature 内失效/Dev 文件（`timeline-ui-preview` 64、`character-build.css` 429、`damage-image-lookup` 50） | 543 |
| React 入口/窗口胶水（`entries/*` 66、`window-events`/`window-ready`/`console-window-background`/窗口 client 107） | 173 |
| Tauri IPC client 层（`lib/tauri/*-client.ts` 19 个 + `command-error.ts`） | 617 |
| CSS 层（`index.css` 878、`theme.css` 214、`ui-foundation.css` 202、`console.css` 66） | 1,360 |
| 前端 i18n 实现（`lib/i18n.ts`） | 43 |
| core 已有等价的 lib 文件（`settings-presentation-model`、`decimal-string`、`character-attribute`、`window-labels`） | 62 |
| 其余失效/重复（`utils.ts` 的 `cn`、`character-color.ts` 的 FNV 兜底） | 60 |
| 前端合计 | **8,618** |
| `src-tauri` Tauri 机制代码（serde/ts 属性 406、Builder/manifest/窗口装配 280、Channel/ack 150、订阅注册表 80、async_runtime 62、命令框架参数 400） | **1,378** |
| **总计** | **9,996** |

**依赖一并删除**：`tauri`、`tauri-plugin-dialog`、`tauri-plugin-log`、`ts-rs`、`@tauri-apps/api`、`react`/`react-dom`、`shadcn`/`@base-ui`/`@radix` 系、`tailwindcss`/`@tailwindcss/vite`/`tw-animate-css`、`dnd-kit` 3 个包、`motion`、`@jsonforms/*`、`lucide-react`、`zustand`、`vite`/`vitest`/`prettier`/`oxlint`、`frontend/pnpm-lock.yaml`。**净新增依赖只有 `gpui-kit` 0.9.1 + `log`/`env_logger`**（gpui-kit 为 Apache-2.0，其 `gpui-pre` 亦为 Apache-2.0，与本仓 AGPL-3.0-or-later 兼容；`gpui-pre` 依赖树较大，首次编译明显变慢，见风险 R6）。

---

## 4. 拆分与落位细则

### 4.1 窗口编排（`shell/windows.rs`）

| 现有行为 | 证据 | gpui 落位 |
|---|---|---|
| 主窗关闭 → 销毁其余 6 窗 | `windows/main_dps.rs:137-153` | `on_window_closed` 里对注册表其余句柄 `remove_window()` |
| Console/详情窗关闭 = 隐藏（不销毁） | `windows/console.rs:53-61`、`combat_details.rs:133-138` | `on_window_should_close` 返回 `false` + 记 `hidden` 标记；显示走 `handle.update(activate_window)` |
| 通知岛关闭 = 隐藏 | `windows/island.rs:173-181` | 同上 |
| 全部窗口 `visible:false` + 首帧后才显示 | `lib.rs:34-45` | 删除：GPUI 首帧后 `show` 是常态 |
| `set_always_on_top`（6 窗可切换） | `commands/desktop_window.rs`、`technical.rs` | 新增 Win32 `SetWindowPos(HWND_TOPMOST)`（capability 全部删除） |
| 窗口几何持久化（3 套 debounce 180–350ms） | `hud.rs:88-157`、`console.rs:36-137`、`combat_details.rs:72-145` | 搬运 debounce，读 `window.window_bounds()`；`UiConfig` 的 `[f32;2]` 逻辑尺寸 ↔ `WindowBounds` 像素换算需一处适配 |
| 首次显示时 DPI 可达性校验 | `windows/window_position.rs` | 纯函数搬运，入参改为 gpui 显示器列表 |

### 4.2 HUD 浮层宿主层（最高风险，`shell/hud_native.rs`）

| 能力 | 现状 | 新实现 |
|---|---|---|
| 透明背景 | `tauri.conf.json transparent:true` | `WindowOptions.window_background = WindowBackgroundAppearance::Transparent`（Win10 <17763 静默失效） |
| Acrylic/圆角/整体不透明度 | core `window_style::{apply_persistent_acrylic, apply_borderless_rounding, apply_uniform_opacity}(hwnd)` | **直接复用**（HWND 来自 `HasWindowHandle`） |
| 置顶 | `window.set_always_on_top` | 新增 `SetWindowPos(HWND_TOPMOST/HWND_NOTOPMOST, SWP_NOACTIVATE)` |
| 跳任务栏 | `skipTaskbar:true` | 新增 `GWL_EXSTYLE |= WS_EX_TOOLWINDOW` |
| 鼠标穿透 | `set_ignore_cursor_events` | 新增 `GWL_EXSTYLE |= WS_EX_TRANSPARENT`（注意与 GPUI 的 `WS_EX_NOREDIRECTIONBITMAP`/DComp 组合，spike 验证） |
| 不抢焦点 | 无（HUD 允许） | 通知岛需要：`WS_EX_NOACTIVATE` + 不 `focus` |
| 穿透热键切换 | core `passthrough_hotkey` + `tauri` 派发 | 派发器改查窗口注册表，热键表/回退逻辑原样 |

**落位建议**：把这 5 个 Win32 调用补进 core `src/platform/window_style.rs`（与现有 3 个函数同风格，入参 `isize` HWND），而不是散落在 app crate —— 这样"core 不依赖 UI 框架"的边界不破，代码也不重复。

### 4.3 每个 View Entity 的字段模板

| 字段类 | 例子 | 来源 |
|---|---|---|
| 共享会话 | `Arc<AppState>` / `Entity<AppSession>` | 搬运 |
| 上一帧快照 | `HudSnapshot`、`MainDpsReadout`、`HistorySnapshot`… | core 投影 |
| 流订阅 | `Vec<Subscription>`、`Option<Task<()>>` | 替代 hooks 的 `useEffect` + `active` 标志 |
| UI-only | 选中页/展开项/搜索词/列宽/排序/滚动锚点/拖拽中 | 从 `use-*.ts` 搬（原为 zustand/useState） |
| 本地输入态 | `Entity<InputState>`（gpui-kit 受控输入） | 新写（替代受控 React state） |
| 窗口句柄 | `AnyWindowHandle`（需要跨窗操作时） | 新写 |

### 4.4 拆分后不允许存在的形态（防回潮）

- 不保留 `contract::*` 的 `TS` 派生、不保留任何 `serde_json::Value` 形式的"给前端用的 DTO"。
- 不在 `crates/desktop` 里重新实现任何 core 已有投影（`hud/timeline/skills/packets/character_data/combat_details`）。
- 不为"旧 React 版兼容"保留双写或迁移开关；删除功能时连带删 i18n key 与命令。
- 不引入 router/Dock/WebView 来"模拟"旧结构；页面切换就是一个 `enum Page` + `match`。

---

## 5. 组件映射（shadcn/React → gpui-kit）

| 现有 | 文件 | gpui-kit 对应 | 缺失度 |
|---|---|---|---|
| `components/ui/*`（button/badge/card/separator/skeleton/empty/alert/switch/tooltip/popover/menu/dialog/alert-dialog/form-controls） | 15 个非测试文件 1,071 行 | 同名组件齐全（`Button`/`Badge`/`GroupBox`/`Separator`/`Skeleton`/`Empty`/`Alert`/`Switch`/`Tooltip`/`Popover`/`Menu`/`Dialog`/`AlertDialog`） | 无 |
| `form-controls` / `FormRow` / `Input` / `NativeSelect` | — | `Form`/`Label`/`Input`/`NumberInput`/`Select`/`Combobox`/`Slider`/`Checkbox`/`Radio`/`Textarea` | 无 |
| `markdown.tsx`（224，更新日志） | 224 | `TextView::markdown`（代码块/表格/`max_lines` 折叠） | 无 |
| `animated-number.tsx` / `max-hp-compression-effect.tsx` | 98 | `AnimationExt` + `gpui-base` motion（spring/transition，尊重减少动效） | 自建数字插值 3–4 人日 |
| `hud-module-sortable.tsx` + `hooks/use-hud-module-sortable.ts`（dnd-kit） | 112 | `List` 的 `on_drag/drag_over/on_drop`（官方拖拽排序示例） | 封装 1 人日 |
| `desktop-titlebar.tsx`（自绘标题栏+4 按钮） | 140 | `TitleBar` + `TitleBar::window_options()`（Windows 窗口按钮 34px） | 无 |
| `window-motion-boundary.tsx`（MotionConfig + 减少动效） | 84 | `cx.reduce_motion()`（gpui-kit init 在 Windows 读取系统偏好） | 无 |
| `page-layout` / `PageHeader` / `PageScrollArea` | 50 | `div` 布局 + `Resizable`/`Scrollable` | 无 |
| `EquipmentCanvasGrid`（绝对定位自绘网格，**680 行 canvas：DPR backing store、裁剪窗口、`measureText` 文本截断、品质/套装/锁定角标、命中测试、右键意图**） | 680 | 自绘卡片 + `canvas()`/`PathBuilder`（GPUI 无 canvas 2D context，需重画全部卡片与滚动虚拟化） | **自建，按普通列表估会低估 2–3 倍** |
| 时间轴 `timeline-chart.tsx`（**742 行 canvas：双模式/累积线/时间停止区/标记/hover lens/tooltip/框选缩放/右键/DPR**） | 742 | `Chart` 覆盖折线；色条/刻度/缩放平移/命中测试自绘 | **自建 6–8 人日** |
| 表格（历史/技能/深渊/角色/命中明细） | — | `DataTable`（行列虚拟化、列宽/固定列/排序 delegate、右键、键盘） | 无（分组行不支持，按扁平化处理） |
| 日志/数据包（长列表） | — | `VirtualList`（变高、粘底需 30 行自建） | 极小 |
| 48 处 `<Dialog>` | — | `window.open_dialog` / `Sheet` / `AlertDialog` | 3–4 人日逐点替换 |
| `@jsonforms`（插件表单，**459 行真渲染器 + Ajv 运行时校验（当前依赖 CSP `unsafe-eval`）+ 数组增删/确认/失败回滚语义**） | `plugin-form-renderers.tsx` 等 | `Form` 只有布局，**schema/uiSchema/校验/rule 联动必须自写**；顺带消除 `unsafe-eval` CSP 依赖 | **自建 10–14 人日** |

**主题映射**（`crates/desktop/src/theme.rs`）：现有 `theme.css` 的语义 token（`background/foreground/card/popover/primary/secondary/muted/accent/destructive/border/input/ring` + `console-success/warning/status` + `chart-1..5` + `hud-role-1..4`）→ gpui-kit theme JSON（38 token，含 `base.{red,green,blue,yellow,magenta,cyan}`）。`success/warning/info/destructive` 4 个语义名 gpui-kit 没有，用 `base.*` 做薄映射。字号在 `Theme::update` 强制 12/14/20/24（gpui-kit 默认 16/13 与 AGENTS.md 冲突），边框用 `px(0.5)` 封装 `hairline()`。`ThemePreset/AccentColor/UiDensity` 继续读 `UiConfig`（core 已有枚举），不新建主题系统。
**i18n**：应用文案继续走 `nte_dps_tool::storage::i18n::t()/tf()`（唯一事实源，2,468 键）；gpui-kit 组件自身的内部文案（日历月份、表格空态等）用 `rust_i18n::extend!(gpui_component)` 可选接入，不影响现有键。

---

## 6. 分阶段实施计划

> 原则：**旧版必须能持续发布，直到 P6 才删 Tauri**。每阶段独立可验收、可回滚（git）。

| 阶段 | 内容 | 产出/验收 | 人日 |
|---|---|---|---:|
| **P0 spike（先做，不通过则整体重审）** | 真机 4 个 spike：S1 透明 HUD + `WS_EX_TRANSPARENT` 穿透 + `HWND_TOPMOST` 置顶；S2 中文 IME（微软拼音/搜狗，组字/候选窗/多屏缩放/Dialog 内）；S3 Acrylic/圆角/透明度与 DComp 组合；S4 组件冒烟（DataTable 1 万行、VirtualList 1 万行、Chart 高频刷新、TitleBar 拖动、List 拖拽排序）。另含环境验证：MSVC/SDK/CMake/`fxc.exe`、单 exe manifest、UPX 兼容 | 4 份 spike 记录 + 通过/不通过结论；**S1/S2 任一不通过即暂停全面迁移** | 6–8 |
| **P1 骨架与状态搬迁** | 新建 `crates/desktop`；搬 `state/`+services+`viewmodel/`（删 TS 属性）；单窗假页面跑通 4,247 行测试；ticker 打通 2 路流（settings/main_dps）；主题层落地；删 `tauri_frontend` feature 与 build.rs 分支 | `cargo test -p nte-dps-tool-desktop` 全绿；能显示主窗骨架 + 设置页只读 | 12–18 |
| **P2 主窗 + 控制台外壳** | DPS 排行、工具栏、数据源、引导、更新提示；`Sidebar` + 11 页导航 + 命令面板（先只挂空页） | 主窗功能与现有对等（除 HUD/详情） | 20–28 |
| **P3 HUD 浮层 + 通知岛** | HUD 6 模块、宽度/高度/位置持久化、穿透热键、置顶；通知岛生命周期/撤销/不抢焦点 | 真机：游戏内可用、穿透正常、不抢焦点 | 15–22 |
| **P4 Console 11 个业务页** | 历史、数据包、技能、时间轴、诊断、角色数据、空幕、加密 ini、快捷键、设置完整版、Mod 工坊（含 JSONForms 自写层、时间轴自绘） | 逐页与现有对等；加密 ini 中文输入可用 | 60–90 |
| **P5 详情/深渊/收尾** | 战斗明细（列宽配置）、深渊；文件对话框、更新安装后退出、日志、UPX/体积、打包 | 全窗口真机验收 | 15–22 |
| **P6 删除旧栈** | 删 `src-tauri/`、`frontend/`、capabilities、ts-rs/生成物、CI 步骤与 AGENTS.md/frontend-guidelines 重写 | CI 全绿；发布产物只剩单 exe | 5–8 |
| **合计** | | | **133–196**（单人 6–9 个月；双人 13–20 周） |

**可并行轨道**（双人时）：① 壳层+状态搬迁（P1/P2）② 页面重写（P2–P5 各自独立页面）③ HUD 宿主层 spike 后独立推进（P3）。轨道之间只共享 `AppSession` 接口与主题层，写作用域不重叠。

**范围收缩选项**（如果工期不可接受）：

| 策略 | 做法 | 工期 | 代价 |
|---|---|---|---|
| A（本文推荐） | 全量替换单 exe | 6–9 个月单人 | — |
| B | 两个 exe 并存：GPUI 先接管 HUD + 主窗，Console 等仍在 Tauri 版；后续按窗口迁移 | 3–4 个月 | 两进程需要 IPC（可复用 CLI JSON-RPC 模式）、两套 UI 维护 |
| C | 只把 HUD 浮层换成 GPUI（性能/穿透/独立进程收益最高的部分），主程序保留 Tauri | 1.5–2 个月 | 长期两套技术栈；AGENTS.md 需允许双 UI |

---

## 7. 风险与未验证项

| # | 风险 | 影响 | 缓解 |
|---|---|---|---|
| **R1** | **透明 + 置顶 + 跳任务栏 + 穿透 + 不抢焦点**（gpui-kit 无 API：置顶/穿透/跳任务栏都要 Win32 自建） | HUD 是核心卖点；任一缺失→抢焦点掉帧/吞鼠标/出现在任务栏 | P0-S1 spike；core 已有 `window_style` 可复用，新增 2 个 `SetWindowLongPtr` 封装；**必须真机验收** |
| **R2** | **Windows 中文 IME**（gpui-pre 源码有完整 IMM 实现，但官方文档零结论） | 加密 ini 编辑、Mod 表单、搜索框 | P0-S2 spike（微软拼音+搜狗）；不通过则降级"复杂编辑改只读+少量输入"或重审选型 |
| **R3** | 单 UI 线程串行 7 窗口（现每视图独立轮询） | 一个慢视图卡全部窗口 | ticker 分帧（每帧最多 1–2 个投影）；history/diagnostics 500ms；HUD 高频投影保持轻量 |
| **R4** | 无边框窗口拖动/双击最大化/边缘 resize/Aero Snap | 自绘标题栏交互退化 | `TitleBar` + `start_window_move`/`titlebar_double_click`；边缘 resize 真机验 |
| **R5** | gpui-kit 0.x 周更、无 API 稳定承诺（0.9.x 内部已有增删） | 升级即改代码 | 锁 `=0.9.1` + `Cargo.lock`；升级作为独立任务；不为未来版本预留抽象 |
| **R6** | 单 exe 体积/编译时间（gpui 渲染栈静态链入；release 需 Windows SDK `fxc.exe` 编译 shader） | 发布体积↑、CI 变慢、开发者环境要求↑ | 接受；CI 保留 UPX 与体积门限；文档写明 MSVC+SDK+CMake 前置 |
| **R7** | 主题只有 38 token，缺 `success/warning/info/destructive`；默认字号 16/13 与规范冲突 | 视觉规范漂移 | `theme.rs` 薄映射层 + 字号强制 12/14/20/24；组件尺寸变体代替"密度" |
| **R8** | 文件对话框、通知岛不抢焦点、多显示器 DPI 运行时切换、GPU 兼容（集显/远程桌面） | 真机差异 | 全部列为"未验证"，交付时明确标注，由用户真机验收 |
| **R9** | **角色颜色兜底在 TS 层**：多个 DTO 的 `color` 可为 `null`（`MainDpsCharacterSnapshot`/`HudCharacterSnapshot` 为 `string|null`，`SkillsRowSnapshot` 无 color），`lib/character-color.ts` 用 FNV 调色板补色 | 直接删前端会让同角色颜色变化/缺失 | 迁移前先在 core 把这些字段填满，或提供 `character_color(id)` 查询（约 0.5 人日），再删前端兜底 |
| **R10** | **i18n 门禁会失明**：2,468 个键里只有约 500 个是 TSX 字面量，其余由 Rust 枚举/`message_key` 动态产生；`scripts/test_i18n_source_coverage.ps1` 目前只扫 TS 字面量 + Rust 常量 | 新 UI 若写死英文或改 key 拼写，不会有任何检查报错 | 第一步就把脚本扫描目标改为 `crates/desktop/src`，并要求视图只用 `t()`/`tf()` |

**必须提前向用户认账的 3 项降级**：① 中文 IME 边缘行为不承诺与 WebView 一致（各输入法/多屏/弹窗候选窗位置）；② 无障碍：标准控件有 AccessKit 语义，但自绘区域（时间轴、DPS 曲线、排行条）只有摘要文本；③ 富文本/动效简化：Markdown/HTML 子集，复杂 CSS 动效不可复现。

---

## 8. 需要同步修改的规范、CI 与文档

| 项 | 现状 | 改法 |
|---|---|---|
| `AGENTS.md` §2 | "桌面 UI 唯一实现：Tauri 2 + Vite + React + TS + Tailwind + shadcn/ui" | 改为"gpui + gpui-kit 单进程"；保留并强化"core 不得依赖 Tauri/WebView/egui/wgpu/窗口库"（gpui 依赖 wgpu，只能出现在 `crates/desktop`） |
| `AGENTS.md` §3 | 信任边界含"Rust → Tauri → React 的 DTO"与"React → Tauri command 参数" | 删除这两条（没有 IPC 了）；新增"UI 层不得校验 core 快照，也不得重算投影" |
| `AGENTS.md` §7 | 前端 UI 规范（Tailwind/shadcn/dnd-kit/Motion/Zustand） | 整节重写为 gpui 版：主题 token、12/14/20/24 字号、0.5px hairline、`TitleBar`/`Form`/`Dialog` 约定、减少动效、图标 aria_label |
| `AGENTS.md` §11 验证表 | `pnpm format/lint/typecheck`、`export_bindings` | 改为 `cargo fmt/clippy/test -p nte-dps-tool-desktop`；删除 ts-rs 相关行 |
| `frontend-guidelines.md` | React/shadcn/Tailwind 规范（含末尾 agent 自检残留段） | 删除，替换为 gpui-kit 设计规范（基于 gpui-kit design-guides） |
| `frontend/README.md` | Vite/React 说明 | 随 `frontend/` 删除 |
| `README.md` / `README_EN.md` / `LICENSING.md` / `NOTICE.md` / `THIRD_PARTY_LICENSES.md` | 技术栈与第三方许可 | 更新技术栈；新增 gpui-kit/gpui-pre（Apache-2.0）条目 |
| `.github/workflows/build.yml` | pnpm install、frontend lint/typecheck/test、export_bindings、`tauri build --no-bundle`、`cargo audit --file src-tauri/Cargo.lock` | 删 node/pnpm 步骤与绑定校验；改 `cargo build --release -p nte-dps-tool-desktop`；审计新增的 gpui 锁文件；保留 UPX/冒烟/体积检查 |
| `scripts/verify_architecture.ps1` | 断言根 crate 无 `rfd/raw-window-handle/gui feature`、无 GUI bin | 追加：`crates/desktop` 存在、core 不依赖 gpui/gpui-kit、`src-tauri`/`frontend` 不得回归 |
| `scripts/verify_runtime_safety.ps1` | 扫描 `src` + `src-tauri/src` + `frontend/src` 的规则 | 路径改为 `src` + `crates/desktop/src`；删除 4 条前端/通道契约规则 |
| `scripts/test_i18n_source_coverage.ps1` | 扫描 `frontend/src` + `src-tauri/src`；但 2,468 键里只有约 500 个是 TSX 字面量 | 扫描目标改为 `src` + `crates/desktop/src`，并要求视图只用 `t()`/`tf()`；否则新 UI 写死英文或改 key 拼写不会被发现（见 R10） |
| `.cargo/config.toml` | `TS_RS_*` 环境变量 | 删除 |
| `res/languages/*.json`、`res/images/` | — | **不变**（build.rs 不再排除前端图片资源，`res/` 全量嵌入） |

---

## 9. 验证与验收

| 改动 | 本地验证 |
|---|---|
| 任意阶段 | `cargo fmt --check`、`cargo clippy --all-targets --workspace -- -D warnings`、`cargo test -p nte-dps-tool-desktop` |
| 核心 crate | `cargo test`、`cargo test --no-default-features --features cli`（必须仍不依赖 UI/窗口库）、`cargo check --features external_resources` |
| 桌面 app | `cargo run -p nte-dps-tool-desktop`（Windows 真机）、`cargo build --release -p nte-dps-tool-desktop`（验证 fxc/manifest） |
| 架构与安全 | `pwsh scripts/verify_architecture.ps1`、`pwsh scripts/verify_runtime_safety.ps1`、`pwsh scripts/test_i18n_source_coverage.ps1` |
| 页面级 | 每页一个验收清单：数据与现状对等、空/加载/错误态、键盘可达、`t()` 文案、减少动效 |
| 未验证（交付时必须标注，需用户真机验收） | 中文 IME 实测、透明+穿透+置顶在游戏内表现、多显示器 DPI、GPU 兼容、AccessKit 读屏、文件拖入 |

**每阶段 DoD**：上一阶段功能不回归；旧 Tauri 版仍可发布（P1–P5）；新增/修改文件无 `.bak`/临时文件；不 commit（除非要求）。

---

## 10. 需要先确认的决策点（不阻塞 P0 spike）

1. **范围**：策略 A（全量替换）还是 B/C（先 HUD/主窗）？
2. **信息架构**：窗口数量是否保持 7 个？可选简化：`character-details`/`team-details` 合并为单窗双 Tab；Console 的 11 页是否精简（重设计允许，但属产品决策）。
3. **gpui-kit 版本策略**：锁 `=0.9.1` 稳定优先，还是允许跟周更（换取新组件）？
4. **发布节奏**：P1–P5 期间是否继续发布 Tauri 版（建议是），以及是否需要在同一安装包里带两个 exe（策略 B 才需要）。

---

## 附录 A：专项测绘报告

| 报告 | 内容 | 关键结论 |
|---|---|---|
| [20 外壳/边界测绘](gpui-refactor/20-shell-core-boundary.md) | 144 条命令逐条、10 路流、7 窗口、contract 层、80 文件处置台账、核心可复用性证明 | core 已 UI 无关（0 处框架调用）；A/B/C = 16,966/10,185/1,378 |
| [30 gpui-kit 能力核实](gpui-refactor/30-gpui-kit-capability.md) | 版本/许可/工具链、窗口能力、IME 源码链、组件矩阵、JSONForms 代价、14 条未验证 | 有条件可行；4 硬缺口；必须移除 Tauri |
| [10 前端逐文件测绘](gpui-refactor/10-frontend-inventory.md) | 前端 405 文件分类（A 可下沉 2,518 / B UI-only 2,035 / C 视图 18,061 / D 删除 8,618）、硬依赖、i18n 与 token 清单、3 个最易低估的 feature | 前端 57.8% 是视图重写、14.6% 逻辑、27.6% 删除；`empty-curtain`/`timeline`/`settings` 最易低估 |

## 附录 B：复现命令

```powershell
# 三层行数
$f = Get-ChildItem src -Recurse -File -Include *.rs; ($f | Get-Content | Measure-Object -Line).Lines
$f = Get-ChildItem src-tauri/src -Recurse -File -Include *.rs; ($f | Get-Content | Measure-Object -Line).Lines
$f = Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx,*.css; ($f | Get-Content | Measure-Object -Line).Lines

# 前端视图/逻辑/测试/生成物拆分
Get-ChildItem frontend/src -Recurse -File -Include *.tsx | Where-Object Name -notmatch '\.test\.tsx$'
Get-ChildItem frontend/src -Recurse -File -Include *.ts  | Where-Object { $_.Name -notmatch '\.test\.ts$' -and $_.FullName -notmatch '\\generated\\' }

# 核心是否依赖 UI 框架（期望：只有 ts-rs 条件属性）
Get-ChildItem src -Recurse -File -Include *.rs | Select-String 'tauri|webview|egui|winit|gpui'

# 命令/通道数量
(Select-String -Path src-tauri/src/commands/*.rs -Pattern '#\[tauri::command\]').Count
(Select-String -Path src-tauri/src/channels/*.rs -Pattern 'subscribe_').Count
```
