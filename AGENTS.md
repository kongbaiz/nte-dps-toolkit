# AGENTS.md

## 0. 执行协议：小步、直接、验证后停止

本文件适用于整个 `NTE DPS TOOL` 仓库。目标是让代理快速交付**满足当前需求的最小正确变更**，而不是展示架构能力、预判未知需求或追求一次性完美。

### 0.1 默认工作流

除非命中 Hardened 条件，按以下顺序一次完成：

1. 用 `git status` / `git diff` 和直接相关文件确认现状，保留无关工作树。
2. 从用户请求提炼 1～3 个可观察验收点；任务清楚时不要另写长计划。
3. 沿用现有结构实现一个最小、自包含、可验证的变更。
4. 只运行能覆盖本次影响面的最小验证集。
5. 验收点满足且验证通过后立即停止；不要继续“顺手优化”。

默认选择顺序：

```text
现有实现内的直接修改
> 复用同目录已有模式
> 小型局部 helper
> 新抽象 / 新依赖 / 新基础设施
```

### 0.2 反过度设计硬门槛

除非**当前需求、已复现缺陷、现有测试或测量数据**证明必要，否则禁止引入：

- 为“以后可能需要”准备的接口、参数、配置项、Feature Flag、扩展点或兼容层；
- 新的 service/repository/factory/manager/provider/wrapper 层；
- 只有一个调用方的 trait、泛型框架、插件机制或内部 DSL；
- 新依赖、新缓存、新队列、新 worker、新锁、新 Atomic、新持久化格式；
- 双实现、并行状态机、全仓迁移或与当前任务无关的重构；
- 没有基准、trace 或结构性证据支撑的性能优化。

复杂方案必须在动手前用不超过 5 行回答：

```text
当前具体问题：
现有直接方案为何不够：
证据（失败测试/trace/测量/明确约束）：
新增复杂度及其边界：
更简单方案为何被排除：
```

答不出来就使用更简单方案。不要为这 5 行再创建 RFC、ADR 或设计文档；只有用户明确要求或不可逆的大范围决策才需要独立文档。

### 0.3 抽象、复用与重构阈值

- 两处相似代码允许保持局部重复；出现第 3 个**真实且稳定**用例后才评估抽象。
- 抽象必须减少净复杂度，并至少有 2 个当前调用方；禁止先造抽象再寻找用途。
- 不因文件长、类型多或“看起来不优雅”单独触发拆分；只有职责混杂已经妨碍本次正确性、测试或维护时才拆。
- 修 bug/加功能时只做完成任务所需的邻近清理；大规模 rename、move、format、架构迁移另开任务。
- 优先删除无用代码，而不是为无用代码增加封装。
- 不复制 transaction、revision、subscription、trust-boundary 这类系统语义；它们应复用现有权威实现。

### 0.4 反过度思考与停止条件

以下情况才写详细计划或比较多个方案：

- 用户明确要求方案/评审；
- 命中 Hardened 通道；
- 存在不可逆的数据、协议、发布或安全决策；
- 仓库事实无法消除且会实质改变用户可见行为的歧义。

其它情况：

- 能从代码、测试、配置推断时直接执行，不要求用户重复信息。
- 只检查直接入口、调用方、测试和边界；不要把局部任务升级成整仓审计。
- 可逆决定直接采用最简单可行选项；失败后基于新证据调整。
- 找到一个满足验收且符合约束的方案后，不继续枚举“更优雅”的替代方案。
- 同一验证失败时先定位根因再改；禁止无证据地连续重写。
- 验收通过即完成。潜在改进只在确实影响当前交付时处理，否则不创建代码、不扩展范围。

### 0.5 文件与 Git

- 修改配置或已有文档前先显示/检查现有内容，不直接盲目覆盖。
- 只修改任务相关文件，保留所有无关未提交改动；禁止顺手清洗工作树。
- 直接编辑目标文件。除非用户明确要求生产回滚机制，否则不创建 backup、`.bak`、`.old`、copy、snapshot、recovery 或 rollback 脚本。
- 实现期间产生的临时文件必须在结束前删除。
- Git 是普通代码变更的回滚事实源。
- 默认不 `git commit`、不 `git push`、不创建 PR。

