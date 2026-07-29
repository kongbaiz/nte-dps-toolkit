# NTE Mods Plugin

`nte-mods-plugin` 是 NTE DPS Toolkit 的原生 Windows x64 受限 Mod 加载器。
它构建为单个 `dwmapi.dll` 代理，从 NTE DPS TOOL 软件目录读取受限 `.nte` 程序。常用游戏会话对象
由 Postman 环境变量风格的 `nte::game::*` 内置值提供；条件判断、状态机和宿主调用顺序
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

## NTE C++ v5

`.nte` 正式使用受限 NTE C++ v5，由 `dwmapi.dll` 解析并编译为定长 VM 指令。源码采用
标准 C++ 的声明、花括号、分号、布尔值、空指针与命名空间调用；DLL 是微内核，只提供共享
事件入口、稳定 `nte::game::*` 会话值、边界校验、IPC 传输和白名单原子宿主 API；外部 Mod
则是操作层，负责 IPC 服务路由、重试、持久状态、条件、循环、状态变化判定和转发时机。
通用会话入口的 Offset 只存在于宿主实现；新功能自己的字段 Offset、采样频率、缓存键、
变化条件、UFunction 参数布局和事件格式都留在外部脚本中。

运行时 ABI 建立后，新增 Mod 的交付单元只有 `.nte` 和资源文件；不增加功能专用 C++ 类型、
IPC 操作或 DLL 服务，也不重新
构建 DLL。DLL 的后续变更只用于运行时 ABI 升级、引擎版本公共入口迁移和宿主缺陷修复，
不用于逐 Mod 适配。`plugins/examples/reflection-events.nte` 展示了只靠现有 ABI 查找
UFunction、组装参数、调用函数、订阅 ProcessEvent 并发布结果的完整流程。

三个默认启用脚本的主控流程有明确分工：

- `equipment.nte` 使用内置 `nte::game::player_state`，在对象变化后重置生命周期，以一秒间隔准备
  RPC 缓存，并用 `NTE_ROUTE_IPC(...)` 发布装备操作服务；只在缓存属于当前 `PlayerState`
  时开放装备 IPC 上下文；
- `combat-clock.nte` 使用内置 `nte::game::player_controller`，分别读取权威
  `pause_mask/state_flags`，与四个持久状态比较，只转发初始值和真实变化，并在脚本内
  发布历史查询服务；
- `enemy-telemetry.nte` 从通用 `nte::game::player_controller` 入口使用只读内存原语定位攻击
  目标和生命值，以目标键缓存首次读取的配置名哈希，并通过通用 Mod 事件流发布多目标
  identity/vitals；DLL 中没有敌人专用查询、结构体或 IPC 操作。

完整实现直接位于 `plugins\nte-mods\equipment.nte`、
`plugins\nte-mods\combat-clock.nte` 与 `plugins\nte-mods\enemy-telemetry.nte`。
默认脚本不再通过仅有一行差异的高层
`observe/pump` 调用隐藏流程。修改重试间隔、变化条件或事件内容只需编辑对应脚本；
对象路径及版本 Offset 由 `game.session` capability 统一维护。每个程序必须用
`NTE_REQUIRES()` 准确声明实际使用的能力；多声明、
漏声明和未知能力都会使该程序保持未激活。

### 热更新与故障隔离

运行时每 250 ms 检查 `nte-mods.enabled` 和已启用的 `.nte` 源码。发生变化时先编译完整
候选集合，全部成功后再原子替换当前程序；缺失文件、配置错误或编译错误会保留上一版程序，
并写入运行时控制台。VM 的每条宿主操作仍执行既有边界校验，单个 Mod 触发 Windows 运行时
异常时会被隔离并暂停，其他 Mod 和游戏主循环继续工作；下一次成功热更新会解除暂停。
声明 `NTE_REQUIRES("log")` 后，可用 `nte::log::info("message")` 在 Mod 工坊底部控制台
打印脚本输出。

### 语言能力

- `nullptr`、`true`、`false`、十进制及 `0x` 十六进制整数；运行值统一为 64 位；
- 最多 12 个函数局部变量；顶层 `std::uint64_t name = VALUE;` 声明最多 16 个 Mod 私有持久状态；
- `+ - * / % & | ^ << >> == != < <= > >= && || !`；一条表达式使用一个二元
  运算，复合计算可拆成多个中间变量；
- 最多八层 `if / else if / else`；
- `for (std::uint64_t index = 0; index < COUNT; ++index)`，`COUNT` 是 `0..64` 的编译期整数；
- `void on_viewport_tick(const nte::viewport_tick_event& event)` 事件函数、`event.viewport` 事件根对象；
- `nte::game::viewport/instance/local_player/player_controller/player_state/player_character`
  是当前 Tick 的只读内置值，统一要求 `NTE_REQUIRES("game.session")`；
