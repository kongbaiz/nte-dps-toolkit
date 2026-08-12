# fix/a-group-hardening 可靠整改方案

> 目标分支：`fix/a-group-hardening`
> 适用范围：Rust Core / Engine / Reducer、Tauri State / Contract / Channel、CLI stdio、History / Replay、React typed client / bootstrap
> 文档目的：把本轮项目级审查发现的问题转化为可执行、可验证、可回滚的整改计划，并避免用一次大重构掩盖多个独立风险。

---

## 1. 总体结论与整改原则

当前整体架构方向可以保留：

```text
Npcap / PCAPNG / JSON / Mod IPC
             ↓
         Rust Engine
             ↓
        Core Reducer
             ↓
       Domain Services
             ↓
      Read Model / DTO
             ↓
      Tauri Adapter
             ↓
       React / shadcn
```

本轮不建议推翻 Core → Reducer → Contract → React 的主结构。真正需要修复的是以下四类系统性薄弱点：

1. **状态变更与 revision 没有建立一一对应的显式协议**；
2. **实时路径与持久化/投影路径耦合过深**，存在深拷贝和锁内 I/O；
3. **外部输入/进程边界缺少统一资源预算和失败语义**；
4. **`AppState` 承担过多职责**，导致锁、revision、生命周期和错误类型难以推理。

整改必须遵循以下原则：

- **先补回归测试，再改行为**；
- **先修数据正确性和崩溃风险，再做性能重构**；
- **实时抓包路径不得等待磁盘、窗口、插件或前端**；
- **权威状态只在 Rust 修改，前端只消费版本化投影**；
- **禁止用“大规模重构”代替逐项证明问题已经关闭**；
- 每个阶段结束后必须满足独立验收标准，阶段之间可以单独提交和回滚。

---

## 2. 风险优先级

| ID | 级别 | 问题 | 主要风险 |
| --- | --- | --- | --- |
| R1 | P1 | `ModScript` 修改 `CombatState` 后不触发 frontend revision | Detail/目标归属长时间显示旧数据 |
| R2 | P1 | 高频投影深拷贝完整 `CombatState` | 5 万 hit 后 CPU、分配、卡顿明显 |
| R3 | P1 | `archive_and_reset` 持 `event_gate` 执行持久化 | 慢盘导致可靠事件队列背压，抓包停顿 |
| R4 | P1 | Replay JSON 无读取前大小限制 | 超大文件导致高内存峰值甚至 OOM |
| R5 | P2 | Replay History 来源被硬编码成 `Live` | 历史来源和诊断语义错误 |
| R6 | P2 | CLI 对未知 Mod 响应 `request_id` 使用 `expect` | 外部异常可直接终止 sidecar |
| R7 | P2 | Channel worker 生命周期主要依赖 JS unsubscribe | 窗口异常销毁时线程可能残留 |
| R8 | P2 | `Unknown` hit 会更新 `last_outgoing_hit_at` | 自动分轮语义不稳定 |
| R9 | P3 | 临时 201 条历史时前端裁剪可能删除 Live round | 极端 undo 边界下实时轮次不可选 |
| R10 | P3 | 游戏进程探测失败被折叠成 `false` | 系统故障被误显示为“游戏未运行” |
| R11 | P2/P3 | `AppState` God Object、重复事务代码、字符串错误 | 后续修改容易重新引入竞态和语义漂移 |
| R12 | P3 | Avatar 刷新依赖 mutable function identity + `cloneElement` | 隐式协议难维护，新增消费者容易漏刷新 |

---

# 3. 实施顺序

建议严格按以下顺序推进：

```text
Phase 0  基线与回归测试
    ↓
Phase 1  正确性 / 崩溃 / 文件资源边界
    ↓
Phase 2  实时热路径与归档解耦
    ↓
Phase 3  Channel 生命周期 / 并发语义
    ↓
Phase 4  契约与边界条件收口
    ↓
Phase 5  AppState 分层、Typed Error、重复代码治理
    ↓
Phase 6  前端 external store 正规化
    ↓
Phase 7  CI / 静态策略固化
```