---

## 1. 优先级与变更分类

优先级：

1. 用户当次明确指令；
2. 更深层目录的 `AGENTS.md`；
3. 本文件。

代码与描述冲突时，以当前代码和可执行测试为准，并只报告与任务相关的偏差。

### 1.1 快速通道（默认）

适用于：

- 文档、注释、配置的局部修正；
- 纯 React 展示、CSS/布局、既有契约内的小交互；
- 不改变共享状态语义的小型 command/window adapter；
- 局部 bug fix；
- 确定性、无 I/O、无并发的小算法。

建议边界：净改动 `<= 200` 行、直接文件 `<= 8`、不新增依赖、不改变 DTO/schema/revision/lifecycle。超出建议边界不自动判错，但应先拆成可独立验证的小步。

快速通道不要求先写计划、RFC、全量设计分析或完整测试矩阵；执行 focused format/check/test 即可。

### 1.2 Hardened 通道

以下任一项出现，进入 Hardened：

- `src/core/reducer.rs`、`src/core/live_capture.rs`、`src-tauri/src/state.rs` 的共享状态语义；
- history/replay/capture session、`EngineEvent` / `CoreSignal`；
- generation/revision/sequence；
- Channel 背压、长期 worker、Mutex/RwLock/Atomic、锁顺序或 cancellation；
- 文件导入、JSON/PCAPNG、CLI JSON-RPC、Mod IPC 等外部输入；
- Tauri Contract version/schema；
- updater、FFI、`unsafe`、native plugin；
- shared root crate、CLI 与 Tauri 同时受影响。

Hardened 只做与当前变更直接相关的工作：

1. 列出受影响的不变量，不复述全仓规范；
2. 优先补能复现问题的 regression test；
3. 明确实际涉及的 trust boundary、lock/backpressure、owner/cancellation；
4. 实现最小修复，不借机重构邻接系统；
5. 按 §7.2 运行完整影响面验证。

改动行数少不能把真实 Hardened 变更降级；命中 Hardened 也不能成为扩大范围的理由。

---

## 2. 产品与架构边界

`NTE DPS TOOL` 是 Windows 桌面实时 DPS 工具。唯一桌面 UI 为：

```text
Tauri 2 + Vite + React + TypeScript + Tailwind CSS + shadcn/ui
```

主要产物：

- `nte-dps-tool`：Tauri 桌面程序；
- `nte-core`：UI-free stdio sidecar，Feature `cli`；
- `nte-updater`：独立更新执行程序；
- `dwmapi.dll`：游戏侧 NTE Mods Plugin；
- `desktop` Feature：共享桌面平台能力，不包含 UI framework/binary。

权威数据流：

```text
Npcap / PCAPNG / JSON ─┐
                       ├→ Rust Engine → Reducer → Domain Services
Game Mods → Mod IPC ───┘                     ↓
                                     bounded read model / contract
                                               ↓
                                      Tauri adapter → React
```

### 2.1 Rust 核心保持唯一事实源

- 抓包、解析、战斗归并、History、Replay、资源、更新、Mod IPC 和系统集成由 Rust 权威实现。
- Tauri 是 adapter，不是第二套业务核心。
- React 只消费 read model 并维护 UI-only state，不复制 Rust 领域规则。
- 不恢复旧 GUI、`gui` Feature、根 crate GUI binary 或平行 UI 状态机。
- `nte-core --no-default-features --features cli` 的依赖树不得出现 Tauri、WebView、React、egui、eframe、wgpu 或窗口库。

### 2.2 目录职责

