## 2026-08-01 第三十步：诊断页面完整迁移

- `support::diagnostics` 提升为 `core::diagnostics`，旧 egui 与 Tauri 复用同一检查规则；
- Tauri 新增版本 1 Diagnostics DTO、运行/导入/导出命令和 500 ms 变更驱动 Channel；
- Console 侧栏启用“诊断”，React 一次接入环境、运行检查、解析质量、脱敏报告和文件操作；
- PCAPNG/JSON 导入复用现有 engine/reducer；最近一次完整原始抓包可经原生位置窗口另存；
- Channel 观察抓包、封包和报告代次，数据变化主动更新，隐藏页释放订阅；
- 页面不接收原始载荷、本机完整路径或内部错误链，简体中文和日文已补齐；
- 完整边界与人工验收见 `docs/TAURI_MIGRATION_PHASE30.md`。

## 2026-08-01 第二十九步：资源页面完整迁移

- `support::resource_audit` 提升为 `desktop/gui` 共用的 `core::resource_audit`，旧 egui 与
  Tauri 复用同一运行资源审计规则；
- Tauri 新增版本 1 Resources DTO 和 blocking worker 命令；扫描只检查软件自身 `res/`，
  不访问游戏导出路径、资源 key 或外部维护工具；
- Console 侧栏启用“资源”，React 页面一次接入刷新、脱敏报告复制、四组摘要、严重级别/
  分类筛选和响应式缺口列表；
- 首次 loading、错误、资源完整、筛选无结果均有独立状态；刷新保留旧结果，隐藏页不轮询，
  返回时保留快照；
- 审计消息在 Tauri 边界映射为稳定 key/参数，简体中文与日文已补齐；TypeScript 对数量、
  枚举和文本边界执行运行时校验；
- 完整边界与人工验收见 `docs/TAURI_MIGRATION_PHASE29.md`。

# Tauri + React 迁移交接

## 2026-08-01 第二十八步：封包页面完整迁移

- 新增 `core::packets` 有界投影，保留旧页展示字段，原始 payload、hex 和 preview 不进入前端；
- 实时抓包复用可靠/调试双有界事件队列，并增加封包专用 generation，不触发无关页面重复投影；
- Tauri 新增版本 1 Packets DTO、完整快照命令和 100 ms 增量 Channel；
- Console 侧栏启用“封包”，React 页面一次接入搜索、仅命中筛选、统计、自动跟随和逐包展开；
- 页面最多保留最新 500 条，隐藏后释放订阅，重新显示时从 Rust 权威快照恢复；
- loading、empty、filtered-empty、error 和流中断均已覆盖，错误浮层不挤占列表；
- 简体中文与日文已补齐新文案，完整边界和人工验收见 `docs/TAURI_MIGRATION_PHASE28.md`。

## 2026-08-01 第二十七步：加密 INI 页面完整迁移

- `core::encrypted_ini` 统一 AES、PKCS#7、key、记录保留、行尾和原子写入，旧 egui 继续复用；
- Tauri 新增 generation 会话、版本化 DTO、五个 typed command 和原生 INI 文件选择窗口；
- Console 侧栏启用“加密 INI”，React 页面一次接入打开、编辑、搜索、保存、重载和清空；
- 搜索支持循环上一个/下一个并选中文本，脏草稿在重载或清空前使用顶部确认浮层；
- 取消文件选择不会覆盖草稿，未修改保存复用原始密文，8 MiB 上限和稳定错误已覆盖；
- 简体中文与日文已补齐新文案，完整边界和人工验收见 `docs/TAURI_MIGRATION_PHASE27.md`。

## 2026-08-01 第二十六步：角色数据页面完整迁移