不要先做 Phase 5。
先拆 `AppState` 会同时改变锁、revision 和调用结构，容易让本轮已知问题变得更难验证。

---

# 4. Phase 0：建立可验证基线

## 4.1 目标

在修改行为前，把当前审查问题变成自动化回归测试。
任何一项修复必须至少有一条“修复前失败、修复后通过”的测试。

## 4.2 必加测试

### T0-1：ModScript revision

场景：

1. 写入一个 target 未知的 Hit；
2. 记录 `LiveCaptureService::revision()`；
3. 发送能够回填 enemy target 的 `ModScriptEvent`；
4. 不再发送其它 hit/event；
5. 验证：
   - hit 的 `target_id` / `target_name` 已改变；
   - frontend/capture revision 增加；
   - Detail snapshot 能看到新 target。

建议位置：

- `src/core/live_capture.rs`
- `src/core/reducer.rs`
- 必要时 `src-tauri/src/contract/main_dps_detail.rs`

### T0-2：Replay provenance

分别执行 JSON replay、PCAPNG replay，完成后生成 History archive，验证：

```text
JSON    -> CaptureQualitySource::JsonReplay
PCAPNG  -> CaptureQualitySource::PcapngReplay
Live    -> CaptureQualitySource::Live
```

来源必须跟随 archive 本身冻结，不能在延迟持久化时重新读取当前 session 的 source。

### T0-3：未知 Mod response 不得 panic

构造：

```text
pending request ids = {1}
response.request_id = 99
```

验证 CLI：

- 不 panic；
- 不删除 request 1；
- 能继续处理下一条正常 JSON-RPC；
- 记录可诊断 warning 或忽略原因。

### T0-4：Replay JSON 大小限制

使用 metadata 模拟或临时文件：

- 等于上限：允许进入解析；
- 大于上限：读取文件内容前返回稳定错误；
- 非文件：拒绝；
- 扩展名和实际入口一致。

不要用真正数百 MB fixture；测试应通过 metadata/小阈值 helper 完成。

### T0-5：History Live round 永远存在

构造：

- 200 条 history；
- 201 条 history（模拟 undo 临时超限）；
- 部分 history 无 details；

验证 Main DPS Contract：

- 最多暴露允许数量的 history；
- 恰好一个 `live == true && id == None`；
- live round 不因前端截断消失。

### T0-6：Unknown direction 的 idle policy

推荐产品语义：**自动分轮只由 confirmed outgoing hit 延长。**

测试：

- incoming 不更新；
- unknown 不更新；
- outgoing 更新。

如果产品明确要求 unknown 也延长，则变量和接口必须重命名为 candidate/output activity，不能继续叫 `last_outgoing_hit_at`。

## 4.3 基线验收

Phase 0 完成条件：

- 新测试能准确描述本轮问题；
- 测试不依赖 UI；
- 无 wall-clock 脆弱阈值；
- 不先改生产行为来“让测试好写”。

---

# 5. Phase 1：正确性、崩溃与资源边界

## 5.1 R1：建立显式 State Mutation → Revision 协议

### 问题本质

当前 `CoreSignal` 同时承担 reducer 事件分类和 frontend/packet 是否刷新，但 `ModScript` 事件可能真正修改现有 hit，却仍被归类为“不影响 frontend”。

### 推荐方案

让 reducer 返回显式 effects，而不是让 `LiveCaptureService` 根据事件名字猜影响范围。

推荐模型：

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoreEffects {
    pub combat_projection_changed: bool,
    pub packet_projection_changed: bool,
    pub inventory_changed: bool,
}

