# 30 · gpui-kit 能力核实：版本 / 窗口 / 透明穿透 / 中文 IME / 组件矩阵

> 任务：`task-3`。取证日期 **2026-11-10**（所有在线来源均为当日读取）。
> 方法：静态读取 gpui-kit.com 文档全文（`llms-full.txt`，4.7 MB，含 379 个小节）、crates.io JSON API、docs.rs、GitHub API，并下载 `gpui-pre 0.3.12` / `gpui-pre-windows 0.3.12` / `gpui-component 0.9.1` / `gpui-base 0.9.1` 的 crate 源码包核对实现（**未在本机建工程、未 cargo build、未运行任何窗口**）。
> 标注约定：**【文档明说】**= 官方文档原文；**【源码可见】**= 该版本 crate 源码里能看到实现；**【推断】**= 由文档/源码推出的结论，未实测；**【未验证】**= 本次无法确认。
> 仓库侧事实来自本仓库文件（非外部来源）：`src-tauri/tauri.conf.json`、`src-tauri/src/windows/*.rs`、`src/platform/window_style.rs`、`src/platform/passthrough_hotkey.rs`、`frontend/package.json`、`frontend/src/**`、`AGENTS.md`。
> 姊妹篇（另一项目的同作者调研残留，已逐条复核，仅作起点）：`nte-drive-calculator/docs/refactor-research/40-gpui-kit.md`。本文件不沿用其"Rust↔Python"部分，且已修正其遗漏（`always_on_top` 不存在、`set_input_region` 仅 Wayland、manifest/fxc 构建要求、Tauri 同进程判定）。

---

## 0. 结论摘要（决策级，直接回答 4 问）

| # | 问题 | 结论 | 置信度 |
|---|---|---|---|
| ① | gpui-kit 能否承担这套 UI | **有条件可行** | 中高 |
| ② | 必须自建的能力 | 5 项核心 + 9 项小件，合计 **约 42–57 人日**（中位 ≈ 49；不含 3 天 spike 与 16 个页面本身的 Rust 重写） | 推断 |
| ③ | 需要提前认账的降级 | ① 中文 IME 边缘行为 ② 无障碍（自绘区域） ③ 富文本/动效体系 | — |
| ④ | 是否必须移除 Tauri | **必须移除 Tauri 运行时**。同一进程"技术上有一线可能"（Tauri Windows 有 `any_thread`），但进程级单例与单 exe manifest 冲突使其不可安全共存；唯一有意义的过渡是**双进程/双 exe** 分阶段替换 | 高 |

关键支撑（详见后文）：

1. 组件层对本项目覆盖率**很高**：DataTable（行/列虚拟化、列宽/固定列/排序/右键）、VirtualList（变高）、Chart（Line/Bar/Area/Pie/Radar/Candlestick/Sankey）、TextView（Markdown+HTML 子集）、Dialog/Sheet/Popup/Tooltip/Menu/ContextMenu/Notification、List 的 `on_drag/on_drop` 拖拽排序、TitleBar（Windows 自绘窗口按钮 34px）、Icon（内置 **1,830** 个图标名，Lucide+Isocons，可替代 `lucide-react`）、主题 JSON（38 token/套 + 38 套主题）、i18n（rust-i18n 4.2 集成，组件自带 zh-CN）。【文档明说】
2. **四个真实硬缺口**：① **窗口置顶**（`WindowOptions` 无 `always_on_top` 字段）；② **鼠标穿透**（`Window::set_input_region` 明确只用于 Wayland/Linux）；③ **JSON Schema 动态表单**（Form 只是布局，校验/提交/联动归应用）；④ **技能时间轴自绘**（Chart 是数据驱动图，不自带任意标注绘图层）。前两项靠 HWND + Win32 补齐（本项目已有 HWND 级 Win32 代码可复用）。
3. **中文 IME 是"源码可见支持、实测未验证"**：gpui-pre-windows 实现了 `WM_IME_STARTCOMPOSITION`/`WM_IME_COMPOSITION`、`ImmSetCompositionWindow`/`ImmSetCandidateWindow`、`replace_and_mark_text_in_range`（预编辑串 + 候选窗定位 + 按焦点启停 IME），但官方文档**没有**任何 Windows IME 结论，必须真机 spike。
4. **透明 + 置顶 + 穿透的组合是最大不确定项**：透明（`WindowBackgroundAppearance::Transparent`）源码可见且实现为 `SetWindowCompositionAttribute(accent_state=2)`，要求 Windows 10 build **≥17763**；GPUI 窗口默认 `WS_EX_NOREDIRECTIONBITMAP`（DirectComposition），而 Tauri/tao 的穿透实现是 `WS_EX_TRANSPARENT | WS_EX_LAYERED`，`LAYERED` 与 DirectComposition 同窗叠加的行为**未验证**，是 1–2 天 spike 的核心。

---

## 1. 版本、依赖、许可、工具链

### 1.1 版本与许可（逐 crate）

| crate | 最新版本 | 发布日期（UTC） | 许可证 | edition | crates.io `rust_version` | 来源 |
|---|---|---|---|---|---|---|
| `gpui-kit` | **0.9.1** | 2026-11-09 09:50 | Apache-2.0 | 2024 | 未声明（文档要求 Rust ≥1.92） | https://crates.io/crates/gpui-kit |
| `gpui-base` | 0.9.1 | 2026-11-09 09:46 | Apache-2.0 | 2024 | 未声明 | https://crates.io/crates/gpui-base |
| `gpui-component` | 0.9.1 | 2026-11-09 09:48 | Apache-2.0 | 2024 | 未声明 | https://crates.io/crates/gpui-component |
| `gpui-kit-assets` | 0.9.1 | 随 0.9.1 线发布（时间未逐条核对） | Apache-2.0 | 2024 | 未声明 | https://crates.io/crates/gpui-kit-assets |
| `gpui-pre` | **0.3.12**（精确锁） | 2026-11-09 00:30 | Apache-2.0 | 2024 | 未声明 | https://crates.io/crates/gpui-pre |
| `gpui-pre-macros` / `gpui-pre-platform` / `gpui-pre-web` / `gpui-pre-windows` / `gpui-pre-wgpu` | 0.3.12（同批） | 2026-11-09 | Apache-2.0 | 2024 | 未声明 | https://crates.io/crates/gpui-pre-platform 、https://crates.io/crates/gpui-pre-windows |

补充事实：

| 项 | 事实 | 证据类型 | 来源 |
|---|---|---|---|
| crate 结构 | `gpui-kit` 是 facade，本体仅 **629 行** Rust；聚合 `gpui-base` + `gpui-component`（默认开）+ `gpui-kit-assets`（默认开）+ 上游 `gpui-pre` | 【源码可见】crates.io `linecounts` | https://crates.io/api/v1/crates/gpui-kit |
| 上游关系 | `gpui-pre` 是 Zed 官方 GPUI 在提交 `zed-rev=d2a4fb6ded7c470be25d1541281da3882495b65b`（zed 版本 0.2.2）的**发布快照**，非 fork；包名/清单为 crates.io 发布调整过 | 【源码可见】`gpui-pre` 包内 `Cargo.toml.orig` 的 `[package.metadata.gpui-pre]` | https://crates.io/crates/gpui-pre |
| 依赖锁 | `gpui-kit 0.9.1` 对 `gpui-pre`、`gpui-pre-macros`、`gpui-pre-platform`、`gpui-pre-web` 全部 `=0.3.12` 精确锁；`gpui-base`/`gpui-component`/`gpui-kit-assets` 为 `^0.9.1` | 【源码可见】crates.io 依赖 API | https://crates.io/api/v1/crates/gpui-kit/0.9.1/dependencies |
| 可选渲染后端 | `gpui-fast` feature（`^0.1.14`，Longbridge 自研 retained 引擎 + platform + web）默认**不开**；文档称切换后不需 `[patch]` | 【文档明说】 | https://gpui-kit.com/docs/installation |
| 文档许可 | 代码 Apache-2.0；文档 prose/插图 CC BY 4.0 | 【文档明说】 | https://gpui-kit.com/docs |
| 仓库许可探测 | GitHub API 对仓库返回 `license=NOASSERTION`（因仓库内有多个许可文件），需以各 crate 的 crates.io 字段为准 | 【文档明说】+【推断】 | https://api.github.com/repos/longbridge/gpui-kit |