| 能力 | 归属 | 禁止 |
| --- | --- | --- |
| 网络字节、Npcap、PCAPNG | `src/engine/` | Tauri/React 依赖 |
| 战斗模型 | `src/engine/model.rs` | UI/window 状态 |
| EngineEvent 归并 | `src/core/reducer.rs` | React 领域逻辑 |
| Capture/Replay session | `src/core/` | WebView/window 操作 |
| History/config/files | `src/storage/` + core service | React 持久化规则 |
| CLI JSON-RPC | `src/api/`, `src/cli/` | Tauri 类型 |
| Windows/FFI/system | `src/platform/` | React Win32 |
| Tauri command/channel/window | `src-tauri/src/` | 复制 reducer/history/parser |
| React UI | `frontend/src/` | 原始包解析/权威业务规则 |
| Native Mods Plugin | `native/nte-mods-plugin/` | 桌面 UI 逻辑 |

`AppState` 是组合 facade。只有当前需求已经形成清晰领域且现有 facade 无法保持不变量时才拆 service；不得仅因新增第 2 个字段机械造层，也不得持续把无关 Mutex/Atomic 堆进 `AppStateInner`。

---

## 3. 不可破坏的运行时不变量

### 3.1 State Mutation 与 Revision

每个权威 mutation 必须明确：

```text
改变了什么状态？
哪些 read model 失效？
哪些 revision 推进？
哪些 Channel 重新投影？
```

- revision 表示用户可观察状态变化，不表示事件数量。
- no-op 不 bump；状态变化不漏 bump；packet/combat/presentation revision 不混用。
- 新增或修改 `EngineEvent` handling 至少测试 mutation、no-op、revision/effect、live/replay 一致性。
- 修改既有 hit 时，覆盖“之后没有其它事件”仍能刷新投影。

### 3.2 高频路径

以下任一满足即为高频：`>= 4Hz`、per hit、per packet、active combat 每个 revision、多窗口消费同一状态。

高频路径禁止：

- clone 完整 `CombatState` 或完整 History；
- 每次刷新深拷贝大量 `String`/`Vec`；
- 为 UI 方便在抓包线程序列化大型 DTO；
- 同一 revision 为多个窗口重复昂贵 projection；
- 无边界的列表、snapshot 或 queue。

优先使用：

```text
authoritative state
→ closure/read projection
→ bounded DTO/page/aggregate
→ revision-aware Channel
```

先消除深拷贝和多余分配，再考虑 cache。cache 只有在测量证明重复成本显著时才引入，并说明 key/revision、invalidation、memory bound 和 stale behavior。

### 3.3 锁、I/O 与背压

热锁包括 `event_gate`、authoritative `CombatState` lock、capture/session 关键锁和每 packet/hit/event 访问的 Mutex。

持热锁时禁止：

```text
文件/持久化 I/O
网络、进程或系统探测
dialog、sleep、join
blocking send
updater/plugin RPC
WebView/window IPC
大型序列化
```

需要慢操作时：锁内提取最小值或 `mem::take` → 解锁 → 慢操作 → 通过 generation/token 合并。

持两个以上 Mutex 的函数必须减少嵌套并保证一致锁序；helper 不得隐式反向取锁。

每个 Channel/queue 必须就地定义：

```text
capacity | ordering | full policy | disconnect policy | droppable | producer | consumer
```

可靠语义事件可以 backpressure，但 consumer 不做慢 I/O；允许 drop 的 debug 数据必须有 drop counter/diagnostic。

### 3.4 外部边界与资源预算

文件、网络字节、JSON-RPC、Mod IPC、Tauri 参数、Npcap/FFI、Win32/system probe 均不可信：

- 外部数据不得触发 `unwrap/expect/assert/panic`；
- 文件读取前先用 metadata 校验 byte size，解析后再校验 count/version/nesting/string length/numeric range；
- 错误使用稳定 code/typed error，区分 `NotAFile`、`TooLarge`、`UnsupportedVersion`、`InvalidFormat`、`Io`、`Validation`；
- `ProbeFailed` 不折叠成 `false`/`None`；允许降级时也保留 degradation 状态；
- poisoned authoritative state 只有证明不变量仍成立时才能恢复，否则 fail closed。