- 新增 `core::character_data`，集中角色表读取、字段校验、未知字段保留、原子写入和写后重载；
- Tauri 新增版本化角色数据 DTO、快照命令和单记录保存命令，文件错误对前端只暴露稳定 code/key；
- Console 侧栏启用“角色数据”，页面通过 typed client 完成新增、搜索、编辑、保存、取消和重新载入；
- 头像、验证状态、属性、颜色预览和响应式双栏布局已迁移，窄窗口自动切换上下布局；
- 未保存草稿阻止切换和重新载入；成功与错误提示使用顶部居中浮层，不挤占页面空间；
- loading、search-empty、load-error、保存错误、契约边界和纯前端筛选均有自动化覆盖；
- 简体中文与日文已补齐新文案，保存后的实时抓包角色映射仍按旧语义在下次启动时更新；
- 本阶段边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE26.md`。

## 2026-08-01 第二十五步：空幕页面功能接入（两步迁移的第 2 步）

- 新增 `core::empty_curtain`，旧 egui 与 Tauri 共用一键配装和 Drive Calculator 导出规则；
- Tauri 新增版本化空幕 DTO、完整快照命令和 100 ms 变更驱动 Channel；
- React 已移除空幕预览 fixture，使用 typed client 校验并订阅 Rust 权威库存投影；
- 页面接入原生导入/导出对话框、角色配装、单件锁定/弃置/装备/卸装和兼容位置查询；
- Plugin 操作状态通过 Channel 回传，错误使用顶部居中提示，不挤占或刷新页面内容；
- 副属性按 Rust 投影的 `+5 / +10 / +15 / +20` 强化门槛展示，且只在尚未解锁时显示锁和门槛；
- Canvas 未解锁副词条的锁图标固定为统一右侧列，详情中的弃置操作补齐简中和日文本地化；
- Canvas 卡片按六行属性预算绘制，驱动块的两条主词条和四条副词条均完整展示，不再截断第二条主词条；
- 筛选改为浏览器顶层模态浮窗，打开时不占用或推动装备页面空间；
- 筛选角色改用 Rust 快照中的本地化名称；角色与卡带支持名称搜索，动态选项分组限高滚动，
  新增主词条 OR 筛选并保留副词条全包含语义，可承载后续角色和装备目录增长；
- 角色装备也改为顶层模态浮窗；装备主体使用 DPR-aware Canvas 虚拟卡片，只绘制可见行，
  HTML 保留筛选、角色操作、单件详情和键盘入口；
- Canvas 卡片补充共享资源中的装备者头像；连续窗口缩放期间复用 backing store 即时重排，
  停止 72 ms 后才按当前 DPR 重建，避免缩放事件中反复分配大位图造成卡顿；
- Console 改用 Tauri/Wry 原生 WebView auto-resize，移除每个 `Resized` 回调中再次排队的
  `set_size`；WebView2 默认 GPU 合成保持启用，缩放链路不再重复提交客户区尺寸；
- 库存 Canvas 使用上下两行超扫描窗口，滚动时由 GPU 合成器移动已绘制位图，仅在接近缓冲区
  边缘时更新下一窗口，避免每个滚动事件重新测量和绘制卡片文字；
- 透明库存 Canvas 保持默认同步合成，不启用 `desynchronized` 低延迟上下文；该提示在部分
  Windows/WebView2 GPU 驱动组合上会产生黑色透明区和模糊文字；
- 空幕内容区固定为 Console 的剩余高度，仅装备 Canvas 容器持有纵向滚动；不再让 331 件卡片的
  spacer 撑开外层页面和 Canvas 客户区，超扫描窗口只覆盖真实可见高度；
- hover 使用独立 HTML 内描边合成层，跨卡片移动仅更新 `translate3d`；基础 Canvas 不再因
  hover 变化整块重绘；命中检测使用专用 spacer 的内容坐标，不依赖会随祖先滚动失效的屏幕坐标
  缓存；12 px 描边与卡片外形一致，滚动开始时隐藏以保持合成滚动路径；
- 库存及角色映射事件现在推进专用订阅 generation，页面在抓包数据变化后主动收到完整快照，且战斗
  hit 不会触发整份库存重复序列化；
- 筛选、loading、empty、error、订阅清理和宽窄窗口布局均保留，页面不复制 Rust 业务规则；
- 本阶段边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE25.md`。

## 2026-08-01 第二十四步：空幕页面 UI（两步迁移的第 1 步）

- 空幕页迁移继续分为两步，本步只完成 React UI，Rust/Tauri 库存与操作接入留到第 2 步；
- Console 侧栏已启用空幕，并使用 React 19.2 `Activity` 保留页内筛选状态；
- 页面沿用旧 egui 的工具栏、筛选分组、响应式装备网格、装备状态、属性行与套装效果信息架构；
- 预览装备名称、套装、属性和图标来自共享装备资源，已装备角色复用共享头像资源；
- 本地预览筛选支持组内 OR、组间 AND，多个副词条要求全部存在，并覆盖无匹配 Empty；
- 网格使用 `auto-fill + minmax` 充分利用宽窗口，窄窗口下工具栏和筛选分组自动换行；
- 驱动计算器导出、角色装备及右键 Plugin 管理均保持禁用并明确标记数据待接入；
- 第 2 步将复用现有 Rust 空幕规则，新增版本化 DTO、typed client、库存快照和装备事务桥接；
- 本阶段边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE24.md`。

## 2026-08-01 第二十三步：技能页面功能接入（两步迁移的第 2 步）

- 新增 UI-neutral `core::skills` 投影，继续复用现有 `summarize_skill_breakdown` 和深渊半场数据；
- Tauri 新增版本化 Skills DTO、整场/上下半快照命令和 100 ms 变更驱动 Channel；
- WebView 只接收聚合角色、技能行和待映射诊断，逐 hit 与内部领域状态继续留在 Rust；
- generation、命中和诊断计数使用十进制字符串，typed client 校验版本、范围、数值、颜色和列表上限；
- 技能页已移除预览 fixture，接入 loading、empty、error、重试和顶部居中流错误提示；
- 角色筛选保持前端只读投影行为，范围切换会重新订阅并立即获取对应完整快照；
- 技能行悬停或键盘聚焦会显示角色、分类、伤害、命中、GA、GE 和 GE Index 完整详情；
- 待映射 GE 恢复逐项复制编号，并提供成功、失败和无障碍状态反馈；
- `Activity` 隐藏页签时清理技能 Channel，重新显示后恢复订阅；
- 本阶段边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE23.md`。

## 2026-08-01 第二十二步：技能页面 UI（两步迁移的第 1 步）

- 技能页迁移继续分为两步，本步只完成 React UI，Rust/Tauri 数据接入留到第 2 步；
- Console 侧栏已启用 Skills，并用 React 19.2 `Activity` 管理页面可见性；
- 页面保留旧 egui 的角色导航、四项归因指标、技能明细和待映射诊断信息架构；
- 角色导航复用共享头像资源，支持整队/角色预览筛选，并在窄窗口转为横向滚动；
- 技能占比使用轻量行内底纹，整体延续 Settings、History、Timeline 的背景、边界和交互 token，
  避免堆叠独立 Card；
