# 10 前端现状测绘：frontend/src 逐文件判定（保留 / 迁移 / 重写 / 删除）

> 目的：为「用 gpui-kit 重构界面」方案提供前端侧基线，回答「31k 行里哪些是可下沉 Rust 的纯展示逻辑、哪些是必须抽成 GPUI Entity 的 UI-only 状态、哪些是纯视图重写、哪些直接删」。
>
> 范围：仓库 `D:\NTE_DPS_TOOL`，`frontend/src`（含 `src/index.css`）全部 405 个文件；规范来源 `AGENTS.md`、`frontend-guidelines.md`、`frontend/README.md`。
> 取数时间：2026-11-10，工作区 `git status` 仅有未跟踪的 `dsh-acl-recovery/`（与本任务无关）。全部数字为本机实测，§附录 A 给出逐条复现命令。
>
> 分类口径：**每个文件只归一个主类**，因此 §4.1 的合计是精确且可复现的；页面内嵌逻辑另在 §4.6 给出「二次拆分估算」（明确标注为估算，不计入 4.1 合计）。

---

## 0. 一句话结论

`frontend/src` 共 **405 文件 / 31,232 行**。按文件主类精确切分：**A 可下沉 Rust 的展示逻辑 2,518 行（8.1%）、B UI-only 状态 2,035 行（6.5%）、C 纯视图重写 18,061 行（57.8%）、D 删除 8,618 行（27.6%，其中测试 3,536 行、ts-rs 生成物 643 行、core 已有等价的展示逻辑约 0.4k 行）**。

**关键修正（依据核心侧实测，见 §4.2 首段）**：核心 `src/` 已有 100% UI 无关的纯投影层（`core/hud.rs`、`timeline.rs`、`skills.rs`、`packets.rs`、`character_data.rs`、`combat_details.rs`、`diagnostics.rs`、`empty_curtain.rs`、`mod_market.rs` 等），所以前端 A 类里**约 380–420 行只是「core 已有等价实现的前端镜像」，应直接删而不是移植**；真正需要新写的 Rust 展示逻辑约 2.1k 行。把最大的 15 个页面里内嵌的处理逻辑再拆出来，估算为 **A ≈ 2.6k 新写 + 0.4k 直接删 / B ≈ 3.8k / C ≈ 15.8k / D ≈ 8.6k**。

结论是：**前端几乎不存在「原封不动」的文件**（框架整体更换），重写主战场是 **44 个视图文件 18k 行**，其中 `empty-curtain`、`timeline`、`settings` 三个 feature 的光栅自绘、交互密度与表单规模最容易被低估；而「格式化/映射/文案键/配色」类逻辑要先查 core 是否已实现，避免第二次造轮子。

---

## 1. 规模表

### 1.1 顶层目录规模

| 目录/文件 | 文件数 | 行数 | 说明 |
|---|---:|---:|---|
| `frontend/src/features` | 108 | 25,761 | 16 个 feature；其中 31 个测试文件 3,029 行 |
| `frontend/src/components/ui` | 16 | 1,114 | shadcn/base-ui 适配层 15 个 + `markdown.test.tsx` 43 行 |
| `frontend/src/components/nte` | 10 | 642 | 项目自有共享组件 9 个 + `ui-foundation.test.tsx` 71 行 |
| `frontend/src/lib` | 257 | 2,234 | 手写 48 文件 1,591 行（含 10 个测试 393 行）；`lib/tauri/generated` 209 文件 643 行 |
| `frontend/src/lib/tauri/generated` | 209 | 643 | ts-rs 生成 DTO，**禁止手改**；平均 3.1 行/文件 |
| `frontend/src/hooks` | 3 | 55 | `use-character-avatar` / `use-close-on-window-blur` / `use-hud-module-sortable` |
| `frontend/src/entries` | 7 | 66 | 6 个窗口入口 + `window-bootstrap.tsx` |
| `frontend/src/styles` | 3 | 482 | `theme.css` 214 / `ui-foundation.css` 202 / `console.css` 66 |
| `frontend/src/index.css` | 1 | 878 | Tailwind 入口、`@theme inline`、28 个 keyframes、全局 base/components |
| **合计（ts+tsx+css）** | **405** | **31,232** | 321 `.ts` + 79 `.tsx` + 5 `.css` |

测试：`*.test.ts/tsx` 共 **43 文件 / 3,536 行**（features 31/3,029、components 2/114、lib 10/393）。
生成物：`lib/tauri/generated` **209 文件 / 643 行**，占文件数 51.6%，却只占行数 2.1%。

### 1.2 逐 feature 规模（含测试，按目录实测）

| feature | 文件 | 行数 | 测试文件 | 测试行 | 主窗口 |
|---|---:|---:|---:|---:|---|
| `main-dps` | 18 | 4,733 | 5 | 500 | main-dps / character-details / team-details |
| `empty-curtain` | 8 | 2,794 | 3 | 318 | console |
| `mod-studio` | 18 | 2,674 | 5 | 640 | console |
| `settings` | 6 | 2,175 | 2 | 167 | console |
| `technical-hud` | 7 | 2,100 | 2 | 287 | hud-spike |
| `timeline` | 6 | 1,696 | 1 | 101 | console |
| `console` | 9 | 1,671 | 3 | 198 | console |
| `history` | 4 | 1,478 | 1 | 66 | console |
| `abyss-values` | 3 | 1,218 | 1 | 98 | abyss-values |
| `character-data` | 7 | 1,497 | 2 | 272 | console |
| `skills` | 6 | 990 | 2 | 130 | console |
| `diagnostics` | 4 | 894 | 1 | 64 | console |
| `encrypted-ini` | 4 | 653 | 1 | 34 | console |
| `packets` | 4 | 598 | 1 | 107 | console |
| `shortcuts` | 3 | 404 | 1 | 47 | console |
| `island` | 1 | 186 | 0 | 0 | notification-island |
| **features 合计** | **108** | **25,761** | **31** | **3,029** | |

> 说明：`character-data` 目录含 `character-build.css` 429 行（页面专属样式），`empty-curtain` 不含 CSS 但含 680 行 canvas 自绘。

### 1.3 非测试、非生成文件规模（分类基数）

| 组 | 文件 | 行数 |
|---|---:|---:|
| features（去 31 测试） | 77 | 22,732 |
| components（去 2 测试） | 24 | 1,642 |
| lib 手写（去 10 测试、去 209 生成） | 38 | 1,198 |
| hooks | 3 | 55 |
| entries | 7 | 66 |
| styles + index.css | 4 | 1,360 |
| **合计** | **153** | **27,053** |
| 另：ts-rs 生成 DTO | 209 | 643 |
| 另：测试 | 43 | 3,536 |
| **总计** | **405** | **31,232** |

---

## 2. 入口与窗口映射

### 2.1 Vite 多页输入（`frontend/vite.config.ts` L16-24）

```ts
input: {
  "main-dps":        "main-dps.html",
  console:           "console.html",
  hud:               "hud.html",
  notification:      "notification.html",
  "combat-details":  "combat-details.html",
  "abyss-values":    "abyss-values.html",
}
```

**6 个 HTML 入口 → 7 个 Tauri 窗口**（`character-details` 与 `team-details` 共用 `combat-details.html`）。`ui-preview.html` 不在构建输入里（dev 预览，见 `frontend/README.md` L92）。

### 2.2 窗口 → HTML → 入口 → feature 完整映射

`frontend/*.html` 只做 `#root` + `<script type="module" src="/src/entries/*.tsx">`；每个 entry 调 `renderWindow(<Page/>, { windowRoute })`。

| 窗口 label（tauri.conf.json） | url | entry | `data-window-route` | 渲染 feature 目录 | 尺寸 | 窗口标志 |
|---|---|---|---|---|---|---|
| `main-dps` | main-dps.html | `entries/main-dps.tsx`（3 行） | `main-dps` | **features/main-dps**（`MainDpsPage`） | 670×560 | `decorations:false`、`transparent:false`、`visible:false`（首帧后由 Rust `show_main_dps_when_ready` 显示）、`backgroundColor:#f6f7f8`、可缩放 |
| `hud-spike` | hud.html | `entries/hud.tsx`（3 行） | `technical-hud` | **features/technical-hud**（`TechnicalHudPage`） | 380×238 | `transparent:true`、`alwaysOnTop:true`、`skipTaskbar:true`、`decorations:false`、无 shadow |
| `notification-island` | notification.html | `entries/notification.tsx`（6 行） | `notification-island` | **features/island**（`IslandPage`，`characterAvatars:false`） | 440×76 | `transparent:true`、`alwaysOnTop:true`、`skipTaskbar:true`、`decorations:false`、不可缩放、max 560×120 |
| `console` | console.html | `entries/console.tsx`（3 行） | `console` | **features/console**（`ConsolePage` 外壳）+ 11 个页面 feature | 1180×760 | `decorations:false`、`visible:false`、`backgroundColor:#f6f7f8` |
| `abyss-values` | abyss-values.html | `entries/abyss-values.tsx`（3 行） | `abyss-values` | **features/abyss-values**（`AbyssValuesPage`） | 1100×720 | `decorations:false`、`visible:false` |
| `character-details` | combat-details.html | `entries/combat-details.tsx`（3 行） | `combat-details` | **features/main-dps**（`MainDpsDetailPage`，`kind=character`） | 880×620 | `decorations:false`、`visible:false` |
| `team-details` | combat-details.html | 同上 | `combat-details` | 同上（`kind=team`） | 980×680 | 同上 |

**关键事实：**

1. `hud-spike` 窗口 label 与前端 `windowRoute`（`technical-hud`）不一致；`lib/tauri/window-labels.ts` 只定义 5 个 label（缺 hud / island），窗口操作一律走 `getCurrentWindow()`。
2. `combat-details` 一页两窗：标题与过滤条件由快照决定（`main-dps-detail-page.tsx` L1241 `detailTitle()` 读 `snapshot.kind` / `snapshot.abyssHalf`），打开时由命令参数区分。
3. 7 个窗口全部走 `entries/window-bootstrap.tsx`（45 行）：设 `data-window-route`、`bootstrapSettingsPresentation()`（读 localStorage → `<html>` 的 `.dark/.console-light`/`data-theme-preset`/`data-accent`/`data-density`/`.reduce-motion`）、禁右键菜单、`StrictMode` + `TooltipProvider` + `WindowMotionBoundary`、`createRoot`；main-dps 额外等 2 帧后 `invoke("show_main_dps_when_ready")`；除 island 外都 `bootstrapCharacterAvatarCatalog()`（`get_character_data_snapshot` → 头像路径表）。
4. HUD 与 island 没有自绘标题栏：HUD 靠 `startHudWindowDragging()`（`getCurrentWindow().startDragging()`），island 由 Rust 侧定位（`islandOffsetX`），前端无拖拽。