pub enum CoreSignal {
    Applied {
        effects: CoreEffects,
        mod_script: Option<ModScriptEvent>,
    },
    Status(String),
    Warning(String),
    Error(String),
    CaptureStopped,
}
```

如果不希望本轮调整过大，可使用最小版本：

```rust
CoreSignal::ModScript {
    event: ModScriptEvent,
    state_changed: bool,
}
```

并修改：

```rust
CombatState::apply_mod_script_event(...)
```

返回：

```rust
enum ModScriptApplyOutcome {
    Unchanged,
    ProjectionChanged,
}
```

禁止只根据事件种类推断是否需要刷新。

### 验收

- T0-1 通过；
- CLI 的 `match CoreSignal` 同步更新；
- 不因为所有 ModScript 都 bump revision 而产生无意义刷新；
- effect 的命名描述“状态影响”，而不是 UI 名称。

---

## 5.2 R4：Replay JSON 资源预算

限制应放在**共享 Rust 导入边界**，不能只放 Tauri drag/drop command。

建议：

```rust
pub const MAX_CAPTURE_JSON_IMPORT_BYTES: u64 = ...;

pub fn validate_capture_json_import(path: &Path) -> Result<(), CaptureImportError>
```

在 `import_capture_json()` 真正 `read_to_string()` 前执行。

Tauri 可以提前校验以提供更快错误，但共享入口必须再次保证。

读取前必须检查：

- path 是普通文件；
- metadata size <= limit；
- 支持的类型；
- 错误返回 typed error。

解析后继续检查：

- packet/hit/item 数量；
- nested arrays；
- 版本；
- numeric finite/range；
- 适用的字符串长度预算。

建议错误类型：

```rust
enum CaptureImportError {
    NotAFile,
    TooLarge { size: u64, limit: u64 },
    UnsupportedVersion(u32),
    InvalidJson(String),
    Io(std::io::Error),
}
```

### 验收

- T0-4 通过；
- 超限文件在 `read_to_string` 前退出；
- Tauri/CLI 映射为稳定错误 code；
- 错误信息不泄露完整本机路径。

---

## 5.3 R5：History 来源随 archive 冻结

新增：

```rust
struct PendingHistoryArchive {
    details: HistoryCombatDetails,
    source: CaptureQualitySource,
}
```

或直接：

```rust
struct PendingHistoryArchive {
    prepared: PreparedHistoryArchive,
}
```

关键要求：`source` 在 round 被切出时确定，之后即使 session 已切换，也不能改变。

修改：

- `prepare_current_history_archive`
- `archive_current_history_round`
- Abyss pending archive queue
- replay stop/auto archive 路径

禁止把 `CaptureQualitySource::Live` 作为通用 history archive 默认值。

### 验收

- T0-2 通过；
- pending retry 后来源仍不变；
- live/replay 连续切换不会污染上一条 archive。

---

## 5.4 R6：外部 Mod 响应不得触发 panic

修改 CLI：

```rust
let Some(id) = self.pending_equipment_requests.remove(&response.request_id) else {
    log::warn!(...);
    return true;
};
```

还要覆盖：

- duplicate response；
- stale response；
- plugin 重启后旧 response；
- submit 失败后迟到 response。

以下边界永远不得用 `expect/unwrap/assert` 处理不可信数据：

- Mod IPC；
- Tauri command 输入；
- JSON/PCAP 文件；
- Npcap/FFI 返回；
- Win32/system probe；
- stdio JSON-RPC；
- 网络解析。

### 验收

- T0-3 通过；
- malformed response 后 sidecar 仍可服务；
- pending map 不被错误清空。

---

# 6. Phase 2：实时热路径与持久化解耦

## 6.1 R2：消除周期性完整 `CombatState` 深拷贝

废弃高频调用中的：

```rust
fn main_presented_combat_state(&self) -> CombatState
```

改成投影式访问：

```rust
fn with_main_presented_state<R>(
    &self,
    project: impl FnOnce(&CombatState) -> R,
) -> R
```

推荐 Presentation 模型：

```rust
enum MainPresentation {
    Live,
    Paused(Arc<CombatState>),
    History {
        record_id: String,
        state: Arc<CombatState>,
    },
}
```

规则：

- **Live**：直接在 `LiveCaptureService::with_state` 内投影，不 clone；
- **Paused**：暂停瞬间只 clone 一次；
- **History**：选择历史轮次时转换一次并缓存；
- UI 每次刷新只 clone 最终 DTO。

### Detail 投影

不要再生成两层 O(N) 中间向量。改成单次扫描：

```text
for hit in source.hits:
    更新 base aggregate
    更新 filter aggregate
    如果 row_index 在 page window：
        生成 row DTO