- 页面明确显示“界面预览 / 数据将在第 2 步接入”，fixture 只用于本阶段布局验收；
- 第 2 步将新增 UI-neutral 技能投影、版本化 DTO、typed client 与变更驱动 Channel，并替换预览数据；
- 本阶段边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE22.md`。

## 2026-08-01 第二十一步：时间线功能接入（两步迁移的第 2 步）

- 新增 UI-neutral `core::timeline` 投影，继续复用 `CombatState::timeline`、深渊半场和战斗分段规则；
- Tauri 新增版本化 Timeline DTO、快照命令、偏好设置命令和 100 ms 变更驱动批量 Channel；
- WebView 只接收聚合 bucket、角色汇总、暂停区间、事件标记和分段，不接收逐 hit 或内部领域状态；
- React typed client 在边界校验版本、十进制代次、有限数值、枚举、颜色和列表上限；
- 时间线已移除预览 fixture，接入当前抓包完整快照，并覆盖 loading、empty、error 和重试状态；
- 整场/上下半、统计间隔、团队/角色曲线和角色图例均已启用；统计间隔与曲线模式复用现有
  `UiConfig` 并事务保存；
- `Activity` 隐藏页签时会清理 Channel，重新显示后新订阅立即发送完整快照；
- 本阶段实现和人工验收清单见 `docs/TAURI_MIGRATION_PHASE21.md`。

## 2026-08-01 第二十步：时间线 UI（两步迁移的第 1 步）

- 时间线迁移固定为两步：先迁移 UI，再接入功能；本步只完成 React UI；
- Console 侧栏已启用 Timeline，并使用 React 19.2 `Activity` 管理隐藏页的 Effect 生命周期；
- 页面复刻现有 egui 时间线的信息结构：范围、统计间隔、团队/角色曲线、摘要指标、战斗分段、
  角色图例、悬停明细、暂停区间、事件标记和保留窗口；
- 密集几何由 DPR-aware Canvas 绘制，文字、控件、提示和无障碍摘要由 HTML/React 持有；
- UI 精修增加 rAF 指针聚光、玻璃分层、渐变边界、曲线辉光/面积渐变、斜纹暂停区间、
  事件标签、时间透镜、角色贡献悬浮卡和遵循减少动态效果设置的揭示动画；
- 图表从固定视口比例高度改为按内容区剩余高度弹性分配，正常窗口完整保留底部状态，
  短窗口同步降低图表高度，内容继续增长时由页面内部滚动承接；
- 页面当前显示明确的 UI 预览标识，数据 fixture 仅用于人工验收布局，不接入 Rust/Tauri 数据；
- 未接入 Rust 的范围、统计间隔、曲线模式和图例控件保持禁用，不提前表现业务功能；
- 第 2 步将新增 UI-neutral Rust 时间线投影、稳定 DTO、typed client 与批量 Channel，并替换预览数据；
- 本步范围、功能边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE20.md`。

## 2026-07-31 第十九步：历史页面

- Console 侧栏的 History 已启用，迁移范围只覆盖历史页签；
- Rust 新增前端无关的历史归档准备入口，egui 与 Tauri 复用同一摘要/详情规则；
- Tauri v2 History DTO 与 typed commands 已覆盖加载、保存、JSON 导入导出、删除/五秒撤销、
  对比和上下半预测队伍；低频 History Channel 会推送初始快照和后续修订，隐藏页签时随 Activity 清理；
- 实时抓包在深渊重开/层数边界前归档上一轮，Tauri History runtime 复用闲置超时规则，
  手动重置也执行先保存后清空；保存失败时保留当前战斗状态；
- React 只接收最多八行的本地化摘要投影，并显示队伍预览、预测可用性和隐藏行数；
  原始 hit、本机路径、撤销记录和内部错误不进入 WebView；
- 页面刷新和组件操作保留 mounted ready 状态及当前选择，不用整页 loading 替换，避免
  操作后的全页闪烁；
- 设置页手动刷新允许接受相同状态代次，刷新期间保留已渲染内容，不再停留在骨架屏；
- 历史页按容器宽度切换记录导航、深渊上下半和角色/技能分栏，并复用共享角色头像资源，
  未知角色显示文字回退；
- Console 文档根节点和原生 WebView 使用同一不透明主题底色，根布局改为继承实际客户区尺寸，
  避免快速拖动窗口大小时暴露黑色原生背景；
- Console 原生 `Resized` 事件会使用最新物理客户区尺寸显式同步 WebView 边界，并忽略最小化产生的
  零尺寸事件，避免快速横向扩展后 WebView 停留在旧宽度；
- Console 的 History、Settings、Mod Studio 页签统一使用 React 19.2 `Activity`：隐藏页保留 DOM/草稿
  状态但清理 Effect 与活跃订阅，重新显示时恢复订阅和编辑器生命周期；
- Switch 的轨道、圆球和选中行程改用同一组固定像素几何，不再随界面密度根字号分别缩放，
  修复宽松密度圆球溢出和紧凑密度圆球向内偏移；
