# NTE 战报：SDK 数据采集函数与报表口径（国服）

本文按战报数据种类整理当前仓库 `SDK/CN/CppSDK` 中与 **战斗事件、参与者、技能归因和统计汇总** 直接相关的函数、字段与结构，说明用途、调用方式以及可以得到的数据。常规探索信息、背包、任务、聊天和表现层环境数据不纳入采集范围。

> SDK 基线：`SDK/CN/CppSDK/SDK.hpp` 标注为 Dumper-7 生成、Unreal Engine `5.6.1`、`HT`。文中的类布局、函数名、参数区大小和偏移只对应当前 SDK 快照；游戏更新后必须重新核对。

## 1. 先区分三种采集方式

| 标记 | 方式 | 用途 | 是否主动调用 |
|---|---|---|---|
| **GET** | 调用 SDK 生成的 getter | 开战/结束属性快照、HP 校验 | 是，只在游戏线程调用 |
| **FIELD** | 读取 SDK 暴露的字段/容器 | getter 不完整时补充结构化状态 | 否，只读快照 |
| **EVENT** | 在 `UObject::ProcessEvent` 入口观察函数 | 记录伤害、治疗、死亡和同步事件 | 否；只观察原始游戏调用 |

对于日志系统，优先级建议为：

```text
原生 EVENT 参数 > 只读 GET > 已确认布局的 FIELD > 仅按名字猜测的字段
```

不要为了“采集事件”主动调用 `Client*`、`Server*`、`NetMulticast*`、`OnRep_*`、`ShowDamageFloaties` 等函数；这些函数应由游戏自然触发，日志模块只在 `ProcessEvent` 中复制参数。

## 2. 通用调用前提

### 2.1 头文件

开发初期可以直接包含完整 SDK：

```cpp
#include "SDK.hpp"
```

稳定后建议只包含实际使用的包，减少编译时间：

```cpp
#include "SDK/CoreUObject_classes.hpp"
#include "SDK/Engine_classes.hpp"
#include "SDK/GameplayAbilities_classes.hpp"
#include "SDK/HTGame_classes.hpp"
#include "SDK/HTGame_parameters.hpp"
#include "SDK/TargetingSystem_classes.hpp"
```

### 2.2 获取本地运行时对象

```cpp
struct NteLogContext
{
    SDK::UWorld* world{};
    SDK::APlayerController* controller{};
    SDK::AHTCharacter* character{};
    SDK::AHTPlayerState* playerState{};
    SDK::UHTAbilitySystemComponent* asc{};
};

NteLogContext CaptureContext()
{
    NteLogContext out{};
    out.world = SDK::UWorld::GetWorld();
    if (out.world == nullptr)
        return out;

    out.controller = SDK::UGameplayStatics::GetPlayerController(out.world, 0);
    SDK::APawn* pawn = SDK::UGameplayStatics::GetPlayerPawn(out.world, 0);
    if (pawn == nullptr || !pawn->IsA(SDK::AHTCharacter::StaticClass()))
        return out;

    out.character = static_cast<SDK::AHTCharacter*>(pawn);
    out.playerState = out.character->GetHTPlayerState();
    out.asc = SDK::UHTAbilitySystemBlueprintLibrary::GetHTAbilitySystemComponent(out.character);
    return out;
}
```

所有 UObject 查找、SDK wrapper 调用、`ProcessEvent` 和容器快照都应在游戏线程执行。日志线程只接收已经复制成自有 POD/string/vector 的记录并负责落盘，不能在后台线程长期保存并解引用 UObject、`FString`、`TArray` 或 `TMap` 的内部地址。

### 2.3 对象名、类型和全局对象表

| 函数/入口 | 类型 | 作用 | 可得到的数据 |
|---|---|---|---|
| `UObject::GetName()` | GET | 获取对象短名 | 例如 `BP_PlayerCharacter_C_214748...` |
| `UObject::GetFullName()` | GET | 获取带类型和包路径的完整名 | 适合作为日志中的 `objectClass/objectName` |
| `UObject::IsA(UClass*)` | GET | 在转换前验证真实类型 | 防止把普通 Pawn 当成 `AHTCharacter` |
| `UObject::FindObject<T>(fullName)` | GET | 按完整名查已加载对象 | UFunction、UClass、单例/实例指针 |
| `UObject::FindClass(fullName)` | GET | 查已加载 UClass | 运行时兼容性探测 |
| `UObject::GObjects->Num()` / `GetByIndex()` | FIELD | 有界扫描已登记 UObject | 未知战斗对象、UI 实例、子系统实例 |

`GObjects` 扫描必须分页或设置每帧上限。不要每帧遍历完整对象表。

## 3. 战斗参与者身份

### 3.1 推荐函数