- 源码最大 16 KiB，最多 256 条编译后指令；超出预算的程序保持未激活；
- `//` 注释；控制流使用配对花括号，编辑器按四空格格式化。

状态和循环示例：

```cpp
#include <nte/mod.hpp>

NTE_SCRIPT(5);
NTE_MOD("character-telemetry");
NTE_REQUIRES("viewport.tick");
NTE_REQUIRES("game.session");
NTE_REQUIRES("sdk.read");
NTE_REQUIRES("ipc");

std::uint64_t last_hp = 0;

void on_viewport_tick(const nte::viewport_tick_event& event)
{
    const auto character = nte::game::player_character;
    const auto hp = nte::sdk::character_hp_milli(character);
    if (hp != last_hp)
    {
        const auto max_hp = nte::sdk::character_hp_max_milli(character, false);
        nte::ipc::emit("pre.character.health", hp, max_hp);
        std::uint64_t health_per_mille = 0;
        if (max_hp > 0)
        {
            const auto scaled_hp = hp * 1000;
            health_per_mille = scaled_hp / max_hp;
        }
        nte::ipc::emit("post.character.health", hp, max_hp, health_per_mille);
        last_hp = hp;
    }
}
```

### 通用宿主 API

| API | 返回值／用途 | capability |
| --- | --- | --- |
| `nte::game::viewport` | 当前 `ViewportClient` | `game.session` |
| `nte::game::instance` | 当前 `GameInstance` | `game.session` |
| `nte::game::local_player` | 当前本地玩家 | `game.session` |
| `nte::game::player_controller` | 当前 `HTPlayerController` | `game.session` |
| `nte::game::player_state` | 当前 `HTPlayerState` | `game.session` |
| `nte::game::player_character` | 当前受控 `HTAbilityCharacter` | `game.session` |
| `nte::memory::read_ptr(base, offset)` | 经边界校验的指针 | `memory.read` |
| `nte::memory::read_u8/u16/u32/u64(base, offset)` | 无符号整数 | `memory.read` |
| `nte::memory::read_i32(base, offset)` | 符号扩展整数 | `memory.read` |
| `nte::memory::tarray_first(base, offset)` | 经 `count/capacity` 校验的首项 | `memory.read` |
| `nte::memory::tarray_count(base, offset)` | 经校验的元素数 | `memory.read` |
| `nte::memory::is_readable(pointer, size)` | 可读区间判断 | `memory.read` |
| `nte::memory::write_u8/u16/u32/u64(base, offset, value)` | 向已提交的可写区间写入整数 | `memory.write` |
| `nte::memory::write_i32(base, offset, value)` | 向已提交的可写区间写入有符号整数 | `memory.write` |
| `nte::memory::write_f32_milli(base, offset, value)` | 将 VM 整数除以 1000 后写入 `float` | `memory.write` |
| `nte::unreal::find_function(object, "Owner", "Function")` | 沿对象类继承链解析 UFunction | `unreal.reflection` |
| `nte::unreal::params_clear(size)` | 清空并设置当前调用参数缓冲区 | `unreal.reflection` |
| `nte::unreal::params_write_u8/u16/u32/u64/i32(offset, value)` | 写入调用参数字段 | `unreal.reflection` |
| `nte::unreal::params_write_f32_milli(offset, value)` | 写入 `float` 调用参数字段 | `unreal.reflection` |
| `nte::unreal::params_read_u8/u16/u32/u64/i32(offset)` | 读取调用后的返回／输出字段 | `unreal.reflection` |
| `nte::unreal::params_read_f32_milli(offset)` | 读取并缩放 `float` 返回／输出字段 | `unreal.reflection` |
| `nte::unreal::call(object, function)` | 参数大小与 UFunction 一致时调用 ProcessEvent | `unreal.reflection` |
| `nte::unreal::watch(object, function)` | 为对象订阅指定 UFunction 的 ProcessEvent | `process.event` |
| `nte::unreal::unwatch(object, function)` | 删除当前 Mod 的指定订阅 | `process.event` |
| `nte::event::next()` | 取出当前 Mod 的下一条已订阅事件 | `process.event` |
| `nte::event::object/function/params_size()` | 读取当前事件元数据 | `process.event` |
| `nte::event::read_u8/u16/u32/u64/i32(offset)` | 读取当前事件参数字段 | `process.event` |
| `nte::event::read_f32_milli(offset)` | 读取并缩放当前事件的 `float` 字段 | `process.event` |
| `nte::time::now_ms()` | 进程单调毫秒计时 | 无额外能力 |
| `nte::ipc::bind(player_state, player_controller)` | 合并装备 IPC 上下文，空参数用 `nullptr` | `ipc` |
| `nte::ipc::emit("event.name", value...)` | 发布最多三个 64 位值的自定义事件 | `ipc` |
| `nte::log::info("message")` | 向 Mod 工坊运行时控制台打印输出 | `log` |
| `nte::equipment::cache_missing()` | 任意装备 RPC 缓存是否尚未建立 | `equipment` |
| `nte::equipment::cache_ready(player_state)` | 缓存是否属于当前 `PlayerState` | `equipment` |
| `nte::equipment::prepare(player_state)` | 尝试为当前对象准备一次装备 RPC 缓存 | `equipment` |
| `nte::combat_clock::pause_mask(controller)` | 当前时停类型掩码 | `combat-clock` |
| `nte::combat_clock::state_flags(controller)` | 当前时停状态标志 | `combat-clock` |
| `nte::combat_clock::forward(pause_mask, state_flags)` | 将脚本判定的单次变化写入查询历史 | `combat-clock` |