### 1.2 与本仓库工具链的兼容性

| 项 | 结论 | 依据 |
|---|---|---|
| 本仓库 `rust-toolchain.toml` | `channel = "1.98.0"`，profile minimal，component clippy/rustfmt | 仓库文件 |
| gpui-kit 要求 | 文档：「Use Rust **1.92 or later** for this repository's current locked dependency graph」（Linux 侧 `oo7 0.6.0` 声明 1.92） | https://gpui-kit.com/docs/installation |
| 兼容判定 | **兼容**（1.98.0 > 1.92 下限）；edition 2024 需要 Rust ≥1.85，亦满足 | 【推断】（下限来自文档，仓库版本已知） |
| nightly 要求 | 桌面 Windows **不需要 nightly**：安装文档要求稳定版 Rust ≥1.92（WebAssembly 目标另论，本项目不涉及） | 【文档明说】https://gpui-kit.com/docs/installation |
| MSRV 风险 | crates.io 各 crate 的 `rust_version` 字段全部为 null，实际下限随依赖图漂移；升级 gpui-kit 可能隐含提高 Rust 下限 | 【源码可见】crates.io 元数据 |

### 1.3 Windows 构建与平台要求

| 项 | 事实 | 证据类型 | 来源 |
|---|---|---|---|
| 系统 | Windows 10 或更高 | 【文档明说】 | https://gpui-kit.com/docs/installation |
| 工具链 | 必须 **MSVC** target（不能 GNU）；VS2022 Build Tools/Community 的 “Desktop development with C++”（含 MSVC + Windows SDK）；**CMake 在 PATH** | 【文档明说】 | 同上 |
| 渲染/文本 | 窗口 Win32；文本 **DirectWrite**；无额外 feature 开关；不依赖 WebView | 【文档明说】 | https://gpui-kit.com/docs/installation ；https://docs.rs/gpui-pre/0.3.12/gpui/ |
| 合成 | 默认 DirectComposition：源码在非 `disable_direct_composition` 时给窗口加 `WS_EX_NOREDIRECTIONBITMAP`；存在环境变量 `GPUI_DISABLE_DIRECT_COMPOSITION=true` 用于 WebView 子窗场景 | 【源码可见】 | https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/window.rs ；https://gpui-kit.com/docs/native-extension |
| 构建期依赖（文档未写） | release 构建的 `gpui-pre-windows/build.rs` 会调用 **Windows SDK 的 `fxc.exe`** 编译 8 个 HLSL shader（可用 `GPUI_FXC_PATH` 覆盖）；debug 构建跳过。`gpui-pre` 默认 feature 含 `windows-manifest`（用 `embed-resource` 嵌入 manifest，内含 `PerMonitorV2` DPI 感知、Common-Controls v6、`asInvoker`、SegmentHeap） | 【源码可见】 | https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/build.rs ；https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui/resources/windows/gpui.manifest.xml |
| DPI | 默认 **PerMonitorV2**（manifest）；`window.scale_factor()` 随显示器变化；布局不乘 scale factor | 【源码可见】manifest + 【文档明说】`/docs/window` | https://gpui-kit.com/docs/window |
| 已知平台缺陷/边界 | ① 无 `always_on_top` API；② `set_input_region` 仅 Wayland；③ 透明/模糊在 build <17763 上静默不生效（源码早退）；④ 菜单原生弹出（NativeMenu）标注 **Platform-dependent**；⑤ 自绘窗口 = 默认"hide_title_bar"路径（WM_NCCALCSIZE 吃非客户区），所有窗口由应用自绘 chrome | 【源码可见】/【文档明说】 | 见 §2 |

### 1.4 Windows 资源 / 图标 / manifest（迁移时必须处理）

| 项 | 事实 | 证据类型 | 来源 |
|---|---|---|---|
| GPUI 自带 manifest | `gpui-pre` 默认 feature `windows-manifest` 在 Windows 目标下用 `embed-resource` 编译 `resources/windows/gpui.rc`；该 rc **只有** `1 RT_MANIFEST "gpui.manifest.xml"`（无图标、无 VERSIONINFO）；manifest 内容为 `asInvoker` + `PerMonitorV2` + Common-Controls v6 + SegmentHeap | 【源码可见】 | https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui/resources/windows/gpui.rc |
| 迁移后仍需自备资源 | exe 图标、文件版本/公司名等需应用自己写 `.rc` + `build.rs`（`embed-resource` / `winres`）；**不要同时在应用 rc 里写 `RT_MANIFEST`**，否则与 gpui-pre 嵌入的 RT_MANIFEST id 1 重复（链接期资源冲突，或行为不确定） | 【推断】（资源 id 冲突机制） | 同上 |
| 若需提权/自定义 manifest | 若要 `requireAdministrator` 或其他 `requestedExecutionLevel`，需关闭 `gpui-pre` 的 `windows-manifest` feature 并自备完整 manifest（**必须自带 PerMonitorV2**，否则 DPI 行为退化） | 【推断】 | https://gpui-kit.com/docs/installation（features）+ 上述 gpui.manifest.xml |
| 现状对照 | 本仓库 Tauri 产物由 `tauri-build`（`tauri-winres`）生成 `resource.rc`（含 `VERSIONINFO` + Tauri manifest），迁移后这部分归 Rust 应用自己负责 | 仓库侧构建产物 `src-tauri/target/**/out/resource.rc` | 本仓库 |

### 1.5 生态与维护活跃度

| 指标 | 数值 | 来源 |
|---|---|---|
| GitHub stars | **20,209**（站点显示 20.2k） | https://api.github.com/repos/longbridge/gpui-kit |
| open issues | **147** | 同上 |
| 最近 push | 2026-11-10 13:48（取证当天仍在提交，PR 编号已到 #3887） | 同上 + `/commits` |
| 发布节奏 | 周更：v0.6.6(09-21) → 0.7.0(09-28) → 0.7.1(10-05) → 0.7.3(10-12) → 0.8.0(10-19) → 0.8.1(10-26) → 0.9.0(11-02) → **0.9.1(11-09)** | https://github.com/longbridge/gpui-kit/releases |
| 版本语义 | 仍为 **0.x**，官方在 GPUI 上游文档中明言会有 breaking change；本次核实到 0.9.x 内部亦有 API 增删（见 §2） | https://docs.rs/gpui-pre/0.3.12/gpui/ （README 段） |
| 维护者 | huacnlee（Jason Lee，Longbridge），crates.io 全部 15 个版本均为同一账号发布 | https://crates.io/api/v1/crates/gpui-kit |
| 下载量 | 全期 317,737；0.9.1 单日 2,237 | 同上 |
| API 稳定性承诺 | 未发现任何稳定性承诺；文档明确 "0.x / breaking change" 与 "Validate it for your product"（Experimental 定义） | https://gpui-kit.com/docs |

---

## 2. 窗口与应用级能力（按本仓库 7 窗口硬需求核对）

### 2.1 本仓库真实窗口清单（仓库侧事实）

`src-tauri/tauri.conf.json` 定义 7 个窗口，全部 `decorations:false`：

| label | 尺寸 | transparent | alwaysOnTop | skipTaskbar | shadow | 备注 |
|---|---|---|---|---|---|---|
| `main-dps` | 670×560 | false | false | false | true | 主窗，手写标题栏，位置/置顶可配 |
| `hud-spike` | 380×238 | **true** | **true** | true | false | HUD 浮层，穿透开关 + 半透明 |
| `notification-island` | 440×76 | **true** | **true** | true | false | 通知岛，圆角 |
| `console` | 1180×760 | false | false | false | true | 无边框控制台 |
| `abyss-values` | 1100×720 | false | false | false | true | 深渊数据 |
| `character-details` | 880×620 | false | false | false | true | 战斗详情 |
| `team-details` | 980×680 | false | false | false | true | 战斗详情 |