- 本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE19.md`。

## 当前结论

分支：`codex/tauri-react-architecture`

首次 Tauri 技术状态面板在 2026-07-30 的人工验收未通过。随后工作树按本文建议
收缩并逐步完成实际 HUD 契约、HTML-in-Canvas 时间线、配置、窗口生命周期和 Home
穿透切换；用户已在 2026-07-31 确认当前 HUD 没有问题。现阶段继续保留 egui HUD
作为对照，不替换或删除其入口。

下一迁移阶段已进入 Mod 工坊。第十一步只新增稳定 `console` 窗口及只读工作区页面，
不改动已验收 HUD，也不提前接入编辑、保存、启用、部署或热更新事务。

首次验收的用户反馈及截图确认的问题：

1. 新窗口是技术状态面板，与原有无背板战斗 HUD 的结构、内容和视觉完全不同；
2. 无边框窗口拖动没有生效；
3. 透明背景表现没有达到原 HUD 的叠加效果；
4. WebView 内出现不均匀色块等显示异常；
5. 当前页面只展示桥接状态、Channel 序号和运行时间，没有展示团队 DPS、角色伤害、
   占比、深渊状态、迷你时间线等原 HUD 数据。

以下首次验收问题是历史记录；后续第 2 至第 10 步已按用户逐项反馈完成修正。

## 2026-07-30 接手进度

- `hud-spike` 已移除整窗 Card、渐变、阴影和 `backdrop-blur`；
- 页面只保留 halo 文字、技术状态和必要的局部控制，其他区域保持透明；
- 独立拖动文字行在主指针按下时显式调用
  `getCurrentWindow().startDragging()`，刷新等交互控件不在拖动区；
- `hud-spike` capability 增加
  `core:window:allow-start-dragging`；
- 窗口初始宽度收敛到现有 HUD 默认的 380 逻辑像素；
- 后续等价 HUD 契约与 HTML-in-Canvas 混合组件规划见
  `docs/TAURI_HUD_MIGRATION_PLAN.md`。

上述透明与拖动基线已通过本轮用户目视确认。多显示器、跨 DPI、游戏高负载、休眠恢复
和显示器拔插仍属于最终 HUD 替换门槛，尚未因此次基础确认而免除。

## 2026-07-30 第二步：实时只读 HUD 契约与 HTML 布局

- 根 Rust crate 新增 UI-neutral `HudSnapshot` 投影，复用 `CombatState`、`HudConfig`、
  DPS 计时口径、反应伤害策略和角色筛选规则；
- 技术契约升级到 v3，有序 Channel 在完整快照中批量携带抓包状态和
  HUD v1 DTO；
- 64 位命中计数继续使用十进制字符串，React 边界对版本、数字、可空字段和模块顺序
  做运行时校验；
- 编辑态无战斗数据时，预览数据在 Rust 中生成；穿透态无数据时返回明确 empty；
- React 已用 HTML 复刻团队摘要、伤害占比条和最多四名角色行，模块显隐与顺序来自
  Rust 的 `HudConfig`；
- 根 crate 新增 frontend-neutral `LiveCaptureService`，持有现有 `CaptureController`、
  `CombatState` 和后台事件 worker；每个 `EngineEvent` 仍只经过共享 reducer；
- Tauri 编辑栏新增 typed 启停命令和抓包生命周期显示；启动/停止工作不占用主线程，
  启动成功后新一轮状态替换预览，停止后保留最终读数；
- 非穿透编辑态增加限定在 HUD 组件范围内的半透明模糊背板；进入穿透态时背板与编辑
  控件同时移除；
- CSS `backdrop-filter` 不负责跨 WebView 模糊；窗口适配层在非穿透态启用持久
  Windows Acrylic accent policy，穿透态清除该原生效果；
- React 背板改为填满实际 viewport，`HudConfig.width` 只决定原生窗口初始宽度，窗口
  缩放后视觉边框与原生缩放边界保持一致；
- 移除 viewport 与内容层之间的外边距和阴影，避免原生 Acrylic 底色形成明显灰框；HTML
  深色叠层保持 40%，圆角只使用内容层裁切，不绘制可见描边；
- 新增共享 Win32 窗口样式 helper，对 Tauri HWND 设置持久 Acrylic、DWM 圆角和无边框
  颜色；Acrylic 只在初始化及穿透切换时设置，不再随焦点或拖动重复设置，从而避免黑底与
  透明/不透明闪烁；
- HUD 根节点默认 `select-none`；仅日志、代码、诊断等显式
  `data-hud-selectable-text` 区域允许文本选择；
- 抓包继续生成本地诊断 PCAPNG，但 DTO、错误和日志不暴露其路径或内部错误链；
- 本步已加载角色表和当前语言技能目录；当前语言角色名的最终等价仍需实战人工确认；
- 迷你时间线继续留给独立的 HTML-in-Canvas 阶段，不在本步放入占位业务数据。

本步的范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE2.md`。

## 2026-07-30 第三步：HTML-in-Canvas 迷你时间线

- `HudSnapshot` 升级到 v2，技术契约升级到 v4；
- Rust 继续复用现有 `TimelineSeries`，按当前时间线 bucket 设置生成序列，并在 HUD
  边界聚合到最多 60 个 bucket；不向 React 发送逐 hit 数据；
- bucket 包含时间范围、伤害、DPS 和十进制字符串命中数；实时与深渊半场继续跟随
  当前 HUD 选择；
- React 增加独立纯 draw model，把 bucket 时间和峰值映射到逻辑像素；
- 原生 Canvas 2D 只绘制透明基线和折线，backing store 按 `devicePixelRatio` 重建，
  数据、尺寸或 DPI 变化通过 `requestAnimationFrame` 合并绘制；
- HTML overlay 提供本地化峰值、时间区间、DPS 和伤害 Tooltip；穿透态 overlay 不接收
  指针；
- 本步未新增绘图库，也未引入新的 Tauri command；模块显隐继续读取 Rust
  `HudConfig.show_mini_timeline`；