---

## 3. 逐 feature 说明

### 3.1 用途总览

| feature | 用途（读代码 + `frontend/README.md` 确认） |
|---|---|
| `main-dps` | 主窗 DPS 面板：回合/深渊半场切换、队伍行、工具栏、新手引导、更新弹窗、命中快照对话框；**同时渲染两个 combat-details 窗口**（明细表、列宽/列可见性、分页加载、命中快照）。 |
| `empty-curtain` | User 插件装备仓库/配装：装备过滤器（品质/角色/主副词条）、`<canvas>` 卡片网格、装备详情、锁定/丢弃/穿戴/卸下/移动、导入导出、配装计算器导出。 |
| `mod-studio` | Mod 工作室：宿主 Loader 状态/安装/游戏目录、插件启停、Mod 市场、**JSON Forms 动态插件表单**、原生响应展示。 |
| `settings` | 设置目录：3 个 Tab（Interface / HUD / Capture and data）共 14 张卡片；HUD 模块拖拽排序与显隐、更新、采集、热键、布局 profile、队伍数据、录制文件。 |
| `technical-hud` | 透明置顶 HUD：标题/摘要/状态/角色排行/曲线 5 个模块，编辑栏（拖拽排序、显隐、宽度、穿透、置顶、采集控制），canvas 迷你曲线。 |
| `timeline` | DPS 时间轴：canvas 折线（队伍/单角色模式）、累积伤害、时间停止区间、标记右键菜单、缩放/框选、hover lens + tooltip。 |
| `console` | 控制台外壳：侧栏（可折叠、收藏、localStorage 持久化）、命令面板（Ctrl+K 模糊搜索、键盘导航）、`<Activity>` 保活页面、4 个 App 级页面 + 诊断/历史等共享数据入口。 |
| `history` | 历史记录：列表、详情、队伍/角色分解、两条记录对比、导入 JSON/文件、导出、删除与撤销、设置预测队伍。 |
| `abyss-values` | 深渊数值：赛季/层/半场导航、怪物行与波次、血量合计/所需 DPS/预计秒数、预测队伍头像与导入。 |
| `character-data` | 只读角色账号快照（User 插件）：16 角色分页搜索、选中角色配装/技能/装备/属性面板、游戏内刷新、部分/过期状态。 |
| `skills` | 技能伤害表：按角色/范围（all/upper/lower）聚合、排行、占比、技能详情、GE Index 复制。 |
| `diagnostics` | 诊断：采集/质量/环境/连接检查列表、运行诊断、导入导出 JSON/PCAPNG、复制脱敏报告。 |
| `encrypted-ini` | 加密 INI 编辑器：打开/重载/保存/清空、搜索命中跳转与高亮、行列定位、居中滚动。 |
| `packets` | 原始包列表：搜索、仅命中过滤、方向/时间戳、解析命中数、详情展开。 |
| `shortcuts` | 全局热键绑定编辑器（复用 `settings/use-settings` 的 hotkeys 草稿）：按键捕获、冲突提示、启停。 |
| `island` | 通知岛：快照轮询 + 变更事件、按 `remainingMs` 自动消失、入场/退场动画、撤销、错误显示。 |

### 3.2 技术特征（自绘 / 依赖 / 通道）

| feature | canvas/SVG | 绝对定位/Portal | dnd-kit | motion | JSONForms | Markdown | 虚拟化/大列表 | 定时器 | tauri client（方法数，含 subscribe/listen） | channel / 事件 |
|---|---|---|---|---|---|---|---|---|---|---|
| `main-dps` | 无 | 6 处 `absolute` | 否 | 是（`AnimatedNumber`、窗口进出场） | 否 | `ui/markdown`（更新日志） | 明细表虚拟行（64px/行，overscan 8，200 行/页追加） | 2（撤销 token 5s、action notice 2.2s） | `main-dps-client`(26)、`main-dps-detail-client`(8)、`update-prompt-client`(4)、`toolkit-client`(8)、`window-events`(2) | Channel `subscribe_main_dps` + `subscribe_main_dps_detail`；Tauri 事件 `main-dps-confirmation-requested`、`main-dps-detail-requested`、`update-available`；窗口拖入文件（pcapng/json） |
| `empty-curtain` | **`<canvas>` 2D 整卡绘制**（680 行，DPR、裁剪、文本截断、命中测试） | 2 Portal + 5 `absolute` | 否 | 否 | 否 | 否 | canvas 渲染窗口（行窗口 + overscan） | 1（resize 防抖） | `empty-curtain-client`(8) | Channel `subscribe_empty_curtain`；`get_empty_curtain_positions` |
| `mod-studio` | 无 | 1 `absolute` | 否 | 否 | **是（唯一使用点，渲染器 314 行）** | 否 | 无 | 2（panel 轮询 3s、动作 notice） | `toolkit-client`(8)、`mod-studio-client`(2)、`plugin-form-catalog`（2 命令） | 无 Channel；`get_plugin_panel` 3s 轮询；`get_plugin_form`/`invoke_plugin_form_action` |
| `settings` | 无 | 2 Portal | **是（HUD 模块排序）** | 否 | 否 | `ui/markdown`（更新说明） | 无 | 1（导出反馈） | `settings-client`(26) | Channel `subscribe_settings` |
| `technical-hud` | **`<canvas>` 迷你曲线**（228 行） | 8 `absolute` | **是（编辑栏排序）** | **是（模块行/通知动画）** | 否 | 否 | 无 | 1（notice 1.8s） | `technical-client`(10)、`hud-window-client`(2) | Channel `subscribe_technical_state`；窗口拖拽 |
| `timeline` | **`<canvas>` 折线图**（742 行） | 13 `absolute` | 否 | 否 | 否 | 否 | 无 | 0 | `timeline-client`(3) | Channel `subscribe_timeline`（scope 参数） |
| `console` | 无 | 3 `absolute` | 否 | CSS + `startViewTransition` | 否 | 否 | `<Activity>` 保活 11 页面 | 0 | `console-control-client`(1) + 各页面 client | Tauri 事件 `console-navigate`；`main-dps` 拖入回放 |
| `history` | 无 | 0 | 否 | 否 | 否 | 否 | 无 | 1（删除撤销） | `history-client`(10)、`diagnostics-client`(1) | Channel `subscribe_history` |
| `abyss-values` | 无 | 3 `absolute` | 否 | 否 | 否 | 否 | 无 | 0 | `abyss-values-client`(5) | 无 Channel（命令拉取） |
| `character-data` | 无 | 0 | 否 | 否 | 否 | 否 | 无（16 角色/页，服务端分页） | 2（搜索防抖） | `user-characters-client`(1) | 无 Channel（命令 301/302/303） |
| `skills` | 无 | 2 Portal | 否 | 否 | 否 | 否 | 无 | 1（复制反馈 1.6s） | `skills-client`(2) | Channel `subscribe_skills`（scope 参数） |
| `diagnostics` | 无 | 2 Portal | 否 | 否 | 否 | 否 | 无 | 1（复制反馈 1.6s） | `diagnostics-client`(7) | Channel `subscribe_diagnostics` |
| `encrypted-ini` | 无 | 2 Portal | 否 | 否 | 否 | 否 | 无 | 0（rAF 滚动定位） | `encrypted-ini-client`(5) | 无 Channel |
| `packets` | 无 | 2 Portal | 否 | 否 | 否 | 否 | 无（列表服务端有界） | 0（rAF 滚动） | `packets-client`(2) | Channel `subscribe_packets` |
| `shortcuts` | 无 | 0 | 否 | 否 | 否 | 否 | 无 | 0 | 复用 `settings-client` | Channel `subscribe_settings`（经 settings） |
| `island` | 无 | 0 | 否 | **CSS keyframes（进出场/图标/进度条）** | 否 | 否 | 无 | 1（`remainingMs` 自动消失） | `island-client`(3+1 事件) | Tauri 事件 `notification-island-changed`（`listen`，非 Channel） |

**补充事实：**

- 全仓 **0 个内联 `<svg>`**、0 个手写指针拖拽排序、0 处 `react-resizable-panels`（未安装）。自绘仅 3 个 canvas 组件（`empty-curtain/equipment-canvas-grid.tsx` 680、`technical-hud/hud-mini-timeline.tsx` 228、`timeline/timeline-chart.tsx` 742）。
- 前端 client 层引用了 `src-tauri` 注册的**全部 144 个 `#[tauri::command]`**（其中 10 对是 Channel 的 subscribe/unsubscribe，其余 124 个为查询/动作命令）；Channel 共 10 条（mainDps、mainDpsDetail、technical、settings、history、timeline、skills、packets、diagnostics、emptyCurtain）。
- `lib/tauri/stream-client.ts` 是统一 Channel 消费器：有序投递 + 逐条 `acknowledge_stream_delivery` + ack 失败 fail-closed 并退订（84 行，§4.3 B 类）。
- 图标：`lucide-react` 在 31 个文件里使用 **80 个不同图标**（清单见 §5.5）。

### 3.3 逐 feature 用到的 UI 组件（来自 import 实测）

| feature | `components/ui` | `components/nte` |
|---|---|---|
| `main-dps` | alert-dialog、badge、button、dialog、form-controls（`NativeSelect`）、markdown、menu（context-menu）、popover | desktop-titlebar、action-notice、animated-number、max-hp-compression-effect、window-motion-context |
| `empty-curtain` | alert、badge、button、dialog、empty、form-controls（`Input`/`NativeSelect`）、skeleton、tooltip | page-layout |
| `mod-studio` | alert、alert-dialog、badge、button、card、empty、form-controls、skeleton、switch、tabs（直接 `@base-ui/react/tabs`） | action-notice、form-row、page-layout |
| `settings` | alert、badge、button、card、form-controls、markdown、separator、skeleton、switch、tabs（直接 `@base-ui/react/tabs`） | form-row、hud-module-sortable、page-layout |
| `technical-hud` | button、form-controls、popover、switch、tooltip | action-notice、animated-number、hud-module-sortable、window-motion-context |
| `timeline` | alert、badge、button、card、empty、menu（context-menu）、skeleton | page-layout |
| `console` | alert、alert-dialog、button、dialog、form-controls、tooltip | desktop-titlebar |
| `history` | alert、badge、button、card、empty、form-controls、menu（context-menu）、skeleton | page-layout |
| `abyss-values` | alert、badge、button、form-controls（`Input`）、skeleton | desktop-titlebar |
| `character-data` | alert、badge、button、form-controls（`Input`） | page-layout |
| `skills` | alert、badge、button、empty、skeleton、tooltip | page-layout |
| `diagnostics` | alert、badge、button、empty、skeleton | page-layout |
| `encrypted-ini` | alert、alert-dialog、badge、button、form-controls、skeleton | page-layout |
| `packets` | alert、badge、button、empty、form-controls、skeleton、switch | page-layout |
| `shortcuts` | alert、button、card、skeleton、switch | page-layout |
| `island` | button | — |