现有实现证据（仓库侧，非外部来源）：
- 穿透：`src-tauri/src/windows/hud.rs`、`main_dps.rs` 调 `window.set_ignore_cursor_events(enabled)`；置顶：`window.set_always_on_top(...)`；透明度/亚克力/圆角/形状：`src/platform/window_style.rs` 用 `SetLayeredWindowAttributes` + `SetWindowCompositionAttribute`(Acrylic) + `DwmSetWindowAttribute`(圆角/去边框)。
- 穿透热键：`src/platform/passthrough_hotkey.rs` 已是**纯 Win32 低级键盘钩子**（`SetWindowsHookExW(WH_KEYBOARD_LL)` + 后台消息泵 + 全局热键表），与 Tauri 无关，可直接复用。

### 2.2 GPUI 的窗口能力与缺口

| 需求 | GPUI 能力 | 缺失度 | 证据/来源 |
|---|---|---|---|
| 多窗口（7 个独立窗口） | `open_window(WindowOptions, cx, builder)` 返回 `(AnyWindowHandle, Entity<V>)`；每窗口套自己的 `Root`；共享 `Entity` 用 observer 同步 | ✅ 直接可用 | 【文档明说】https://gpui-kit.com/docs/multi-window |
| 无边框窗口 | 默认即自绘：`titlebar.appears_transparent` 缺省 → `hide_title_bar=true`，`WM_NCCALCSIZE` 吃掉非客户区；也可 `WindowDecorations::{Server,Client}` 请求 | ✅ 默认行为 | 【源码可见】https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/window.rs ；https://docs.rs/gpui-pre/0.3.12/gpui/struct.WindowOptions.html |
| 自绘标题栏 + 窗口按钮 | `TitleBar` 组件：Windows 下"custom window control buttons + `WindowControlArea`"，按钮固定宽 34px，左 padding 12px；`TitleBar::window_options()` 一次配好拖拽/双击 | ✅ 直接可用 | 【文档明说】https://gpui-kit.com/component/title-bar |
| 拖拽移动 / 双击最大化 | `window.start_window_move()`、`window.titlebar_double_click()` | ✅ | 【源码可见】https://docs.rs/gpui-pre/0.3.12/gpui/struct.Window.html |
| 最小化 / 最大化 / 全屏 / 关闭 | `minimize_window()`、`zoom_window()`、`toggle_fullscreen()`、`remove_window()`、`is_maximized()`；关闭前拦截 `on_window_should_close` | ✅ | 【文档明说】`/docs/window`、`/docs/multi-window` |
| **窗口置顶（普通窗）** | **`WindowOptions` 20 个字段中没有 `always_on_top`**（字段完整清单见下）；仅 `WindowKind::PopUp` 在 Win32 层带 `WS_EX_TOPMOST`（源码），但 PopUp 同时是 `WS_EX_TOOLWINDOW`、无 caption、style 从 0 起算（不适合作为可缩放 HUD 的唯一方案） | ❌ 需自建（`SetWindowPos(HWND_TOPMOST)`） | 【源码可见】WindowOptions 字段：https://docs.rs/gpui-pre/0.3.12/gpui/struct.WindowOptions.html ；Win32 样式：https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/window.rs |
| 窗口透明 | `WindowOptions.window_background: WindowBackgroundAppearance`，枚举含 `Opaque / Transparent / Blurred / MicaBackdrop / MicaAltBackdrop`；Windows 实现：`Transparent` → `SetWindowCompositionAttribute(accent_state=2)`，`Blurred` → accent 4（Acrylic），Mica 走 `DwmSetWindowAttribute`；**build <17763 直接 return（静默失效）** | ⚠️ 可用但有版本下限 | 【源码可见】https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/window.rs |
| 窗口背景模糊/材质 | 同上：`Blurred` 对应现有 Acrylic；Mica/MicaAlt 额外可选 | ✅ | 同上 |
| 亚克力在失焦时保持 | 现有 `window_style.rs` 的注释说明 Tauri 的 `Acrylic` 会在失焦时淡出、需用 legacy accent policy；GPUI 的 `Blurred` 走同一个 `SetWindowCompositionAttribute` accent 4 路径（**推断等价**） | ⚠️ 推断，需实测 | 仓库 `src/platform/window_style.rs` + 上述源码 |
| 半透明/整体不透明度 | 仓库现用 `SetLayeredWindowAttributes`（`WS_EX_LAYERED`）；GPUI 默认 DirectComposition 窗是否可与 `LAYERED` 叠加**未验证**；替代路径是把 alpha 烘进 GPUI 颜色（更稳） | ⚠️ 需 spike | 同上；【未验证】 |
| **鼠标穿透** | **`Window::set_input_region` 文档明确：「Linux (wayland) only」**；Windows 无 API | ❌ 需自建（`WS_EX_TRANSPARENT`） | 【源码可见】https://docs.rs/gpui-pre/0.3.12/gpui/struct.Window.html （`set_input_region` 文档注释）；实现见 `crates/gpui/src/window.rs` |
| 拿到原生 HWND | `use raw_window_handle::{HasWindowHandle, RawWindowHandle}; HasWindowHandle::window_handle(window)` → `RawWindowHandle::Win32(handle.hwnd)`；文档要求"结果借用活窗口、不要缓存到窗口销毁后" | ✅ 文档化的官方路径 | 【文档明说】https://gpui-kit.com/docs/native-extension |
| 窗口位置/大小持久化 | `window.window_bounds() -> WindowBounds`（可序列化：Windowed/Maximized/Fullscreen），下次 `WindowOptions { window_bounds: Some(saved), .. }`；多屏拓扑变化需自行校验 | ✅ + 自建存储 | 【文档明说】https://gpui-kit.com/docs/multi-window |
| 单实例 | 无 API | ❌ 自建（命名 mutex，见来源清单） | 【推断】文档无此页 |
| 系统托盘 | 无 API（文档/组件库均无） | ❌ 自建（`Shell_NotifyIcon`） | 【推断】https://gpui-kit.com/component 无托盘页 |
| 全局热键 | 无 API | ❌ 自建，但**本仓库已有可复用实现**（`WH_KEYBOARD_LL`） | 仓库 `src/platform/passthrough_hotkey.rs` |
| 开机自启 | 无 API | ❌ 自建 | 【推断】 |
| 文件对话框 | `App::prompt_for_paths`、`App::prompt_for_new_path`（Windows 后端有原生 dialog 实现 `dialog.rs`，含取消定时器） | ✅ | 【源码可见】https://docs.rs/gpui-pre/0.3.12/gpui/struct.App.html + `gpui-pre-windows 0.3.12/src/dialog.rs` |
| 打开链接/文件 | `cx.open_url(url)`（交给系统默认浏览器/处理器，无返回结果） | ✅ | 【文档明说】https://gpui-kit.com/docs/getting-started （或 `/docs/native-extension` 引用 `cx.open_url`） |
| 剪贴板 | `cx.read_from_clipboard()` / `read_from_clipboard_async` / `write_to_clipboard(ClipboardItem)`；Windows 后端有 `clipboard.rs` | ✅ | 【源码可见】https://docs.rs/gpui-pre/0.3.12/gpui/struct.App.html + `gpui-pre-windows 0.3.12/src/clipboard.rs` |
| 文件拖入（drag & drop） | GPUI 有 `ExternalPaths`/`ClipboardEntry::ExternalPaths`，Windows 后端有 `destination_list.rs`（OLE drop target）；gpui-kit 文档未提供"文件拖入"教程 | ⚠️ 底层可见，API 用法未验证 | 【源码可见】+【未验证】 |
| 系统通知（OS toast） | `cx.set_app_identity(id, name)`（Windows 未打包应用必需）+ `cx.show_system_notification(...)`；`Notification::…system()` 可转发 | ✅ | 【文档明说】https://gpui-kit.com/docs/system-notification |
| 多显示器 DPI | 默认 PerMonitorV2 manifest；`scale_factor()` 随显示器变化 | ⚠️ 框架基础在，运行时切换行为**未验证** | 【源码可见】manifest +【文档明说】`/docs/window` |
| 窗口显隐动画 | 无窗口级 API；内容层用 `gpui-base` 的 transition/spring/presence（尊重减少动效） | ⚠️ 内容层可做，窗口层自建 | 【文档明说】https://gpui-kit.com/docs/animation |