- Tauri 初始高度按已启用的标题、状态和时间线模块预留空间，默认 HUD 高度保持不变；
- 纯映射、指针命中、bucket 上限、DTO 版本和畸形边界均有自动化覆盖。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE3.md`。

## 2026-07-30 第三步补充：低延迟变更驱动刷新

- 原固定 750 ms 全量快照改为 100 ms 轻量代次检查，真实数据变化后的最大发送等待降至
  一个 100 ms 合并窗口；
- 抓包 worker 只为会影响 HUD 投影或抓包状态的事件递增代次，逐包质量计数、调试包和
  Mod 消息不会触发 HUD 全量投影；
- 窗口穿透和置顶状态使用独立展示代次，同值重复设置不会产生额外快照；
- 空闲时 Channel 线程只读取两个原子代次并休眠，不再重复计算时间线、序列化完整 DTO
  或跨 WebView IPC；
- 选择 100 ms 而不是 50 ms，是为了把可见延迟从 750 ms 显著降低，同时避免为文本 HUD
  翻倍增加唤醒、投影和 IPC 次数；200 ms 在连续伤害场景中仍可能感到滞后。

## 2026-07-31 第四步：HUD 模块显隐编辑

- 用户确认当前 HTML-in-Canvas 时间线和 100 ms 变更驱动刷新没有明显问题；
- 非穿透编辑栏新增 HTML“HUD 模块”面板，按 Rust 下发顺序显示标题、汇总、状态、
  角色排行和曲线开关；
- 前端只发送 `{ module, visible }` typed intent，不做权威配置的乐观修改；
- Tauri 在窗口边界校验稳定模块 ID，Rust 复用现有 `HudConfig::set_module_visible`
  语义；
- 完整 `UiConfig` 原子保存成功后才替换内存投影并递增展示代次；保存失败保留上一配置，
  前端只接收稳定错误 key；
- 模块变化后原生窗口保持当前宽度并同步内容高度；编辑态保留模块面板的最小恢复空间，
  穿透态使用显示内容高度；
- 全部模块隐藏时编辑态仍保留完整五行模块面板高度；鼠标穿透与窗口置顶改为单个高亮
  图标按钮直接切换，不再在图标旁附加开关；
- 模块拖拽排序和宽度编辑仍作为阶段 D 的后续独立切片。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE4.md`。

## 2026-07-31 第五步：HUD 模块拖拽排序

- 用户确认显隐面板完整高度和单按钮式穿透/置顶交互没有明显问题；
- 模块面板每行新增独立 Pointer Capture 拖拽手柄，目标行上半区表示插入目标前，下半区
  表示插入目标后；不进入 WebView2 原生 HTML 拖放会话，避免系统禁止光标；
- 拖动时只显示本地拖动状态与青色插入线，权威顺序仍等待 Rust 返回的新快照；
- 手柄支持上下方向键移动，键盘与拖放共用同一 `{ dragged, target, insertAfter }`
  typed intent；
- Tauri 校验两个稳定模块 ID，Rust 直接复用现有 `HudConfig::move_module`；
- 显隐与排序共用同一原子配置事务，保存失败保留此前的内存投影和磁盘配置；
- 排序不会改变窗口尺寸；HUD 宽度编辑继续作为阶段 D 的后续独立切片。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE5.md`。

## 2026-07-31 第六步：HUD 宽度编辑

- 用户确认 Pointer Capture 模块排序没有明显问题；
- 模块面板增加 HUD 宽度整数输入，仅在 Enter 或失焦时提交一次，不按键逐次执行 IPC
  或配置保存；
- Escape、空值和非整数草稿恢复最新 Rust 权威宽度；
- Tauri command 使用根配置已有的 `HUD_WIDTH_MIN` / `HUD_WIDTH_MAX` 校正范围，
  TypeScript 不复制 280..3840 业务规则；
- 宽度与模块显隐、排序复用同一 save-before-publish 原子配置事务；
- 保存成功后同步原生窗口逻辑宽度并保留当前高度，返回的新快照统一输入框显示值；
- 非穿透编辑态最小高度包含五行模块与宽度字段，所有模块隐藏时仍可完整操作；
- 阶段 D 的实现项至此完成，重启恢复、窄窗口和 100%/125%/150% DPI 仍属于人工门槛。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE6.md`。

## 2026-07-31 第七步：HUD 内容高度锁定

- 用户确认宽度编辑没有明显问题，并指出原版 HUD 只允许水平缩放；
- Tauri 启动时把原生窗口最小高度和最大高度同时设为 Rust 计算出的内容高度；
- 模块显隐或穿透状态改变内容高度时，先按增大/缩小方向安全更新 min/max 约束，再同步
  原生尺寸，避免旧约束阻挡新的权威高度；
- 最小宽度和最大宽度继续复用根配置的 280 与 3840，保留水平调整能力；
- 宽度 command 同样重新应用当前权威高度，因此尺寸同步不会保留外部产生的临时高度；
- 新增纯 Rust 测试覆盖等高约束和高度扩大/缩小时的约束更新顺序。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE7.md`。

## 2026-07-31 第八步：原生水平缩放持久化

- 用户确认内容高度锁定没有明显问题，继续保持原版仅水平缩放的行为；
- Tauri 窗口层监听 `Resized` 和 `ScaleFactorChanged`，将原生物理宽度按当前 DPI 换算、
  四舍五入并限制到根配置已有范围；
- resize 热路径只发送轻量宽度值，独立工作线程等待 350 ms 静默期并合并中间事件，
  连续拖动只提交最终宽度；
- 原生拖动和模块面板输入共用 `AppState::set_hud_width`，完整配置原子保存成功后才更新
  Rust 投影与 Channel 代次；
- 程序化宽度同步产生的 resize 会在状态层命中 no-op，不重复写配置；
- 窗口销毁时刷新最后一个待保存宽度并结束工作线程；
- 新增纯 Rust 测试覆盖物理/逻辑像素换算、范围校正和连续 resize 合并。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE8.md`。

