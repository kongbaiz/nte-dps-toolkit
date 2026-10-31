# AGENTS.md

适用于整个仓库。优先级：用户当次明确要求 → 目标目录更深层的 `AGENTS.md` → 本文件。文档与代码不一致时以当前代码和可运行的测试为准。

本项目的主要技术债不是“校验不够”，而是**校验太多、测试太多、函数太长**。本文件的目的是让每次改动都比上一次更短、更直接。

## 1. 工作方式

- 交付满足当前需求的最小正确改动。不预建扩展点、兼容层、开关或“以后可能用到”的参数。
- 新规则约束**新写和被修改的代码**。修改某个函数时，顺手清理该函数内违反本文件的代码，例如重复校验、死分支或测试钩子；不在无关任务里做整仓清理。整片存量清理作为单独任务。
- 删除优于保留，内联优于抽象，直接调用优于间接层。一次只有一个调用方的 wrapper、trait、工厂、配置参数，没有存在理由就不要写。
- 发现问题先读代码确认，不靠猜；能用编译器、类型和现有测试证明的事，不另写检查。

## 2. 架构边界（硬约束）

- 桌面 UI 唯一实现：**Tauri 2 + Vite + React + TypeScript + Tailwind CSS + shadcn/ui**。Rust 核心（`src/`）是领域状态唯一事实源；`src-tauri/` 只做薄 adapter；React 只渲染 read model 和维护 UI-only state，不复制解析、reducer、history 或持久化规则。
- `nte-core --no-default-features --features cli` 不依赖 Tauri、WebView、egui、eframe、wgpu 或窗口库。不恢复旧 `gui` Feature 或平行 UI 状态机。
- Mod 热更新：候选全部成功才原子替换，失败保留上一版本。`master` 不恢复已关闭的研究功能。

## 3. 信任边界：只在这里校验

**外部输入**（必须校验，且只在入口校验一次）：

| 边界 | 入口 |
| --- | --- |
| 抓包帧、pcapng、回放/历史/队伍导入文件 | `src/engine/capture.rs` 导入入口、`src/storage/` 读取入口 |
| Mod / Loader IPC、原生插件返回 | `src/platform/mods_plugin.rs`、`src/core/toolkit*.rs` |
| CLI stdin JSON-RPC | `src/api/request.rs` |
| 网络（updater、Mod 市场） | `src/platform/update_http.rs`、`src/core/mod_market.rs` |
| 用户可手改的磁盘文件（config、i18n） | 对应 `storage` 的 load 函数 |

在这些入口：外部数据不得导致 panic；先用 metadata 限制文件大小，再解析成**带类型的值**（parse, don't validate）。错误使用已有的 typed error 和稳定 code。

**不是边界**（不得再校验）：

- Rust 模块之间的调用；Rust 刚产出或刚持久化的数据。
- Rust → Tauri → React 的 DTO。前端与 Rust 打包在同一个二进制里，不存在版本漂移。
- React → Tauri command 参数。用 serde 类型（含 `#[serde(rename_all)]` 枚举）直接反序列化，不手写 `parse_*`。
- 已被上游类型或入口保证的值，例如 enum、非空、范围、已排序、“恰好一个 live row”。

由此推出的禁止项：

- 同一份数据在第二层再次检查 size、count、range、budget。同一概念只保留一个 `MAX_*` 常量，其他地方引用它，不复制数值。
- 对内部计数器、索引、长度使用 `saturating_*` / `checked_*` / `.max(0)` 来“防御”不可能的溢出或下溢。不变量用普通运算；需要表达时用 `debug_assert!`。只有在值确实可能越界时才饱和。
- 用 `unwrap_or_default()`、`.ok()`、`?? 0`、`?? []`、`if let Some(..) else {}` 静默吞掉“不应发生”的状态。不可能发生就 `expect("原因")` 或 `unreachable!`；可能发生就返回 typed error。
- 永远不会被构造的错误变体；只做改名的错误映射函数。一个领域只写一次 `From<DomainError> for CommandError`，保留原始 code。
- 为不存在的威胁增加检查（“防止将来有人传错”）。
- 新增 DTO 版本号常量或运行时版本比对。版本号只用于真正跨发布边界的格式：CLI JSON-RPC、持久化文件、updater 协议、Mod ABI。