**`WindowOptions` 全字段（gpui-pre 0.3.12 实测清单）**：`window_bounds`、`titlebar`、`focus`、`show`、`kind`、`is_movable`、`app_owns_titlebar_drag`、`inactive_frame_interval`、`low_power_frame_interval`、`thermal_frame_interval`、`is_resizable`、`is_minimizable`、`display_id`、`window_background`、`app_id`、`window_min_size`、`window_decorations`、`icon`、`tabbing_identifier`、`use_display_p3`。
→ **没有** `always_on_top` / `skip_taskbar` / `transparent`（透明经 `window_background`）/ `shadow` / `focusable` / `no_activate`。这些都要经 HWND 自建。
【源码可见】https://docs.rs/gpui-pre/0.3.12/gpui/struct.WindowOptions.html

### 2.3 自建 Win32 扩展层的可行性（本仓库可复用度）

| 自建项 | 所需 Win32 | 本仓库已有代码 | 复用判定 |
|---|---|---|---|
| 点击穿透开关 | `GetWindowLongPtrW/SetWindowLongPtrW(GWL_EXSTYLE)`，加/去 `WS_EX_TRANSPARENT`（tao 的做法是 `WS_EX_TRANSPARENT | WS_EX_LAYERED`） | `src/platform/window_style.rs` 已有同族调用；穿透开关现走 Tauri API | 高（改 20–40 行 + spike 验证 DComp 组合） |
| 置顶 | `SetWindowPos(hwnd, HWND_TOPMOST/HWND_NOTOPMOST, …, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)` | 状态/回滚逻辑已存在（`hud.rs::set_always_on_top`），只是调用点换成 Win32 | 高 |
| 圆角/去系统边框 | `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR)` | `window_style.rs::apply_borderless_rounding` 已实现 | 直接复用 |
| 亚克力 | `SetWindowCompositionAttribute(ACCENT_ENABLE_ACRYLIC_BLUR_BEHIND)` | `window_style.rs::apply_persistent_acrylic` 已实现 | 直接复用（或改用 GPUI `Blurred`） |
| 整体透明度 | `SetLayeredWindowAttributes` / DWM | `window_style.rs::apply_window_opacity` 已实现 | 中（DComp 下需验证，或改 GPUI 颜色 alpha） |
| 热键 | `WH_KEYBOARD_LL` 低级钩子 | `src/platform/passthrough_hotkey.rs` 完整实现 | 直接复用（换成把事件投递给 GPUI 前台任务） |
| 托盘/单实例/自启 | `Shell_NotifyIcon` / `CreateMutexW` / 注册表 Run 键 | 无 | 全部新建（约 4–5 人日） |

---

## 3. 输入与中文 IME（本项目最高危项）

### 3.1 能力链（源码可见）

| 层 | 实现 | 证据 |
|---|---|---|
| 输入抽象 | `EntityInputHandler` trait：`text_for_range` / `selected_text_range` / `marked_text_range`（IME 预编辑串）/ `replace_text_in_range` / `replace_and_mark_text_in_range` | 【源码可见】`gpui/src/input.rs`（crates.io `gpui-pre 0.3.12` 包） |
| Windows 消息 | `WM_IME_STARTCOMPOSITION` → `handle_ime_position`；`WM_IME_COMPOSITION` → `handle_ime_composition` | 【源码可见】https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/events.rs |
| 组字与上屏 | `GCS_RESULTSTR` → `replace_text_in_range(None, result)`；`GCS_COMPSTR` → `replace_and_mark_text_in_range(None, comp, caret)`（**带预编辑显示**，非只吞最终串） | 同上 |
| 候选窗定位 | `WM_IME_STARTCOMPOSITION` 时 `ImmSetCompositionWindow(CFS_POINT)` + `ImmSetCandidateWindow(CFS_CANDIDATEPOS)`，坐标由 GPUI caret 位置 × `scale_factor` 换算 | 同上 |
| IME 启停 | 按当前输入处理器 `query_accepts_text_input()` 调 `ImmAssociateContextEx`；失焦窗口不会误提交别窗组字（`GetFocus()` 校验） | 同上 |
| 应用层 | `gpui-component` 的 `Input`/`Textarea`/`Editor` 共用同一套编辑引擎（`InputState`，含撤销/选择/剪贴板/掩码） | 【文档明说】https://gpui-kit.com/component/input |

### 3.2 判定与风险

| 项 | 判定 |
|---|---|
| 微软拼音/搜狗/QQ 输入法的组字、候选窗、上屏 | 【未验证】——官方文档**没有任何 Windows IME 结论**；源码实现完整，但"实现存在"≠"各家输入法实测正常" |
| 候选窗在透明/置顶/无边框窗口内的位置 | 【未验证】——候选窗是系统窗口，定位依赖 caret 坐标与窗口 client 坐标换算；本项目 HUD 的 DPI/多屏场景必须实测 |
| 弹窗内输入（Dialog 内 Input） | 【未验证】——GPUI 的焦点路径 + IME 上下文按窗口/焦点启停，弹窗属于同一 Window 的 overlay，理论可行 |
| 与现有 WebView 输入行为差异 | 【推断】React 侧 `compositionstart/end`、Ctrl+A、双击选词等由 WebView 与浏览器文本控件提供；GPUI 是自绘文本引擎，边缘行为需要逐项对齐（掩码输入时 GPUI 明确：Copy/Cut 禁用、双击全选、Paste 保留——**已有文档化差异**） |
| 硬性结论 | 中文"能输入"证据充分（源码）；**输入法边缘行为不承诺与现状一致** → 必须在迁移前做真机 spike（列为本文件 §9 的降级项 1） |

另：GPUI 的 `Input` 还提供 `masked`/`mask_toggle`、`content_type`（Password/OneTimeCode/DateTime/…，其中 `CellularImei` 等是内容类型不是输入法）、`cleanable`、prefix/suffix、readonly/disabled、sizes、验证状态样式；数字用 `NumberInput`。【文档明说】https://gpui-kit.com/component/input 、https://gpui-kit.com/component/number-input

---

## 4. 组件矩阵（需求 → 组件/API → 缺失度）

> 官方口径 75+ 组件；组件目录页实际列出约 70 个组件页，另有 `gpui-base` 的 40+ primitives 文档页。成熟度标签：无标签=Stable；其余为 Preview / Experimental / Showcase only / Platform-dependent。【文档明说】https://gpui-kit.com/component

### 4.1 窗口与外壳

| 需求 | 组件/API | 缺失度 | 结论 |
|---|---|---|---|
| 7 独立窗口 | `open_window` + `AnyWindowHandle` + 每窗 `Root` | 无 | ✅ 直接用；自建"窗口注册表 + 显隐策略" |
| 无边框 + 自绘标题栏 + 拖动 | `TitleBar` + `TitleBar::window_options()` + `start_window_move` | 无 | ✅ |
| 最小化/最大化/关闭 | `TitleBar` 的 Windows 窗口按钮（34px, `WindowControlArea`） | 无 | ✅ |
| 置顶切换（主窗/HUD/控制台 pin） | 无 API | 缺 1 项 | ❌ Win32 自建（0.5 人日 + 复用现有状态机） |
| 透明/亚克力/圆角 | `WindowBackgroundAppearance` + `apply_borderless_rounding` | 透明有；圆角/去边框需 Win32 | ⚠️ 混合（见 §2） |
| 穿透开关 + 穿透热键 | 无 API | 缺 1 项 | ❌ Win32 + 复用现有钩子 |
| 通知岛（圆角小窗） | 同上 | — | ⚠️ 同 HUD，较轻 |

### 4.2 数值展示 / 表格 / 图表 / 自绘