## 2026-07-31 第九步：HUD 置顶偏好事务

- 用户确认阶段 D 现有交互、水平缩放恢复和多 DPI 检查均无明显问题；
- 对照原版后确认 Tauri 置顶按钮此前只修改原生窗口和内存原子值，重启会重新读取旧的
  `UiConfig.always_on_top`；
- `AppState::set_always_on_top` 现复用完整配置原子保存，保存成功后才更新内存投影和
  Channel 展示代次；
- 原生窗口先执行置顶切换；配置保存失败时保持旧 Rust 投影，并尝试把原生窗口恢复为
  切换前状态；
- 原生窗口操作与配置事务由 `AppState` 内的专用锁顺序执行，避免并发 typed intent
  交叉覆盖最终原生状态；
- 同值重复设置返回 no-op，不重复写配置或发布快照；
- 现有 `hud_config_save_failed` 稳定错误契约和中日文本继续复用，没有新增 DTO、command
  或翻译源；
- 新增测试覆盖成功保存、同值 no-op 和保存失败时的投影回滚。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE9.md`。

## 2026-07-31 第十步：Home 穿透恢复与窗口位置生命周期

- 按用户澄清恢复原版交互：开启穿透时不创建控制窗口，根配置中的穿透快捷键继续作为
  唯一恢复入口，旧配置和默认配置均为 Home；
- frontend-neutral Windows `WH_KEYBOARD_LL` 监听器只发布首次无修饰键按下事件，
  保留按键向游戏继续传递；Tauri 适配线程负责原生窗口切换；
- Home 在游戏前台或 HUD 已穿透时仍可切回编辑；按键重复在 key-up 前只触发一次，
  Ctrl/Alt/Shift 组合不触发；
- 原生按钮与 Home 事件共用穿透事务锁；钩子安装成功前拒绝开启穿透，运行故障时恢复
  编辑状态；Tauri 仍不注册全局 F12；
- HUD 物理虚拟桌面坐标进入根 `UiConfig`，支持副屏负坐标；移动事件经独立线程合并，
  停止 350 ms 后才原子保存最后位置；
- 启动时检查保存位置的标题栏是否仍与任一显示器工作区相交；断开的显示器坐标回退为
  主显示器工作区居中位置；
- 位置变化不发布 React 投影代次，迁移期 egui 保存共享配置时保留其不负责的 Tauri
  HUD 坐标；
- 新增 Windows 键值/修饰键、运行时就绪状态、位置纯函数与配置事务测试。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE10.md`。

## 2026-07-31 第十一步：Mod 工坊只读工作区

- 新增稳定 `console` 窗口，HUD 窗口及其权限保持独立；
- 根 crate 新增 frontend-neutral Mod 工作区只读投影，复用现有
  `storage::mod_scripts` 读取语义；
- 列表命令只返回 ID、启用状态、行数和字节数，选中后再通过独立 detail 命令读取最多
  16 KiB 的源码；
- Mod ID 在 Rust 信任边界校验，工作区限制为 256 个文档；绝对路径和文件系统详情不进入
  前端错误；
- Tauri 文件读取使用 blocking worker，React 页面只通过 typed client 调用；
- React 增加资源管理器、只读源码预览、刷新、loading、empty、workspace error 和
  document error 状态；
- 首次视觉验收确认独立深色卡片方案偏离现有界面后，已按既有 Console / Mod 工坊重排为
  分组侧栏、浅色工具栏、资源管理器、编辑器标题、快速上手、能力面包屑、源码区、蓝色状态栏
  和运行时控制台；
- 只读源码区增加行号与无额外依赖的 C++ 展示高亮，使用 React 文本节点，不注入 HTML；
- 本步尚未迁移的创建、保存、还原、打开目录、加载器状态、启停和运行时日志控件保持禁用，
  不伪装成已经接入；
- 当前选择在刷新后仍存在时继续保留，快速切换时忽略上一文档的迟到响应；
- 仅源码预览允许文字选择；页面其他区域保持不可选；
- Alert、Skeleton 和 Empty 由现有 shadcn CLI 加入源码，没有新增 Node 依赖；
- 新增 `desktop` 根 Feature 供 Tauri 复用 Mod 存储，CLI-only 依赖树保持隔离；
- 本步不接入 Monaco、编辑保存、启停 Mod、部署、运行时日志或热更新。

本步范围与人工验收清单见 `docs/TAURI_MIGRATION_PHASE11.md`，后续规划见
`docs/TAURI_MOD_STUDIO_MIGRATION_PLAN.md`。

## 2026-07-31 第十八步：设置页软件更新事务完成

- Settings v4 投影全部可用应用/Mods Plugin 组件、版本、发布时间、发行说明、包大小、
  下载进度、已准备组件、安装可用性和稳定阻断原因；
- 启动检查和手动检查继续复用签名清单、已安装组件版本比较及 WinHTTP 系统代理回退；
- 自动下载偏好现在会在检查发现更新后启动第一个签名组件下载，而不是只保存开关；
- 手动下载与自动下载共用 `storage::update::prepare_update`，保留大小、SHA-256、归档结构
  和事务文件校验，进度按 100 ms 最小间隔合并后经 Settings Channel 发布；
- 应用更新复用独立 `nte-updater`，仅在抓包停止后启动事务并退出当前 Tauri 进程；
- Mods Plugin 更新复用既有备份、原子替换、部署刷新和失败回滚，游戏运行时保持安装按钮
  禁用；