## 4. 死代码

- 不写 `#[allow(dead_code)]`、`cfg_attr(not(test), allow(dead_code))`。用不到就删；只有测试用就移进 `mod tests`。
- 生产代码中不放 `#[cfg(test)]` 的函数、字段、计数器、访问器、常量或“test-only 复制版”。测试必须驱动真实的生产路径。需要测试的行为若无法从公开入口观察，应改进设计，而不是开后门。
- 前端不为测试保留 `create*Client(transport)` 工厂或 test-only export；测试 mock `invoke` 模块。
- 删除功能时一并删除它的命令注册、client 方法、hook、i18n key、测试和文档。不写断言“旧功能已不存在”的测试。
- 兼容/迁移代码（`legacy_*`、`migrate_*`、`compatibility_*`）必须在注释中写明移除条件或版本；没有条件的兼容层不新增。
- 已 export 但只在本文件使用的符号去掉 `export` / `pub`。

## 5. 可读性

- 一个函数只做一个阶段的事。热路径（如 `process_capture_frame`）只允许变短：新增逻辑放进独立的阶段函数，不往大函数里继续加分支。
- 修改一个已超过约 150 行或嵌套超过 4 层的函数时，先把要改的那一段提取成命名函数再改。这是默认做法，不是硬阈值。
- 协议、wire 格式里的魔法数字用命名常量；同一数值的不同含义使用不同常量，同一含义只定义一次。
- 同一词汇表（主题、密度、语言、热键动作、HUD 模块等）在 Rust 端用一个 serde enum 定义，序列化和反序列化共用；TS 端只 export 一份字面量联合类型，其他地方 import。
- 重复 3 次以上的样板（锁获取、poison 处理、Channel subscribe/unsubscribe、窗口 blur 关闭、错误构造）提取成一个 helper；2 次以内允许重复。
- Rust 文件中测试超过约 1,000 行时，放到同目录 `tests.rs`（`#[cfg(test)] mod tests;`），不和生产代码混在一个文件里。
- React 组件超过约 300 行，或 `useState` 超过约 6 个时，修改它就按视觉区块拆子组件，或把独立状态提取成 hook。effect 依赖不得包含每帧变化的 snapshot；用 ref 或 `useEffectEvent`。
- 异步请求防竞态只用一种机制：effect 内的 `active` 标志，或一个 generation ref。不要叠加 mounted + generation + active 三重守卫。

## 6. 测试

测试的目的是锁定**用户可观察的行为**和真实缺陷，不是证明代码写过。

**要写的测试：**

- 修复缺陷时：写一个能复现该缺陷的回归测试。
- 新增行为时：每个行为一个正常路径测试，外加真实可能出现的失败路径。外部输入入口要有畸形输入测试。
- 同一行为的多个输入变体：写成一个表驱动测试，不写 N 个近似复制的测试。

**不写、并且在修改相关代码时删除的测试：**

- 断言 serde 字段名或 camelCase 的测试（如 `value["someKey"]`）。每个 DTO 族最多保留一个往返测试。
- 逐个镜像 `invoke` 命令名和参数的 client 测试。
- 前端 contract validator 针对每个字段的接受/拒绝测试。
- 以 `?raw` 读取源码后断言文本的测试，以及断言 CSS class 字符串的测试。
- 断言私有实现形状的测试：访问次数、cache 命中计数、内部容量、调用顺序。
- 只测试 std、serde、React 或第三方库本身行为的测试。
- “已删除功能不存在”的负向断言；仅为迁移临时写的守卫测试。
- 近似重复测试。保留覆盖最完整的一个，或合并为表驱动。

**数量约束：**