| 函数 | 类型 | 作用/输入 | 返回或可记录数据 |
|---|---|---|---|
| `AHTCharacter::GetHTPlayerState()` | GET | 本地 `AHTCharacter` 调用 | `AHTPlayerState*`，后续玩家数据入口 |
| `AHTCharacter::GetHTPlayerController()` | GET | 本地角色调用 | `AHTPlayerController*` |
| `APlayerState::GetPlayerName()` | GET | `AHTPlayerState` 继承调用 | 当前显示名 `FString` |
| `AHottaPlayerState::GetRoleId()` | GET | 无参数 | NTE RoleId `int64` |
| `AHTPlayerState::GetRoleLevel()` | GET | 无参数 | 角色等级 `int32` |
| `AHTPlayerState::GetMainCharacter()` | GET | 无参数 | 当前主角色 `AHTPlayerCharacter*` |
| `AHTPlayerState::GetEquippedPlayers()` | GET | 无参数 | 当前编队角色指针数组 |
| `AHTPlayerState::GetPlayerCharacterByCharacterID(id)` | GET | 角色配置 ID | 对应已生成角色实例 |

### 3.2 直接调用示例

```cpp
if (ctx.playerState != nullptr)
{
    const std::string playerName = ctx.playerState->GetPlayerName().ToString();
    const int64_t roleId = ctx.playerState->GetRoleId();
    const int32_t roleLevel = ctx.playerState->GetRoleLevel();
    // 立即复制到日志记录；不要把临时 FString 的内部指针交给 writer 线程。
}
```

战报只保留 `RoleId`、显示名、等级、当前出战角色和编队角色映射。登录时间、累计在线时间、性别、经验等账号属性不进入战报。

## 4. 命中位置与目标上下文

| 函数 | 类型 | 调用对象/输入 | 返回数据 |
|---|---|---|---|
| `AActor::K2_GetActorLocation()` | GET | 任意 Actor | 世界坐标 `FVector`（UE5 下为 3 个 double） |
| `AActor::GetDistanceTo(other)` | GET | 另一个 Actor | 三维距离 `float` |
| `AActor::GetHorizontalDistanceTo(other)` | GET | 另一个 Actor | 水平距离 `float` |

```cpp
if (ctx.character != nullptr)
{
    const SDK::FVector p = ctx.character->K2_GetActorLocation();
}
```

战报不持续采集移动轨迹，只在伤害、死亡、阶段变化等事件发生时保存命中点、攻击者/目标位置和距离。

## 5. 战斗伤害事件

### 5.1 首选：伤害飘字事件

```cpp
void UHTUI_DamageFloatiesForm::ShowDamageFloaties(
    const SDK::FHTDamageTextInfo& InDamageInfo);
```

完整 UFunction 名：

```text
Function HTGame.HTUI_DamageFloatiesForm.ShowDamageFloaties
```

采集方式：**EVENT**。在 `ProcessEvent` 入口读取 `Params::HTUI_DamageFloatiesForm_ShowDamageFloaties`，不要主动调用。

`FHTDamageTextInfo` 可直接得到：

| 字段 | 数据 |
|---|---|
| `HitLocation` | 命中世界坐标 |
| `DamageDisplayTextType` | 飘字显示类别 |
| `iDamageValue` | 本次显示伤害/治疗整数值 |
| `DamageTypeEX` | NTE 执行伤害类型 |
| `bCrit` | 是否暴击 |
| `bHitHead` | 是否命中头部 |
| `bIsWeakUnbalDamage` | 是否弱点/失衡相关伤害 |
| `Attacker` | 攻击者弱引用 |
| `Victim` | 受击者弱引用 |
| `CombatStatistics.InjurySourceName` | 伤害来源名，通常是技能、Effect 或来源配置名 |
| `CombatStatistics.BasicInjury` | 基础伤害 |
| `CombatStatistics.FinalInjury` | 最终伤害 |
| `ReactionType` | 反应结果类型 |
| `ReactionDisplayType` | 反应显示类型 |

当前 SDK 参数区为 `0x48` 字节；核心布局为 `iDamageValue @ 0x1C`、`Attacker @ 0x24`、`Victim @ 0x2C`、`CombatStatistics @ 0x34`。这些偏移必须在每次 SDK 更新后重新验证。

示意 Hook：

```cpp
void OnProcessEvent(SDK::UObject* object, SDK::UFunction* function, void* params)
{
    static SDK::UFunction* showDamage = SDK::UObject::FindObject<SDK::UFunction>(
        "Function HTGame.HTUI_DamageFloatiesForm.ShowDamageFloaties");

    if (function == showDamage && params != nullptr)
    {
        const auto* p = static_cast<
            const SDK::Params::HTUI_DamageFloatiesForm_ShowDamageFloaties*>(params);
        const SDK::FHTDamageTextInfo& d = p->InDamageInfo;
        // 在这里复制整数、枚举、位置和已解析的 UObject 名称到自有记录。
    }
}
```

### 5.2 飘字组件轮询兜底

| 函数/字段 | 类型 | 可得到的数据 |
|---|---|---|
| `UHTUI_DamageFloatiesForm::GetCurrentDamageFloatiesWidgetArray()` | GET | 当前活动飘字 Widget 数组 |
| `UHTUI_DamageFloatiesWidget::DamageInfo` | FIELD | 每个 Widget 的 `FHTDamageTextInfo` |
| `UHTUI_DamageFloatiesForm::ActiveDamageFloatiesWidgetArray` | FIELD | 活动飘字实例 |