`NTE_ROUTE_IPC(operation, "kernel.service");` 是顶层声明，不是 Tick 语句。它把固定 IPC
操作号连接到一个经过边界校验的内核服务；删除该声明即可撤下服务，不需要修改或重新
编译 DLL。当前内置服务表直接位于两份默认 `.nte` 中，代码内的 Mod 清单也可编辑这些路由。

Offset 上限为 `0x4000`；指针与 `TArray` Offset 还必须按指针宽度对齐。读取失败返回
零，写入只接受单个已提交且具备写权限的内存区间。`combat_clock.forward` 只接受宿主读取能产生的时停位和状态
标志；装备 RPC 仍只开放既有十种白名单操作。

除 `memory.read_ptr/u8/u16/u32/u64/i32` 外，运行时还提供：

- `nte::memory::read_f32_milli(base, offset)`：读取 `float` 并乘以 1000，仍以整数进入 VM；
- `nte::memory::read_fname_hash(base, offset)`：读取通用 FName 字段，解码后返回小写 ASCII
  FNV-1a 64 位哈希；
- `nte::cache::get(key)`：读取当前 Mod 的缓存值；
- `nte::cache::remember(key, value)`：每个键只接受首次非零值，后续返回首次值。每个 Mod
  最多 32 个条目，脚本重新加载时清空。

这些原语不包含敌人、角色或具体功能语义。`nte::memory::*` 需要
与调用方向一致的 `memory.read` 或 `memory.write` capability；`nte::cache::*` 是 VM 自身
的定长状态，不增加 capability。

### 通用 Unreal 调用与事件

反射调用使用每次 Tick 独立的 256 字节参数缓冲区。脚本先按本地 SDK 或资源元数据写明
Owner 类名、函数名、`ParmsSize` 和字段 Offset；`unreal.call` 会核对实际
UFunction `ParmsSize`，不一致时返回 `False`。调用完成后，同一缓冲区包含返回值与
输出参数，脚本通过 `unreal.params_read_*` 读取。

`nte::unreal::watch` 只为脚本明确给出的对象安装共享 ProcessEvent shadow-vtable Hook，并只
复制指定 UFunction 调用完成后的参数。每个 Mod 拥有独立的 32 条定长队列，满时丢弃
最旧记录；全局最多 16 个被观察对象和 32 条订阅。`nte::event::next()` 取出一条记录后，
`nte::event::object()`、`nte::event::function()`、`nte::event::params_size()` 与 `event.read_*`
访问该记录。动态对象退出观察范围时可调用 `nte::unreal::unwatch` 回收其订阅；脚本重新
加载、禁用或运行时关闭时，订阅队列和相关 Hook 一并清除。

### SDK 读取 API

以下白名单来自仓库本地 China/Global SDK 中布局一致的
`HTPlayerController` / `HTAbilityCharacter` UFunction。UFunction 在运行时按类名和
函数名解析，没有编译期生成 SDK 依赖：

- `nte::sdk::player_character(controller)` → `GetPlayerCharacter`；
- `nte::sdk::player_state(controller)` → `GetHTPlayerState`；
- `nte::sdk::game_paused(controller)` → `IsGamePaused`；
- `nte::sdk::attack_target(character)` → `GetAttackTarget`；
- `nte::sdk::current_weapon(character)` → `GetCurrentWeapon`；
- `nte::sdk::character_level(character)` → `GetCharacterLevel`；
- `nte::sdk::character_hp_milli(character)` → `GetHP`，结果乘以 1000；
- `nte::sdk::character_hp_max_milli(character, fixed)` → `GetHPMax`，结果乘以 1000；
- `nte::sdk::character_is_alive(character)` → `CharacterIsAlive`；
- `nte::sdk::character_is_dead(character)` → `GetIsDead`；
- `nte::sdk::character_is_controlled(character)` → `GetIsControlledCharacter`；
- `nte::sdk::character_slomo_milli(character)` → `GetSlomoValue`，结果乘以 1000。