> 结论：事实上的共享 UI 面只有 15 个 `components/ui` 适配件 + 9 个 `components/nte` 自研件；所有 16 个 feature 都只通过这两层渲染，没有第三套组件。gpui-kit 迁移时这 24 个非测试文件（C 类 6 个 409 行 + D 类 18 个 1,233 行，另加 `hooks/use-hud-module-sortable.ts` 34 行）是唯一样式来源。

---

## 4. 文件分类（核心交付物）

### 4.1 汇总（按文件主类，精确）

| 类 | 含义 | 文件 | 行数 | 占 `frontend/src` |
|---|---|---:|---:|---:|
| **A** | 可下沉 Rust 的展示逻辑（无 React/DOM；格式化/计算/映射/几何刻度/配色键；**不含 core 已有等价实现**） | 23 | **2,518** | 8.1% |
| **B** | UI-only 状态（zustand store / hooks / 窗口生命周期 / 防抖竞态 / 拖拽状态） | 20 | **2,035** | 6.5% |
| **C** | 纯视图，需用 gpui-kit 重写（页面/组件） | 44 | **18,061** | 57.8% |
| **D** | 删除（测试、ts-rs 生成物、shadcn/base-ui 适配、motion/dnd/jsonforms 适配、React/Tauri 胶水、CSS、i18n 前端实现、core 已有等价文件；分组明细与校验见 §4.5） | 318 | **8,618** | 27.6% |
| **合计** | | **405** | **31,232** | 100% |

> A 类内部再分两档：
> - **A1 直接删（core 已有等价，约 380–420 行）**：`settings-presentation-model`、`decimal-string`、`window-labels`、`character-attribute`、`damage-image-lookup` 等已整体移入 D；A 表内 `main-dps-detail-model`、`technical-view-model`、`settings-view-model` 等文件中的镜像默认值/代际门/可见性判定约 390 行也属于 A1。
> - **A2 需新写（约 2.1k 行）**：core 没有对应实现，主要是 UI 模型（画布几何、图表刻度、命令目录、导航、过滤/排序、格式化）。
>
> 校验：A+B+C+D = 31,232；文件 23+20+44+318 = 405（命令见附录 A.12）。

逐 feature 的主类小计（含该 feature 的测试行，归 D）：

| feature | A | B | C | D | 合计 |
|---|---:|---:|---:|---:|---:|
| `main-dps` | 265 | 103 | 3,815 | 550 | 4,733 |
| `empty-curtain` | 301 | 207 | 1,968 | 318 | 2,794 |
| `mod-studio` | 100 | 162 | 1,458 | 954 | 2,674 |
| `settings` | 84 | 240 | 1,684 | 167 | 2,175 |
| `technical-hud` | 284 | 124 | 1,405 | 287 | 2,100 |
| `timeline` | 247 | 131 | 1,153 | 165 | 1,696 |
| `console` | 492 | 0 | 981 | 198 | 1,671 |
| `history` | 88 | 278 | 1,046 | 66 | 1,478 |
| `abyss-values` | 182 | 0 | 938 | 98 | 1,218 |
| `character-data` | 121 | 69 | 606 | 701 | 1,497 |
| `skills` | 73 | 97 | 690 | 130 | 990 |
| `diagnostics` | 50 | 127 | 653 | 64 | 894 |
| `encrypted-ini` | 46 | 163 | 410 | 34 | 653 |
| `packets` | 82 | 52 | 357 | 107 | 598 |
| `shortcuts` | 55 | 0 | 302 | 47 | 404 |
| `island` | 0 | 0 | 186 | 0 | 186 |
| **features 小计** | **2,470** | **1,753** | **17,652** | **3,886** | **25,761** |

非 feature 部分：components A0/B0/C409/D1,347；lib A48/B261/D1,925（含生成 643、测试 393）；hooks A0/B21/D34；entries A0/B0/D66；styles A0（token 值由 core enum 提供，前端只做映射 → 归 D）/D1,360。

### 4.2 A 类：可下沉 Rust 的展示逻辑（23 文件 / 2,518 行，逐个点名）

**先决判断（依据核心侧实测，可直接引用）**：核心 `src/` 85k 行已 100% UI 无关，且已有纯投影层：

| 核心模块 | 已承担的投影 |
|---|---|
| `src/core/hud.rs::project_hud` | HUD 快照（模块配置/状态/角色行/曲线） |
| `src/core/timeline.rs::project_timeline` | 时间轴桶/角色/标记/时间停止区间 |
| `src/core/skills.rs::project_skills` + `skill_label_translation_key` | 技能行/角色汇总/伤害占比派生 |
| `src/core/packets.rs::project_recent_packets/project_packets_since` | 包列表与增量投影 |
| `src/core/combat_details.rs` | 明细过滤 `matches`、反应/伤害数字图片 key、列默认值 |
| `src/core/diagnostics.rs::run_capture_diagnostics` | 诊断检查项与建议 |
| `src/core/empty_curtain.rs` | 库存投影、推荐配装、副词条解锁 |
| `src/core/mod_market.rs` | 目录解析、本地状态、包校验/更新判定 |
| `src/core/character_data.rs` / `user_characters.rs` | 角色快照投影（名称/头像/字段） |
| `src/storage/i18n.rs` | `t/t_for/tf/tf_for` + 占位符语法（前端 i18n 镜像，见 §6） |
| `src/storage/config.rs` | `ThemePreset/AccentColor/UiDensity/HudModule/HitDetailColumn/MainDpsMetric` serde enum（见 §7） |

因此下表把 A 类分成三种处置：
- **A2 新写**：core 无等价实现，需要新增 Rust 展示逻辑（目标统一为 `crates/desktop/src/presentation/`（纯逻辑），页面专属状态与模型并入 `crates/desktop/src/viewmodel/`）；
- **A1 删**：core 已有等价实现，前端这份是 IPC 时代的镜像/校验/默认值，**直接删，0 行新代码**；
- **混合**：一个文件里两类都有，给出估算行数（不计入 4.1 的精确合计）。