该路径适合补足绕过 `ProcessEvent` 的 native 调用，但 Widget 会复用，必须按 Widget 地址和内容指纹去重；不要把同一飘字重复记成多次伤害。

### 5.3 服务器战斗同步事件

| 函数 | 类型 | 可得到的数据 | 使用说明 |
|---|---|---|---|
| `AHTPlayerController::ClientSetReplicatedTargetData(const FClientReplicatedTargetDataContainer&)` | EVENT | 来源、多个目标的剩余 HP、死亡状态、护盾伤害、伤害包装数据、治疗数据、时间戳、反应数据 | 战斗批次的主要服务端同步入口 |
| `AHTPlayerState::ClientDamageBoss(AHTAICharacter*, float DamagedHP)` | EVENT | Boss 指针和 `DamagedHP` | 当前仓库分析链将其作为 Boss 剩余 HP 候选使用；仍应与相邻 HP 变化交叉验证 |
| `AActor::ReceiveAnyDamage(...)` | EVENT | Damage、DamageType、Instigator、DamageCauser | UE 通用入口；GAS/native 路径可能绕过它，不应作为唯一伤害源 |
| `UHTAbilitySystemComponent::NetMulticast_OnSendHandleDamageInfos(...)` | EVENT | 网络伤害队列 | 结构复杂，适合协议研究，不宜直接替代已验证飘字事件 |
| `UHTAbilitySystemComponent::NetMulticast_OnSendPlayGamePlayEffect(...)` | EVENT | 网络 GameplayEffect 队列 | 可用于 Effect 应用时间线 |

`FClientReplicatedTargetDataContainer` 的关键内容：

```text
MsgIndex
NetSourceActor                         FCharacterForNet
ClientFightDataArray[]:
  NetTarget                           FCharacterForNet
  fCurHealth                          float
  DeadState                           int32
  fShieldDamage                       float
  LockTarget                          int32
  DamageTextInfoArray                 FParameterWrapperArray
ClientExtraDamageInfos[]:
  NetTarget, DeadState, fCurHealth
ClientRecoverDataArray[]:
  NetTarget, fCurHealth
TimesStamp                            double
NetReactionData                       FReactionData
```

`FCharacterForNet` 在当前生成头中是 `0x28` 字节的 opaque struct，Dumper-7 没有恢复其成员。日志可以保存其 40 字节值作为会话内关联键，但不能把未知字节直接命名为永久角色 ID。只有经过同版本运行时验证后，才应把它映射到 Actor/RoleId。

### 5.4 战斗汇总辅助值

| 函数 | 类型 | 返回数据 |
|---|---|---|
| `UHTAbilitySystemComponent::GetHitActorsTotalDamage()` | GET | 当前组件维护的命中 Actor 总伤害 `float` |
| `AHTPlayerState::GetKillStreakCount()` | GET | 当前连杀数 |
| `AHTPlayerState::GetKillStreakBestTierIndex()` | GET | 连杀最高档位索引 |
| `AHTPlayerState::GetKillStreakRemainingTime()` | GET | 连杀剩余时间 |
| `AHTPlayerState::bIsInFightState` | FIELD | replicated 战斗状态兜底 |

`GetHitActorsTotalDamage()` 的生命周期边界仅靠 SDK 名字无法确定，因此适合诊断/校验，不适合作为逐击日志的唯一事实源。

## 6. 血量、攻击、防御、暴击与失衡属性

本地 `UHTAbilitySystemComponent` 继承 `UHTAttributeComponent`，可直接调用以下只读 getter。

### 6.1 生存与资源

| 函数 | 返回 |
|---|---|
| `GetHPCurrent()` | 当前 HP |
| `GetHPMax(bool bIsFixHPMax)` | 当前最大 HP；参数决定是否使用修正值 |
| `GetHPMaxBase()` | 基础最大 HP |
| `GetShieldHealth()` | 当前护盾值 |
| `GetOwnerShieldComponent()` | `UHTShieldComponent*` |
| `UHTShieldComponent::GetShieldHealth()` | 护盾组件的当前总护盾 |
| `GetChargeCurrent()` / `GetChargeMax()` | 当前/最大充能 |
| `GetUnbalCurrent()` / `GetUnbalMax()` | 当前/最大失衡值 |
| `GetUnbalSpeed()` | 失衡恢复/变化速度相关值 |
| `GetUnbalAccrueEfficiency()` | 失衡积累效率 |
| `GetUnbalAntiAccrueEfficiency()` | 抗失衡积累效率 |
| `GetUnbalIntensity()` | 失衡强度 |
| `GetCharacterLevel()` | 属性组件对应角色等级 |

### 6.2 攻防与伤害系数

