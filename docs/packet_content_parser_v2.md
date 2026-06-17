# Packet Content Parser V2

## 当前实现目标

本阶段只建立“对象/目标解析基础层”。目标是为后续把每次伤害稳定关联到具体敌人打基础，而不是完整实现 Unreal Engine Generic Replication、PackageMap、NetGUID 或 Iris NetRefHandle 解码。

## 已实现内容

- `ue_bitstream`：LSB-first bit reader、任意 bit offset 读取、shifted byte decode、基础数值读取和 FString-like/path candidate 提取。
- `object_state`：`ObjectStateStore` 保存路径候选、Attribute GUID HP 时间线、对象证据、置信度和过期清理。
- `resource_index`：预留 `res/data/targets/*.json` 目标资源索引；目录不存在时静默降级，并从路径 basename 生成 fallback name。
- `target_resolver`：按可解释 reason 生成 `TargetCandidate`，并只在 probable/confirmed 时填充 `target_name`。
- `PacketDecoder` 集成：S2C 观察 Boss HP、CurrentHP 和路径候选；C2S 观察伤害、角色声明、GameplayEffect 和路径候选；发送 `Hit` 前附加 `target_id`、`target_name`、`target_context`。
- AttributeGuid 会在短时间窗口内链接唯一近邻目标路径，从而让 HP 属性实例获得 `object_path` / fallback name。
- 原始 Hit 仍立即发送；后到的 S2C HP/path 证据会通过 `HitTargetUpdate` 回填最近 Hit 的 target 字段。
- 覆纹推断 Hit 也会经过同一套 TargetResolver。

## 未实现内容

- 未实现完整 ActorChannel/Bunch 字段级解析。
- 未实现 PackageMap/SerializeObject 的 NetGUID 建图。
- 未实现 Iris NetRefHandle 和 NetSerializer 解码。
- 未解析 PacketHandler 加密、认证、完整性校验，也不会尝试绕过。
- 未建立 DataTable/StringTable/locres 的真实敌人中文名索引。

## 已知限制

- 当前对象路径、类路径和目标候选均为 heuristic/candidate。
- Attribute GUID 目前只确认可作为 HP 属性实例候选，不等价于 Actor 或 Monster 实例。
- HitTargetUpdate 只回填短时间窗口内的最近 Hit；跨长窗口或乱序严重的包仍可能无法更新。
- 多目标、多 Boss、召唤物、分身场景仍可能只能输出 possible/unknown。

## 目标匹配评分规则

- AttributeGuid 已链接近邻 Monster/Boss/Enemy/Character/HTCharacter 等路径：`+30`。
- AttributeGuid 已链接 `/Game/` 且包含 Monster/Boss/NPC/Enemy 的路径：额外 `+40`。
- 只有 PathOnly、没有 HP/handle 证据的路径候选：最高 possible，不填 `target_name`。
- Boss HP update 的 HP delta 与伤害在 1 秒窗口内匹配：`+50`。
- `target_hp_before`/`target_hp_after` 与同一 HP GUID 时间线匹配：`+50`。
- 时间差越小额外加分，最高 `+20`。
- 当前窗口只有一个高置信 Boss/Monster 对象：`+25`。
- 只有 `target_max_hp` 大小判断：最多 `+5`。
- 多候选无直接 HP 证据时：`-20`。

置信度：

- `score >= 90`：confirmed
- `60 <= score < 90`：probable
- `35 <= score < 60`：possible
- `score < 35`：unknown

## 为什么低置信不填 target_name

`target_name` 会被 UI 和导出 JSON 视为对用户有直接解释力的字段。possible/unknown 只代表候选证据存在，不能证明本次攻击命中了该对象。低置信时只写入 `target_context`，保留 score、reason、path、GUID 等证据，避免把启发式结果伪装成确定敌人名称。

## 参考资料

本阶段联网查阅过以下资料，均只作为结构设计参考；NTE 实际协议格式仍以本仓库抓包证据为准。

- Epic UE API: UActorChannel - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/UActorChannel
- Epic UE API: UChannel - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/Engine/Engine/UChannel
- Epic UE API: FNetBitReader - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/CoreUObject/FNetBitReader
- Epic UE Iris overview - https://dev.epicgames.com/documentation/unreal-engine/iris-replication-system-in-unreal-engine
- Epic UE Iris filtering / FNetRefHandle usage - https://dev.epicgames.com/documentation/unreal-engine/iris-filtering-in-unreal-engine
- Epic UE PacketHandler - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/PacketHandler/PacketHandler
- Epic UE HandlerComponent - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/PacketHandler/HandlerComponent
- Epic UE EncryptionComponent - https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/PacketHandler/FEncryptionComponent
- Epic UE Oodle Network - https://dev.epicgames.com/documentation/unreal-engine/oodle-network
- GitHub boxcars - https://github.com/nickbabcock/boxcars
- GitHub rrrocket - https://github.com/nickbabcock/rrrocket
- GitHub subtr-actor - https://github.com/rlrml/subtr-actor
- GitHub CUE4Parse - https://github.com/FabianFG/CUE4Parse
- GitHub UEExtractor - https://github.com/SolicenTEAM/UEExtractor

## 后续阶段计划

1. Generic ActorChannel / PackageMap / NetGUID 解析。
2. Iris NetRefHandle 解析。
3. DataTable/StringTable/locres 敌人名称索引。
4. 多目标最小费用匹配。
5. Networking Insights / NetTrace 对照验证。