| 需求 | 组件/API | 缺失度 | 说明 |
|---|---|---|---|
| DPS 实时排行（行内条、占比、名次） | 无"排行行"组件；`Progress` + `div` 布局 + GPUI 动画通道 | 中 | 自建 3–4 人日：行结构、条形/占比、数字插值、max-hp 压缩动效 |
| 只读表格（虚拟化/排序/列宽/固定列/选中/右键菜单） | **DataTable**（`TableState`+`TableDelegate`+`Column`+`ColumnSort`+`ColumnFixed`） | 无 | ✅ 行虚拟滚动、行列选择、列宽拖拽/固定/移动、排序（排序数据由 delegate 提供）、自定义单元格、键盘导航、右键、无限加载 |
| 1 万行日志 / 历史列表 | **VirtualList**（`v_virtual_list`/`h_virtual_list`，支持变高，示例 5000 项）+ `Scrollbar` | 无 | ✅；"粘底/自动滚到底"需自建 30 行逻辑 |
| 技能时间轴（色条 + 刻度 + 缩放/平移） | `canvas()` + `PathBuilder`（`paint_path`/`paint_quad`）+ 自定义 `Element` | **缺整块** | ❌ 自建 6–8 人日（命中测试/tooltip 要自己做；Chart 不适合任意标注） |
| DPS 曲线（折线/阶梯/面积） | **Chart**：`LineChart/BarChart/AreaChart/PieChart/RadarChart/CandlestickChart/SankeyChart`；x/y 闭包取字段、`y_domain`、tick 控制、reference_line、grid | 无 | ✅；高频"每帧刷新"的边界未验证（自建 2–3 人日封装） |
| 条形占比 / 饼图 | `BarChart` / `PieChart` | 无 | ✅ |
| 树 | `Tree`（`tree`/`TreeState`/`TreeItem`） | 无 | ✅ |
| 低层自绘（连线/棋盘/仪表） | `Paint` 文档：`canvas(prepaint, paint)`、`PathBuilder::fill/stroke`、`window.paint_path/paint_quad`；自定义 `Element` | 无 | ✅ |

### 4.3 输入 / 表单 / 弹窗 / 菜单

| 需求 | 组件/API | 缺失度 | 结论 |
|---|---|---|---|
| 设置搜索框 | `Input`（cleanable/prefix/suffix） | 无 | ✅ |
| 加密 ini 编辑 | `Textarea`（auto-grow/固定行）/ `Editor`（Rope、语法高亮、折叠、诊断，200K 行） | 无 | ✅（解密/加密仍在 Rust core） |
| 数字输入 | `NumberInput` | 无 | ✅ |
| 中文 IME | `Input`/`Textarea`/`Editor` 均走 `EntityInputHandler` | 无（源码） | ⚠️ 实测未验证（§3） |
| JSON Schema 动态表单（mod-studio） | `Form`/`Field` 仅布局；**值、校验、提交、联动都归应用** | **缺整块** | ❌ 必须自写渲染层（§5） |
| 下拉/可搜多选 | `Select` / `Combobox`（搜索、多选） | 无 | ✅ |
| 开关/复选/单选/滑杆 | `Switch`/`Checkbox`/`Radio`/`Slider`/`Toggle` | 无 | ✅ |
| 日期/时间 | `DatePicker`/`Calendar`/`TimeField` | 无 | ✅ |
| 弹窗 | `Dialog`/`AlertDialog`/`Sheet`/`Popover`/`HoverCard`；`window.open_dialog(...)`，每窗 `Root` 管 overlay/焦点陷阱 | 无 | ✅（现有 48 处 `<Dialog>` 需逐个映射） |
| Toast | `window.push_notification(Notification::success/info/warning/error, cx)`；`.system()` 走 Windows 通知中心 | 无 | ✅ |
| 菜单/右键菜单 | `Menu`（菜单+上下文菜单+下拉）、`DropdownButton`、`Command`（命令面板） | 无 | ✅ |
| 超出小窗边界的原生菜单 | `NativeMenu`（Windows：`TrackPopupMenuEx` + HWND） | 标注 **Platform-dependent** | ⚠️ 可用但成熟度低；多数场景用 GPUI PopupMenu 更稳 |
| 拖拽排序（HUD 模块顺序） | `List` + GPUI 原生 `on_drag`/`drag_over`/`on_drop`；官方文档给出完整"Drag and Drop Reordering"示例（含拖影实体、`move_item`） | 无现成 sortable 组件，但 API 完整 | ✅ 自建封装 1 人日 |
| 分栏/可停靠 | `Resizable`（分栏拖宽）、`Dock`（可拖 tab/嵌套分割/布局 serde 持久化）、`Sidebar`、`Tabs` | 无 | ✅（Dock 可不用，7 窗结构更贴近现状） |
| 长文本/更新日志 Markdown | `TextView::markdown` / `markdown()` 帮助函数：可选文本、代码块、表格、`max_lines` 截断、行内 HTML 透传；`TextView::html` 支持标题/段落/链接/图片/列表/表格，但**无通用 CSS/脚本** | 部分 | ✅ Markdown；HTML 是子集（§9 降级项） |
| 代码片段/Diff | `Editor`（tree-sitter 可选）、`Diff`（只读 patch 展示） | 无 | ✅ |

### 4.4 主题 / 字体 / 图标 / i18n / 无障碍 / 动效

| 需求 | 能力 | 缺失度 | 说明 |
|---|---|---|---|
| 主题 JSON token | `Theme`/`ThemeRegistry`/`ActiveTheme`；主题文件 `{colors:{...}}`，支持 `#RRGGBB` 与 `linear-gradient(...)`；`ThemeRegistry::watch_dir` 热加载 | 无 | ✅ 但 token 集合固定 |
| token 映射到现有语义色 | 每个主题 **38 个 token**（实测 Ayu：`background/foreground/border/ring/muted.*/primary.*/secondary.*/accent.*/popover.*/panel.*/list.*/tab.*/title_bar.*/scrollbar.*/base.{red,green,blue,yellow,magenta,cyan}`）；**没有 success/warning/info/destructive** | 缺 4 个语义名 | ⚠️ 自建薄映射层：`success→base.green`、`warning→base.yellow`、`info→base.blue`、`destructive→base.red`（3 人日含字号/边框规范） |
| 38 套内置主题 | 仓库 `themes/*.json` 21 个文件，官方称 38 套（36 变体 + Default Light/Dark） | 无 | ✅ 可挑一套做基底改 token |
| 暗色/系统跟随 | `ThemeRegistry` + 系统外观监听（Windows 后端监听 `ImmersiveColorSet`） | 无 | ✅ |
| 字号 12/14/20/24 | GPUI 用 `px()` 任意值；**主题默认 UI 字号 16px、mono 13px**（与 AGENTS.md 冲突） | 需约定 | ⚠️ 必须在主题/封装层覆盖为 12/14/20/24，禁止用组件默认 `text_sm` 等不经审的字号 |
| 边框 0.5px | `border_*` 接受 `px(0.5)` | 需实测 hairline 渲染 | ⚠️【未验证】 |
| 自定义字体 | 系统字体按名解析（DirectWrite）；`cx.text_system().add_fonts(include_bytes!(...))` 需在开窗前注册；缺字形走 fallback | 无 | ✅；中文字体建议显式 `Microsoft YaHei UI` 并实测 CJK/emoji 混排 |
| 图标（替代 lucide-react） | `IconName`：`IconName::ALL` 枚举 **1,830 个**名字（Lucide + Isocons），`Icon::new(IconName::X)`；默认 assets feature 自带 | 无 | ✅ 覆盖现有图标；自定义 SVG 走 `AssetSource` |
| i18n | 组件字符串自带 en/zh-CN/zh-HK/zh-TW/it（`crates/component/locales/ui.yml`）；应用侧接 **rust-i18n 4.2**，`rust_i18n::extend!(gpui_component)`，`set_locale("zh-CN")` + `cx.notify()`/`refresh_windows()` | 无框架缺失 | ✅；本项目 **2,468 个键**（`res/languages/zh-CN.json`，171 KB；`ja.json` 215 KB）需迁移（2–3 人日） |
| 无障碍 | AccessKit 0.24；`.aria_label()`/`.role(Role::…)`/`.test_support()`；Windows 后端依赖 `accesskit`+`accesskit_windows`（UIA）；官方要求"逐平台实测" | 基础有；自绘区无语义 | ⚠️ 满足 AGENTS.md "图标按钮 tooltip/aria-label" 可行；读屏整体效果【未验证】 |
| 减少动态效果 | `cx.reduce_motion()`/`cx.set_reduce_motion()`；`gpui_kit::init` 在 **macOS 与 Windows** 读取系统偏好；motion 组件在 reduced 下吸附终态、停止请求帧 | 无 | ✅ 满足 AGENTS.md |
| 动效体系 | `AnimationExt`（`with_animation`）+ `gpui-base` motion：transition/spring/keyframes/presence/measured reveal，CSS 对齐缓动；主题里有 `motion_tokens`（如 `spring_move` 280ms） | 无 CSS 过渡 | ✅ 但需把 `lib/motion.ts` 的刻度映射到 motion_tokens |
| 密度 | 无全局密度开关；组件有 `large/medium/small/xsmall` 尺寸变体 | 无全局 | ⚠️ 用尺寸变体 + 间距常量近似 |