### 3.5 Provenance、History 与 Replay

- source/generation/context 在对象产生时冻结；延迟持久化或 retry 不读取“当前值”反推旧来源。
- round cutover 是实时事务，history write 是锁外 side effect：不丢新事件、不混 round、慢盘不阻塞 capture、失败可 retry。
- Rust Contract 保证 `bounded history rows + exactly one live row`；前端验证，不用 `slice()` 修剪成正确状态。
- live、JSON import、PCAPNG replay 尽量复用同一 Engine/Reducer 语义。

### 3.6 Worker 生命周期

每个长期 worker 必须有 Rust 侧 owner 和 cancellation。owner 至少落到 window、capture session、replay operation、update transaction 或 plugin request group。

必须处理 normal stop、duplicate stop、owner destroyed、replacement、disconnect、operation failure 和 registry cleanup；React unsubscribe 不是唯一生命周期保证。除必须阻塞 OS API 外，优先已有 async runtime，不为每个订阅新增永久 OS thread。

---

## 4. 稳定 Contract 与 UI 边界

### 4.1 Tauri Contract / Command / Channel

- 跨边界只传显式 serde DTO、command、Channel、event 和稳定错误码；不暴露内部可变 state、裸句柄或领域内部结构。
- DTO 使用 `camelCase`，大整数保证 JS 安全，大列表分页/游标，Rust 端保证 required invariant。
- schema/version 变化时 Rust/TypeScript 同步；不为假想旧客户端增加兼容层。
- Command 用于有限操作；持续高频流使用有界 Channel，不做每 hit/packet/frame `invoke()`。
- subscription 有 id、owner、cancel；replacement/window destroy/disconnect 均停止旧 worker。
- TypeScript parser 在 typed client boundary 对 `unknown` 验证一次；不得用 `slice/default/?? []` 静默隐藏 required contract 错误。

### 4.2 TypeScript / React

- TypeScript 保持 `strict`；禁止无说明 `any` / `@ts-ignore`。
- 页面不直接裸 `invoke()`，使用现有 typed client。
- render 保持纯函数；Channel/timer/window 在 effect/hook 中完整 cleanup。
- Rust authoritative snapshot 与 UI-only state 分开；不订阅巨型 object 导致全树刷新。
- selector 保持稳定引用；loading/empty/error/stale 明确处理。
- 只有跨窗口或真正异步共享资源才引入 external store，并提供 `subscribe/getSnapshot/revision`；局部组件状态不升级为全局 store。

### 4.3 UI / i18n / Windows

- 优先现有 shadcn/ui + NTE 组合组件，不为一个页面创建新设计系统。
- 使用语义 Tailwind token，不散落主题常量。
- 透明、穿透、置顶、快捷键、多屏 DPI、窗口恢复由 Rust/Tauri 协调；React 只发送意图。
- user-visible 文案走 i18n；英文字符串为稳定 key，简中以 `res/languages/zh-CN.json` 为源。
- Dialog/Sheet/Drawer 有标题和可访问性；图标按钮有 Tooltip 或 `aria-label`。
- HUD/window 平台行为需要人工验收。

---

## 5. 实现约束

### 5.1 Rust

- Rust 2024 + rustfmt；遵循现有命名和模块风格。
- 新业务规则放到已有正确 domain 层；Tauri 不复制。
- 外部边界和领域层优先 typed `Result<T, DomainError>`，只在最终日志/contract boundary 格式化。
- `unsafe` 最小化并写具体 `SAFETY:` 依据。
- 序列化改动只处理已存在兼容要求，不猜测未来格式。
- 一个函数若同时承担 lock、I/O、projection、persistence、revision bump 中 3 项以上，拆出当前所需的最小边界。
- getter 不隐藏 O(N) 深拷贝；昂贵操作使用 `snapshot_` / `project_` / `load_` 等明确名称。