| 函数 | 返回 |
|---|---|
| `GetAtk()` / `GetAtkBase()` | 当前/基础攻击 |
| `GetMag()` / `GetMagBase()` | 当前/基础 Mag 属性 |
| `GetDef()` / `GetDefBase()` | 当前/基础防御 |
| `GetCrit()` | 暴击属性 |
| `GetCritDamage()` | 暴击伤害属性 |
| `GetDamageBaseAdd()` | 基础伤害加成 |
| `GetDamageUpGeneral()` | 通用增伤 |
| `GetDamageUpNormal/Cosmos/Nature/Incantation/Chaos/Psyche/Lakshana/Psychically()` | 各类型增伤 |
| `GetDamageResistNormal/Cosmos/Nature/Incantation/Chaos/Psyche/Lakshana/Psychically()` | 各类型抗性 |
| `GetDamageResist*Base()` | 对应基础抗性 |
| `GetReactionGeneralDamageUp()` | 通用反应增伤 |
| `GetReactionGuangLing/LingZhou/ZhouAn/AnHun/HunXiangDamageUp()` | 各反应增伤 |
| `GetFinalDamageUpFinalCoefficient(target)` | 针对目标计算的最终增伤系数 |
| `GetFinalReactionDamageUpCoefficient(type)` | 指定反应的最终增伤系数 |
| `GetRealDamage()` | 组件当前真实伤害辅助值 |
| `CurrentDamageIsCrit()` | 当前伤害上下文是否暴击 |

```cpp
if (ctx.asc != nullptr)
{
    LogGauge("hp.current", ctx.asc->GetHPCurrent());
    LogGauge("hp.max", ctx.asc->GetHPMax(true));
    LogGauge("attack", ctx.asc->GetAtk());
    LogGauge("crit", ctx.asc->GetCrit());
    LogGauge("crit_damage", ctx.asc->GetCritDamage());
    LogGauge("shield", ctx.asc->GetShieldHealth());
}
```

`UGameplayAttributeSet` 还暴露约 150 个 `FGameplayAttributeData` 字段。每个字段包含 `BaseValue` 和 `CurrentValue`。重要字段包括 `HPMaxBase`、`AtkBase/Up/Add`、`CritBase/Add`、`CritDamageBase/Add`、`DefBase/Up/Add`、`DamageUp*`、`DamageResist*`、`DamagePenetrate*`、`DamageImmu*`、`CooldownReduction`、`HealUp`、`HealBeUp`。批量属性快照可以读取 `ctx.asc->HTAttribute`，但对外日志字段应使用稳定名字并在版本更新时验证偏移。

属性变化事件可观察 `UGameplayAttributeSet::OnRep_*` 与 `UHTAttributeComponent::OnRep_HPCurrent/OnRep_Atk/OnRep_MaxHP/...`。`OnRep_*` 表示网络复制通知，不覆盖本地预测或非复制变化；需要完整时间线时应同时保留周期快照。

## 7. 技能、Ability 与冷却

| 函数/容器 | 类型 | 输入 | 可得到的数据 |
|---|---|---|---|
| `UAbilitySystemComponent::GetAllAbilities(outHandles)` | GET | 输出数组 | 所有 Ability Spec Handle |
| `UAbilitySystemComponent::ActivatableAbilities.Items` | FIELD | 无 | Ability 对象、Level、InputID、ActiveCount、InputPressed、Handle、动态 Tag |
| `UHTAbilitySystemComponent::GetActiveAbilitiesWithTags(tags, out)` | GET | GameplayTag 集合 | 当前活动 Ability 对象数组 |
| `FindAbilitySpecHandleForClass(class, source)` | GET | Ability class、可选来源 | 对应 Spec Handle |
| `UHTAbilitySystemBlueprintLibrary::GetGameplayAbilityFromSpec(spec, outIsInstance)` | GET | AbilitySpec | Ability/实例及实例标志 |
| `GetPrimaryAbilityInstanceFromHandle(asc, handle)` | GET | ASC + Handle | NTE Ability 主实例 |
| `IsPrimaryAbilityInstanceActive(asc, handle)` | GET | ASC + Handle | 是否活动 |
| `UAbilitySystemBlueprintLibrary::IsGameplayAbilityActive(ability)` | GET | Ability | 是否活动 |
| `GetCooldownRemainingForTag(tags, outRemaining, outDuration)` | GET | 冷却 Tag | 是否存在冷却、剩余时间、总时长 |
| `GetActiveEffectTimeRemainingAndDuration(effectClass, ...)` | GET | GameplayEffect class | Effect 剩余时间、总时长 |
| `GetCurSkillInstanceByMontage(montage)` | GET | Montage | 正在使用该 Montage 的技能实例 |

遍历 `ActivatableAbilities.Items` 的最小示例：

```cpp
if (ctx.asc != nullptr)
{
    for (const SDK::FGameplayAbilitySpec& spec : ctx.asc->ActivatableAbilities.Items)
    {
        if (spec.Ability == nullptr)
            continue;
        const std::string ability = spec.Ability->GetFullName();
        const int32_t level = spec.Level;
        const int32_t inputId = spec.InputID;
        const uint8_t activeCount = spec.ActiveCount;
        const bool inputPressed = spec.InputPressed;
    }
}
```