- 启动时补齐更新健康标记和已完成事务清理；内部网络、签名、文件和部署错误只进入 Rust
  日志，WebView 只接收稳定错误 key；
- React 更新卡显示每个组件的发行说明、大小、下载按钮、实时进度、安装按钮和阻断说明。

本步实现与人工验收清单见 `docs/TAURI_MIGRATION_PHASE18.md`。

## 已完成内容

### 工程结构

- `src-tauri/`：独立的 Tauri 2 shell，通过
  `nte-dps-tool = { path = "..", default-features = false }` 复用 Rust crate；
- `frontend/`：Vite + React + TypeScript + Tailwind CSS + shadcn/ui；
- `hud-spike`：已验收的 HUD 窗口；
- `console`：第十一步新增的 Mod 工坊只读页面窗口；
- typed command：读取技术快照、启停抓包、设置置顶、设置鼠标穿透；
- Tauri Channel：每 100 ms 检查一次轻量代次，仅在状态变化时推送带抓包状态和 HUD
  投影的有序完整快照，支持显式取消订阅；
- TypeScript 边界：对 Rust DTO 做运行时校验，64 位计数使用十进制字符串；
- i18n：React 继续读取 `res/languages/zh-CN.json`，没有建立第二份中文资源；
- CLI 隔离：Tauri 和 WebView 依赖没有进入 `nte-core` 的 CLI-only 依赖树；
- F12：Tauri shell 不注册全局 F12，也不依赖
  `tauri-plugin-global-shortcut`。鼠标穿透使用根配置快捷键，默认 Home。

### 当前窗口配置

`src-tauri/tauri.conf.json` 已设置：

- `transparent: true`
- `decorations: false`
- `alwaysOnTop: true`
- `shadow: false`
- `skipTaskbar: true`

`frontend/src/index.css` 也已把 `html`、`body` 和 `#root` 设为透明。
透明、圆角、失焦模糊和拖动合成问题已按用户当前测试结果通过本轮基础验收；最终替换
门槛中的多屏、DPI、高负载和恢复场景仍需单独覆盖。

## 未通过项分析

### 1. 与原 HUD 不一致

根因明确：第一阶段实现的是独立技术状态卡片，而不是对原 HUD 的结构和数据进行迁移。

原 HUD 的事实源仍在：

- `src/app/main_view.rs` 的 `hud_panel` 及相关绘制逻辑；
- `src/app/hud.rs` 的窗口尺寸、模块布局和文本 halo；
- `src/storage/config.rs` 的 `HudConfig`、模块顺序、显隐和尺寸配置；
- `src/app/theme.rs` 的 HUD 主题 token。

下一步应先把这些行为整理成 UI-neutral 的 HUD snapshot 和配置契约，再由 React
复刻原布局。不要继续扩展当前技术仪表盘。

### 2. 窗口拖动失效

当前实现只在 React 标题区域添加了 `data-tauri-drag-region`。用户机器上未生效。

接手后按以下顺序定位：

1. 检查 `src-tauri/capabilities/default.json` 是否需要显式加入
   `core:window:allow-start-dragging`；
2. 在最小透明窗口中单独验证 drag region，不与 Card、Tooltip、按钮组合；
3. 若属性方案仍不稳定，在标题栏空白区域的主指针按下事件中调用
   `getCurrentWindow().startDragging()`；
4. 交互控件必须排除拖动，避免刷新按钮和开关吞掉或触发拖动；
5. 验证普通 DPI、125%、150% 以及多显示器。

### 3. 透明背景未达到要求

当前 React 页面使用了接近整窗尺寸的深色 Card：

```tsx
<Card className="... bg-slate-950/88 ... backdrop-blur-xl">
```

即使 WebView alpha 正常，这种结构也不是原 HUD 的“无背板直接叠加游戏画面”。此外，
用户截图中还出现了窗口区域合成不均匀的现象，需要把 CSS 绘制问题与原生窗口 alpha
问题分开定位。

推荐用最小化验证顺序：

1. 暂时只保留透明窗口和一行纯文字，移除 Card、渐变、阴影、blur 和全屏 surface；
2. 在纯色桌面背景和游戏无边框窗口上分别观察 WebView 矩形区域；
3. 确认原生透明有效后，再逐项恢复文字 halo、角色行和必要的局部半透明编辑背板；
4. 穿透展示模式不画整窗背景；编辑模式只在实际控件范围内画局部背板；
5. 若最小页面仍有矩形底色，再检查 WebView2/Tauri Windows 原生合成链路，而不是继续
   调整 Tailwind 色值。

### 4. 显示异常

用户截图可见中央区域存在不均匀深色块。当前页面同时使用透明 WebView、半透明 Card、
`backdrop-blur-xl`、多层渐变和透明 footer，变量过多。

先删除 blur、shadow 和多层渐变完成透明基线验证，再逐项恢复效果。每恢复一项都要截图
对比，定位具体触发条件。

## 推荐接手顺序

1. **保留 egui HUD**：继续作为行为和视觉基线；
2. **列出等价契约**：团队 DPS、角色行、占比、状态行、深渊半场、迷你时间线、模块顺序、
   显隐、宽度和编辑状态；
3. **最小透明壳**：只验证透明、拖动、置顶、穿透和控制台恢复入口；
4. **只读 HUD 数据**：由 Rust 投影明确 DTO，通过有序 Channel 批量推送；
5. **复刻原布局**：先完成穿透展示模式，再完成局部编辑模式；
6. **接入配置**：复用现有 `HudConfig`，不在 TypeScript 复制业务默认值；
7. **专项人工验收**：透明、拖动、穿透恢复、游戏前台置顶、多 DPI、多显示器和高负载；
8. 上述验收全部通过后，再讨论替换 egui HUD。