### 5.2 依赖与基础设施

新增或升级 Rust、Node、Tauri plugin、shadcn dependency 前，必须有当前需求并获得用户确认，同时说明用途、维护状态、许可证、体积和现有替代方案。

- 使用 lockfile 对应 package manager，不混用 npm/pnpm/yarn/bun。
- 优先已有标准库、现有 crate/package 和平台能力。
- CLI-only 依赖树不得引入 desktop UI crate。
- 普通任务不修改 updater protocol、`vendor/`、`[patch]`、发布 profile 或自定义签名更新协议。
- 资源维护/导出工具不成为主程序运行时依赖。

### 5.3 仓库卫生与敏感数据

不提交：

```text
target/ logs/ data/ NTE_Assets/ nte-resource-exporter/ Dumper-7/ tools/
node_modules/ C# bin/obj .env 抓包样本 密钥 完整解包数据
```

不把 PCAP 内容、完整 payload、本机完整路径、token 或 key 写入日志、前端 error、测试 snapshot 或 commit message。

保留现有产品边界：Mod 热更新候选全部成功后原子替换，失败保留上一工作版本；`master` 不恢复已关闭的研究功能；研究内容留在 research branch。

---

## 6. 测试策略：只证明本次行为

- 测试用户可观察行为、边界和不变量，不测试私有实现形状。
- 新逻辑覆盖 success 和最相关的 failure/boundary；不要为理论上不可达的排列组合制造测试矩阵。
- regression test 应先能失败、修复后通过；纯文档或机械改名不强制补测试。
- 不为写测试而引入生产抽象；优先从现有 public boundary 测试。
- 不使用脆弱的 CI 毫秒阈值；性能热点验证结构性不变量或使用显式 benchmark/trace。
- focused 验证通过后不要重复运行等价命令；只有 Hardened 或 CI 要求才跑全套。

### 6.1 常见边界

仅在变更实际涉及对应能力时覆盖：

- state mutation：success、no-op、boundary/error、revision effect；
- external input：valid、malformed、empty/null、too large/many、unsupported version，且失败后进程仍可用；
- lifecycle：normal/duplicate stop、owner destroyed、replacement、disconnect、failure、无 registry 泄漏；
- history/replay：live/json/pcapng source、retry provenance、round cutover、persist failure 不阻塞 capture。

---

## 7. 验证矩阵

选择能覆盖实际影响面的**最小充分集合**。未触及的层不跑；未运行项仅在用户预期或 Hardened 矩阵要求时说明原因。

### 7.1 文档 / 配置 / 快速 UI

文档或 `AGENTS.md`：

```text
检查 Markdown 结构、重复/矛盾规则、命令与路径是否与仓库一致
git diff --check -- <changed-files>
git diff -- <changed-files>
```

纯 React/UI：

```text
Prettier/format（仅改动文件）
pnpm --dir frontend typecheck
focused Vitest
```

小 Tauri adapter 且不触及 Hardened：

```powershell
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
focused cargo test --manifest-path src-tauri/Cargo.toml <filter>
```

局部 Rust：运行对应 crate 的 `cargo fmt --check`、`cargo check` 和 focused `cargo test <filter>`；不因一行局部修复自动跑三套全仓命令。

### 7.2 Hardened 完整验证

涉及 core/reducer/history/replay/concurrency/contract 时，至少执行：

```powershell
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --features desktop -- -D warnings

cargo check --bin nte-core --no-default-features --features cli
cargo test --no-default-features --features cli
cargo clippy --all-targets --no-default-features --features cli -- -D warnings

cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test

pwsh -NoProfile -File scripts/verify_architecture.ps1
pwsh -NoProfile -File scripts/verify_runtime_safety.ps1
```

仅在变更 frontend entry/build config 时加 `pnpm --dir frontend build`。涉及 native plugin 时执行现有 Release x64 MSBuild 验证。