```

一次扫描完成：

- total hits；
- total damage；
- max row damage；
- hit type metrics；
- skill/qte aggregates；
- pagination rows。

### 性能硬约束

任何 >= 4Hz 的刷新路径：

- 禁止 clone 整个 `CombatState`；
- 禁止按 hit 深复制 String，除最终返回页；
- 禁止多个 O(N) 中间 Vec；
- O(N) 扫描必须低分配；
- 引入 cache 前先证明单扫描仍不足。

### 验收

- Main/Detail live 路径无 `with_state(Clone::clone)`；
- paused/history 只在切换时 clone/转换一次；
- 50k hit 下结果正确；
- 分配行为不再随“完整 state clone”线性放大。

---

## 6.2 R3：归档持久化移出 `event_gate`

不要采用“释放 gate 写盘，然后重新加锁 reset 旧 state”的方案，因为写盘期间新事件会进入旧 round。

推荐：**先原子切轮，再独立持久化。**

极短的 `event_gate` 临界区只做：

```text
1. mem::take 当前 CombatState
2. 安装新的空 CombatState
3. 重置 round runtime
4. bump packet session / revision
5. 释放 event_gate
```

随后：

```text
旧 state
  ↓
prepare archive
  ↓
persist
  ├─ success -> history revision++
  └─ failure -> PendingHistoryArchive retry queue + warning
```

示例：

```rust
pub struct CutRound {
    pub state: CombatState,
    pub source: CaptureQualitySource,
}

pub fn cut_round(&self) -> Option<CutRound> {
    let _gate = self.event_gate.lock().recover_poison();
    let mut state = self.state.lock().recover_poison();

    if !state_has_archivable_data(&state) {
        return None;
    }

    let archived = std::mem::take(&mut *state);
    let source = *self.quality_source.lock().recover_poison();

    self.reset_round_runtime_locked();
    self.bump_packet_session();
    self.bump_revision();

    Some(CutRound { state: archived, source })
}
```

### 手工 New Round 语义

推荐：

- round 切换成功即认为用户操作完成；
- history persist 失败时不回滚新 round；
- 旧 round 进入 retry queue；
- UI 显示 warning。

优先保证：

1. 实时数据不丢；
2. round 边界不混乱；
3. 磁盘故障不阻塞抓包。

### 热锁内禁止

- `std::fs::*`
- atomic file write
- 大对象 JSON serialization
- network/process probe
- file dialog
- `thread::sleep`
- `JoinHandle::join`
- blocking channel `send`
- updater/plugin RPC

### 验收

- history I/O 不发生在 `event_gate` 内；
- 持久化失败时旧轮次可 retry；
- 新 hit 在切轮后进入新 round；
- reliable channel 不因历史保存而停顿。

---

# 7. Phase 3：Channel 生命周期与并发语义

## 7.1 R7：Stream 必须绑定窗口生命周期

Stream registry 增加 owner：

```rust
struct StreamEntry {
    owner_window: String,
    cancel: CancellationHandle,
}
```

提供：

```rust
begin_stream(owner_window, subscription_id)
stop_stream(subscription_id)
stop_streams_for_window(owner_window)
```

窗口 destroyed/closed 时 Rust 主动调用：

```rust
state.stop_streams_for_window(window.label())
```

### worker

优先 Tauri async runtime + cancellation token/watch + async interval。

如果本轮不迁移 async runtime，至少做到：

- `window.is_visible()` 返回 Err 时退出；
- owner window destroyed 时 stop；
- `finish_stream` 永远执行；
- replacement subscription 不遗留旧 worker。

### 验收

- 显式 unsubscribe：退出；
- subscription replacement：旧 worker 退出；
- window destroy without unsubscribe：退出；
- channel disconnect：退出；
- registry 无遗留 entry。

---

## 7.2 明确锁层级

建议：

```text
event_gate
  -> state
  -> 小型 runtime mutex