适合记录的技能事件还包括 `AHTPlayerState` 上的 `OnOwningCharacterTriggerAbility`、`OnOwningCharacterRealTriggerAbility`、`OnOwningCharacterAbilityEnd` 和 `OnOwningCharacterAbilityJumpSection` delegate。生成 SDK 只给出了 delegate 内存布局；若现有工程没有稳定 delegate 绑定工具，优先观察对应 UFunction/`ProcessEvent`，不要直接修改 `InvocationList`。

## 8. Effect、Buff、Debuff、GameplayTag 与 Cue

| 函数/容器 | 类型 | 可得到的数据 |
|---|---|---|
| `UAbilitySystemComponent::GetActiveEffects(query)` | GET | 匹配查询的 ActiveEffect Handle 数组 |
| `GetActiveEffectsWithAllTags(tags)` | GET | 同时包含所有 Tag 的 Effect Handle |
| `GetGameplayEffectCount(effectClass, instigatorAsc, ongoing)` | GET | 指定 Effect 的数量 |
| `GetGameplayEffectMagnitude(handle, attribute)` | GET | Effect 对指定属性的 magnitude |
| `UHTAbilitySystemComponent::K2_GetGameplayEffectStackCountWithClass(...)` | GET | Effect stack 数量 |
| `K2_GetGameplayEffectStackMaxWithClass(effectClass)` | GET | 最大 stack |
| `K2_HaveGameplayEffectWithClass(...)` | GET | 是否存在指定 Effect |
| `K2_HaveGameplayEffectWithTag(tag)` | GET | 是否存在带 Tag 的 Effect |
| `UHTAbilitySystemBlueprintLibrary::GetTotalStackCountOfActiveEffectsWithAllTags(asc, tags)` | GET | 匹配 Tags 的总 stack |
| `UAbilitySystemComponent::GetGameplayTagCount(tag)` | GET | Tag 计数 |
| `UAbilitySystemComponent::IsGameplayCueActive(tag)` | GET | Cue 是否活动 |
| `ActiveGameplayEffects.GameplayEffects_Internal` | FIELD | Effect spec、开始时间、是否 inhibited、授予的 Ability Handle |
| `ActiveGameplayCues.GameplayCues` | FIELD | 当前 GameplayCue Tag/参数 |

`FActiveGameplayEffect` 的日志关键字段：

```text
Spec.Def / Spec.Level / Spec.StackCount
Spec.duration / Spec.Period
StartServerWorldTime
StartWorldTime
bIsInhibited
GrantedAbilityHandles[]
```

NTE 还维护 `AHTPlayerState::m_GESnapshotList`，元素 `FGameplayEffectSnapshotItem` 包含发送角色 ID、动态授予 Tags、EffectClass、Level、Duration、Period、StackCount、强度乘/加值与开始时间，适合用于 Buff 快照交叉验证。

## 9. 目标、命中与锁定

| 函数/字段 | 类型 | 输入 | 可得到的数据 |
|---|---|---|---|
| `AHTPlayerState::m_AttackTarget` | FIELD | 无 | 当前攻击目标 `AHTAbilityCharacter*` |
| `AHTPlayerState::GetOnLockedList(out)` | GET | 输出数组 | 当前锁定的 AI 角色列表 |
| `UHTAbilitySystemComponent::m_AttackTarget` | FIELD | 无 | ASC replicated 当前目标 |
| `UHTAbilitySystemBlueprintLibrary::GetTargetsByTargetType(owner, type, outHits)` | GET | Owner + NTE TargetType | `FHitResult[]` |
| `GetTargetsByTargetTypeWithEventDataTarget(owner, eventTarget, type, outHits)` | GET | 多一个事件目标 | `FHitResult[]` |
| `UAbilityTask_PerformTargeting::GetTargetingHandle()` | GET | Targeting task | 请求 Handle |
| `UTargetingSubsystem::GetTargetingResults(handle, outHits)` | GET | 请求 Handle | 命中结果数组 |
| `UTargetingSubsystem::GetTargetingResultsActors(handle, outActors)` | GET | 请求 Handle | 目标 Actor 数组 |
| `UTargetingSubsystem::GetTargetingSourceContext(handle)` | GET | 请求 Handle | 请求来源 Actor/位置上下文 |

`FHitResult` 可以记录目标 Actor、Component、ImpactPoint、ImpactNormal、TraceStart、TraceEnd、BoneName、距离和 blocking-hit 标志。Targeting 结果的生命周期通常短于一帧到数帧，应在相关事件当下复制。

## 10. 战斗区间：开始、结束与分段

### 10.1 可用信号