---

## 5. JSON Schema 动态表单（mod-studio）自建代价

### 5.1 现状（仓库侧）

| 项 | 事实 |
|---|---|
| 依赖 | `@jsonforms/core` **3.8.0**、`@jsonforms/react` **3.8.0**（`frontend/package.json`） |
| 实现 | `frontend/src/features/mod-studio/plugin-form-renderers.tsx`：自定义 `LayoutRenderer`/`ButtonRenderer`/`ControlRenderer`，用 `JsonFormsDispatch` 递归，`withJsonFormsLayoutProps`/`withJsonFormsControlProps`，校验状态取自 `jsonforms.core.errors` |
| 语义用量 | `JsonForms` 相关符号在 frontend 出现 14 处；插件 schema 由 Mod 插件提供（字段/校验/布局） |

### 5.2 gpui-kit 能提供什么

- `Form`/`Field`/`v_form`/`h_form`：**只有布局**（单列/两列、label 位置、col_span、footer）；文档原文：「The application owns values, validation, submission, and responsive column choices」。【文档明说】https://gpui-kit.com/component/form
- 控件齐全：`Input`/`NumberInput`/`Switch`/`Checkbox`/`Radio`/`Slider`/`Select`/`Combobox`/`Textarea`/`ColorPicker`/`DatePicker`/`TimeField`。
- 没有：JSON Schema 解析、`uiSchema`（`VerticalLayout`/`HorizontalLayout`/`Group`/`Categorization`）、`rule`/条件可见性、`required`/`minLength`/`pattern`/`format` 校验、数组项增删排序、错误消息汇总。

### 5.3 组成方案（推荐 A）

| 方案 | 做法 | 估工 | 风险 |
|---|---|---|---|
| **A. Rust 原生渲染层（推荐）** | ①`serde_json` 解析插件 JSON Schema → 内部 `FormModel`（字段树 + 类型 + 默认值）；②控件映射表（string→Input、number/integer→NumberInput、boolean→Switch、enum→Select、array→List+增删、object→嵌套 Group）；③`uiSchema` 子集（VerticalLayout/HorizontalLayout/Group/Control/options.colspan）；④校验器（required/type/enum/min/max/minLength/pattern/format，错误挂在 `Field` 下）；⑤联动（`rule.effect=HIDE/DISABLE/SHOW`，按当前值重算可见性）；⑥错误汇总 + 保存禁用；⑦i18n 接入 | **10–14 人日** | JSONForms 的 `rule`/数组/`oneOf` 语义复杂，插件若用到高级特性需降级或逐个补 |
| B. 内嵌 WebView 跑 React JSONForms | `gpui-webview`（Wry）嵌一个本地页面 | 4–6 人日 | 官方标注 **experimental**；「GPUI popovers/dialogs/tooltips cannot reliably cover it」；Windows 需 `GPUI_DISABLE_DIRECT_COMPOSITION=true`；等于保留一整套前端栈，**不推荐** |
| C. 降级为只读/固定字段 | 只渲染已知字段的子集 | 3–5 人日 | 插件生态能力缩水（要提前认账） |

---

## 6. 主题、字体、i18n、无障碍、动效的落地差异清单

| 现有约定（AGENTS.md / frontend-guidelines.md） | gpui-kit 侧的落法 | 差异/工作量 |
|---|---|---|
| 语义色 token（`text-success`/`warning`/`info`/`destructive`/`muted-foreground`） | 主题只有 38 个固定 token，需自建 `AppTheme` 扩展：把 4 个语义色映射到 `base.*`，或按主题文件新增自定义 token 名（ThemeRegistry 的 `colors` 键可自定义则更省事，**该点未验证**） | 1–2 人日 |
| 字号只用 12/14/20/24px | `Theme::update` 里设 `font_size=14`，封装 `text_12/text_14/text_20/text_24` 帮助函数；禁止直接用组件默认 16px 档 | 0.5 人日 |
| 边框 0.5px（`--ui-border-width`） | `px(0.5)` 直写；封装 `hairline()` | 0.5 人日；hairline 渲染需目测 |
| 减少动态效果 | `cx.reduce_motion()` + motion_tokens；默认遵守 | 0（框架已做） |
| 图标按钮必须 tooltip/aria-label | `Tooltip` + `.aria_label()` | 0（约定照搬） |
| i18n：英文 key，zh-CN 在 `res/languages/zh-CN.json` | rust-i18n 支持多格式（`locales/ui.yml` 示例是 YAML；JSON 后端支持情况**未验证**）；英文整句作 key 与 rust-i18n 的 key 规则可能冲突（点号层级），迁移时建议把 JSON 转成扁平 key 表 + 自建 `t()` | 2–3 人日（2,468 键 + 运行时切换 + 组件 locale） |
| 拖拽排序只用 dnd-kit | 换 `List` 的 `on_drag/on_drop`（官方示例） | 1 人日 |
| 弹窗统一 `ui/dialog` | 换 `window.open_dialog` + `Dialog`/`Sheet`/`AlertDialog`；48 处调用点需逐个映射 | 3–4 人日 |
| 虚拟化列表 | `VirtualList`/`DataTable` | 1–2 人日 |
| Markdown 更新日志 | `TextView::markdown`（含 `max_lines` 折叠） | 0.5 人日 |

---

## 7. 与 Tauri 并存的判定（第 ④ 问）

| 维度 | 事实 | 判定 |
|---|---|---|
| 主线程要求 | GPUI：`App::new` 内部 `assert(app.rs, "must construct App on main thread")`；`Application::run` 阻塞主线程消息泵 | 【源码可见】https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui/src/app.rs |
| Tauri 侧 | Tauri 2 默认要求主线程，但提供 `Builder::any_thread()`（**仅 Linux/Windows**）绕过主线程要求 | 【文档明说】https://docs.rs/tauri/latest/tauri/struct.Builder.html#method.any_thread |
| 线程层面 | 因此 Windows 上"GPUI 主线程 + Tauri 副线程"理论可行 | 【推断】不完全排除 |
| 进程级单例 | DPI 感知上下文（manifest 声明 PerMonitorV2，进程只能有一套）、COM 套间/WebView2 环境、窗口类注册、托盘、输入法上下文、主题监听 | 只能一个框架拥有；两套并存必然争用 |
| 构建产物 | 一个 exe 只能嵌一个 `RT_MANIFEST`：`gpui-pre` 默认 feature `windows-manifest` 用 `embed-resource` 嵌 `gpui.manifest.xml`；`tauri-build` 依赖 `tauri-winres` 嵌 Tauri 的 manifest → **同 exe 冲突/互相覆盖**（需禁用一方，禁用 GPUI 侧则失去 PerMonitorV2/Common-Controls） | 【推断】（双方 build 依赖已核实） |
| 收益 | 本项目 Rust 核心（85k 行）+`src-tauri` 薄 adapter 的存在意义就是给 React 提供 IPC；改用 GPUI 后 UI 直接调 Rust 函数，IPC/DTO/channel 层可整层删除 | 保留 Tauri 无收益 |