## 自动化验证状态

已通过：

```powershell
cargo fmt --check
cargo check
cargo test

cargo check --bin nte-dps-tool --features gui
cargo check --bin nte-core --no-default-features --features cli
cargo clippy --bin nte-dps-tool --features gui -- -D warnings
cargo clippy --bin nte-core --no-default-features --features cli -- -D warnings
cargo test --features gui
cargo test --no-default-features --features cli
cargo tree -e normal --no-default-features --features cli

cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test
pnpm --dir frontend build
pnpm --dir frontend tauri:build -- --debug --no-bundle
```

任务相关前端与文档文件的 Prettier 定向检查通过。整目录
`pnpm --dir frontend format:check` 仍会报告 20 个本次范围外的既有文件，包括
shadcn 基础组件、工程配置和既有启动/辅助文件；本步保持这些无关文件不变。

完整根项目、GUI、CLI 和 Tauri 严格 Clippy 均已通过；CLI 依赖树未出现 Tauri、
WebView 或现有 GUI 依赖。

## 人工复现

```powershell
pnpm --dir frontend tauri:dev
```

观察 `hud-spike`：

1. 启动前确认预览和 idle 状态，点击播放按钮后确认 starting → running；
2. 实战造成伤害，确认最多 100 ms 合并窗口内显示真实摘要、角色排序和占比；
3. 点击停止按钮，确认 stopping → stopped 且最终读数保留；
4. 分别覆盖游戏未启动、Npcap 或网卡异常，确认明确原因与修复后的重试；
5. 拖动顶部标题行；确认抓包、刷新和开关不会触发拖动；
6. 把窗口放在亮色或明显纹理背景上，确认非穿透态模糊背板可读且边界稳定；
7. 从左右边缘调整宽度，确认视觉边框随 viewport 同步变化；拖动上下边缘和四角时高度
   保持由当前模块内容决定；
8. 按 Home 开启穿透，确认模糊背板消失且组件外保持透明；保持游戏前台再次按 Home
   恢复编辑；
9. 在窄窗口和 100%、125%、150% DPI 下检查状态截断、裁切、字号与数值列；
10. 关闭 egui 程序后单独截图 Tauri HUD，再分别运行同类实战，逐项对比摘要、排序、
    占比、深渊状态和名称。

## 提交边界

- 本分支增加迁移规范、Tauri shell、React HUD、共享抓包服务、共享翻译键和迁移文档；
- 没有删除或替换现有 egui 入口；
- 没有复制或改写抓包解析、reducer、历史、更新或 Mod IPC 规则；
- 没有注册 Tauri 全局 F12；
- `frontend/dist/` 和 `src-tauri/target/` 是忽略的本地构建产物，不进入提交。

## 第 31 步补充：主 DPS 窗口

主 DPS 窗口已按用户提供的新版截图迁移到 `main-dps` Tauri 窗口；Rust 继续提供实时/历史
投影和抓包控制，React 负责响应式展示。主窗口现为桌面生命周期所有者，Console、深渊数值和
命中详情窗口关闭时隐藏，并与主窗口共享自绘标题栏。主窗口已补齐短高度自适应、完整控制区折叠、
伤害归属/角色/队伍详情入口、深渊半场自动跟随，以及与旧窗口一致的中性深色层级；Monaco 直接
跟随界面主题。本阶段的完整契约、行为与人工验收步骤见 `docs/TAURI_MIGRATION_PHASE31.md`。
深渊总览详情仍继续使用旧 egui 作为后续独立窗口迁移对照。

## 第 32 步补充：新旧行为差异收口

主 DPS 及已有辅助窗口已补齐暂停展示快照、历史自动返回实时、Start/Reset/Import 确认、
5 秒 Reset Undo、首次引导、独立通知岛、布局预设的窗口协调、文件拖放与快捷键。角色和队伍
命中详情现为两个独立窗口并分别保存几何信息；深渊数值窗口补齐“导入当前队伍”和几何恢复。
Rust 仍是会话与窗口配置的唯一事实源，主 DPS 契约更新为 v3。完整范围和人工验收清单见
`docs/TAURI_MIGRATION_PHASE32.md`。

## 第 33 步补充：旧版交互与视觉语义补齐

深渊线路标题恢复当前队伍头像、预计时间、DPS 与清除操作，并由 Rust 契约分别声明上下半场
当前队伍可用性；抓包回放加载期间只显示加载动画，完成后才恢复完整结果。HUD 编辑态可以直接
拖动实际模块排序并恢复角色头像，主 DPS 与 HUD 则统一消费旧版头像像素算法生成的固定角色色。
深渊契约更新为 v2，技术契约更新为 v5、HUD 快照更新为 v3。完整边界和人工验收清单见
`docs/TAURI_MIGRATION_PHASE33.md`。

## 第 34 步补充：HUD 排序、窗口互斥与角色色区分度修复

实际 HUD 与 Settings 的模块排序已统一改用 Pointer Events，不再触发 WebView 原生禁止拖放
光标；Settings 顺序固定为一行一个。Console 打开 HUD 编辑器时会隐藏主 DPS 面板，从 HUD
返回时则先恢复主面板再隐藏 HUD。缺失显式颜色的角色改为由 Rust 按完整目录稳定分配 32 色
高区分序列，不再依赖头像像素或角色属性，并保证当前目录角色颜色唯一。现有 DTO 版本保持不变。
完整边界和人工验收清单见 `docs/TAURI_MIGRATION_PHASE34.md`。