| 函数/字段 | 类型 | 可得到的数据 | 用法 |
|---|---|---|---|
| `AHTPlayerState::bIsInFightState` | FIELD | 当前是否处于战斗 | 观察 `false -> true` 和 `true -> false` |
| `AHTPlayerState::OnFightStateChanged` | delegate | `bool bInFight` | 有稳定 delegate 绑定方式时作为即时信号 |
| `AHTAbilityCharacter::OnDeadStateChanged(...)` | EVENT | 死亡状态、击杀者、伤害来源 Actor | 目标死亡和玩家死亡 |
| `AHTAbilityCharacter::NetMulticast_CharacterDead(...)` | EVENT | `DeadReason`、是否全队死亡 | 结算辅助信号 |
| `AHTAICharacter::NetOnBossStageBegin()` | EVENT | Boss 阶段开始 | 可选阶段分段 |
| `AHTAICharacter::NetOnBossStageEnd()` | EVENT | Boss 阶段结束 | 可选阶段分段 |

### 10.2 推荐分段规则

```text
开始：bIsInFightState 从 false 变为 true，或空闲状态收到第一条有效战斗事件。
持续：伤害、治疗、HP 同步、技能事件写入同一个 encounterId。
候选结束：bIsInFightState 从 true 变为 false。
确认结束：候选结束后等待短暂宽限期，期间没有新战斗事件。
强制结束：World/Pawn 更换、本地 PlayerState 失效、进程退出。
```

宽限期是聚合策略，不是 SDK 字段。建议从 `3 s` 起步并通过实战样本调整。Boss 阶段变化默认写入同一战斗的 `phase`，不要自动拆成多份战报。

持续时间和 DPS 使用进程单调时钟；UTC 只用于展示和跨进程对齐。

## 11. 治疗、护盾、承伤与死亡

### 11.1 治疗

`FClientReplicatedTargetDataContainer::ClientRecoverDataArray[]` 的元素 `FClientRepRecoverData` 包含：

```text
NetTarget    FCharacterForNet
fCurHealth   float
```

对已经映射的同一目标保存前一次 HP：

```text
effectiveHeal = max(0, currentHp - previousHp)
```

该结构没有直接提供治疗来源和请求治疗量，因此目标和有效治疗可以直接确定，来源技能只能与邻近 Ability/Effect 事件关联并标记为 `correlated`；没有请求治疗量时不能可靠计算过量治疗。

### 11.2 护盾和承伤

| 函数/事件 | 可得到的数据 | 战报指标 |
|---|---|---|
| `UHTAttributeComponent::GetHPCurrent()` | 当前 HP | HP 曲线、有效治疗、承伤校验 |
| `GetHPMax(true)` | 修正后最大 HP | 有效治疗上限 |
| `GetShieldHealth()` | 当前护盾 | 护盾曲线 |
| `UHTShieldComponent::GetShieldHealth()` | 护盾组件总值 | 护盾校验 |
| `FClientRepFightData::fShieldDamage` | 本批次护盾伤害 | 护盾承伤 |
| `AHTAbilityCharacter::OnDamaged(...)` | DamageAmount、HitInfo、Tags、Effect、攻击者、DamageCauser | 本地承伤候选 |

`OnDamaged` 可能和飘字/服务端同步描述同一命中，默认作为校验或本地承伤来源，不能未经去重同时累加。

### 11.3 死亡与击杀

| 事件 | 可得到的数据 |
|---|---|
| `AHTAbilityCharacter::OnDeadStateChanged(bDeadState, InstigatorCharacter, DamageCauser)` | 死亡状态、击杀者、伤害来源 |
| `AHTAbilityCharacter::NetMulticast_CharacterDead(DeadReason, bCharacterAllDead)` | 死亡原因、是否全队死亡 |
| `AHTPlayerState::OnSlayToActor` | 被击杀角色 |
| `AHTPlayerState::OnSlay` | 被击杀角色名 |

同一目标在同一死亡周期只生成一条 canonical `combat.death`。复活或重新生成后开始新的死亡周期。

## 12. 规范化战斗事件

采集层只输出战报需要的类别：

| `category` | 推荐来源 | 最小字段 |
|---|---|---|
| `combat.encounter` | 战斗状态与分段器 | encounterId、start/end、reason、primaryTarget |
| `combat.participant` | PlayerState、角色、TeamInfo | combatantKey、roleId、name、kind、level |
| `combat.damage` | `ShowDamageFloaties` | source、target、value、crit、head、type、skill、reaction |
| `combat.sync` | `ClientSetReplicatedTargetData` | msgIndex、source、target、hp、dead、shieldDamage |
| `combat.heal` | `ClientRecoverDataArray` | target、hpBefore、hpAfter、effectiveHeal、quality |
| `combat.death` | 死亡事件 | target、killer、causer、reason |
| `combat.ability` | Ability delegate/spec | source、ability、phase、handle、level |
| `combat.effect` | ActiveEffect/Tag | source、target、effect、stack、start、duration |
| `combat.attribute` | Attribute getter | combatant、hp、maxHp、shield、atk、crit、def、unbal |
| `combat.target` | Targeting/HitResult | source、target、point、bone、distance、lockState |

每条事件增加统一字段：

```json
{
  "schemaVersion": 1,
  "sequence": 1,
  "encounterId": "session-sequence",
  "category": "combat.damage",
  "utcMicros": 0,
  "monotonicMicros": 0,
  "sourceFunction": "Function HTGame.HTUI_DamageFloatiesForm.ShowDamageFloaties",
  "attributionQuality": "direct"
}
```