这些接口统一要求 `NTE_REQUIRES("sdk.read")`。函数只在脚本实际执行对应调用时解析。

### 加载和 IPC

三个脚本相互独立：

```text
nte-mods\equipment.nte
nte-mods\combat-clock.nte
nte-mods\enemy-telemetry.nte
```

`nte-mods.enabled` 决定实际读取的脚本。只加载装备功能：

```text
nte_mod_set 1
load equipment
```

从集合中删掉该行即可停用敌人遥测；重新启用时加入：

```text
load enemy-telemetry
```

它只发布通用 `ipc.query_mod_events` 路由。PlayerState、攻击目标、属性组件、HP 和
配置名字段均由 `.nte` 代码通过通用只读原语获取，不调用目标查询 UFunction，也不走
敌人专用 DLL 服务。脚本为每个目标键通过 `cache.remember` 固定首次配置哈希，后续
采样只读取缓存，并以 250 ms 心跳重复发送缓存后的 identity，便于抓包中途开始时同步。
`enemy.identity` 的三个值依次为目标实例、配置名稳定哈希和保留值；
`enemy.vitals` 为目标实例、当前 HP×1000、最大 HP×1000。
桌面端只在配置哈希命中 `res/data/enemies/enemies.json`，且目标最大 HP 与
当前/前后 HP 连续性唯一吻合时，才把本地化名称和怪物头像投影到对应伤害记录；
相同 HP 流无法区分时保持未识别。

只保留第一行表示不加载任何 Mod。运行时监听器会移除 Viewport Tick Hook 并关闭
IPC 管道；后续再次启用 Mod 时会重新解析脚本并安装共享 Hook。没有 `equipment`、
`combat-clock`、`game.session`、`sdk.read`、`memory.write`、`unreal.reflection`、
`process.event` 或 `ipc` capability 时，对应会话对象、写入、反射、事件 Hook、
缓存和 IPC 分支保持未激活。

公开协议位于 `include\nte_mods_ipc.h`。IPC v7 保留装备操作和权威时停历史，
并提供统一的 `NTE_MODS_IPC_QUERY_MOD_EVENTS`。每条 `NteModEvent` 包含序号、FILETIME
时间戳、Mod ID、事件名和最多三个值；客户端按序号去重即可。标准库 Python
查询示例位于 `plugins\examples\query_mod_events.py`，可直接加载的
`character-telemetry.nte` 与 `reflection-events.nte` 自定义 Mod 示例也位于同一目录。

NTE DPS TOOL 在实时抓包期间持续读取这条事件流，并把它送入共享
`EngineEvent -> CoreSignal` 管线。事件名以 `pre.` 开头时进入 `Preprocess` 阶段，
以 `post.` 开头时进入 `Postprocess` 阶段，其余事件进入普通 `Event` 阶段；前缀在
进入程序后会被去除。例如 `nte::ipc::emit("pre.hit", id, value)` 会得到名称为 `hit`
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
`nte-mods.enabled` 和受限 C++ 程序。监听器会在游戏运行期间检测保存、启用和
禁用更改；最后一个 Mod 被禁用后共享 Hook 与 IPC 都会退出。
刷新托管安装时，发行包原始的 `.nte` v1/v2/v3/v4 默认程序会迁移到当前 NTE C++ v5
版本；用户编辑过的脚本保持原内容。

关闭该选项会删除带有本工具二进制签名的 DLL。程序不会覆盖其他
`dwmapi.dll`，也不会删除被外部替换的文件；这两种情况都会提示用户手动
处理。旧版曾写入游戏目录的启用集合与脚本会先迁移到软件工作区，再从游戏目录清理。
游戏目录最终只保留启用状态下的 `dwmapi.dll`。游戏目录改动可能触发完整性或反作弊检查，启用前应阅读并接受
GUI 中的完整风险声明。

公开的固定 IPC 布局位于 `include\nte_mods_ipc.h`。底层内存边界校验、Viewport Hook、
稳定会话对象、IPC 传输和白名单宿主原语保留在 DLL 内部；服务发布、功能分支、持久
状态和执行顺序位于外部 `.nte` 程序。