| 文件 | 行 | 内容（读代码确认） | core 现状 | 处置 / 目标位置 |
|---|---:|---|---|---|
| `features/abyss-values/abyss-values-model.ts` | 182 | 楼层选择、怪物过滤/半场分组、`monsterTotalHp/lineTotalHp/requiredDps/predictedSeconds`、血量格式、**怪物立绘文件名 stem 归并算法** | **无反投影**（`src/core/` 无 abyss 模块，DTO 只给 `hpMaxBase/count/team.dps`） | **新写** `crates/desktop/src/presentation/abyss.rs`；立绘建议由 Rust 下发路径，stem 算法降级为兜底 |
| `features/character-data/character-build-model.ts` | 121 | 面板数值格式化（plain/attack/integer/percent/score、`%` 单位、去 −0、超安全整数不四舍五入）、装备/秘宝/角色名多语言、`attributeLabel` | 名称/单位由 core `user_equipment.rs`/`equipment.json` 目录提供（README 已写明 exact attribute IDs） | **混合**：名称/单位映射删（约 41 行）；数值格式化新写 → `crates/desktop/src/presentation/character_build.rs` |
| `features/console/console-command-palette-model.ts` | 327 | 30+ 命令目录（控制/导航/导入导出/布局/主题/强调色/密度）、模糊匹配、上下选择、行样式 | 无（`src-tauri/commands/console_control.rs` 只执行动作） | **新写** `crates/desktop/src/console/commands.rs`；行样式（约 20 行 Tailwind）删除 |
| `features/console/console-navigation.ts` | 139 | 11 页面 ID、相邻页、快捷键（Ctrl+K/PageUp/Down/可编辑目标）、侧栏折叠与收藏 localStorage | 无 | **新写** `crates/desktop/src/console/navigation.rs`；持久化并入 Rust config |
| `features/console/console-sidebar-model.ts` | 26 | 折叠呈现解析、行样式 | 无 | **新写**（约 11 行）+ 行样式删（约 15 行） |
| `features/diagnostics/diagnostics-model.ts` | 50 | 内容状态、generation 单调门、decimal 格式化、**脱敏报告文本生成** | `core/diagnostics.rs` 已产检查项/建议；generation 单调由 core 命令层保证 | **混合**：generation 门删（约 10 行）；报告文本新写 → `crates/desktop/src/presentation/diagnostics.rs`（或核心直接产出文本） |
| `features/empty-curtain/equipment-canvas-layout.ts` | 238 | 网格布局、可见 cell、`indexAtContent` 命中测试、渲染窗口/刷新阈值、DPR backing store、锁定列偏移 | `core/empty_curtain.rs` 只做库存/推荐/副词条，无画布几何 | **新写** `crates/desktop/src/presentation/equipment_canvas.rs`（GPUI 有布局/裁剪后仅保留 cell 索引与命中） |
| `features/empty-curtain/equipment-filters.ts` | 63 | 品质色常量、过滤器结构、有效计数、过滤组合、搜索匹配 | 无过滤实现 | **新写** `crates/desktop/src/presentation/equipment_filters.rs` |
| `features/encrypted-ini/encrypted-ini-model.ts` | 46 | 全文搜索命中、上/下一个、行列定位、居中滚动偏移 | `core/encrypted_ini.rs` 有文档/加解密，无搜索 | **新写** `crates/desktop/src/presentation/encrypted_ini.rs` |
| `features/history/history-view-model.ts` | 88 | 记录选择/相邻/比较合法性、时长格式 key+args、比较警告、delta tone | 比较合法性由 core `commands/history.rs::compare_history_records` 校验（`HistoryComparisonSnapshot::from_records`） | **混合**：合法性预检删（约 15 行）；其余新写 → `crates/desktop/src/presentation/history.rs` |
| `features/main-dps/main-dps-detail-model.ts` | 185 | 7 列默认宽度/可见性、列宽夹取、虚拟行范围、live/page 快照合并、暴击 key、效果计数文本 | `MainDpsDetailColumns` DTO 已含 core 默认值；快照合并是 IPC 分页/流补丁 | **混合**：列默认值/列宽（约 70 行）删；虚拟行范围/快照合并（约 115 行）新写 → `crates/desktop/src/main_dps/detail_model.rs` |
| `features/main-dps/main-dps-model.ts` | 80 | 主数字/时长格式、伤害占比、最大生命压缩、回合标签、generation 比较、采集 tone、列表状态 | generation 单调/采集状态由 core 投影给出 | **混合**：generation 比较删（约 10 行）；格式化/compression/tone 新写 → `crates/desktop/src/presentation/main_dps.rs` |
| `features/mod-studio/mod-market-model.ts` | 87 | 本地化文本、搜索文本、本地状态映射（未装/已装/不可读）、分类过滤、更新判定 | `core/mod_market.rs` 已解析目录/本地状态/`mod_market_package_is_current`；`localState.current` 直接下发 | **混合**：本地状态映射（约 45 行）删；分类/搜索过滤（约 42 行）新写 → `crates/desktop/src/presentation/mod_market.rs` |
| `features/mod-studio/release-control-groups.ts` | 13 | 11 个插件状态→英文 key 映射 | `core/mod_studio.rs`/contract 已有状态枚举 | **A1 删**（key 映射可并进 Rust 枚举 `label_key()`） |
| `features/packets/packets-model.ts` | 82 | 内容状态、事件增量合并（Map by sequence + 排序）、搜索、时间戳、decimal 比较 | `core/packets.rs::project_packets_since` 已做增量投影 | **混合**：内容状态/增量合并（约 45 行）删；搜索/时间戳格式化（约 37 行）新写 → `crates/desktop/src/presentation/packets.rs` |
| `features/settings/settings-view-model.ts` | 84 | 分区 pending、generation 门、HUD 模块可见性、相邻模块移动 | `core/hud.rs` 已算模块可见性，`HudConfigSnapshot` 直接下发 | **混合**：pending/generation/可见性（约 60 行）删；模块移动（约 24 行）新写 |
| `features/shortcuts/shortcut-binding.ts` | 55 | 键盘事件→`HotkeyBindingSnapshot`、格式化 `Ctrl+Alt+Shift+Key` | 全局热键注册在 `src/platform/hotkey.rs`，GPUI 直接拿 `Keystroke` | **新写**（按键捕获适配，约 55 行）→ `crates/desktop/src/presentation/hotkeys.rs`（其余可删） |
| `features/skills/skills-interactions.ts` | 33 | GA/GE/GE Index 技术详情、剪贴板写出 | `core/skills.rs::skill_label_translation_key` 管 label key；剪贴板 GPUI 内建 | **混合**：label 映射删（约 13 行）；详情组装/复制新写（约 20 行） |
| `features/skills/skills-ui-model.ts` | 40 | 角色摘要+占比、按角色过滤并按伤害排序、指标合计 | `core/skills.rs` 已投影角色 `damage`；`HudCharacterSnapshot` 已有 `damageSharePercent` 先例 | **混合**：share 可让 core 补字段后删（约 15 行）；排序/过滤新写（约 25 行） |
| `features/technical-hud/mini-timeline.ts` | 110 | 迷你曲线绘制模型、指针→桶命中、DPR backing store | `core/timeline.rs` 提供桶；绘制几何是 UI | **新写** `crates/desktop/src/presentation/mini_timeline.rs` |
| `features/technical-hud/technical-view-model.ts` | 174 | 页面状态/序号门、tone 映射、采集动作/消息、HUD 模块可见性/键盘移动、宽度草稿、数值/时长格式 | `core/hud.rs::project_hud` 已产出状态/配置/角色行；tone/可见性判定与 core 重叠 | **混合**：tone/可见性/序号门（约 100 行）删；labelKey/宽度草稿/格式化（约 74 行）新写 → `crates/desktop/src/presentation/hud.rs` |
| `features/timeline/timeline-ui-model.ts` | 247 | 图表模型（plot 边距、maxDps、team/角色双模式、viewRange）、时间↔X、缩放夹取、标记增删、canvas backing size | `core/timeline.rs` 已给桶/标记/峰值；图表几何是 UI | **新写** `crates/desktop/src/presentation/timeline_chart.rs` |
| `lib/update-presentation.ts` | 48 | bytes → `KiB/MiB/GiB`（十分位、bigint）、进度百分比、组件标签 key | `src/storage/capture_logs.rs::format_bytes` 已有近似实现（KB/MB 命名不同） | **混合/合并**：统一成核心单一实现 → 归 `crates/desktop/src/presentation/` 或核心；前端删 |

**A 类账**：精确合计 2,518 行；其中 **A1 直接删 ≈ 390 行**（含上表混合文件里的 core 镜像部分，以及已整文件移入 D 的 `lib/settings-presentation-model.ts` 16、`lib/decimal-string.ts` 24、`lib/character-attribute.ts` 17、`lib/tauri/window-labels.ts` 5、`features/main-dps/damage-image-lookup.ts` 50），**A2 需新写 ≈ 2,130 行**。

> 已整体移入 D 的 6 个「core 已有等价」文件（不再算 A）：`styles/theme.css` 214（token 值已在 `storage/config.rs` 的 serde enum，前端只是 CSS 映射）、`lib/settings-presentation-model.ts` 16（DTO 加 `PartialEq` 即可）、`lib/decimal-string.ts` 24（generation 单调门由核心投影/命令层保证）、`lib/character-attribute.ts` 17（属性表在核心 equipment catalog）、`lib/tauri/window-labels.ts` 5（窗口 label 在 `src-tauri/src/windows/`）、`features/main-dps/damage-image-lookup.ts` 50（`core/combat_details.rs` 已提供 `damage_digit_key_for_hit` / `reaction_text_key_for_hit`）。

### 4.3 B 类：UI-only 状态 → GPUI Entity（20 文件 / 2,035 行）

建议每个窗口一个根 Entity，跨窗口共享的放 `crates/desktop/src/session/` 的 `AppContext`（Rust 侧单例，替代 zustand）。

| 文件 | 行 | 抽成 Entity/State | 必须保留的字段（读 hook 确认） |
|---|---:|---|---|
| `lib/tauri/stream-client.ts` | 84 | `StreamSubscription`（每数据源一个，`crates/desktop/src/stream.rs`） | `stream_kind`、`subscription_id`、`stream_generation`、`delivery_sequence`、`pending_event`、`closed`、`reported_failure`；语义：有序投递、逐条 ack、ack 失败 fail-closed 并退订 |
| `lib/settings-presentation.ts` | 87 | `PresentationState`（AppContext，替代 zustand） | `language`、`dark_mode`、`theme_preset`、`accent`、`density`、`reduce_motion`、`island_notifications`、`island_offset_x`、持久化 key `nte.settings.presentation.v1`、`storage` 事件同步、opaque 窗口背景同步 |
| `lib/character-avatar.ts` | 45 | `AvatarCatalog`（AppContext） | `Map<character_id, avatar_path>`、bootstrap 来源 `get_character_data_snapshot`、订阅者通知 |
| `lib/motion.ts` | 25 | `MotionState`（AppContext） | `MOTION_DURATION {120,180,250}`、reduce-motion 判定、`wait_for_motion`、view-transition 开关 |
| `lib/async-cleanup.ts` | 20 | 通用 `AsyncRegistration`（Rust 无需等价物，用 drop/取消令牌） | `active`、`cleanup: Option<fn>`；对应 Rust `Drop`/`CancellationToken` |
| `hooks/use-character-avatar.ts` | 10 | 并入 `AvatarCatalog` | `char_id → Option<path>` |
| `hooks/use-close-on-window-blur.ts` | 11 | `WindowFocus`（`crates/desktop/src/shell/`） | 窗口 blur 订阅、popover 打开标志 |
| `features/main-dps/use-main-dps.ts` | 103 | `MainDpsEntity` | `snapshot`、`error`、`pending_action`、`action_notice{id,action,status,timer}`、`mounted`、`action_id` |
| `features/technical-hud/use-technical-state.ts` | 124 | `HudEntity` | `state(loading/error/ready)`、`action_notice{id,action,status,timer}`、`action_id`、`refresh/reset/set_passthrough/set_always_on_top/set_module_visibility/move_module/set_width/start/stop` 的 pending 标志 |
| `features/settings/use-settings.ts` | 240 | `SettingsEntity` | `state`、`pending_action`、`mutation_error`、`mounted`、`mutation_pending`、`load_generation`；`apply_snapshot/refresh/mutate` |
| `features/history/use-history.ts` | 278 | `HistoryEntity` | `state`、`pending_action`、`mutation_error`、`comparison`、`status_message_key`、`preferred_record_id`、`undo_deletion{record_id,token,timer}`、`mounted`、`request_generation`、`operation_pending`、`comparison_generation` |
| `features/empty-curtain/use-empty-curtain.ts` | 207 | `EmptyCurtainEntity` | `state`、`notice`、`action_pending`、`refreshing`、`reload_key`、`mounted`、`request_generation`；动作：snapshot/file/positions/character |
| `features/encrypted-ini/use-encrypted-ini.ts` | 163 | `EncryptedIniEntity` | `snapshot`、`loading`、`busy`、`notice{kind,messageKey}`、`mounted`；动作 open/reload/save/clear |
| `features/timeline/use-timeline.ts` | 131 | `TimelineEntity` | `scope`、`state`、`pending`、`reload_key`、`mutation_error`、`mounted`、`request_generation` |
| `features/skills/use-skills.ts` | 97 | `SkillsEntity` | `scope`、`state`、`stream_error`、`reload_key`、`mounted`、`request_generation` |
| `features/diagnostics/use-diagnostics.ts` | 127 | `DiagnosticsEntity` | `state`、`stream_error`、`action_error`、`pending_action`、`reload_key`、`mounted`、`request_generation` |
| `features/packets/use-packets.ts` | 52 | `PacketsEntity` | `state`、`stream_error`、`reload_key` |
| `features/mod-studio/use-mod-market.ts` | 82 | `ModMarketEntity` | `state`、`install_states`、`request` 序号、`installing` 集合 |
| `features/mod-studio/plugin-form-catalog.ts` | 80 | `PluginFormCatalog`（AppContext 共享） | `entries: Map<file,{key,snapshot,loading,error}>`；按 `(identity,owner)` 去重、reload 语义 |
| `features/character-data/use-user-characters.ts` | 69 | `CharacterDataEntity` | `snapshot`、`pending`、`error`、`generation`、搜索防抖 timer |

> 另外 **页面内嵌的 B 状态**（§4.6）必须一起进 Entity：`MainDpsDetailEntity`（columns/scroll/selected_hit/loading_more）、`ConsoleShellEntity`（active_page/sidebar_collapsed/favorites/palette 状态）、`SettingsCatalog` 草稿（interface/capture/update/hotkeys/display 五份 draft + keepMounted）、`EmptyCurtain` 过滤器与对话框、`Timeline` 缩放/markers/hover、`ModStudio` 面板轮询与 plugin form 草稿、`Island` 的 exiting/undo/dismiss timer。

### 4.4 C 类：纯视图，用 gpui-kit 重写（44 文件 / 18,061 行）

| feature | 视图文件（行数） |
|---|---|
| `main-dps` | `main-dps-page.tsx` 658、`main-dps-detail-page.tsx` **1254**、`main-dps-team.tsx` 518、`main-dps-toolbar.tsx` 226、`hit-snapshot-dialog.tsx` 459、`update-prompt-dialog.tsx` 262、`main-dps-onboarding.tsx` 165、`effect-modifier-values.tsx` 99、`data-source-control.tsx` 174 |
| `empty-curtain` | `empty-curtain-page.tsx` **1288**、`equipment-canvas-grid.tsx` **680**（canvas 自绘） |
| `mod-studio` | `mod-studio-page.tsx` 287、`host-controls.tsx` 234、`host-loading-controls.tsx` 184、`host-runtime-controls.tsx` 84、`plugin-controls.tsx` 135、`plugin-form-panel.tsx` 145、`mod-market-panel.tsx` 323、`native-operation.tsx` 66 |
| `settings` | `settings-page.tsx` 525、`settings-catalog.tsx` **1159** |
| `technical-hud` | `technical-hud-page.tsx` **1177**、`hud-mini-timeline.tsx` 228（canvas） |
| `timeline` | `timeline-chart.tsx` **742**（canvas）、`timeline-page.tsx` 411 |
| `console` | `console-page.tsx` 517、`console-sidebar.tsx` 301、`console-command-palette.tsx` 163 |
| `history` | `history-page.tsx` **1046** |
| `abyss-values` | `abyss-values-page.tsx` **938** |
| `character-data` | `character-build-sheet.tsx` 335、`character-data-page.tsx` 271 |
| `skills` | `skills-page.tsx` 690 |
| `diagnostics` | `diagnostics-page.tsx` 653 |
| `packets` | `packets-page.tsx` 357 |
| `encrypted-ini` | `encrypted-ini-page.tsx` 410 |
| `shortcuts` | `shortcuts-page.tsx` 302 |
| `island` | `island-page.tsx` 186 |
| 共享组件（`components/nte`） | `desktop-titlebar.tsx` 140、`action-notice.tsx` 79、`max-hp-compression-effect.tsx` 55、`page-layout.tsx` 50、`animated-number.tsx` 43、`form-row.tsx` 42 |

重写要点（读代码确认）：7 个窗口全部无边框，标题栏 4 个按钮（置顶/最小化/最大化/关闭）+ 拖拽区 + 左侧状态区；`PageHeader/PageScrollArea/PreferencesGrid/FormRow/ActionNotice` 是事实上的设计系统，必须在 gpui-kit 里找到/自建等价物；`<Activity>` 保活（console 页面切换保留状态）需要 GPUI 的「隐藏但保留 Entity」。

### 4.5 D 类：删除（318 文件 / 8,618 行，分组点名）

| 分组 | 文件 | 行 | 删除理由 |
|---|---|---:|---|
| **测试** | 43 个 `*.test.ts(x)`（清单见 §8） | 3,536 | 其中 2,085 行逻辑测试迁成 Rust `#[test]`，其余 1,451 行直接删（视图契约测试 + React/Tauri 胶水测试） |
| **ts-rs 生成 DTO** | `lib/tauri/generated/*.ts` 209 文件 | 643 | 单进程后直接用 Rust 类型，无需序列化镜像 |
| **shadcn/base-ui 适配层** | `components/ui/*.tsx` 15 文件：alert-dialog 85、alert 69、badge 48、button 54、card 94、dialog 73、empty 95、form-controls 23、markdown 224、menu 120、popover 59、separator 20、skeleton 11、switch 37、tooltip 59 | 1,071 | 组件本体换 gpui-kit；`markdown.tsx` 中约 120 行解析规则需改用 Rust markdown 实现或移植 |
| **motion/dnd-kit 适配** | `window-motion-boundary.tsx` 71、`window-motion-context.ts` 13、`hud-module-sortable.tsx` 78、`hooks/use-hud-module-sortable.ts` 34 | 196 | GPUI 自带动画；拖拽排序需自建（`AnyDrag`/自绘），排序状态与箭头键路径进 Entity |
| **@jsonforms 适配** | `features/mod-studio/plugin-form-renderers.tsx` 314 | 314 | schema 渲染层必须自写（Layout/Control/Button 三类 renderer + 数组增删 + 错误显示 + 确认流程） |
| **feature 内失效/Dev 文件** | `timeline/timeline-ui-preview.ts` 64（dev/测试 fixture）、`character-data/character-build.css` 429（页面专属样式）、`main-dps/damage-image-lookup.ts` 50（core 已有图片 key） | 543 | 见 §4.2 末段；fixture 随浏览器预览一起删 |
| **React 入口/窗口胶水** | `entries/*` 7 文件 66；`lib/tauri/window-events.ts` 20、`window-ready.ts` 30、`console-window-background.ts` 41、`desktop-window-client.ts` 11、`hud-window-client.ts` 5 | 173 | Tauri 窗口/事件 API 换 GPUI 原生窗口与直接调用 |
| **Tauri IPC client 层** | `lib/tauri/*-client.ts` 19 个 + `command-error.ts` 25 | 617 | 换进程内 Rust 调用；144 个命令的语义记入 20 号文档 |
| **CSS 层** | `src/index.css` 878、`styles/theme.css` 214、`styles/ui-foundation.css` 202、`styles/console.css` 66 | 1,360 | 布局/组件样式由 gpui-kit 承担；token 值已在 core serde enum，前端只做映射；28 个 keyframes 按 §5.2 重写 |
| **前端 i18n 实现** | `lib/i18n.ts` 43 | 43 | Rust 已有同语义 `src/storage/i18n.rs`（见 §6） |
| **core 已有等价的 lib 文件** | `lib/settings-presentation-model.ts` 16、`lib/decimal-string.ts` 24、`lib/character-attribute.ts` 17、`lib/tauri/window-labels.ts` 5 | 62 | DTO 加 `PartialEq`、generation 单调由核心保证、属性表/窗口 label 核心已有 |
| **其余失效/重复** | `lib/utils.ts` 5（`cn`）、`lib/character-color.ts` 55（Rust 已算 `color`，见 §9 风险 1） | 60 | 见 §9.3 |

> D 行数校验：3,536+643+1,071+196+314+543+173+617+1,360+43+62+60 = 8,618。

### 4.6 混合文件二次拆分（估算，不计入 4.1 合计）

页面把「取数/合并/格式化」写在组件里。逐文件读后给出可下沉行数（±20%），用来回答「纯展示逻辑到底多少」：

| 文件 | 总行 | 估 C | 估 A（格式化/映射/几何） | 估 B（状态/竞态） | 依据（行号） |
|---|---:|---:|---:|---:|---|
| `empty-curtain-page.tsx` | 1288 | 1030 | 38 | 220 | 过滤器/对话框/选择状态；L1290-1312 `formatStat/toggle/uniqueBy/fileName` 31 行 A；FilterPanel L533-707 组合逻辑 |
| `technical-hud-page.tsx` | 1177 | 1000 | 57 | 120 | L1174-1240 `summaryLabel/actionLabel/noticeWorthy/hudRoleColor/hudCharacterColor` 67 行 A；编辑栏状态 B |
| `main-dps-detail-page.tsx` | 1254 | 900 | 44 | 310 | L1192-1300 列/标题/share/时间格式约 110 行 A；L59-181 load/loadMore/setView/persistColumns/订阅 B |
| `settings-catalog.tsx` | 1159 | 960 | 59 | 140 | L63-121 选项表/指标表约 58 行 A；5 份 draft + keepMounted B |
| `history-page.tsx` | 1046 | 900 | 26 | 120 | L1066-1085 number/signed/duration 约 20 行 A；对比状态 B |
| `abyss-values-page.tsx` | 938 | 800 | 48 | 90 | 表格/导航/详情渲染；楼层与队伍选择 B |
| `timeline-chart.tsx` | 742 | 560 | 32 | 150 | L533-742 draw 为主；zoom/marker/hover/context-menu 状态 B |
| `equipment-canvas-grid.tsx` | 680 | 540 | 50 | 90 | draw 为主；指针/滚动/渲染窗口状态 B |
| `skills-page.tsx` | 690 | 610 | 20 | 60 | 选择/过滤状态 B；列表/详情渲染 C |
| `main-dps-page.tsx` | 658 | 540 | 18 | 100 | 确认对话框/引导/撤销 token/拖入 B |
| `diagnostics-page.tsx` | 653 | 580 | 23 | 50 | 复制状态/动作 B；报告展示 C |
| `settings-page.tsx` | 525 | 400 | 15 | 110 | tabs/HUD 拖拽/pending B |
| `main-dps-team.tsx` | 518 | 450 | 28 | 40 | 角色行/属性显示；配色/格式 A |
| `console-page.tsx` | 517 | 380 | 17 | 120 | 页面切换/收藏/命令面板/lifecycle B |
| `hit-snapshot-dialog.tsx` | 459 | 400 | 39 | 20 | 属性/效果数值格式化 A |
| 其余 29 个 C 文件 | 9,234 | 8,234 | 约 100 | 约 900 | 体量小，多为纯渲染 + 少量选择状态 |
| **C 类合计修正** | **18,061** | **≈15,807** | **≈+514** | **≈+1,740** | A≈3,032（9.7%）、B≈3,775（12.1%） |

> 修正后的四类账（把页面内嵌逻辑计入 A/B）：
> **A ≈ 3.0k（9.7%；其中约 0.4k 是 core 已有等价、直接删，真正需新写 ≈ 2.6k）/ B ≈ 3.8k（12.1%）/ C ≈ 15.8k（50.6%）/ D ≈ 8.6k（27.6%）**，合计 31,232。

---

## 5. 硬依赖清单

### 5.1 dnd-kit（2 文件 + 1 hook）

- `@dnd-kit/core` + `@dnd-kit/sortable` + `@dnd-kit/utilities`（`package.json` L21-23）。
- 唯一封装：`components/nte/hud-module-sortable.tsx`（78）用 `DndContext + SortableContext + verticalListSortingStrategy`、PointerSensor 激活距离 4px、`useOptimistic` 乐观顺序、静默英文 announcements；`hooks/use-hud-module-sortable.ts`（34）用 `useSortable` + Motion 过渡。
- 使用点 2 处：`settings/settings-page.tsx` L369-392（HUD 模块顺序）、`technical-hud/technical-hud-page.tsx` L505-526 与 L778-805（HUD 编辑栏 + 投影）。模块类型只有 5 个（title/summary/status/characters/timeline），排序语义为「拖到 target 前/后 + 箭头键可访问路径」。**GPUI 无等价列表排序组件，需自建**。

### 5.2 Motion / 动画（4 文件 + CSS 28 keyframes）

- `motion` v14：`components/nte/animated-number.tsx` 43（`useMotionValue/useTransform/animate`，数字滚动）、`components/nte/window-motion-boundary.tsx` 71（`MotionConfig` + 窗口 enter/exit + `startViewTransition` 卸载前等待）、`technical-hud/technical-hud-page.tsx` L24（模块行动画）、`hooks/use-hud-module-sortable.ts`。
- 时长刻度：`MOTION_DURATION.fast 120 / base 180 / slow 250ms`（`lib/motion.ts`，与 `--motion-duration-*` 重复定义两处）。
- CSS 动画：`index.css` 28 个 `@keyframes`（页面进出、窗口进出、内容 reveal、skeleton shimmer、notice、popover、island 进出/图标/进度、列表项、max-HP 压缩、overlay/dialog/alert、tooltip）+ `.reduce-motion *` 强制 0.01ms + `prefers-reduced-motion` 媒体查询双保险。
- GPUI 侧等价：窗口显隐动画、数字滚动（自绘帧）、页面切换（view transition 或淡入）、island 进度条（定时器驱动）。**降级风险最高的是 island 的 5s 进度条与窗口 enter/exit 的时序**。

### 5.3 @jsonforms（唯一动态表单点，314 行渲染器）

- `@jsonforms/core` 3.8.0 + `@jsonforms/react` 3.8.0（MIT）。CSP 为 Ajv 运行时校验加了 `script-src 'unsafe-eval'`（`tauri.conf.json` security.csp；许可证在 `public/licenses/jsonforms.txt`）。
- 数据来源：`PluginForm` DTO（Rust 产出）`{ version, title, schema: unknown, uischema: unknown, data: unknown, translations: {lang: {key: text}}, actions: [{id,label,confirmation,requiresIdle,dev}] }`；命令 `get_plugin_form(file, expectedIdentity)` 按需加载、`invoke_plugin_form_action(request)` 提交。
- 渲染器只实现 3 类：`Layout`（Vertical/Horizontal/Group，Group 渲染成 Card）、`Control`（object/array/oneOf|enum/boolean/number/text 分支 + errors + `required` 校验）、`Button`（action + `validate` + `requiresIdle`）。`plugin-form-panel.tsx` 145 行管草稿/确认/回滚/错误边界。
- **规模**：真代码 459 行（314+145）；但语义面大（数组增删、dev 过滤、条件可见性、失败回滚、确认流程）。GPUI 必须自写等价 schema 渲染层，不能简单「删一个依赖」。

### 5.4 Markdown（1 渲染器 + 2 使用点）

- 自写渲染器 `components/ui/markdown.tsx` 224 行：支持 `#` 标题、`-/*/+` 无序、`1.` 有序、`>` 引用、``` 代码块、`---` 分隔线、行内 code/**bold**/__bold__/~~del~~/*em*/_em_/[link](https://)，链接白名单 `https?:|mailto:|#`，**不渲染原始 HTML（XSS 安全）**。
- 使用点：`main-dps/update-prompt-dialog.tsx` L262（更新提示发布说明）、`settings/settings-catalog.tsx` L441（更新卡片 notes）。内容来自 updater 拉到并存在 `UpdateSettingsSnapshot.available.notes` 的 GitHub Release Markdown（外部数据，不校验信任）。
- GPUI 方案：换成 Rust markdown 渲染（如 `pulldown-cmark`）或把本文件 120 行解析规则移植到 `crates/desktop/src/presentation/`，视图部分用 gpui-kit 文本/富文本元素。

### 5.5 图标与字体

- **图标**：`lucide-react`，31 个文件、**80 个不同图标**：Activity, AlertTriangle, AppWindow, ArrowDown, ArrowDownRight, ArrowRight, ArrowUp, ArrowUpRight, Backpack, BadgeCheck, BarChart3, Cable, ChartNoAxesCombined, Check, ChevronDown, ChevronLeft, ChevronRight, ChevronsLeft, CircleCheck, Clipboard, Clock3, Copy, Cpu, Download, Eye, EyeOff, FileJson, FileKey2, FileSearch, Filter, FolderOpen, Gauge, GitBranch, GitCompareArrows, GripHorizontal, GripVertical, HardDrive, History, Info, Keyboard, Languages, LayoutGrid, ListChecks, LoaderCircle, LockKeyhole, Maximize2, Minus, MonitorUp, MousePointer2, Network, PackageOpen, Palette, Pin, Play, Power, Puzzle, Radio, RefreshCw, RotateCcw, Save, Search, Server, Settings, Settings2, ShieldAlert, ShieldCheck, SlidersHorizontal, Sparkles, Square, Star, Store, Table2, Timeline, TimerReset, Trash2, TriangleAlert, Upload, UserRound, Users, UsersRound。**全仓 0 个内联 `<svg>`**，无 SVG sprite；GPUI 需导出这 80 个图标的 SVG/PNG 资源（lucide 原图可直接下载同款）。
- **字体**：`@fontsource-variable/geist`（`index.css` L4）→ `--font-sans: "Geist Variable", "Microsoft YaHei UI", sans-serif`；canvas 内也硬编码 `"600 14px Geist Variable, sans-serif"`（`equipment-canvas-grid.tsx` L457/494/543）。字体文件在 `res/images/font/`（251 个文件，游戏字体/图标资源，非 UI 字体）。
- **图片资源**：`res/images` 共 493 文件（attributes 6、character-panel 80、characters 27、fangkuai 36、kongmu 36、monsters 57、font 251），前端通过 `import.meta.glob` 在构建期转 URL；GPUI 直接读盘（核心 `src/storage/resource.rs` 已有内嵌资源表与路径校验）。

### 5.6 @base-ui / shadcn 组件逐个清单

`components/ui` 15 个非测试文件，底层 `@base-ui/react` 原语：`alert-dialog`、`badge`（`merge-props`/`use-render`）、`button`、`dialog`、`menu`（`menu` + `context-menu`）、`popover`、`separator`、`switch`、`tooltip`、`tabs`（`settings-catalog.tsx`/`settings-page.tsx` 直接用 `@base-ui/react/tabs`）、`side-panel` 无、`scroll-area` 无。另有 `shadcn/tailwind.css`（Nova 风格）+ `tw-animate-css` + `class-variance-authority`/`clsx`/`tailwind-merge`（`cn`）。对应 GPUI：Dialog/AlertDialog/Popover/ContextMenu/Tooltip/Switch/Tabs/Card/Badge/Button/Empty/Skeleton/Separator 均需 gpui-kit 组件或自建。

### 5.7 虚拟列表与大列表

- **无第三方虚拟列表依赖**。两处手写：
  1. `main-dps-detail-page.tsx` L694-760：`DETAIL_ROW_HEIGHT=64`、`detailVisibleRowRange(rowCount, scrollTop, viewportHeight, overscan=8)`、上下 spacer、滚动到底前 3 行触发 `loadMore`（每页 200 行，`mergePagedDetailSnapshot` 去重合并）。
  2. `equipment-canvas-grid.tsx`：canvas 渲染窗口（`equipmentCanvasRenderWindow`，overscan=2 行，滚动阈值 35% 刷新），DOM 只保留一块 `<canvas>`。
- `packets` 列表为服务端有界（无虚拟化）；`history`/`skills`/`timeline` 列表都有界。

---

## 6. i18n 现状与 gpui 方案

### 6.1 机制

- 前端 `lib/i18n.ts`（43 行）：`zustand` 存当前语言；`t(key)` = 英文 key（语言 en 时原样返回）→ `res/languages/zh-CN.json` / `ja.json` 覆盖映射；`tf(key,args)` 走共享占位符语法；`formatTemplate` 规则：`{n}` 按索引替换、同模板可多次复用，无索引 token 时消费下一个 `{}`，未知/缺参保持字面量。
- 语言类型与默认值来自 Rust DTO `Language = "en" | "ja" | "zh-CN"`（默认 zh-CN）。
- **Rust 侧已有同语义模块 `src/storage/i18n.rs`**：进程级 `RwLock<Store>`、`Language::code()/system_default()/resource_path()`、`t / t_for / tf / tf_for`、`format_template` 与前端逐字同规则、`res/languages/<code>.json` 经 `build.rs` 内嵌（zlib），另有 `LocaleLoadDiagnostic` 与 `placeholder-conformance.json` 共享测试数据。
- 键数量：`zh-CN.json` **2,468 键**、`ja.json` 2,468 键（同集合）；英文即 key，不需要 en 文件。
- 命名惯例（读 2,468 键 + 全仓 `t()` 用法确认）：key 是**英文完整短语/句子、Title Case**（如 `Start / Stop Capture`、`The plugin interface could not be rendered. Refresh or update the plugin.`），不用点分隔命名空间；占位符用 `{}`（顺序消费）或 `{n}`（按索引、可重复），未知/缺参保持字面量；文件是扁平 `"English key": "本地化文本"` 映射，无嵌套结构；Rust 的 `message_key`/`title_key`/`label_key` 直接使用同一批 key（共 55 处静态键 + 枚举动态键）。
- 使用规模：TSX/TS 共 **837 处 `t()/tf()` 调用**，其中 **701 处是字面量 key、去重 500 个**；其余是动态 key（Rust 枚举/消息键）。

### 6.2 哪些文案由 Rust 生成

Rust 只产出 **key + 参数**，不产出本地化后的文本（除角色/技能名等资源数据按 `Language` 选列）：
- `CommandError { code, messageKey, messageArguments }`（`src-tauri/src/contract.rs`）；
- 诊断 `title_key`/`message_key` + `suggestion.messageKey/args`（`contract/diagnostics.rs`）；
- 采集 issue、设备/装备操作、主机运行时、island 通知（`contract/island.rs` `message_key`）、timeline 标记 `label_key`、settings 消息；
- 角色名/技能名/属性名/物品名来自 `res/data/*`，按 `Language` 取 `name_zh/ja/en`（`src/engine/parser.rs` L171-227、`src/core/timeline.rs` L295）。

### 6.3 gpui 侧最小方案（建议直接定案）

1. **保留 `res/languages/*.json` 为唯一事实源，保留 `src/storage/i18n.rs` 原样不动**；gpui 视图直接调用 `i18n::t/tf`。删除前端 `lib/i18n.ts` 与 Vite 的 JSON 导入，键集合零迁移。
2. 语言 = Rust `Language` 枚举，存 `PresentationState`（B 类），切换时调用 `i18n::set_language`（现有函数）；不要再造第二套 store。
3. 占位符语法沿用 `format_template` + `res/languages/placeholder-conformance.json` 7 个用例，两端测试继续共享。
4. 覆盖门禁 `scripts/test_i18n_source_coverage.ps1` 目前扫描 **TS 的 `t()/tf()` 字面量**与 Rust 的 `message_key` 常量，并与 locale JSON 做集合相等校验。迁移后把扫描目标从 `frontend/src` 改为 `crates/ui-*`/`src` 的 Rust 调用点即可；**这是必须改的门禁，否则键会悄悄漏掉**（见 §9 风险 3）。

---

## 7. 主题与样式 token

### 7.1 语义 token 清单（`styles/theme.css` 214 行）

> 处置：`styles/theme.css` 归 **D（删除）**——token 的权威定义已在核心 `src/storage/config.rs` 的 serde enum（`ThemePreset`、`AccentColor`、`UiDensity`、`HudModule`、`HitDetailColumn`、`MainDpsMetric` 等），本文件只是「core enum → CSS 变量」的渲染映射。下表保留为 gpui-kit theme JSON 的映射依据（映射不是移植新逻辑）。

两套主题：默认 dark（`:root, .dark`）与 `.console-light`（仅 console/main-dps/abyss-values/combat-details 四个不透明窗口启用）。每个主题 38 个色 token：

| 组 | token | dark 值 | light 值 |
|---|---|---|---|
| 基础 | `--background` / `--foreground` | `#111113` / `#edeef0` | `#fcfcfd` / `#1c2024` |
| 面 | `--card` / `--card-foreground` | `#18191b` / `#edeef0` | `#ffffff` / `#1c2024` |
| 浮层 | `--popover` / `--popover-foreground` | `#212225` / `#edeef0` | `#ffffff` / `#1c2024` |
| 主 | `--primary` / `--primary-foreground` | `#edeef0` / `#111113` | `#1c2024` / `#fcfcfd` |
| 次/弱 | `--secondary`、`--muted`、`--accent`（+ foreground） | `#212225`、`#212225`、`#272a2d` | `#f0f0f3`、`#f0f0f3`、`#e8e8ec` |
| 文本弱 | `--muted-foreground` | `#b0b4ba` | `#60646c` |
| 状态 | `--destructive` | `#ff9592` | `#ce2c31` |
| 线 | `--border` / `--input` / `--ring` | `#363a3f` / `#43484e` / `#5a6169` | `#d9d9e0` / `#cdced6` / `#b9bbc6` |
| 侧栏 | `--sidebar`、`--sidebar-foreground`、`--sidebar-primary(+fg)`、`--sidebar-accent(+fg)`、`--sidebar-border`、`--sidebar-ring` | 8 个 | 8 个 |
| 控制台状态 | `--console-success` / `--console-warning` / `--console-status` | `#3dd68c` / `#ffca16` / `#70b8ff` | `#218358` / `#ab6400` / `#0d74ce` |
| 图表 | `--chart-1..5` | `#0090ff/#6e56cf/#30a46c/#ffc53d/#e5484d` | 同 |
| HUD 角色 | `--hud-role-1..4` | `#00a2c7/#6e56cf/#30a46c/#ffc53d` | 同 |

覆盖表：
- **accent 5 档**：`zinc`（默认）、`blue #70b8ff/#0d74ce`、`violet #baa7ff/#6550b9`、`orange #ffa057/#cc4e00`、`green #3dd68c/#218358`（各自覆盖 `--primary/--ring/--sidebar-primary` 与 foreground）。
- **theme preset 3 档**：`zinc`（默认）、`tactical`（`--radius:0.35rem`、border/input = console-success）、`high_contrast`（`--radius:0.2rem`、`--ui-border-width:1px`、border/input = currentColor）。
- **density 3 档**：`cozy`（默认，24/16/20/12/36px…）、`compact`（16/12/12/8/28/26/32）、`comfortable`（28/20/20/16/36/32/40）。
- **尺寸/字号/边框/动效**：`--ui-page-padding 24`、`--ui-section-gap 16`、`--ui-content-width 1280`、`--ui-column-min 384`、`--ui-card-padding 20/12`、`--ui-field-padding 12`、control 高 36/24/28/40、`--ui-font-title 20` / body 14 / caption 12、`--ui-border-width 0.5px`、`--radius 0.5rem`（`--radius-sm..4xl` 5 档缩放）、`--motion-duration-fast/base/slow 120/180/250ms`、`--motion-ease-standard/emphasized`。
- **窗口/密度例外**（`theme.css` L196-214 + `index.css` base）：console 恒 16px；非 console compact 14px / comfortable 17px；main-dps 15/14/16px。密度只改间距与控件高，不改 rem 断点与字号（README 明确）。
- **HUD 例外**：透明窗口背景 `background: transparent !important`；`.hud-text-halo` 文字描边阴影（`index.css` L838）。

### 7.2 Tailwind / 样式使用规模（实测）

| 指标 | 数值 | 说明 |
|---|---:|---|
| `className=` 出现 | 1,439 | TSX 中 |
| `className="…"` 字面量 | 1,288 | 含 4,863 个 class token、**793 个去重 token** |
| 字号类 | `text-xs` 233、`text-sm` 141、`text-xl` 10、`text-2xl` 3（`text-base/3xl` 0） | 与规范 12/14/20(+24) 一致 |
| 任意值 `[...]` | 394 | 需逐个映射到 token/gpui 尺寸 |
| 调色板族类（`emerald-*`/`zinc-*` 等） | **0** | 符合 `AGENTS.md` §7 |
| CSS 总量 | 1,789 行（`index.css` 878 + theme 214 + ui-foundation 202 + console 66 + character-build 429） | 其中 28 个 keyframes |

### 7.3 必须映射到 gpui-kit theme JSON 的 token

| 类别 | 数量 | 去向 |
|---|---:|---|
| 语义颜色（含 sidebar/chart/hud-role/console 状态） | 38 × 2 主题 | theme JSON 的语义色板 |
| accent 覆盖 | 5 × 2 主题 | variant overlay（启动时按设置注入） |
| preset 覆盖 | 3（radius/border 宽度） | variant overlay |
| density 覆盖 | 3 × 6 个尺寸 token | 运行时尺寸表 |
| 尺寸/间距 | 15 个 `--ui-*` + 5 档 radius | theme JSON spacing/radius |
| 字号 | 20/14/12/24 | theme JSON typography |
| 边框 | 0.5px（高对比 1px） | theme JSON borderWidth |
| 动效 | 3 时长 + 2 缓动 | theme JSON motion |
| 字体 | Geist Variable + Microsoft YaHei UI 回退 | app 字体加载 |

---

## 8. 前端测试：移植 vs 删除

43 文件 / 3,536 行。分三类：

### 8.1 L1：纯逻辑测试 → 迁成 Rust `#[test]`（27 文件 / 2,085 行）

`abyss-values-model` 98、`character-build-model` 59、`console-command-palette-model` 65、`console-navigation` 111、`console-sidebar-model` 22、`diagnostics-model` 64、`equipment-canvas-layout` 125、`equipment-filters` 93、`encrypted-ini-model` 34、`history-view-model` 66、`damage-image-lookup` 86、`main-dps-detail-model` 147、`main-dps-model` 82、`mod-market-model` 123、`packets-model` 107、`settings-view-model` 68、`shortcuts-page`（实际测 `shortcut-binding`）47、`skills-interactions` 34、`skills-ui-model` 96、`mini-timeline` 81、`technical-view-model` 206、`timeline-ui-model` 101、`decimal-string` 30、`character-avatar` 41、`i18n` 31、`settings-presentation-model` 28、`update-presentation` 40。

> 注：`character-avatar` 的路径解析与 `i18n` 的占位符规则在 Rust 已有实现，迁移时优先「加 Rust 回归测试」而不是重写测试。

### 8.2 L2：视图契约测试 → 直接删（10 文件 / 1,183 行）

全部使用 `react-dom/server` 的 `renderToStaticMarkup` 断言 HTML 片段（含 `<h2`、`disabled`、文本、CSS class 等），违反 `AGENTS.md` §6「不写断言 CSS class / 视图契约的测试」：

`components/nte/ui-foundation.test.tsx` 71、`components/ui/markdown.test.tsx` 43、`character-data-page.test.tsx` 213、`empty-curtain-page.test.tsx` 100、`settings-catalog.test.tsx` 99、`mod-market-panel.test.tsx` 61、`plugin-control-pages.test.tsx` 237、`plugin-form-renderers.test.tsx` 174、`hit-snapshot-dialog.test.tsx` 147、`effect-modifier-values.test.tsx` 38。

### 8.3 L3：React/Tauri 胶水测试 → 直接删（6 文件 / 268 行）

`stream-client.test.ts` 108（Channel ack/fail-closed 行为应改用 Rust 集成测试验证）、`plugin-form-catalog.test.ts` 45（zustand + mock invoke）、`console-window-background.test.ts` 37、`window-ready.test.ts` 18、`async-cleanup.test.ts` 31、`motion.test.ts` 29。

**合计账：测试 3,536 行 = 迁 2,085 + 删 1,451。**

---

## 9. 结论、最易低估的 feature、风险

### 9.1 一句话账

> 前端 31,232 行：**A 2,518（精确，8.1%）可下沉 Rust，其中约 390 行是 core 已有等价实现、直接删，真正需新写 ≈ 2.1k；把页面内嵌逻辑拆出后 A ≈ 3.0k；B 2,035 / ≈3.8k（12.1%）UI-only 状态抽 Entity；C 18,061 / ≈15.8k（50.6%）纯视图重写；D 8,618（27.6%）删除**（含测试 3,536、生成物 643、shadcn 适配 1,071、CSS 1,360、Tauri client 层 617、JSONForms 适配 314 等）。

「原封不动」的只剩 **Rust 侧资产**：`res/languages`、`res/data`、`res/images`、`src/storage/i18n.rs`、`src/storage/resource.rs`、`src/storage/config.rs` 的主题/HUD 枚举、角色配色/设备目录等核心逻辑。前端文件没有一个可以原样保留：A 类 23 个文件里约 2.1k 行需要新写、约 0.4k 行属于 core 已有等价（直接删）。