参与者键的优先级：

```text
已验证 RoleId           -> role:<id>
已解析稳定游戏 ID      -> actor:<game-id>
只有 UObject           -> session-object:<index>:<serial/full-name>
opaque FCharacterForNet -> net-character:<40-byte-session-hash>
```

## 13. 战报聚合口径

### 13.1 玩家总览

| 指标 | 计算方式 |
|---|---|
| 总伤害 | 当前玩家作为 source 的 canonical `combat.damage.value` 之和 |
| 有效伤害 | `min(damage, targetHpBefore)`；没有可靠 HP 前值时留空 |
| DPS | 总伤害 / 战斗总时长秒数 |
| Active DPS | 总伤害 / 玩家首末有效战斗事件区间 |
| 承受伤害 | 当前玩家作为 target 的去重伤害之和，并用 HP 差分校验 |
| 有效治疗 | `combat.heal.effectiveHeal` 之和 |
| 击杀/死亡 | canonical death event 按 killer/target 计数 |
| 暴击率 | 暴击伤害命中数 / 可暴击伤害命中数 |
| 爆头率 | `headshot=true` 命中数 / 有效命中数 |
| 失衡次数 | 失衡完成事件或已验证状态转换次数 |

“DPS”和“Active DPS”必须同时标明分母口径，不能只输出一个没有定义的 DPS。

### 13.2 技能明细

按 `sourceKey + skillOrEffectId` 聚合：

```text
damage
damageShare = skillDamage / playerTotalDamage
hitCount
critCount
critRate
averageHit
maxHit
firstEventOffsetMs
lastEventOffsetMs
attributionQualityCounts
```

伤害到技能的归因优先级：

```text
direct     : CombatStatistics.InjurySourceName
direct     : DamageGameplayEffect / GameplayEffect 类
correlated : 同一攻击者当前活跃 Ability + 短时间窗
inferred   : Montage/Section、DamageCauser 或命名规则
unknown    : 没有足够证据
```

无法归因的伤害进入 `unknown`，不能为了让技能占比达到 100% 而强行归类。

### 13.3 目标、阶段与 Buff

- 按 `targetKey` 聚合伤害占比、首次/末次命中、死亡时间。
- Boss 阶段信号存在时统计阶段伤害/DPS；没有阶段信号时整场使用 `phase=0`。
- Buff 覆盖率按 `effectKey + sourceKey + targetKey` 合并有效区间后计算。
- Effect 快照的轮询次数和重复记录数不能直接当作覆盖时长。

## 14. 去重与数据质量

1. **canonical 伤害源**：优先 `ShowDamageFloaties`；其他伤害入口默认只校验。
2. **短窗指纹**：`source + target + value + type + skill + position + timeBucket`。
3. **批次去重**：`ClientSetReplicatedTargetData` 优先使用 `MsgIndex`，同一 MsgIndex 不重复处理。
4. **HP 有序性**：按单调时间和批次序列更新目标 HP，旧同步不得覆盖新值。
5. **事件和快照分开**：伤害/死亡/技能使用 EVENT，属性只做开战、结束和低频校验快照。
6. **未知值不猜语义**：`DamagedHP`、opaque `FCharacterForNet` 和 `FParameterWrapperArray` 保留原始值与质量标记。
7. **对象生命周期**：不跨线程保存裸 UObject 或 UE 容器内部地址。
8. **有界采集**：Hook 队列固定容量并记录 dropped counter；容器复制设置元素上限。

## 15. 通过现有 NTE MCP 调用/观察

仓库 `UETools-NTE` 已提供游戏内 MCP bridge。对于无参数 getter，可以直接按 UFunction 调用。

### 15.1 查玩家状态并读取等级

```json
{"query":"HTPlayerState","limit":32,"max_scan":131072,"timeout_ms":5000}
```

从 `nte_object_find` 选择当前本地实例后：

```json
{
  "object_address": "0xOBJECT_ADDRESS",
  "function": "Function HTGame.HTPlayerState.GetRoleLevel",
  "params_hex": "",
  "timeout_ms": 5000
}
```

当前 `Params::HTPlayerState_GetRoleLevel` 总大小为 4 字节，返回参数区 `offset 0x00` 是 little-endian `int32 ReturnValue`。

### 15.2 读取战斗属性

```json
{
  "object_address": "0xABILITY_SYSTEM_COMPONENT_ADDRESS",
  "function": "Function HTGame.HTAttributeComponent.GetHPCurrent",
  "params_hex": "",
  "timeout_ms": 5000
}
```

同类无参数 getter 还包括 `GetAtk`、`GetCrit`、`GetCritDamage`、`GetDef` 和 `GetShieldHealth`。返回参数区必须按当前 `HTGame_parameters.hpp` 解析。

### 15.3 观察伤害事件

```json
{"pattern":"Function HTGame.HTUI_DamageFloatiesForm.ShowDamageFloaties","capture_bytes":72}
```

使用返回的 hook id，然后循环调用：