```

禁止反向顺序。

任何函数持两个以上 Mutex 时：

- 注释说明原因/顺序，或拆分；
- 测试覆盖 stop/reset/replay 并发；
- 禁止嵌套锁中 I/O。

---

## 7.3 bounded channel 定义 Full/Disconnected 语义

每个新 Channel 写明：

```text
capacity
ordering
full policy
disconnect policy
droppable?
```

可靠 lane 可以 backpressure，但 consumer 热路径不能做慢 I/O。debug lane 可以 drop，但必须计数。

---

# 8. Phase 4：契约与边界条件收口

## 8.1 R8：Unknown direction 与 auto-round

推荐：

```rust
last_confirmed_outgoing_hit_at
```

仅 `HitDirection::Outgoing` 更新。

如果业务确实要让 Unknown 阻止 auto-round，则改名为 `last_output_activity_at`，并明确 outgoing + unknown 语义。

---

## 8.2 R9：History 上限由 Rust Contract 保证

Rust 输出保证：

```text
<= MAX_HISTORY_RECORDS 条历史 + 1 条 Live
```

不要依赖 TypeScript `.slice(0, 201)` 静默修复。

前端 parser 应验证：

- 列表长度；
- 恰好一个 live row；
- live row `id == null`；
- 超过协议上限直接 contract error。

---

## 8.3 R10：游戏进程探测使用三态

建议：

```rust
enum GameDetectionStatus {
    Running,
    NotRunning,
    ProbeFailed,
}
```

Contract 示例：

```json
{
  "gameDetected": false,
  "gameDetectionStatus": "probeFailed"
}
```

如改变 Contract：

- 明确 bump version；
- Rust / TS parser 同步；
- onboarding 对 `ProbeFailed` 显示检测失败，而不是“请启动游戏”。

---

# 9. Phase 5：AppState 分层与 Typed Error

此阶段只在 P1/P2 功能风险关闭后执行。

## 9.1 保留 `AppState` façade，内部逐步拆服务

目标：

```text
AppState
├── CaptureSessionService
├── PresentationState
├── HistoryService
├── SettingsService
├── EquipmentService
├── UpdateService
└── DiagnosticsService
```

### 第一批：PresentationState

迁移：

- main presentation paused；
- paused snapshot；
- selected round；
- selected abyss half；
- detail request；
- presentation/main revisions。

### 第二批：HistoryService

迁移：

- history transaction；
- revision；
- cache；
- pending retry；
- history undo；
- archive orchestration。

Capture 只负责切出 round，History 负责保存。

### 第三批

再拆 Settings / Update / Equipment。

`AppState` 保持 façade method，避免一次改动全部 command。

---

## 9.2 Typed Error

逐步消除领域层：

```rust
Result<_, String>
```

改为：

```rust
enum SettingsError { ... }
enum HistoryError { ... }
enum ReplayImportError { ... }
enum StreamError { ... }
```

只在 adapter boundary：

```rust
impl From<HistoryError> for CommandError
```

禁止依赖错误字符串比较业务状态。

---

## 9.3 去除重复事务代码

统一 config update 的 sanitize / equality / persist / swap / revision effect。

例如：

```rust
enum ConfigRevisionEffect {
    None,
    Settings,
    PresentationAndSettings,
}
```

不要为了两处调用建立复杂 trait。

---

# 10. Phase 6：React Avatar External Store

把：

```text
mutable exported function binding
+ function identity invalidation
+ cloneElement
```

改成显式 external store：

```ts
export function subscribeCharacterAvatarCatalog(listener: () => void): () => void
export function getCharacterAvatarCatalogRevision(): number
export function resolveCharacterAvatar(charId: number): string | null
```

React：

```ts
export function useCharacterAvatar(charId: number): string | null {
  useSyncExternalStore(
    subscribeCharacterAvatarCatalog,
    getCharacterAvatarCatalogRevision,
    getCharacterAvatarCatalogRevision,
  )
  return resolveCharacterAvatar(charId)
}
```

Canvas/non-React：

```ts
subscribeCharacterAvatarCatalog(redraw)
```

### 验收

- 首屏不等待 avatar catalog；
- catalog 完成后 React 自动更新；
- Canvas 自动 redraw；
- 不依赖 function identity；
- `renderWindow` 不再需要 `cloneElement` 来刷新目录。

---

# 11. Phase 7：固化到 CI / Policy

## 11.1 新增 runtime safety policy

建议新增：

```text
scripts/verify_runtime_safety.ps1
```

机械检查至少包括：

1. `src-tauri` 高频投影不得出现 `with_state(Clone::clone)`；
2. `channels/` 新增裸 `thread::spawn + sleep` 需要 whitelist；
3. shared replay import 入口必须经过 size limit；
4. 外部边界新增 `expect/unwrap/assert` 提醒 review；
5. Contract required list 不允许前端 silent slice。

静态脚本不能证明锁/I/O 正确性，仍需 tests + review checklist。

## 11.2 CI 常驻回归

- ModScript revision；
- replay source；
- oversized replay；
- unknown Mod response resilience；
- history live row invariant；
- stream lifecycle；
- round cutover + persist failure retry。

## 11.3 性能防回归

不要使用易抖动的 CI wall-clock 阈值作为唯一 gate。

优先验证结构：

- 无完整 state clone；
- bounded output；
- 单扫描；
- 无热锁 I/O。

---

# 12. 修改文件建议

预计主要文件：

```text
src/core/reducer.rs
src/core/live_capture.rs
src/core/history.rs
src/engine/capture.rs
src/engine/model.rs
src/cli/...                 # 以实际 CLI server 文件为准
src/storage/history.rs

