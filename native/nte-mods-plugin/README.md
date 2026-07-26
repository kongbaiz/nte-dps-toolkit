# NTE Mods Plugin

`nte-mods-plugin` 是 NTE DPS Toolkit 的原生 Windows x64 受限 Mod 加载器。
它构建为单个 `dwmapi.dll` 代理，从 NTE DPS TOOL 软件目录读取受限 `.nte` 程序。常用游戏会话对象
由 Postman 环境变量风格的 `game.*` 内置值提供；条件判断、状态机和宿主调用顺序
由外部程序定义。DLL 只提供解释器、稳定对象入口、共享事件入口和白名单宿主原语。

本目录只保留编译和运行需要的源码、IPC 头文件及 Visual Studio 工程，不依赖
vcpkg 或客户端 SDK。

## 构建

要求：

- Visual Studio 2022，安装“使用 C++ 的桌面开发”工作负载；
- Windows 10/11 SDK；
- MSVC v143 x64 工具集。

```powershell
# 在“Developer PowerShell for VS 2022”中执行
msbuild .\nte-mods-plugin.sln /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m
```

原始输出：`x64\Release\dwmapi.dll`。工程的构建后步骤会将它自动同步到仓库根目录的
`plugins\dwmapi.dll`。随发行包提供的脚本位于 `plugins\nte-mods\`，默认启用集合
位于 `plugins\nte-mods.enabled`。

## NTE Script v4

`.nte` v4 由 `dwmapi.dll` 直接解析并编译为定长 VM 指令。DLL 只提供共享事件入口、
稳定 `game.*` 会话值、边界校验、IPC 传输和白名单原子宿主 API；重试、持久状态、
条件、循环、状态变化判定和转发时机位于外部脚本。客户端版本相关的 Offset 只存在
于宿主实现，普通 Mod 作者不需要维护指针链。

两个内置脚本的主控流程有明确分工：

- `equipment.nte` 使用内置 `game.player_state`，在对象变化后重置生命周期，以一秒间隔准备
  RPC 缓存，并且只在缓存属于当前 `PlayerState` 时开放装备 IPC 上下文；
- `combat-clock.nte` 使用内置 `game.player_controller`，分别读取权威
  `pause_mask/state_flags`，与四个持久状态比较，只转发初始值和真实变化。

完整实现和逐段注释直接位于 `plugins\nte-mods\equipment.nte` 与
`plugins\nte-mods\combat-clock.nte`。两份脚本不再通过仅有一行差异的高层
`observe/pump` 调用隐藏流程。修改重试间隔、变化条件或事件内容只需编辑对应脚本；
对象路径及版本 Offset 由 `game.session` capability 统一维护。每个程序必须用
`requires()` 准确声明实际使用的能力；多声明、
漏声明和未知能力都会使该程序保持未激活。

### 语言能力

- `None`、`True`、`False`、十进制及 `0x` 十六进制整数；运行值统一为 64 位；
- 最多 12 个函数局部变量；`state.name = VALUE` 声明最多 16 个 Mod 私有持久状态；
- `+ - * / % & | ^ << >> == != < <= > >= and or not`；一条表达式使用一个二元
  运算，复合计算可拆成多个中间变量；
- 最多八层 `if / elif / else`；
- `for name in range(COUNT):`，`COUNT` 是 `0..64` 的编译期整数；
- `def on_viewport_tick(event):` 事件函数、`event.viewport` 事件根对象；
- `game.viewport/instance/local_player/player_controller/player_state/player_character`
  是当前 Tick 的只读内置值，统一要求 `requires("game.session")`；
- 源码最大 16 KiB，最多 256 条编译后指令；超出预算的程序保持未激活；
- `#` 开头的整行注释，固定四空格缩进。

状态和循环示例：