**结论**：**必须移除 Tauri 运行时**（不是"技术上 100% 不可能"，而是"不可安全共存 + 无收益 + 构建期冲突"）。过渡只能按**进程**分阶段：
1. 阶段 A：新增独立 GPUI exe（先做 HUD 浮层或控制台），与现有 Tauri exe 并存于同一安装包，两进程共用一个 Rust 核心库（`nte-core`）；
2. 阶段 B：窗口逐个切换，主窗最后切；
3. 阶段 C：删除 `src-tauri/` 与 `frontend/`。
注意：若采用阶段 A，`nte-core --no-default-features --features cli` 必须继续保持不依赖 Tauri/窗口库（AGENTS.md §2 已约束），这也是本路线的可行性前提。

---

## 8. 与替代方案对比（1 段）

继续 Tauri（现状）在中文 IME、无障碍、生态、团队熟悉度上最强，但透明浮层/穿透/置顶/无边框是靠 Tauri 的 `set_ignore_cursor_events`/`set_always_on_top` + Win32 才凑出来的，且 HUD 与主窗共用 WebView2 的渲染与内存开销；egui/eframe 被 AGENTS.md 明确禁止恢复 `gui` feature，且自绘表格/中文/透明浮层要大量自建；iced 的透明/穿透/性能边界同样要自建，组件与表格虚拟化远弱于 gpui-kit；Slint 组件最少、GPL/商业双许可、Windows 透明与中文输入需自行验证。gpui-kit 是唯一一个把「DataTable 行列虚拟化 + VirtualList + Chart + TextView Markdown + 主题 + i18n + AccessKit + 拖拽 + 无边框标题栏」放在同一 Rust 栈里的选项，代价是 0.x 周更、Windows IME/透明组合未实测、以及本文列出的四个硬缺口。

| 维度 | gpui-kit | 继续 Tauri | egui/eframe | iced | Slint |
|---|---|---|---|---|---|
| 透明+置顶+穿透 | 透明/材质有，置顶/穿透自建（HWND） | 已有实现（tauri API + Win32） | 可自建（winapi） | 弱/自建 | 待验证 |
| 中文 IME | 源码支持，未实测 | ✅ 成熟 | 需自配字体，IME 弱 | 一般 | 待验证 |
| 表格/虚拟化/图表 | ✅ 组件齐全 | ✅ Web 生态 | 弱（egui_extras/plot） | 弱（自建） | 中 |
| 许可 | Apache-2.0 | MIT/Apache-2.0 | MIT/Apache-2.0 | MIT | GPLv3/商业 |
| 迁移成本 | 高（Rust 重写 UI） | 0 | 高 | 高 | 高 |

---

## 9. 结论

### ① 可行判定：**有条件可行**

组件层足以承载本项目的绝大多数界面；四个硬缺口都有明确的技术路径（前三个是 Win32/自绘，第四个是自写渲染层）。**前置条件**（未通过则不应全面启动）：

| 前置 spike（建议 3 天，真机） | 通过标准 |
|---|---|
| S1 透明 HUD + `WS_EX_TRANSPARENT` 穿透 + `HWND_TOPMOST` 置顶 | 游戏全屏/无边框窗口化下：透明确认（非黑底）、鼠标点击穿透到游戏、Alt-Tab 后仍置顶、穿透热键切换生效；DComp 与 LAYERED 选型明确 |
| S2 中文 IME | 微软拼音 + 搜狗：组字、候选窗位置（含多屏/缩放）、上屏、退格/ESC、Dialog 内输入正常 |
| S3 亚克力/圆角/透明度 | 失焦不闪、拖动不黑边、`SetLayeredWindowAttributes` 与 DComp 组合行为明确 |
| S4 组件冒烟 | DataTable 1 万行、VirtualList 1 万行、Chart 高频刷新、TitleBar 拖动/双击、`List` 拖拽排序 |

### ② 必须自建的能力清单与估工（人日，推断）

| # | 自建项 | 具体内容 | 估工 |
|---|---|---|---|
| 1 | **透明/置顶/穿透 HUD 宿主层** | HWND 获取封装、`WS_EX_TRANSPARENT` 开关、`HWND_TOPMOST`、DWM 圆角/边框、Acrylic 复用、穿透热键接线、HUD 位置/尺寸/透明度持久化 | 5–7 |
| 2 | **JSON Schema 动态表单渲染层** | schema→模型、控件映射、uiSchema 布局子集、校验、rule 联动、数组增删、错误汇总 | 10–14 |
| 3 | **技能时间轴自绘** | canvas/PathBuilder、刻度、色条、缩放/平移、命中测试 + tooltip | 6–8 |
| 4 | **主题/字号/边框规范层** | 4 个语义色映射、12/14/20/24px 封装、0.5px hairline、组件尺寸覆盖、38 套主题择一改 token | 3 |
| 5 | **i18n 迁移层** | 2,468 键迁移、运行时切换、组件 locale 联动、格式化 | 2–3 |
| 6 | 窗口编排与系统集成 | 7 窗注册表/显隐策略、单实例、托盘、开机自启 | 4–5 |
| 7 | DPS 排行/占比/数值动效 | 行结构、条形/占比、max-hp 压缩动效、数字插值 | 3–4 |
| 8 | 日志/历史虚拟化落地 | 列定义、行高、粘底、滚动条 | 1–2 |
| 9 | 弹窗迁移（48 处） | `open_dialog`/Sheet/AlertDialog 统一封装 + 逐点替换 | 3–4 |
| 10 | 图表封装（DPS 曲线） | Chart 包装 + 实时刷新验证 | 2–3 |
| 11 | 标题栏/窗口按钮 | `TitleBar` 接入 + pin/其他自定义按钮 | 1–2 |
| 12 | 拖拽排序封装 | `on_drag/on_drop` + 拖影 + 顺序持久化 | 1 |
| 13 | Markdown 更新日志 | `TextView::markdown` 封装 | 0.5 |
| 14 | Windows exe 资源 | 自备 `.rc`（图标 + VERSIONINFO）+ `build.rs`；与 gpui-pre 的 `RT_MANIFEST` 共存/关闭策略 | 0.5–1 |
| — | **合计** | — | **42–57（中位 ≈ 49）** |

> 另需单独估算（不在上表）：**16 个 feature 页面本身的 Rust 重写**。现状 frontend 400 个 ts/tsx、29,443 行（`features/` 16 个目录，含 main-dps/hud/console/timeline/history/mod-studio/settings/…）。按 150–250 行/人日折算约 **60–120 人日**（推断），且不含设计还原与联调。这个量级说明：**gpui-kit 的组件能力不是瓶颈，页面重写才是**。

### ③ 需提前向用户认账的功能降级（3 项）

| # | 降级项 | 现状 | 迁移后承诺 | 触发条件 |
|---|---|---|---|---|
| 1 | **中文 IME 边缘行为** | WebView 由 Chromium 提供完整组字/候选/快捷键 | 中文可正常输入（组字+候选窗+上屏，源码可见支持）；**不承诺**与各输入法（搜狗/QQ/小鹤/双拼）的边缘行为、候选窗在多屏/高缩放/弹窗内的位置完全一致 | S2 spike 未覆盖的输入法/场景；根因在 GPUI 层，修不了只能等上游或用 Win32 原生编辑子窗（代价≈成倍增加） |
| 2 | **无障碍** | DOM 天然有可访问树；现有 `aria-label`/Tooltip 可用 | 标准控件（按钮/输入/表格/树/菜单）有 AccessKit 语义 + 键盘 Tab；**自绘区域（技能时间轴、DPS 曲线、排行条）只有摘要文本，无逐点语义**；Windows UIA 读屏效果未实测 | 自绘即失去自动语义；补齐需为图表写 AccessKit 节点（额外 2–3 人日/图） |
| 3 | **富文本与动效体系** | React+Tailwind：CSS 过渡、任意 HTML/Markdown 库、复杂布局 | Markdown 子集 + HTML 子集（无通用 CSS/脚本）；动效改用 GPUI 动画通道（可尊重"减少动态效果"），**复杂 CSS 动效（渐变遮罩、复杂 transition 组合）会简化**；`max-hp` 压缩动效需重写（允许保留但形态可能变化） | `TextView::html` 明确不支持通用 CSS 布局/脚本（文档明说） |