若某项因环境或已知无关失败未通过，记录**原命令、首个根因和影响**；不要为了得到全绿修改无关文件。

---

## 8. Code Review：阻断真实问题，不追求完美

审查顺序：correctness → trust boundary → concurrency/lifecycle → performance bounds → contract → maintainability。只审本次变更及其直接调用面。

### 8.1 Blocker

只有以下情况要求当前变更必须处理：

- 不满足用户验收或存在可复现 bug；
- 破坏本文件的项目不变量；
- 引入安全、数据丢失、死锁、泄漏、无界资源或兼容性回归；
- 缺少足以证明已改行为的必要验证；
- 明显增加复杂度且 §0.2 无当前证据。

### 8.2 Non-blocking

以下默认为 `Nit:` 或留到独立任务，不阻塞交付：

- 个人风格偏好；
- 与当前行为无关的 rename/move/cleanup；
- 为未来场景准备的抽象或防御；
- 没有数据的微优化；
- “也许以后更统一”的全仓改造。

事实、测试和测量优先于意见。变更已经明确改善代码健康并满足验收时，不因不完美而继续打磨。

Hardened review 只回答与变更相关的问题：

- mutation 的 revision/effect 是否正确；
- 外部输入是否有 panic/资源预算/typed error；
- 热锁内是否有 I/O/sleep/join/blocking send，worker 是否可取消；
- 高频路径是否深拷贝或无界；
- Rust contract 与 TS parser 是否保持同一 invariant；
- 是否新增了没有证据的层、抽象或基础设施。

---

## 9. 发布与最终交付

- 默认不 commit/push/创建 PR；用户明确要求时只 stage 任务文件。
- 发布任务才验证版本、local/remote commit、CI、published release 和生产 rollback；普通编辑不预制发布/回滚流程。
- 最终回复保持简洁：改了什么、文件、验证命令与结果；仅在 Hardened 时补充不变量、未运行项和人工验证点。
- 不输出与交付无关的长篇推演、通用最佳实践或后续路线图。

---

## 10. 外部工程原则的项目化来源

以下公开资料只支撑本文件的“简单、小步、用证据决策”原则；项目具体边界仍以本文件前述规则为准：

- [Google Engineering Practices: Small CLs](https://google.github.io/eng-practices/review/developer/small-cls.html)：一个变更只解决一件事，小改动更易审查、验证和回滚。
- [Google Engineering Practices: What to look for](https://google.github.io/eng-practices/review/reviewer/looking-for.html)：检查复杂度，不实现尚未确认需要的未来功能。
- [Google Engineering Practices: The Standard of Code Review](https://google.github.io/eng-practices/review/reviewer/standard.html)：改善代码健康即可，不以“完美”阻塞进展。
- [Amazon Leadership Principles](https://www.amazon.jobs/content/en/our-workplace/leadership-principles)：Invent and Simplify；可逆决策不需要过度研究，强调 Bias for Action。
- [Microsoft Azure Well-Architected: Simplify](https://learn.microsoft.com/en-us/azure/well-architected/reliability/simplify)：只引入支撑当前目标的组件，避免趋势驱动和过细拆分。
- [Spotify Engineering: Agile à la Spotify](https://engineering.atspotify.com/2013/3/agile-a-la-spotify)：保持简单、频繁交付，用数据验证假设，同时不走捷径。

## 11.File modification policy

- Do not create backup, rollback, `.bak`, `.old`, copy, snapshot, or recovery files unless explicitly requested.
- Do not duplicate existing files solely to preserve their previous contents.
- Git is the source of truth for rollback and recovery.
- Before editing, use `git status` / `git diff` when necessary to understand existing changes.
- Preserve unrelated uncommitted user changes.
- Make edits directly to the intended files.
- Temporary files created during implementation must be removed before finishing.
- Do not create rollback scripts or rollback artifacts unless the task explicitly requires a production rollback mechanism.
- Do not spend time preparing rollback artifacts for normal code edits.