src-tauri/src/state.rs
src-tauri/src/contract/main_dps.rs
src-tauri/src/contract/main_dps_detail.rs
src-tauri/src/channels/main_dps.rs
src-tauri/src/channels/main_dps_detail.rs
src-tauri/src/commands/main_dps.rs

frontend/src/lib/tauri/main-dps-contract.ts
frontend/src/lib/character-avatar.ts
frontend/src/entries/window-bootstrap.tsx

scripts/verify_runtime_safety.ps1
AGENTS.md
```

不要在一个提交中同时完成所有阶段。

---

# 13. 推荐提交拆分

1. `test: cover capture revision and replay hardening regressions`
2. `fix: harden reducer effects and replay boundaries`
3. `perf: remove full combat-state clones from desktop projections`
4. `refactor: decouple history persistence from capture event gate`
5. `fix: bind desktop streams to window lifecycle`
6. `fix: make detection and history contract states explicit`
7. `refactor: split presentation and history runtime from app state`
8. `refactor: use explicit avatar external store`
9. `chore: enforce runtime safety invariants`

---

# 14. 验证矩阵

本轮涉及 reducer、共享状态、并发、历史、CLI、Tauri Contract，**不允许用普通局部快速通道代替完整验证**。

## Rust Root

```powershell
cargo fmt --check
cargo check
cargo test
cargo clippy --all-targets --features desktop -- -D warnings

cargo check --bin nte-core --no-default-features --features cli
cargo test --no-default-features --features cli
cargo clippy --all-targets --no-default-features --features cli -- -D warnings
```

## Tauri

```powershell
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## Frontend

```powershell
pnpm --dir frontend lint
pnpm --dir frontend typecheck
pnpm --dir frontend test
```

涉及 bootstrap / contract / build entry：

```powershell
pnpm --dir frontend build
```

## Architecture / Safety

```powershell
pwsh -NoProfile -File scripts/verify_architecture.ps1
pwsh -NoProfile -File scripts/verify_runtime_safety.ps1
```

所有阶段合并前至少执行一次仓库现行完整 CI 对应矩阵。

---

# 15. 并发专项矩阵

| 场景 | 预期 |
| --- | --- |
| live capture + 手工 new round | 新 hit 进入新 round，不丢、不混 |
| live capture + 慢/失败 history persist | capture 不停顿，旧 archive 进入 retry |
| replay stop + archive | source 不丢失 |
| replay -> live 快速切换 | 上一 archive source 不被新 session 覆盖 |
| pause -> live events -> resume | pause snapshot 稳定，resume 回到最新 live |
| selected history -> 新 outgoing hit | 按产品规则返回 live |
| detail window destroy without unsubscribe | worker 退出 |
| stream subscription replacement | 旧 worker 退出、新 worker 保留 |
| Mod duplicate/stale response | sidecar 不 panic |
| oversized JSON replay | 读取前拒绝 |

---

# 16. 数据正确性不变量

### Capture / Reducer

- 所有真正改变用户可见 projection 的 reducer 操作都会推进对应 revision；
- revision 描述状态改变，不描述单纯事件到达；
- `ModScript` 是否 bump 由实际 mutation outcome 决定。

### Round / History

- round boundary 与磁盘持久化分离；
- round 切换原子；
- persist 失败不得导致实时事件丢失；
- archive source 在切轮时冻结；
- retry 不改变内容和来源。

### Projection

- >= 4Hz 路径不深拷贝 `CombatState`；
- live/paused/history 使用同一 projection 规则；
- page limit 由 Rust Contract 保证；
- TS parser 校验协议，不静默删除必需记录。

### External Boundary

- 文件有 byte/count/version budget；
- IPC response 不可信；
- system probe 的失败和正常 false 分开；
- external failure 不 panic。

### Lifecycle

- 每个长期任务有 owner；
- 每个长期任务有 cancellation；
- owner 消失后任务退出；
- registry 不保留结束任务。

---

# 17. 回滚策略

- CoreEffects 回归：只回滚 R1 commit；
- no-clone projection 回归：只回滚 performance commit；
- cut-round 语义回归：只回滚 archive decoupling；
- async worker 不稳定：可暂留 thread worker，但必须保留 owner-window cancellation。

---

# 18. Done Definition

- [ ] R1～R10 均有自动测试或明确验证；
- [ ] 外部数据路径不存在可由输入触发的新增 panic；
- [ ] Replay JSON 在完整读取前有大小限制；
- [ ] History source 对 live/json/pcapng 正确；
- [ ] 高频 Main DPS / Detail 不完整 clone `CombatState`；
- [ ] History 持久化不在 `event_gate` 临界区；
- [ ] stream 能在窗口销毁时退出；
- [ ] History Contract 永远包含 live row；
- [ ] system probe failure 不再等价于 false；
- [ ] `AppState` 至少拆出 Presentation / History 两个明确子职责，或已有等价 façade；
- [ ] 新 `AGENTS.md` 纳入仓库规则；
- [ ] runtime safety policy 进入 CI；
- [ ] Rust / CLI / Tauri / Frontend / Architecture 门禁通过；
- [ ] 未通过项有明确 blocker，不以“理论上应该没问题”代替验证。

---

# 19. 推荐最终结构

```text
src/core/
  capture/
  reducer.rs
  live_capture.rs
  history.rs
  presentation/

src-tauri/src/
  state.rs             # composition façade
  services/
    presentation.rs
    history.rs
    settings.rs
    equipment.rs
    updates.rs
  contract/
  commands/
  channels/
  windows/

frontend/src/
  lib/tauri/
  stores/
  features/
```

不要求一次性移动文件。优先通过“小 façade + 内部 service”渐进迁移。

---

# 20. 最终需要消灭的错误模式

```text
状态真的变了，但没有显式 mutation effect
高频展示为了方便直接复制权威状态
为了事务一致性，把磁盘 I/O 放进实时锁
把外部输入当内部不变量使用 expect
任务只靠前端 cleanup，没有 Rust owner 生命周期
服务端输出越界后让前端 slice 掉
错误和“正常 false”共用同一个值
```

后续 `AGENTS.md` 和 CI 应把这些模式提升为仓库级阻断项。