```json
{"after_sequence":0,"limit":64}
```

每次使用上次返回的 `next_sequence`，结束后调用 `nte_hook_remove`。`params_hex` 按 `Params::HTUI_DamageFloatiesForm_ShowDamageFloaties` 和 `FHTDamageTextInfo` 解析。

带 `FName`、`FString`、`TArray`、`TMap`、delegate 或 UObject 指针的 raw `params_hex` 不能根据字符串表面值手工拼接；参数区必须按当前进程的 ABI、对象地址和所有权构造。此类函数优先在 C++ 中调用，MCP 主要用于无参数 getter、已有对象指针参数和 EVENT 观察。

## 16. 战报输出分类

| 报表 | 维度 | 核心指标 |
|---|---|---|
| 战斗总览 | encounter | 开始/结束、总时长、结束原因、主要目标 |
| 玩家总览 | participant | 总伤害、DPS、Active DPS、承伤、有效治疗、死亡 |
| 技能明细 | participant + skill | 伤害、占比、命中、暴击率、平均/最大单次 |
| 目标明细 | participant + target | 伤害、占比、首次/末次命中、击杀 |
| 类型明细 | participant + damageType | 类型伤害、占比、暴击率 |
| 反应明细 | participant + reactionType | 次数、伤害、占比 |
| Buff 覆盖 | participant + effect | 覆盖时长、覆盖率、平均/最大层数 |
| 阶段明细 | encounter + phase | 阶段时长、伤害、DPS、死亡 |

战报汇总至少保留来源质量：

```json
{
  "directDamage": 0,
  "correlatedDamage": 0,
  "inferredDamage": 0,
  "unknownDamage": 0,
  "droppedEventCount": 0
}
```

## 17. 采集边界与去重

1. **游戏线程采集，后台线程写盘**：Hook 中只复制有界数据，不执行 JSON 序列化或文件 I/O。
2. **不长期持有裸 UObject 指针**：日志记录中保存当时的地址、Index、FullName 和稳定业务 ID；再次读取前重新验证对象仍在 `GObjects`。
3. **事件和快照分开**：伤害、治疗、死亡和技能使用 EVENT；HP、属性只做开战、结束或低频校验快照。
4. **伤害去重**：飘字 Hook 与 Widget 轮询若同时启用，按短时间窗、攻击者、受击者、伤害值、类型和来源名合并。
5. **网络值不猜语义**：`DamagedHP`、opaque `FCharacterForNet` 和 `FParameterWrapperArray` 必须通过同版本样本验证后再升级为稳定字段。
6. **有界输出**：数组设置最大元素数，字符串设置最大字节数，writer queue 明确容量和 dropped counter。
7. **版本指纹**：Session 记录至少保存主模块版本/哈希、SDK 生成基线以及关键 UFunction/属性是否存在。

## 18. SDK 源码索引

| 内容 | 文件 |
|---|---|
| UObject、GObjects、FindObject、ProcessEvent | `SDK/CN/CppSDK/SDK/CoreUObject_classes.hpp` |
| World、Actor、GameplayStatics、PlayerState | `SDK/CN/CppSDK/SDK/Engine_classes.hpp` |
| Actor/GameplayStatics 参数布局 | `SDK/CN/CppSDK/SDK/Engine_parameters.hpp` |
| AbilitySystemComponent | `SDK/CN/CppSDK/SDK/GameplayAbilities_classes.hpp` |
| Ability/Effect/AttributeData 结构 | `SDK/CN/CppSDK/SDK/GameplayAbilities_structs.hpp` |
| NTE/HT 类和 getter | `SDK/CN/CppSDK/SDK/HTGame_classes.hpp` |
| NTE/HT UFunction 参数布局 | `SDK/CN/CppSDK/SDK/HTGame_parameters.hpp` |
| NTE/HT 数据结构 | `SDK/CN/CppSDK/SDK/HTGame_structs.hpp` |
| Targeting 请求和结果 | `SDK/CN/CppSDK/SDK/TargetingSystem_classes.hpp` |
| 现有 ProcessEvent/MCP 调用说明 | `UETools-NTE/MCP_SERVICE.md` |
| 现有战斗 RPC/transport 日志 schema | `UETools-NTE/NETWORK_TRACE_SCHEMA.md` |

## 19. 实施顺序

1. 实现 `BattleContext`、参与者映射和 `bIsInFightState` 分段器。
2. Hook `ShowDamageFloaties`，输出 canonical `combat.damage`。
3. Hook `ClientSetReplicatedTargetData`，增加 HP、护盾、死亡和治疗校验。
4. 增加开战/结束属性快照，生成第一版总览、技能和目标明细。
5. 接入 Ability delegate 与 Effect 快照，提高技能归因和 Buff 覆盖率质量。
6. 用实战样本校验重复事件、结束宽限期、Boss 阶段和治疗归因。

完成第 4 步即可生成可用战报：总伤害、DPS、暴击率、技能占比、目标占比、承伤、有效治疗和死亡。后续采集只围绕战报字段补充，不扩展成通用游戏日志。