```python
nte_mod(4)
mod("character-telemetry")
requires("viewport.tick")
requires("game.session")
requires("sdk.read")
requires("ipc")
state.last_hp = 0

def on_viewport_tick(event):
    character = game.player_character
    hp = sdk.character_hp_milli(character)
    if hp != state.last_hp:
        max_hp = sdk.character_hp_max_milli(character, False)
        ipc.emit("pre.character.health", hp, max_hp)
        health_per_mille = 0
        if max_hp > 0:
            scaled_hp = hp * 1000
            health_per_mille = scaled_hp / max_hp
        ipc.emit("post.character.health", hp, max_hp, health_per_mille)
        state.last_hp = hp
```

### 通用宿主 API

| API | 返回值／用途 | capability |
| --- | --- | --- |
| `game.viewport` | 当前 `ViewportClient` | `game.session` |
| `game.instance` | 当前 `GameInstance` | `game.session` |
| `game.local_player` | 当前本地玩家 | `game.session` |
| `game.player_controller` | 当前 `HTPlayerController` | `game.session` |
| `game.player_state` | 当前 `HTPlayerState` | `game.session` |
| `game.player_character` | 当前受控 `HTAbilityCharacter` | `game.session` |
| `memory.read_ptr(base, offset)` | 经边界校验的指针 | `memory.read` |
| `memory.read_u8/u16/u32/u64(base, offset)` | 无符号整数 | `memory.read` |
| `memory.read_i32(base, offset)` | 符号扩展整数 | `memory.read` |
| `memory.tarray_first(base, offset)` | 经 `count/capacity` 校验的首项 | `memory.read` |
| `memory.tarray_count(base, offset)` | 经校验的元素数 | `memory.read` |
| `memory.is_readable(pointer, size)` | 可读区间判断 | `memory.read` |
| `time.now_ms()` | 进程单调毫秒计时 | 无额外能力 |
| `ipc.bind(player_state, player_controller)` | 合并装备 IPC 上下文，空参数用 `None` | `ipc` |
| `ipc.emit("event.name", value...)` | 发布最多三个 64 位值的自定义事件 | `ipc` |
| `log.info("message")` | Debug 构建调试输出 | `log` |
| `equipment.cache_missing()` | 任意装备 RPC 缓存是否尚未建立 | `equipment` |
| `equipment.cache_ready(player_state)` | 缓存是否属于当前 `PlayerState` | `equipment` |
| `equipment.prepare(player_state)` | 尝试为当前对象准备一次装备 RPC 缓存 | `equipment` |
| `combat_clock.pause_mask(controller)` | 当前时停类型掩码 | `combat-clock` |
| `combat_clock.state_flags(controller)` | 当前时停状态标志 | `combat-clock` |
| `combat_clock.forward(pause_mask, state_flags)` | 将脚本判定的单次变化写入查询历史 | `combat-clock` |

Offset 上限为 `0x4000`；指针与 `TArray` Offset 还必须按指针宽度对齐。读取失败返回
零，不产生写内存操作。`combat_clock.forward` 只接受宿主读取能产生的时停位和状态
标志；装备 RPC 仍只开放既有十种白名单操作。

### SDK 读取 API

以下白名单来自仓库本地 China/Global SDK 中布局一致的
`HTPlayerController` 与 `HTAbilityCharacter` UFunction。运行时按类名和函数名解析，
没有编译期生成 SDK 依赖：

- `sdk.player_character(controller)` → `GetPlayerCharacter`；
- `sdk.player_state(controller)` → `GetHTPlayerState`；
- `sdk.game_paused(controller)` → `IsGamePaused`；
- `sdk.attack_target(character)` → `GetAttackTarget`；
- `sdk.current_weapon(character)` → `GetCurrentWeapon`；
- `sdk.character_level(character)` → `GetCharacterLevel`；
- `sdk.character_hp_milli(character)` → `GetHP`，结果乘以 1000；
- `sdk.character_hp_max_milli(character, fixed)` → `GetHPMax`，结果乘以 1000；
- `sdk.character_is_alive(character)` → `CharacterIsAlive`；
- `sdk.character_is_dead(character)` → `GetIsDead`；
- `sdk.character_is_controlled(character)` → `GetIsControlledCharacter`；
- `sdk.character_slomo_milli(character)` → `GetSlomoValue`，结果乘以 1000。

这些接口统一要求 `requires("sdk.read")`。函数只在脚本实际执行对应调用时解析。