补充提醒（不算降级，但需用户知情）：**透明/置顶/穿透组合、多显示器 DPI 切换、高刷/GPU 兼容（集显、远程桌面、录屏/覆盖层）、Windows 10 <1809 透明失效** 都属"未验证"，需在真实机器上验收。

### ④ Tauri 结论

**必须彻底移除 Tauri 运行时**（理由见 §7：进程级单例 + 单 manifest 构建冲突 + 零收益）。允许的渐进方式是**两个进程/两个 exe 并存**，不是"同一进程内 Tauri 与 GPUI 共存"。迁移期不要引入 `gpui-webview`（官方 Experimental，且覆盖层不可靠）来"保留一点 React"。

---

## 10. 明确"未验证"清单（交付/验收时不得当作已知事实）

1. **Windows 中文 IME 实测**（微软拼音/搜狗/QQ 拼音、候选窗位置、多屏高缩放、Dialog 内输入）。
2. **透明 + DirectComposition + `WS_EX_TRANSPARENT`/`WS_EX_LAYERED` 组合**的真实表现（是否会黑底、穿透是否稳定、是否影响亚克力）。
3. **`SetLayeredWindowAttributes` 整体透明度**在 GPUI DComp 窗口上是否可用（替代：GPUI 颜色 alpha）。
4. **多显示器 DPI 运行时切换**、混合缩放、120Hz 实际表现。
5. **GPU 环境**：GPU 后端具体图形 API 文档未声明（源码可见 Windows 侧走 DirectX/DirectComposition 相关实现），集显/远程桌面/无独显的最低要求未知。
6. **AccessKit/UIA 在 Windows 的读屏实测**。
7. **文件拖入窗口**的公开 API 用法（只看到 `ExternalPaths`/`destination_list.rs` 源码）。
8. **DataTable 多列排序/分组/复杂表头**（文档未提，按不支持处理）。
9. **i18n 是否支持直接加载 JSON 语言包**（文档示例为 YAML；`res/languages/zh-CN.json` 需转换或自建加载器）。
10. **`gpui-fast` 可选后端**（0.1.14）在 Windows 的成熟度。
11. **release 构建在缺 `fxc.exe` 时的具体报错形态**（源码可见会找 SDK，失败路径未读完）。
12. **`TitleBar` 在 Windows 上自定义按钮（pin 等）的插入位置与 `WindowControlArea` 交互细节**。
13. **二进制体积/内存/启动时间**：官方对比表口径 Hello World ~12 MB，不是本项目实测。
14. **第三方依赖体量**：`gpui-pre` 依赖树含 `accesskit/windows 0.62/directx/lyon/taffy/rustybuzz/…`，首次编译时间未测。

---

## 11. 来源清单（2026-11-10 读取）

**crates.io / docs.rs**
- https://crates.io/crates/gpui-kit （0.9.1、Apache-2.0、317,737 下载、15 版本）
- https://crates.io/api/v1/crates/gpui-kit/0.9.1/dependencies （`gpui-pre =0.3.12` 等）
- https://crates.io/crates/gpui-base ｜ https://crates.io/crates/gpui-component ｜ https://crates.io/crates/gpui-kit-assets ｜ https://crates.io/crates/gpui-pre ｜ https://crates.io/crates/gpui-pre-windows
- https://docs.rs/gpui-pre/0.3.12/gpui/ （crate 文档：Win32/DirectWrite、AccessKit 0.24、方法索引）
- https://docs.rs/gpui-pre/0.3.12/gpui/struct.WindowOptions.html （20 字段，无 always_on_top）
- https://docs.rs/gpui-pre/0.3.12/gpui/struct.Window.html （`set_input_region` 明确 Wayland only 等）

**gpui-kit 官方文档**
- **文档全文语料**（本次逐小节检索用）：https://gpui-kit.com/llms-full.txt （4.7 MB，含全部 `/docs`、`/component`、`/base` 页面正文）
- https://gpui-kit.com/docs （分层、75+ 组件、成熟度定义、许可证、特性）
- https://gpui-kit.com/docs/installation （Windows 10+/MSVC/CMake/Rust 1.92/`gpui-pre` 说明/开发 profile）
- https://gpui-kit.com/docs/window （Window 方法、几何/缩放、关闭拦截、spawn/defer）
- https://gpui-kit.com/docs/multi-window （多窗口、`AnyWindowHandle`、窗口位置恢复）
- https://gpui-kit.com/docs/native-extension （`HasWindowHandle` → HWND、NativeMenu TrackPopupMenuEx、gpui-webview 限制、`GPUI_DISABLE_DIRECT_COMPOSITION`）
- https://gpui-kit.com/docs/accessibility （AccessKit、aria_label/role、逐平台实测要求）
- https://gpui-kit.com/docs/fonts （系统字体/DirectWrite、add_fonts、CJK 需实测、默认 16/13px）
- https://gpui-kit.com/docs/animation （reduced motion：`cx.reduce_motion`、Windows 系统偏好）
- https://gpui-kit.com/docs/i18n （rust-i18n 4.2、`extend!`、`set_locale`、组件 zh-CN）
- https://gpui-kit.com/docs/system-notification （`set_app_identity`、Windows 投递条件）
- https://gpui-kit.com/docs/packaging （Windows Inno Setup、`cargo build --release --locked`）
- https://gpui-kit.com/docs/comparison （与 Iced/egui/Qt/Slint 对比、DataTable 行列虚拟化、38 主题、~12 MB）
- https://gpui-kit.com/component （组件总目录）
- 组件页：https://gpui-kit.com/component/data-table ｜ /virtual-list ｜ /chart ｜ /text-view ｜ /input ｜ /form ｜ /title-bar ｜ /list ｜ /theme ｜ /notification ｜ /dialog ｜ /icon ｜ /tree ｜ /table ｜ /resizable ｜ /combobox ｜ /number-input ｜ /textarea ｜ /editor ｜ /menu

**GitHub（含锁定提交 d2a4fb6 的源码证据）**
- 仓库/活跃度：https://github.com/longbridge/gpui-kit ｜ https://api.github.com/repos/longbridge/gpui-kit ｜ https://github.com/longbridge/gpui-kit/releases
- Windows 平台实现：`https://github.com/zed-industries/zed/blob/d2a4fb6ded7c470be25d1541281da3882495b65b/crates/gpui_windows/src/events.rs`（IME/`WM_NCCALCSIZE`/命中测试）
- 窗口透明/材质/样式/HWND：`.../crates/gpui_windows/src/window.rs`
- manifest（PerMonitorV2）：`.../crates/gpui/resources/windows/gpui.manifest.xml`
- shader 构建（fxc）：`.../crates/gpui_windows/build.rs`
- 主线程断言：`.../crates/gpui/src/app.rs`
- `set_input_region`：`.../crates/gpui/src/window.rs`；`WindowKind`：`.../crates/gpui/src/platform.rs`
- 主题样例（38 token）：https://raw.githubusercontent.com/longbridge/gpui-kit/main/themes/ayu.json

**Tauri / tao（用于判定）**
- `tauri::Builder::any_thread`（仅 Linux/Windows）：https://docs.rs/tauri/latest/tauri/struct.Builder.html#method.any_thread
- tao 穿透实现（`IGNORE_CURSOR_EVENT → WS_EX_TRANSPARENT | WS_EX_LAYERED`）：https://github.com/tauri-apps/tao/blob/dev/src/platform_impl/windows/window_state.rs
- `tauri-build` 依赖 `tauri-winres`（嵌入 Windows 资源/manifest）：https://crates.io/crates/tauri-build

**Windows 系统语义（自建参考）**
- 分层窗口/点击穿透：https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles
- `SetWindowCompositionAttribute`（非公开 API，GPUI/tao 均在用）：https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ （DWM 相关接口）
- 单实例 mutex：https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-createmutexw
- 托盘：https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyiconw
