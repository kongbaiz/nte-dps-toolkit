# NTE DPS Toolkit 交互系统优化方案

> 版本：v1.0 · 2026-07-10
> 适用代码基线：`master` @ 91bbfb3（egui/eframe 0.34.3 + wgpu，多视口架构）
>
> 本文档给出一套 **模块化、可组合搭配** 的交互系统优化方案：4 条轨道（动效 / 视觉 / 布局 / 交互），
> 每条轨道分 **Lite（低成本收敛）** 与 **Pro（深度定制）** 两档，可按需自由拼装，文末给出三个推荐套餐与路线图。

---

## 目录

1. [现状盘点](#1-现状盘点)
2. [问题诊断](#2-问题诊断)
3. [设计原则](#3-设计原则)
4. [方案模块](#4-方案模块)
   - [M1 动效系统](#m1-动效系统)
   - [M2 视觉与主题系统](#m2-视觉与主题系统)
   - [M3 布局与信息架构](#m3-布局与信息架构)
   - [M4 交互模型](#m4-交互模型)
5. [组合套餐](#5-组合套餐)
6. [关键实现要点（egui 0.34）](#6-关键实现要点egui-034)
7. [性能与风险](#7-性能与风险)
8. [路线图](#8-路线图)
9. [参考资料](#9-参考资料)

---

## 1. 现状盘点

### 1.1 信息架构（已定型，本方案不推翻）

| 窗口（视口） | 职责 | 代码位置 |
|---|---|---|
| 主窗口（root） | 战斗实时读数：汇总条 + 队伍排行；可切换为无背景 Combat HUD 覆盖在游戏上 | `src/app/main_view.rs`、`src/app/hud.rs` |
| Console 窗口 | 设置与工具页签：设置 / 时间轴 / 技能 / 空幕 / 历史 / 角色 / 加密 INI / 封包 / 资源 / 诊断（后三个 Debug 构建限定） | `src/app/console_view.rs` |
| 深渊总览窗口 | 深渊上下行线统计、怪物数值、预测 | `src/app/abyss.rs` |
| 命中明细 / 队伍明细窗口 | 可筛选命中列表 | `src/app/hit_detail.rs`、`src/app/detail_panels.rs` |

既定原则：**主窗口 = 实时读数**；**Console = 设置与工具集**；**深渊是上下文（context）而非模式（mode）**——深渊激活时主窗口进入紧凑态，而不是切换成另一个界面。本方案全部模块都在此框架内做增强。

### 1.2 视觉系统

- 主题基调为 shadcn/ui 的 Zinc 中性色系（`src/app/theme.rs`），深浅双主题，语义色（success/warning/danger）成对定义。
- 强调色 `theme_accent` 目前是**单色**（深色下近白、浅色下近黑），无彩色 accent。
- 自绘窗口框（无系统标题栏），透明圆角 + wgpu DirectComposition 合成（`vendor/egui-winit-0.34.3` 补丁 + `Dx12SwapchainKind::DxgiFromVisual`）。
- HUD 模式有专门的可读性处理：`paint_haloed` 六向描边光晕文字（`hud.rs:5`）。

### 1.3 既有动效清单

| 动效 | 时长 | 位置 |
|---|---|---|
| 指标卡 hover 背景/描边渐变 | 0.14s | `theme.rs:39` |
| 行 hover | 0.12s | `main_view.rs:1107` |
| 控制条展开/收起（深渊紧凑态） | 0.22s + `ease_out_cubic` 高度裁剪 + 透明度 | `main_view.rs:326` |
| 深渊上下半场切换 | 0.22s 横向位移 14px + 透明度 | `main_view.rs:359` |
| 伤害占比条份额过渡 | `animate_value_with_time` | `main_view.rs:1131` |
| 主题切换背景色过渡 | 手写 `theme_transition_from` + 时间戳 | `main_view.rs:305` |

结论：**已经有动效意识，但都是散点式硬编码**——时长 0.12 / 0.14 / 0.22 三种值分散各处，easing 只有 `ease_out_cubic` 一个，没有统一的 token 层。

### 1.4 交互与持久化

- 快捷键：`Home`（可换 Insert/F8/F9）切换鼠标穿透；Debug 构建 `F12` 开关 Debug 面板。
- 状态提示：4 秒 toast（`STATUS_TOAST_DURATION`），状态文本按关键词判定语义色（`theme.rs:105`）。
- 持久化：透明度、主题、置顶、各窗口尺寸、HUD 模块开关（三档预设：极简/标准/详细）等存 `config.json`，350ms 防抖写盘。
- 每帧 UI 事件预算（4ms / 2048 条）、滚动时降级——性能护栏已经比较完善。

---

## 2. 问题诊断

按影响面排序：

| # | 问题 | 现象 | 涉及模块 |
|---|---|---|---|
| P1 | **动效无系统** | 时长/easing 硬编码散落，新增动效只能拍脑袋；快慢感不一致 | M1 |
| P2 | **Console 页签过平** | 一排最多 10 个页签（含 Debug），"设置"和"加密 INI 编辑器"并列，使用频率差两个数量级；找功能靠扫一遍 | M3、M4 |
| P3 | **数字跳变** | DPS/总伤害等核心读数每次刷新直接跳变，战斗中高频闪动，视觉噪声大且难以感知趋势 | M1 |
| P4 | **强调色缺彩** | 单色 accent 使"当前激活/可点击/强调"层次只能靠明度差表达；角色色是彩色的，与框架的黑白灰之间缺少过渡层 | M2 |
| P5 | **密度单一** | 行高、字号、间距一套值；HUD 宽度固定 380px；复盘时想要更密的表格、挂机看数时想要更大的字，都做不到 | M3 |
| P6 | **键盘可达性弱** | 除穿透热键外，开始/停止抓包、切窗口、切页签均需鼠标；HUD 穿透态下想操作必须先解除穿透 | M4 |
| P7 | **空态/加载态朴素** | 导入回放有 loading 卡片，但无进度感；无战斗数据时的主窗口缺少引导（下一步该干什么） | M1、M4 |
| P8 | **动效不可关** | 无 reduced-motion 开关；透明置顶窗口常驻游戏画面旁，任何多余重绘都是 GPU/功耗成本 | M1、M7（性能） |

---

## 3. 设计原则

参考游戏 HUD 设计与桌面工具设计的通行准则（见[参考资料](#9-参考资料)），为本工具定五条原则，后续所有模块围绕它们展开：

1. **战斗态一瞥可读（Glanceability first）**
   战斗中用户视线在游戏画面上，扫一眼工具只有 0.3~1 秒。核心读数（团队 DPS、时长、排行）必须在任何主题/透明度下高对比、无遮挡、无动画干扰。动效只用于"状态变化的确认"，绝不用于"吸引注意"。
2. **动效为信息服务（Motion = meaning）**
   每个动画必须回答"它帮用户理解了什么"：数值滚动表达趋势方向，条形过渡表达份额变化，滑入方向表达上下半场的空间关系。装饰性动画一律不做。
3. **密度分层（Density tiers）**
   战斗（HUD）→ 常规（主窗口）→ 复盘（Console/明细）是三种越来越"坐下来看"的场景，信息密度、字号、行高应有三套档位而非一套。
4. **键盘与热键优先（Hands-on-game）**
   用户双手在键鼠上打游戏，工具的高频操作（穿透、重置、暂停、切半场）都应可以不离开游戏完成。
5. **性能即体验（Perf budget）**
   透明置顶窗口 + 实时解析流，动画引起的额外重绘必须有预算：无交互、无数据变更时零重绘；所有动效可全局降级/关闭。

---

## 4. 方案模块

> 每个模块给出 Lite / Pro 两档。**同轨道内 Pro 包含 Lite**；跨轨道自由组合，依赖关系在[组合套餐](#5-组合套餐)注明。
> 工作量估算按"熟悉本仓库的单人开发"计。

---

### M1 动效系统

#### M1-Lite：动效 Token 收敛（约 2~3 天）

把散落的硬编码收敛为一个 `src/app/motion.rs` 模块，不改变任何现有视觉效果，只建立系统：

```rust
//! Motion tokens. 参考 Carbon（productive/expressive 双速）与 M3（duration scale）。
pub mod dur {
    /// 微交互：hover、按下、选中态。用户几乎无感知，只觉得"跟手"。
    pub const FAST: f32 = 0.10;
    /// 标准：控件展开、页签内容切换、份额条过渡。
    pub const BASE: f32 = 0.18;
    /// 强调：面板进出、半场切换、模式切换（HUD ↔ 窗口）。
    pub const SLOW: f32 = 0.28;
}

pub mod ease {
    /// 元素全程可见（位移、变宽）：标准缓动。
    pub fn standard(t: f32) -> f32 { /* cubic-bezier(0.2, 0, 0.38, 0.9) 近似 */ }
    /// 入场（出现、展开）：减速曲线，现 ease_out_cubic 迁移至此。
    pub fn entrance(t: f32) -> f32 { 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3) }
    /// 出场（消失、收起）：加速曲线。
    pub fn exit(t: f32) -> f32 { t.clamp(0.0, 1.0).powi(3) }
}
```

- 全仓库替换：`0.12/0.14` → `dur::FAST`，`0.22` → `dur::BASE`（半场切换可升 `SLOW`），`ease_out_cubic` 移入 `ease::entrance`。
- 新增 **`reduce_motion: bool`** 到 `UiConfig`（设置页一个开关）：为 true 时所有 `dur::*` 归零（egui 的 `animate_*_with_time` 传 0 即瞬时完成），一举解决 P8 与低配机顾虑。
- 收益：P1 解决；之后所有 Pro 档动效都建立在这层之上。

#### M1-Pro：深度动效定制（约 1.5~2 周，依赖 M1-Lite）

1. **数值滚动（rolling counter）** —— 解决 P3
   核心读数（团队 DPS、总伤害）不再跳变，而是用 `animate_value_with_time(id, target, dur::BASE)` 做指数趋近式滚动；配合小型 **趋势标记**（▲/▼ 半透明箭头，0.8s 后淡出）表达升降方向。
   注意：滚动只用于"汇总类"数字；命中明细列表里的数字**不做**动画（高频插入，动画反而是噪声）。
2. **排行榜位次交换动画**
   队伍排行的行在位次变化时做 y 位移过渡（对每个 `char_id` 用 `animate_value_with_time` 记忆其目标 y，绘制在动画位置上）。魔兽 Details!、FF14 ACT 悬浮窗等成熟伤害统计的标志性动效，位次变化从"闪一下"变成"看得见的超越"。
   已有 `main_view.rs:606` 的 `animated_y` 雏形，此项是将它推广到全排行。
3. **份额条动效统一升级**
   占比条从左向右生长 + 尾端 2px 高亮渐隐（表达"新增伤害在哪"）；同色份额变化用 `ease::standard`。
4. **窗口/面板进出场**
   - Console、深渊、明细窗口打开时内容层 8px 上移淡入（`dur::SLOW` + `ease::entrance`）；
   - 关闭无动画（桌面工具惯例：出场快于入场，甚至省略）。
   - HUD ↔ 窗口模式切换：背景/边框透明度与标题栏高度联动过渡，替代当前的瞬切。
5. **战斗状态转场**
   战斗开始：汇总条底色一次 300ms 的 accent 淡入淡出脉冲（仅一次，不循环）；战斗结束（分段切割）：时长数字定格 + 轻微放大回弹（`1.0 → 1.06 → 1.0`）。给"这一段结束了"一个明确的视觉句号。
6. **toast 体系动效**
   toast 从右下 12px 滑入、悬停暂停倒计时、点击立即消失；多条堆叠时向上顶。配合 M4 的 undo 型 toast。

---

### M2 视觉与主题系统

#### M2-Lite：Token 化 + 彩色 Accent（约 3~4 天）

1. **`theme.rs` 语义 token 重排**
   现有函数式取色（`shadcn_card(dark_mode)` 等）已接近 token，补齐缺口并归档成一张表：
   `bg / bg-elevated / card / card-hover / border / border-strong / fg / fg-muted / fg-faint / accent / accent-fg / success / warning / danger / info`。
   所有 UI 代码只允许引用 token，禁止内联 `Color32::from_rgb`（现存内联色约十余处，一次清理）。
2. **可选彩色 accent** —— 解决 P4
   `UiConfig` 增加 `accent: AccentColor`（枚举：Zinc 默认 / Blue / Violet / Orange / Green，各配深浅两版并保证与 `contrast_text` 的对比度）。accent 用于：主按钮、选中页签底条、滑杆、当前激活筛选、脉冲动效。中性灰保持为框架色，彩色只做"当前焦点"一层——层次立刻拉开且不干扰角色色。
3. **数据可视化色板**
   角色 fallback 色、时间轴曲线、技能占比图共用一套 8 色分类色板（深浅主题各一组，参考 dataviz 惯例：亮度对齐、色相均布、色盲友好）。替换当前 hash 出来的随机 fallback 色。

#### M2-Pro：多主题预设 + 材质层（约 1~1.5 周，依赖 M2-Lite）

1. **主题预设（Theme presets）**
   在深/浅二元之上提供 3 套预设，本质是 token 表的不同实例：
   - **Zinc**（现状，默认）——克制、桌面工具感；
   - **Tactical**——近黑底 + 高饱和霓虹 accent（青/品红），HUD 模式下与游戏科幻 UI 融合度更高；
   - **High Contrast**——纯黑白 + 加粗描边 + 语义色提亮，服务低视力与强光环境。
   预设选择持久化到 `config.json`，token 层（M2-Lite）保证切换是 O(1) 换表。
2. **材质层（Surface elevation）**
   利用已有的 DirectComposition 透明合成，定义三级表面：
   - L0 窗口底：半透明（现 opacity 滑杆控制）；
   - L1 卡片：不透明度 +8%，1px 边框；
   - L2 浮层（菜单/popup/toast）：不透明 + 阴影 + 边框提亮。
   效果上接近 Fluent 的 Acrylic 层级但**不做实时高斯模糊**（成本高且 wgpu 下需要离屏 pass，见 §7）。
3. **HUD 专用视觉档**
   HUD 模式下切换到专用 token 子集：文字全部走 `paint_haloed`、份额条加 1px 深色描边、语义色提高饱和度一档——保证叠加在任何游戏画面上可读（对应游戏 HUD 设计中"对比度必须在复杂 3D 背景上存活"的准则）。
4. **图标体系**
   引入一套线性图标（建议 Lucide，shadcn 同源，MIT 协议，SVG 转 egui `Image` 或路径绘制），替代当前纯文字按钮：标题栏、控制条、页签配 16px 图标。文字+图标双通道，也为 M3 的窄宽度自适应（窄时只留图标）打基础。

---

### M3 布局与信息架构

#### M3-Lite：页签分组 + 密度档（约 4~5 天）

1. **Console 页签两级化** —— 解决 P2
   一排 10 个平级页签改为**分组侧栏**（左侧 160px，可折叠成 48px 图标栏）：
   - **常用**：设置、历史
   - **复盘**：时间轴、技能、空幕
   - **高级**（Debug 构建再多三项）：角色编辑、加密 INI、封包、资源、诊断
   分组标题小写弱化，当前项 accent 底条。窄窗口（< 720px）自动折叠为图标栏。
   实现要点：`ConsoleTab` 枚举不动，只换绘制结构，各 `*_contents` 函数原样复用。
2. **密度三档（Compact / Cozy / Comfortable）** —— 解决 P5
   `UiConfig` 增加 `density` 枚举，映射为一组 spacing token（行高、`item_spacing`、字号缩放 0.9/1.0/1.15）。主窗口与明细列表读取该 token；HUD 不受影响（有自己的字号逻辑）。
   实现要点：集中在一个 `apply_density(style: &mut egui::Style)`，在每视口帧首应用。
3. **响应式断点**
   主窗口按宽度三档：< 420px 隐藏次要列（受击、命中数）只留排行；420~560px 当前布局；> 560px 汇总条横向展开为 4~6 个指标卡。窗口已可自由拉伸（`window_resize_grips`），断点让拉伸真正"有含义"。

#### M3-Pro：模块化布局 + 布局 Profile（约 2~3 周，依赖 M3-Lite）

1. **HUD 模块编辑器**
   现在 HUD 模块是设置页里的一排 checkbox（`HudConfig` 9 个开关 + 3 预设）。升级为**所见即所得编辑**：进入"HUD 编辑模式"后，HUD 每个模块显示虚线框，可拖拽上下排序、右键隐藏；宽度改为可拖拽调整（替代固定 `HUD_WINDOW_WIDTH = 380`），并持久化 `hud.module_order: Vec<HudModule>` 与 `hud.width`。
   对应游戏 HUD 设计准则："玩家应能自定义所有 HUD 元素的大小与位置"。
2. **布局 Profile（场景预设）**
   把「窗口开关 + 各窗口位置尺寸 + HUD 配置 + 密度」打包为可命名 Profile，内置三个：
   - **战斗**：仅 HUD（极简），穿透默认开；
   - **复盘**：主窗口 + Console（时间轴页）并排；
   - **研究**：Console（封包页）+ 明细窗口。
   热键或命令面板（M4-Pro）一键切换。实现上是 `UiConfig` 的快照子集，切换时批量应用视口指令。
3. **明细窗口列自定义**
   命中明细表列可右键显隐、拖宽，持久化。复盘场景最高频的表格操作补齐。

---

### M4 交互模型

#### M4-Lite：热键补全 + 反馈闭环（约 4~5 天）

1. **热键体系扩展** —— 解决 P6
   在现有 `PassthroughHotkey` 基础上扩为可配置热键表（全局注册沿用 `platform/hotkey.rs` 的 Win32 实现）：
   | 动作 | 默认键 | 说明 |
   |---|---|---|
   | 鼠标穿透 | Home（现状） | 不变 |
   | 开始/停止抓包 | Ctrl+F9 | 战斗前后免鼠标 |
   | 重置会话 | Ctrl+F10 | 带 undo toast，见下 |
   | HUD ↔ 窗口模式 | Ctrl+F11 | |
   | 深渊上/下半场切换 | Tab（仅窗口聚焦时） | 局部热键走 egui 输入 |
   设置页做成"点击录制"式改键（复用角色编辑器的组合框宽度规范）。
2. **Undo 型 toast**
   重置会话、删除历史记录等破坏性操作改为"先执行 + 5 秒内可撤销"的 toast（保留被清数据的快照直到超时），替代部分二次确认弹窗——高频操作从"每次多点一下"变成"想反悔才多点一下"。低频高危操作（清空全部历史）保留确认框。
3. **空态设计** —— 解决 P7
   - 主窗口无数据：居中显示三步引导（① 启动游戏 → ② 开始抓包 → ③ 进入战斗），每步实时打勾（检测到 `HTGame.exe` / 抓包中 / 有伤害事件），把"自动诊断向导"的结论前置到空态。
   - 明细/历史/技能页空态：一句话说明 + 指向对应操作的按钮。
4. **Tooltip 与状态可见性统一**
   - 所有仅图标控件必须有 `on_hover_text`（现状大部分有，补漏 + lint 惯例写入 `AGENTS.md`）；
   - 穿透状态在 HUD 上的提示从可选行升级为：切换瞬间必显 1.2s 大号提示（"已穿透 · Home 恢复"），常态可隐藏——解决"开了穿透忘了怎么关"的经典问题。

#### M4-Pro：命令面板 + 引导体系（约 1.5~2 周，依赖 M4-Lite；命令面板建议搭配 M2-Lite 图标）

1. **命令面板（Ctrl+K）**
   全局浮层，模糊搜索并执行所有动作：切页签、开窗口、开始/停止抓包、切主题/密度/Profile、导入导出、打开日志目录……
   实现要点：定义 `Command { id, title_key, category, hotkey, action }` 注册表；面板本身是 egui `Area` + `TextEdit` + 过滤列表，↑↓ 选择、Enter 执行；动作复用现有方法。10 个页签 + 4 个窗口 + 20 余个操作的长尾功能从此都有 2 秒直达路径，也天然成为热键的"可发现性"入口（每条命令右侧显示其热键）。
2. **右键上下文菜单**
   - 排行行：筛选该角色明细 / 复制数值 / 隐藏该角色；
   - 历史记录：对比（自动选中上一条选择项）/ 导出 / 删除；
   - 时间轴：在此处添加标记 / 缩放到选区。
   egui `Response::context_menu` 原生支持，成本主要在梳理每处"就地可做的事"。
3. **首次运行引导（一次性）**
   首启四步卡片式引导：Npcap 检查 → 网卡自动选择说明 → 穿透热键演示 → HUD 预设选择。写 `config.json` 的 `onboarding_done` 标记。与 M4-Lite 空态引导互补：引导讲"这工具怎么用"，空态讲"现在该做什么"。
4. **键盘导航兜底**
   Console 内 Ctrl+PgUp/PgDn 切页签；表格支持 ↑↓ 移动选中行、Enter 展开详情。egui 0.34 的 accesskit 特性已启用，此项同时改善读屏可达性。

---

## 5. 组合套餐

### 套餐一「顺手」—— 全 Lite（约 2.5~3.5 周）

> M1-Lite + M2-Lite + M3-Lite + M4-Lite

不改变产品形态，把现有交互全面"收紧"：动效统一、accent 提层、页签分组、密度档、热键补全、空态引导。**风险最低、单位收益最高**，任何后续档位的公共地基。适合作为下一个版本（v0.3）的交互主题。

### 套餐二「精致」—— 动效与反馈优先（约 5~6 周）

> 套餐一 + M1-Pro + M4-Pro

判断依据：本工具的核心场景是**盯数字**（战斗）与**找功能**（复盘/调试），M1-Pro（数值滚动、位次交换、战斗转场）直接优化前者，M4-Pro（命令面板、右键菜单）直接优化后者；而 M2-Pro/M3-Pro 更多是锦上添花。**这是综合推荐档**。

### 套餐三「旗舰」—— 全 Pro（约 9~11 周）

> 全部 8 个模块

补上多主题预设、材质层、图标体系、HUD 拖拽编辑器、布局 Profile。适合把工具当长期产品运营、且用户群体扩大到需要个性化（主题/布局）的阶段。可在套餐二交付后按 M2-Pro → M3-Pro 顺序增量推进。

### 自由搭配矩阵

| 模块 | 前置依赖 | 可独立交付 | 对其他模块的增益 |
|---|---|---|---|
| M1-Lite | 无 | ✅ | 所有动效类需求的地基 |
| M1-Pro | M1-Lite | ✅ | 战斗转场为 M3-Pro Profile 切换提供转场 |
| M2-Lite | 无 | ✅ | accent 被 M3 分组侧栏、M4 命令面板使用 |
| M2-Pro | M2-Lite | ✅ | 图标体系被 M3 折叠侧栏、M4 面板使用 |
| M3-Lite | 无（建议先 M2-Lite） | ✅ | 密度 token 被 M3-Pro 列自定义复用 |
| M3-Pro | M3-Lite | ✅ | Profile 切换建议接入 M4-Pro 命令面板 |
| M4-Lite | 无 | ✅ | undo toast 复用 M1 toast 动效（可先无动效交付） |
| M4-Pro | M4-Lite | ✅ | 命令面板是所有功能的兜底入口 |

最小可感知组合（如果只有一周）：**M1-Lite + M4-Lite 的热键扩展 + M3-Lite 的页签分组**——用户每天都碰的三个面。

---

## 6. 关键实现要点（egui 0.34）

### 6.1 动效底座

- 一律使用 `ctx.animate_bool_with_time(id, target, dur)` / `ctx.animate_value_with_time(id, target, dur)`，**不要**自己攒 `Instant` 时间戳（现有 `theme_transition_from` 手写计时是历史遗留，M1-Lite 时一并迁移）。egui 会在动画进行期间自动 `request_repaint`，动画结束自动停——天然满足"无变化零重绘"。
- easing：egui 的 `animate_bool_with_easing` 可传曲线；`animate_value_with_time` 是线性趋近，需要曲线时先动画 0→1 的 progress 再自己映射（现 `animated_controls` 的写法即范式，抽成 `motion::animate_progress(ui, id, expanded, dur, ease)` 助手）。
- `reduce_motion` 实现：`dur` 统一经过 `motion::scaled(dur)`，内部读全局开关返回 `0.0` 或原值，一处生效。
- 数值滚动的 id 用 `("rolling", metric_key)`，避免与 hover 动画的 id 冲突；角色行位次动画 id 必须绑定 `char_id` 而非行索引，否则换位时动画对象错乱。

### 6.2 多视口注意事项

- 每个 `show_viewport_immediate` 闭包里的 `ui.ctx()` 与 root 共享动画状态存储，token 化后各窗口动效速度自然一致，无需分别配置。
- 布局 Profile 切换涉及批量 `ViewportCommand`（位置/尺寸/可见性），要沿用现有 `ViewportBuilder::patch` 的注意点（chrome.rs:466 注释）：避免重复发 `InnerSize` 导致用户手动拉伸被回弹。

### 6.3 性能护栏

- 战斗中本来就在持续收事件、持续重绘，动效边际成本≈0；**空闲时**是关键：确认所有循环型效果（如脉冲）只由事件触发单次播放，不做常驻循环动画（呼吸灯之类一律不做）。
- 材质层不引入实时模糊。若未来确需毛玻璃，方案是 Windows 的 `DwmEnableBlurBehindWindow`/`ACCENT_ENABLE_ACRYLICBLURBEHIND` 系统级背板（窗口级、零 per-frame 成本），而不是 wgpu 离屏模糊 pass。
- 列表动效（位次交换）只对**可见行**计算；明细列表沿用现有滚动降级策略，动效在 `is_scrolling` 时旁路。

### 6.4 持久化与迁移

新增 `UiConfig` 字段全部 `#[serde(default)]`，老配置无损升级（仓库已有惯例，如 `main_window_size`）。Profile 建议存独立 `profiles.json`，避免 config.json 越写越频。

---

## 7. 性能与风险

| 风险 | 等级 | 缓解 |
|---|---|---|
| 动效增加空闲重绘、GPU 占用上升 | 中 | §6.3 护栏；`reduce_motion` 全局开关；交付前用 PresentMon 对比空闲帧率 |
| 透明窗口 + 驱动兼容性（历史上 glow 后端有 NVIDIA 上下文丢失崩溃，已迁 wgpu 规避） | 低 | 不回退渲染后端；材质层改动仅调 alpha，不动交换链配置 |
| Console 侧栏改版打断老用户肌肉记忆 | 中 | 分组顺序保持原页签顺序；首次进入显示一次性"页签搬到左边了"提示 |
| 热键与游戏按键冲突 | 中 | 全局热键默认都带修饰键（Ctrl+Fx）；提供改键与一键禁用 |
| 图标/字体引入增大二进制 | 低 | 只打包用到的 Lucide 子集（SVG 编译期转路径）；`opt-level="z"` 下评估增量 |
| i18n 回归（新增大量 UI 文案） | 中 | 所有新文案走 `t()/tf()`；命令面板搜索需同时匹配中英文标题 |

---

## 8. 路线图

以**套餐二「精致」**为目标的建议排期（可随时在阶段边界收束）：

| 阶段 | 内容 | 产出 | 预估 |
|---|---|---|---|
| Phase 0 | M1-Lite：motion.rs + token 替换 + reduce_motion | 不可见的地基版本，回归测试动效一致性 | 3 天 |
| Phase 1 | M2-Lite + M3-Lite：token 表、彩色 accent、页签分组、密度档、断点 | v0.3 候选：界面焕新可感知 | 1.5 周 |
| Phase 2 | M4-Lite：热键表、undo toast、空态、穿透提示 | v0.3 正式：操作效率提升 | 1 周 |
| Phase 3 | M1-Pro：数值滚动、位次动画、战斗转场、toast 动效 | v0.4 候选：战斗观感质变 | 1.5~2 周 |
| Phase 4 | M4-Pro：命令面板、右键菜单、首启引导、键盘导航 | v0.4 正式 | 1.5~2 周 |
| Phase 5（可选） | M2-Pro → M3-Pro 增量 | v0.5+：主题预设、HUD 编辑器、Profile | 3~4 周 |

每阶段验收标准建议：① 空闲（无战斗、无鼠标交互）时 1 分钟内重绘次数不高于改版前；② `reduce_motion` 开启后所有新动效瞬时完成；③ 中英文两种语言下无布局溢出。

---

## 9. 参考资料

**动效系统（token 化方法论）**
- [Material Design 3 — Easing and duration tokens](https://m3.material.io/styles/motion/easing-and-duration/tokens-specs)：duration 分档与物理弹簧动效规范
- [Carbon Design System — Motion](https://carbondesignsystem.com/elements/motion/overview/)：productive/expressive 双速模型、entrance/exit 曲线的场景划分（M1 token 三曲线的直接出处）
- [Motion Design System — A Practical Guide](https://medium.com/@aviadtend/motion-design-system-practical-guide-8c15599262fe)、[Motion Design System Explained](https://dev.to/uianimation/motion-design-system-explained-what-it-is-why-you-need-one-and-how-to-build-it-for-scalable-ui-41fa)：时长 token 命名与落地步骤
- [Designing Systems — 5 steps for including motion](https://www.designsystems.com/5-steps-for-including-motion-design-in-your-system/)

**游戏 HUD / 覆盖层可读性**
- [Accessible Game Design — HUD Guidelines](https://accessiblegamedesign.com/guidelines/HUD.html)：HUD 元素可自定义大小位置、对比度、隐藏非关键信息（M3-Pro HUD 编辑器与 M2-Pro HUD 视觉档的依据）
- [7 obvious beginner mistakes in your game HUD](https://bootcamp.uxdesign.cc/7-obvious-beginner-mistakes-with-your-games-hud-from-a-ui-ux-art-director-d852e255184a)：视线中心、按需显隐
- [Sunstrike Studios — HUD design in games](https://sunstrikestudios.com/en/blog/HUD_design_in_games/)、[Game UX Design Guide (UXPin)](https://www.uxpin.com/studio/blog/game-ux/)：信息优先级与一瞥可读原则

**egui 实现**
- [egui Context docs — animate_bool / animate_value 系列](https://docs.rs/egui/latest/egui/struct.Context.html)
- [egui_animation crate](https://docs.rs/egui_animation)：位置动画（列表换位）与循环动画参考实现（位次交换动画可参考其思路，不必引依赖）
- [egui style.rs — 全局 animation_time](https://github.com/emilk/egui/blob/main/crates/egui/src/style.rs)

**视觉体系**
- [shadcn/ui](https://ui.shadcn.com/)（现有 Zinc 色系来源）与 [Lucide Icons](https://lucide.dev/)（M2-Pro 图标体系候选，MIT/ISC 协议）