### 9.2 最容易低估工作量的 3 个 feature

1. **`empty-curtain`（2,794 行）**：账面上 5 个文件，但 `equipment-canvas-grid.tsx` 680 行是**整卡 2D 自绘**（DPR backing store、裁剪窗口、逐行 stat 文本截断 `measureText`、品质/套装/锁定角标、命中测试、右键意图），加上 1,288 行页面和 238 行布局几何。GPUI 没有 canvas 2D context，需要用 `Canvas`/`PathBuilder` 重画全部卡片与滚动虚拟化；`docs/gpui-refactor/30` 若把「自绘」按普通列表估工，会低估 2-3 倍。
2. **`timeline`（1,696 行）**：`timeline-chart.tsx` 742 行 canvas 折线（双模式、累积线、时间停止区、标记、hover lens、tooltip、框选缩放、右键菜单、DPR），`timeline-ui-model.ts` 247 行全是图表几何/缩放/命中的纯逻辑——这部分可搬，但渲染与手势要重写；同样没有现成图表组件可直接替代。
3. **`settings`（2,175 行）**：`settings-catalog.tsx` 1,159 行 = 14 张卡片、约 90 个控件、5 份表单草稿（`keepMounted` 语义：切 Tab 不丢未保存输入）、HUD 模块拖拽排序 + 显隐双份逻辑（settings 与 technical-hud 各一套）、更新/Markdown/文件选择。它不是「一个设置页」，而是一个中型表单框架；gpui-kit 的 Form 能力（§5.6、30 号文档）直接决定这里要写多少自建组件。

### 9.3 必须让 Lead 知道的风险

1. **`lib/character-color.ts` 不能直接删**：Rust 已算角色色（`src/core/timeline.rs` L307、`src/storage/resource.rs` L247），但多个 DTO 的 `color` 可为 `null`（`SkillsRowSnapshot` 无 color、`MainDpsCharacterSnapshot.color: string|null`、`HudCharacterSnapshot.color: string|null`），前端用 FNV 兜底调色板补色。删前端前必须让核心把这些字段填满（或提供 `character_color(id)` 查询），否则同角色颜色会变。
2. **JSONForms 不是「删依赖」而是「自写 schema 渲染层」**：459 行真代码 + Ajv 运行时校验（CSP unsafe-eval）+ 数组增删/确认/失败回滚语义；gpui-kit 无 JSON Schema 表单能力（待 30 号文档确认），按 0.5-1 周估工起步。
3. **i18n 门禁必须跟着迁移**：2,468 个键里只有约 500 个是 TSX 字面量，其余靠 Rust 枚举/`message_key` 动态产生，Rust 已用同一 JSON 与同一 `{}/{}` 语法。如果 gpui 侧直接写死英文或改 key 拼写，`scripts/test_i18n_source_coverage.ps1` 会失明（它只扫 TS 字面量 + Rust 常量），出现「英文回退但没人发现」。建议第一件事就是把该脚本的扫描目标改到 Rust 视图代码。
4. **主题/动效是两处最容易静默降级的地方**：38×2 个语义色 + 5 accent + 3 preset + 3 density + 0.5px 边框 + 28 个 keyframes；`reduce-motion` 现在是 CSS 全局兜底，GPUI 需要在动画 API 层保证同样行为，否则无障碍验收会挂。
5. **窗口能力风险不在本文件**：透明 HUD、置顶、鼠标穿透、无边框拖拽、island 定位、首帧显示、拖入文件、DWM 圆角/背景色——全部由 Tauri 窗口 API 提供，换 gpui 后须由 `crates/desktop/src/shell` 用 Win32 自建，见 20 号文档。

---

## 附录 A：复现命令（PowerShell，仓库根目录 `D:\NTE_DPS_TOOL`）

> 说明：项目前端用 pnpm；本文件所有数字用 pwsh 统计，不依赖 node_modules。

```powershell
# A.1 文件与总行数
(Get-ChildItem frontend/src -Recurse -File | Group-Object Extension |
  ForEach-Object { "$($_.Name): $($_.Count)" })
(Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx,*.css |
  Get-Content | Measure-Object -Line).Lines      # 31232

# A.2 按目录（features/components/ui/components/nte/lib/hooks/entries/styles）
foreach ($d in 'features','components/ui','components/nte','lib','hooks','entries','styles') {
  $f = Get-ChildItem "frontend/src/$d" -Recurse -File -Include *.ts,*.tsx,*.css
  "$d files=$($f.Count) loc=$(($f | Get-Content | Measure-Object -Line).Lines)"
}
(Get-Content frontend/src/index.css | Measure-Object -Line).Lines   # 878

# A.3 逐 feature
foreach ($d in Get-ChildItem frontend/src/features -Directory) {
  $f = Get-ChildItem $d.FullName -Recurse -File -Include *.ts,*.tsx,*.css
  $t = $f | Where-Object Name -match '\.test\.'
  "$($d.Name) files=$($f.Count) loc=$(($f|Get-Content|Measure-Object -Line).Lines) tests=$($t.Count)"
}

# A.4 测试
$t = Get-ChildItem frontend/src -Recurse -File -Include *.test.ts,*.test.tsx
"$($t.Count) files / $(($t | Get-Content | Measure-Object -Line).Lines) lines"

# A.5 生成物
$g = Get-ChildItem frontend/src/lib/tauri/generated/*.ts
"$($g.Count) files / $(($g | Get-Content | Measure-Object -Line).Lines) lines"

# A.6 窗口配置
(Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).app.windows |
  Format-Table label,url,width,height,transparent,decorations,alwaysOnTop,skipTaskbar,visible

# A.7 Vite 多页输入（读文件即可）
Select-String frontend/vite.config.ts -Pattern 'html'
Get-ChildItem frontend/*.html | Select-Object Name

# A.8 依赖使用面
$pats = '@dnd-kit','from "motion','@jsonforms','components/ui/markdown','<canvas','requestAnimationFrame',
        'ResizeObserver','setTimeout','from "zustand"','lucide-react','@base-ui/react'
foreach ($p in $pats) {
  "$p = $((Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx |
    Select-String -Pattern $p | Measure-Object).Count)"
}
Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx |
  Select-String -Pattern '@base-ui/react/([a-z-]+)' -AllMatches |
  ForEach-Object { ($_.Matches | ForEach-Object { $_.Groups[1].Value }) -join ',' } |
  Sort-Object -Unique

# A.9 Tailwind 使用规模
$tsx = Get-ChildItem frontend/src -Recurse -File -Include *.tsx
($tsx | Select-String 'className=' | Measure-Object).Count                    # 1439
$cls = $tsx | Select-String -Pattern 'className="([^"]*)"' -AllMatches |
  ForEach-Object { $_.Matches | ForEach-Object { $_.Groups[1].Value } }
$tokens = $cls | ForEach-Object { $_ -split '\s+' } | Where-Object { $_ }
"literals=$($cls.Count) tokens=$($tokens.Count) unique=$(($tokens|Sort-Object -Unique).Count)"  # 1288/4863/793
foreach ($c in 'text-xs','text-sm','text-xl','text-2xl') {
  "$c = $(($tsx | Select-String -Pattern "\b$c\b" -AllMatches |
    ForEach-Object { $_.Matches.Count } | Measure-Object -Sum).Sum)"
}

# A.10 i18n
(Get-Content res/languages/zh-CN.json -Raw | ConvertFrom-Json -AsHashtable).Count   # 2468
(Get-Content res/languages/ja.json -Raw | ConvertFrom-Json -AsHashtable).Count      # 2468
$k = Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx |
  Select-String -Pattern '\b(?:t|tf)\(\s*"([^"]+)"' -AllMatches |
  ForEach-Object { $_.Matches | ForEach-Object { $_.Groups[1].Value } }
"callSites=$($k.Count) unique=$(($k|Sort-Object -Unique).Count)"                   # 701/500
Select-String src/storage/i18n.rs -Pattern 'pub fn (t|tf|t_for|tf_for)|fn format_template'
Get-Content scripts/test_i18n_source_coverage.ps1 | Select-Object -First 20

# A.11 主题 token
Select-String frontend/src/styles/theme.css -Pattern '^\s*--' |
  ForEach-Object { $_.Line.Trim() }
Select-String frontend/src/index.css -Pattern '@theme inline|@keyframes|reduce-motion'

# A.12 分类合计自检（把 §4.2/4.4/4.5 的行数录入后）
# A+B+C+D = 2518+2035+18061+8618 = 31232；文件数 23+20+44+318 = 405

# A.13 Tauri 命令覆盖（Rust 注册 vs 前端字面量）
$names = @()
$files = Get-ChildItem src-tauri/src -Recurse -File |
  Where-Object { $_.Extension -eq '.rs' -and $_.FullName -notmatch '\\target\\' }
foreach ($f in $files) {
  $lines = Get-Content $f.FullName
  for ($i=0; $i -lt $lines.Count; $i++) {
    if (-not $lines[$i].Contains('#[tauri::command]')) { continue }
    for ($j=$i+1; $j -le [Math]::Min($i+5,$lines.Count-1); $j++) {
      if ($lines[$j] -match 'pub(?:\(crate\))? (?:async )?fn ([a-z0-9_]+)') { $names += $matches[1]; break }
    }
  }
}
$rust = @($names | Sort-Object -Unique)                    # 144
$front = ((Get-ChildItem frontend/src -Recurse -File -Include *.ts,*.tsx |
  Get-Content -Raw) -join "`n")
@($rust | Where-Object { $front.IndexOf('"' + $_ + '"') -ge 0 }).Count   # 144
```

## 附录 B：范围外但相关的旁证

- `frontend/dev/`（6 文件 1,542 行：fixtures 442、ui-preview 395、user-character-fixture 192、hit-snapshot-fixture 200、ui-foundation-preview 156、tsconfig 7）不在生产构建输入；gpui 重构后浏览器预览失去意义，随测试一起删，但 fixtures 中的数据结构可作为 GPUI demo 数据参考。
- `package.json` 依赖共 20 个运行时依赖；换 gpui 后预计删除：`@base-ui/react`、`@dnd-kit/*`、`@fontsource-variable/geist`（字体改 Rust 加载）、`@jsonforms/*`、`@tauri-apps/api`、`class-variance-authority`、`clsx`、`lucide-react`（需导出 80 图标）、`motion`、`react`、`react-dom`、`shadcn`、`tailwind-merge`、`tailwindcss`、`tw-animate-css`、`zustand`——共 16 个包（详见 20/30 号文档的依赖处置）。