- 纯重构不新增测试，现有测试保持通过即可。
- 一次改动新增的测试代码量通常不应超过生产代码改动量。超过时需在交付说明里给出理由。
- 不为测试新增生产抽象、参数（如 `_with_limits`）或注入点。

## 7. 前端 UI 规范

规范来源：[frontend-guidelines.md](frontend-guidelines.md) 第一至四节，以及 [frontend/README.md](frontend/README.md) 的「UI 样式入口与约定」一节。第五节的“循环打磨”不作为完成条件；完成标准以下列可检查条款为准。

- **组件**：使用 `components/ui` 与 `components/nte`。文本输入用 `Input`，下拉用 `NativeSelect`，表单用 `FormRow`，提示用 `ActionNotice`，弹窗用 `ui/dialog`。不写原生 `<select>`、文本 `<input>` 或原生 `<dialog>` 来重造外观。控制台长页面使用 `PageHeader` + `PageScrollArea`，不手写固定 `p-3`/`p-4` 的滚动容器。
- **颜色**：只用语义 token，如 `text-success`、`text-warning`、`text-info`、`text-destructive`、`text-muted-foreground`、`bg-success`。UI 外框中不使用 `emerald-*`、`amber-*`、`zinc-*` 等调色板类，也不写 hex/rgb/oklch 字面量或 `bg-[var(--x)]`（已有对应 utility 时）。游戏领域色（角色、品质、元素）放在 `lib/character-color.ts` 等领域文件中。
- **字号**：只用 12/14/20px，关键统计数字可用 24px，即 `text-xs`/`text-sm`/`text-xl`/`text-2xl` 对应的 token。不写任意字号，不小于 12px。
- **边框与装饰**：默认使用 `--ui-border-width`（0.5px），不写 `1px`/`border-2` 外框。禁止卡片左侧彩色竖条，包括 `w-[3px]` 绝对定位条、`border-l-2` 色条和 inset 阴影条纹。同一层级的浮层使用同一档阴影。
- **文案**：不用 “·” 作分隔符（包括 Rust 侧生成给 UI 的文案）；不加眉题/uppercase overline；页面描述不复述标题。用户可见文案走 i18n，英文为 key，简中在 `res/languages/zh-CN.json`。删除文案时同时删除 key。
- **动效**：时长与缓动只用 `lib/motion.ts` 和 CSS token 中的一套刻度，默认 `ease-out 250ms`；尊重减少动态效果偏好。装饰性动效不新增。
- **库**：拖拽排序使用 dnd-kit，可调面板使用 react-resizable-panels，动画使用 Motion，跨组件共享客户端状态使用 Zustand。这些库目前尚未引入，首次引入按第 9 节确认。确认之前不扩写现有的手写实现（`use-hud-module-pointer-reorder.ts`、`use-layout-flip.ts` 等），也不新增第二份手写实现。
- **样式层**：不从祖先选择器覆盖组件内部（`[data-slot]`、`> div:first-child`、`textarea` 等）；`!important` 仅限透明窗口与减少动效这两个平台例外。
- **无障碍**：图标按钮需要有 Tooltip 或 `aria-label`；Dialog、Sheet、Drawer 需要有可访问标题。

## 8. 真实存在的运行时约束

只有在改动涉及对应能力时才需要考虑以下约束；不涉及时不为它们加代码或测试。

- **抓包线程**：不做 I/O、大型序列化或深拷贝完整 `CombatState`/History。慢操作移到锁外或 worker 中执行。
- **热锁**（`event_gate`、权威状态、capture/session 锁）：持锁期间不做 I/O、dialog、sleep、join、阻塞 send 或 IPC。多锁保持一致的加锁顺序。poison 处理统一走一个共享 helper，不再手写新的恢复代码。
- **revision**：只在用户可观察的状态变化时 bump；no-op 不 bump。packet、combat、presentation 三类 revision 不混用。
- **history**：保持 `bounded history rows + exactly one live row` 由 Rust 保证；live、import、replay 复用同一套领域逻辑。
- **Channel / worker**：可靠事件通道断开时停止 worker，不逐条 `let _ = send(..)` 静默丢弃。长期运行的 worker 必须有 Rust 侧 owner 和 cancel。
- **`unsafe`**：保持最小范围，并写明具体的 `SAFETY:` 依据。