### 加载和 IPC

两个内置脚本相互独立：

```text
nte-mods\equipment.nte
nte-mods\combat-clock.nte
```

`nte-mods.enabled` 决定实际读取的脚本。只加载装备功能：

```text
nte_mod_set 1
load equipment
```

只保留第一行表示不加载任何 Mod。运行时监听器会移除 Viewport Tick Hook 并关闭
IPC 管道；后续再次启用 Mod 时会重新解析脚本并安装共享 Hook。没有 `equipment`、
`combat-clock`、`game.session`、`sdk.read` 或 `ipc` capability 时，对应会话对象、
缓存、UFunction 和 IPC 分支保持未激活。

公开协议位于 `include\nte_mods_ipc.h`。IPC v7 保留装备操作和权威时停历史，
新增 `NTE_MODS_IPC_QUERY_MOD_EVENTS`。每条 `NteModEvent` 包含序号、FILETIME
时间戳、Mod ID、事件名和最多三个值；客户端按序号去重即可。标准库 Python
查询示例位于 `plugins\examples\query_mod_events.py`，可直接加载的
`character-telemetry.nte` 自定义 Mod 示例也位于同一目录。

NTE DPS TOOL 在实时抓包期间持续读取这条事件流，并把它送入共享
`EngineEvent -> CoreSignal` 管线。事件名以 `pre.` 开头时进入 `Preprocess` 阶段，
以 `post.` 开头时进入 `Postprocess` 阶段，其余事件进入普通 `Event` 阶段；前缀在
进入程序后会被去除。例如 `ipc.emit("pre.hit", id, value)` 会得到名称为 `hit`
的预处理消息。程序侧以序号去重，并在 Mod 工作台状态中保留最近 256 条消息，供后续
数据处理器消费。

脚本 ID 接受小写 ASCII 字母、数字、`-`、`_`、`.`。单个脚本语法错误、能力不匹配
或预算超限时仅跳过该 Mod；启用集合本身包含重复 ID、路径字符或格式错误时，整个
集合保持未激活。

## 通过 GUI 安装

Windows GUI 发布压缩包会把 `plugins` 目录放在 `nte-dps-tool.exe` 同级。在
“控制台 → Mod 工坊”中启用“游戏内 Mod 加载器”后，程序会展示风险与加载原理，
并锁定启用按钮 5 秒；取消按钮和
`Esc` 可立即关闭弹窗。
确认时必须先关闭 `HTGame.exe`；若同时检测到国服与国际服，需先选择客户端。
程序随后只将 DLL 写入所选客户端的
`Client\WindowsNoEditor\HT\Binaries\Win64` 目录。默认启用集合及脚本保留在
`nte-dps-tool.exe` 同级的 `plugins` 目录；程序把该工作区写入当前用户注册表，
游戏启动时会把 DLL 作为 `dwmapi.dll` 代理加载，DLL 再从软件工作区读取
`nte-mods.enabled` 和 Python 风格程序。监听器会在游戏运行期间检测保存、启用和
禁用更改；最后一个 Mod 被禁用后共享 Hook 与 IPC 都会退出。
刷新托管安装时，发行包原始的 `.nte` v1/v2/v3/v4 默认程序会迁移到当前内置变量
版本；用户编辑过的脚本保持原内容。

关闭该选项会删除带有本工具二进制签名的 DLL。程序不会覆盖其他
`dwmapi.dll`，也不会删除被外部替换的文件；这两种情况都会提示用户手动
处理。旧版曾写入游戏目录的启用集合与脚本会先迁移到软件工作区，再从游戏目录清理。
游戏目录最终只保留启用状态下的 `dwmapi.dll`。游戏目录改动可能触发完整性或反作弊检查，启用前应阅读并接受
GUI 中的完整风险声明。

公开的固定 IPC 布局位于 `include\nte_mods_ipc.h`。底层内存边界校验、
Viewport Hook、稳定会话对象、IPC 传输和白名单宿主原语保留在 DLL 内部；功能分支、
持久状态和执行顺序位于外部 `.nte` 程序。
