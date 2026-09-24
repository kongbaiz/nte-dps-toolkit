> 迁移处理状态：本文件下方保留迁移前的审计证据与当时行号。工作副本现已接入精确事件/历史/未知值契约，并在 live 与 PCAP 导入入口启用 exact mode；7 类旧逻辑均已对新包数据隔离。旧函数保留供旧记录/回归及非战斗功能使用，不作为新入口失败后的 fallback。详见 `exact_packet_parser.md`。原工作树与安装目录没有部署改动。

# 旧算法遗留审计（2026-09-24）

对象：本工作副本中的实际生产调用链，不是只搜索旧函数名称。
本轮仅检查与记录问题，没有切换入口、修改算法或重复运行已确认的五场回放。

## 1. 正式入口仍调用旧解析器（最高优先级）

`capture.rs:6232-6250` 在出站包路径调用 `parse_damage_payload`。
新的 `settlement::Ledger` / `transport::Decoder` 外部使用者只有
`examples/replay_settlement.rs`，尚未接入 `PacketDecoder`。
新候选的 1192 条正确回放结果不能代表当前正式入口已经使用新算法。

## 2. 请求记录仍被转换为伤害和推算剩余 HP

`parser.rs:3462-3487` 使用 `record.damage`，并计算
`(target_hp_before-damage).max(0.0)`；`capture.rs:4634-4673` 的待定/无目标记录
超时后仍会返回待发射 Hit，`capture.rs:6151-6154` 会发射这些记录。
因此，候选等待机制不是“无服务器结算绝不记账”的门槛。
新账本必须完全绕过该请求记账/超时释放路径，不能仅在后面追加一个结算纠正器。

## 3. 来源角色仍有会话回退和技能反推覆盖

- `capture.rs:6238-6247` 从 session_characters 取 fallback。
- `parser.rs:3467-3468` 使用 packet/aligned/fallback 角色选择链。
- `capture.rs:5965-5985` 根据 GE 的 owner_character_id 改写角色。
- `capture.rs:6016-6037` 根据 Ability 名称改写角色；特殊条件允许覆盖 Packet 来源。
- `capture.rs:6267` 的正式路径实际调用了该逻辑，不是测试专用死代码。

这些路径不能用于新结算记录。资源所属角色只做一致性检查，不能替代 NetSourceActor。

## 4. 旧去重/补技能/补目标不使用结算身份

`capture.rs:5384-5421` 仍按短时间窗口、伤害/HP 快照、旧 DamageWireEvent 比较，
`same_exact_wire_damage_event` 在两边都有 wire_event 时不再检查目标 ID。
`DamageWireEvent` 本身没有 MsgIndex、完整来源/目标引用或分量编号。
因此，不同目标若旧快照字段全相同，有被当成同一事件的代码路径；这里是源码条件风险，
不是声称五场样本已经出现此误合并。

`capture.rs:4678` 起的目标补配还使用目标 HP 匹配，并带一对一候选检查。
该检查可减少歧义，但不能把血量相等变成协议目标身份的证据。

新分量必须以连接/代次、消息身份、目标/分量序号管理，并保留完整源/目标引用做一致性验证。

## 5. 后续统计仍会按 HP 修正/补造伤害

- `capture.rs:5054` 的 reconcile_boss_hp_updates 仍在正式路径被调用。
- `reducer.rs:49-78` 处理旧 Hit、HitFollowUp、HitDamageCorrection；修正后还能触发目标总额重算。
- `model.rs:5324-5399` 的 reconcile_server_target_limit 将 HP 派生目标额度与已记伤害比较，
  可新增 residual Hit，或改变已有 hit 的 overkill。
- `model.rs:294-345` 以 HP 近似相等和时间窗口分配过量伤害；`model.rs:5452` 在常规插入时调用。

因此，“解析器已经精确读出服务器伤害”并不足够。若把新行直接转成旧 Hit，
旧模型仍有再次改变有效统计值的路径。必须将原始结算账本与推算/过量分析隔离。

## 6. 正式事件、历史和定位键尚不能承接新语义

`model.rs:6502` 的 EngineEvent 没有新核心的按 MessageKey 替换/撤回事件，
`reducer.rs:49` 对普通 Hit 仍是追加。
`model.rs:239` 的 wire_event 还带 serde(skip)，不会随历史持久化。
`model.rs:6589` 的 HitLocator 使用角色、时间、字节位置、GE、最大 HP 等字段定位，
不是冻结的连接/代次/MsgIndex/原始时间戳/目标/分量身份。

这是未接入的风险：直接追加新 Change.rows 会使晚到请求重复记账，冲突撤回也无对应通路。
必须实现幂等 upsert/retract、正确 revision/no-op、回合归属和历史往返身份保留。

## 7. 未知值与时点语义尚未传到旧 DTO

`model.rs:188-193` 和 `api/battle.rs:188-191` 的 HP 字段是不可空数值，
没有区分请求快照与结算当前值。新核心的 Option/raw-bits 信息不能靠填 0 或旧缓存强塞进去。
在启用新入口前，需明确未知显示和兼容契约，而不是把“缺少最大 HP”显示为“最大 HP 为 0”。

## 不应误删的路径

旧 parser/capture 中还混有装备、库存、文本与其他非战斗功能。不能整文件清空。
`model.rs:5024-5033` 的敌人遥测目标补全有适用条件，不会无条件覆盖任意已存在的 wire 目标 ID；
迁移时应保留它的功能边界，但不能把近时遥测关联升级为纯封包精确身份。
新核心仍依赖显式版本/连接 profile 的问题属于新入口资格待完成，不是发现了一个新的旧函数缺陷。

## 处理顺序

先补正式事件/历史/未知值契约，再接入捕获和导入共同入口；
让新记录绕过旧请求记账、角色反推、快照去重和 HP 修正链，最后清理不再使用的旧战斗路径。
不能只删几个 fallback 或把新 Decoder 接到旧 Hit 末端就宣称迁移完成。