## 9. 需要先确认

用户已经明确要求且范围清楚的事项，不必重复确认。

- 新增或升级依赖：说明用途、许可证、体积和替代方案。前端用 pnpm（`frontend/pnpm-lock.yaml`）。
- 破坏性的跨边界变更：CLI JSON-RPC 协议、持久化文件格式、updater 协议、Mod ABI，以及删除用户数据。
- 修改 updater 或签名机制、`vendor/`、`[patch]`、发布 profile、Tauri capabilities 安全配置。
- 删除仍可能被用户使用的功能或迁移路径，例如 legacy JSON 回放、旧配置迁移。

## 10. 文件与 Git

- 保留无关的未提交改动。不创建 `.bak`、copy、snapshot 等备份文件，Git 就是回滚手段。本次产生的临时文件在结束前清理。
- 未经要求不 commit、push 或创建 PR；获准提交时只 stage 本次任务涉及的文件。
- 不提交以下内容：`target/`、`logs/`、`data/`、`NTE_Assets/`、`nte-resource-exporter/`、`Dumper-7/`、`tools/`、`node_modules/`、C# `bin/obj`、`.env`、抓包样本、解包数据。不把 payload、token、key 或本机完整路径写入日志、错误信息、测试 snapshot 或 commit message。

## 11. 验证

只运行与改动相关的检查。CI（[.github/workflows/build.yml](.github/workflows/build.yml)）负责全量检查。

| 改动 | 本地验证 |
| --- | --- |
| 文档 | `git diff --check -- <files>` |
| 前端 | `pnpm --dir frontend format:check`、`lint`、`typecheck`，以及相关的 `vitest` 文件 |
| Rust 核心 | `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`，以及 `cargo test <filter>` |
| Tauri adapter | 上一行命令加上 `--manifest-path src-tauri/Cargo.toml` |
| 依赖或 feature | `pwsh scripts/verify_architecture.ps1`、`cargo check --no-default-features --features cli` |

- 依靠编译器、类型、clippy 和行为测试来验证。不新增基于正则扫描源码的门禁脚本，也不为现有门禁脚本扩写规则。
- 本次引入的失败要定位根因并修复。与本次无关的失败只需记录命令和首个错误，不为了全绿去改无关文件。
- 窗口、HUD、游戏内行为需要用户在真实环境验收。未实际验证的内容在交付时明确标注为“未验证”。

## 12. 交付

说明改了什么、涉及哪些文件、运行了哪些验证命令及结果、哪些内容尚未验证，以及删除了多少行、新增了多少行。达到验收标准即停止，不顺手优化无关代码。

## 导航

| 主题 | 入口 |
| --- | --- |
| 包解析、战斗模型 | `src/engine/`、`tests/` |
| 状态归并、capture/history/replay | `src/core/reducer.rs`、`src/core/live_capture.rs`、`src/storage/history.rs` |
| CLI 协议 | `docs/CLI_PROTOCOL_ZH.md`、`src/api/`、`src/cli/` |
| Tauri 命令与 Channel | `src-tauri/src/state.rs`、`src-tauri/src/commands/`、`src-tauri/src/channels/`、`frontend/src/lib/tauri/` |
| 前端与 i18n | `frontend/README.md`、`frontend-guidelines.md`、`frontend/src/`、`res/languages/zh-CN.json` |
| Windows、HUD、窗口 | `src/platform/`、`src-tauri/src/windows/` |
| Mod ABI / IPC | `src/platform/mods_plugin.rs`；Loader 源码在私有 UETools-NTE 工作区 |
| 发布与更新 | `src/core/update.rs`、`src/platform/update_install.rs`、`docs/releases/` |
