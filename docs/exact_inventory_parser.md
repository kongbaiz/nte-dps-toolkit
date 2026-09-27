# 纯抓包空幕：登录库存解析

## 当前范围

`src/engine/inventory.rs` 从原始 UDP 的 Bunch / 分片 / RPC 边界读取登录库存，
替代生产路径中的逐位扫描猜测。运行时不需要用户插件、MCP 或 observations 文件。
当前验收只覆盖 2026-09-27 提供的**登录抓包**；不代表锁定、装卸、强化等增量通知已经验证。

流按端点和 component prefix 隔离。只在 channel 3 的 actor-open 导出与已核对的
PlayerController archetype 路径及导出 checksum `3604383830` 相符后，使用
field upper-exclusive `219`、inventory RPC index `131`。此 checksum 是导出字段，
不是整个程序或序列化布局的密码学证明；未来版本仍需重新核对。

`inventory/schema.json` 保存当前 SDK 的 15 个嵌套结构、87 个序列化字段。
旧记录布局遗漏的 `ExpireTime`、`CanUseUtcTimeTicks` 现在按字段顺序消费。
InitItems（类型 3）必须完整消费 RPC，数组、递归、流数及保留数据均有预算。
尾随未完成分片、冲突、布局不支持或已知影响装备的未支持增量通知会清空可用结果并告警，
不回退旧扫描器。登录包装参数保留为未知业务含义，不能用它宣布“全背包已到齐”。

## 数值和配装依据

- 唯一 ID、等级、锁定、弃置、装备归属来自登录记录。
- 配装位置必须通过角色槽位的装备唯一 ID 与装备反向角色 ID 一致性校验；不按名字、
  当前队伍或时间邻近推断。
- 副词条使用包内 Float32；核对按 Float32 位模式进行，不经过显示舍入。
- 主词条数值不是直接在包中传输：用包内属性 ID、等级查询现有资源表的**精确采样点**。
  缺采样点返回不可用，不沿用旧插值。例如已验证 AtkAdd 对应 20 级为 63，
  同曲线缺少 19 级采样点时不能把插值得到的值当作已验证数据。
- `PacketInventory` 同时归并装备及角色；重复结果不递增 revision。
  发布合并到最多每 250 ms 一次，导入结束时刷新待发布结果。

## 离线验收（2026-09-27）

样本 SHA-256：

- PCAP：`59af93be8656452f444b33f369dadb9be70c9985e4bb74b52010a9d27e43598c`
- API observations：`d02498b2109c01e881ad98c0085f87efa18fac19e19f5cbd0d1440fb21e47dab`

正式链路命令（输出文件须不存在）：

```powershell
cargo run --no-default-features --features cli --example replay_equipment -- INPUT.pcapng OUTPUT.json
```

同一 PCAP 的旧路径结果为 0 件装备、0 个角色。新路径经过真实 capture decoder 和
core reducer 得到 561 件装备、21 个角色，12 次装备/角色原子事件，错误 0、库存告警 0。
与独立 Python 按边界解码的结果核对：

| 检查 | 结果 |
| --- | --- |
| 库存 RPC 完整消费 | 79 条，其中 73 条 InitItems |
| 装备归属 | 144 件一致 |
| 驱动块位置 | 126 件一致，另外 18 件为卡带 |
| 副词条 Float32 | 2,244 项逐位一致 |
| 锁定/弃置标记 | 1,122 项一致 |
| 等级分布 | 341 件 0 级、220 件 20 级 |

本机详细结果位于 `out/packet-equipment-login/offline-acceptance.json`、
`agreement.json`；包含账号装备记录的产物及抓包不得提交。

## 证据边界

独立解码比较证明两条实现对这份捕获字节的读取一致，不等于游戏原生逐字段同包校验。
API observations 有 1,926 条数据与 PCAP 字节精确重合，但提供方标记
`captureComplete: false`、`semanticLineage: unavailable`，不能据此宣称网络捕获完整。
已做部分当前结构反射核对；游戏重启后未补完全部嵌套结构的 live 校验，
用户选择先完成离线验证。`nativeSamePacketSemanticLineageVerified` 仍为 `false`。
未验证增量操作、未来协议版本，以及本样本没有覆盖的等级/属性组合；
主程序构建、单测或本次回放不能替代这些实机验收。
