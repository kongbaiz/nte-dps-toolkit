# 正式版插件对接状态

范围：仅 UE Tools Release / Toolkit v1。DeveloperRelease、MCP、Dumper7、旧脚本执行不对外提供；网络插件采证不作为桌面 DPS 的外部抓包数据源。

## 已接入口

- 保留既有互斥数据源：Plugin -> Release 原生 Capture v1 推送 -> 共用 reducer；PacketCapture -> 外部抓包。插件失败不回退抓包。Toolkit Report 只用于显式预览/归档对照，不再轮询整份战报进行实时统计。
- 既有插件启用/禁用/重载、7 档日志等级、桌面采集启停。
- 具名 Release 操作：HostStatus、RuntimeStatus/Scan、Shutdown；CombatStatus/Reset/Export/StopExport/Operation/Report、EvidenceStatus；NetworkStatus/Enable/Flush；RuntimeRefresh、RadarRefresh、TraceEnable/Clear；UserStatus/Refresh/Operation/Snapshot/Export/Cancel；HudStatus/HudConfigure。
- 新操作由前后端固定枚举选择，必须由当次 Describe 声明支持。没有任意命令号、脚本或任意游戏函数调用入口。
- PluginPanel v2 保留宿主连接身份、实际命令集与本地 collectorActive；Combat 未加载不再使宿主管理失联。开发宿主（ui=true / mode 非 toolkit）拒绝连接。
- 新操作绑定 PID 与进程创建时间；进程更换拒绝旧请求。有限 IPC 期间复用采集/模式切换 reservation，不持锁调用插件；结束和失败自动释放。
- 本地采集中禁止直接重置、停止并导出、SDK 重扫和宿主关闭；这些动作及清空追踪/取消读取需要确认。普通采集启停仍由采集服务负责。
- 原生响应仅为显式、有字节数的最多 8 KiB 文本预览；不是完整数据视图。完整战报/账号数据走原生导出。异步 Accepted 不当成完成，用户可查询 operation。
- 控制台仅保留插件管理与正式版操作，不显示伤害统计或正常采集启停；主页/HUD 使用同一采集服务。控制台状态刷新间隔为 3 秒，隐藏时暂停。
- 游戏内增强 HUD 有独立总开关及冷却、敌人条、就绪提示、生命值、倾陷值开关。通过正式宿主 118/119 控制，采集中也可调整；不启动/停止采集，不关闭桌面统计。只在原生回读成功后更新开关，旧宿主未声明能力时禁用。设置属于当前插件实例，重载后按原生状态重新读取，不冒充已持久化。

## 推送采集与命名

- 先通过 Toolkit Describe 校验 Release，再以 PID/创建时间绑定原生 Capture v1 管道并校验服务端 PID、providerId、captureId、能力与通知序号。已有采集返回 Busy，不抢占；连接所有者退出或失败关闭管道，停止其拥有的采集。
- 帧上限 256 KiB，逐批最多 64 击。复用核心的有界可靠事件队列，不抽样伤害；序号缺口、末尾计数不符或原生丢记录/上下文均显式报错。rawEvidence=false 不申请管道原始证据回执；当前正式插件内部仍写 CombatEvidence，不能据此宣称没有原生日志开销。
- 命中身份仅使用命中绑定的 contextId，加属性/效果快照中的 actor index + serial，匹配该上下文原生读取的 DefaultCharacterID。最多保留 256 个完整上下文，淘汰后无法匹配则保持未知；不用当前出场角色、时间邻近或类名补归属。
- 技能使用原生 skillKey/skillName、attackDetailKey/attackDetailName 与现有资源名称规则；damageLane（例如 character）不再当作招式名称。服务器明确反应类型且角色证据完整时，缺少技能元数据不再抹掉角色归属；原始 quality 仍保留。
- 时钟以 250 ms 间隔读取原生缓存的暂停转换，不请求完整战报。每次采集的首条记录建立基线，允许宿主时钟序号跨轮次递增；基线之后仍严格检查连续性和时间顺序。首个有效时钟到达前暂存最多 4096 击，5 秒内未就绪、缺边界或无效时钟均显式终止采集，不把插件统计切换到现实时间。

## 本轮验证边界

- 保存的原生 schema 14 战报：109 击、总伤害 1,416,528；投影后数额/次数不变，8 次原先误判未知归属的显式反应伤害（黯星/浊燃）有原生参与者证据，已正确归类。
- 同一战报格式化后约 3.46 MB，单次读取及解析约 240 ms。这是外部调用耗时，不是插件 CPU 或游戏帧时间。
- 当前 Release 实机推送连接完成 hello/start/context/clock/stop/end，120 秒窗口内收到 0 击，无错误。因此只验证连接生命周期，不能宣称真实命中延迟或 FPS 已验收。
- 用户反馈为刚开启时短暂低帧、之后恢复。临时卸载后的反馈不能证明卸载导致恢复；新版启动阶段仍需匹配实战验证。没有重编译/更换游戏 DLL。

## 尚未完成

- User 装备写操作使用独立 Combat JSON-RPC 管道及 domainKey/epoch/requestId，不是 Toolkit 300–305；当前空幕写操作仍保持 unsupported，未迁移为新协议。
- Runtime/Radar/Trace 已开放正式版请求，但详细视图需要原生查询 DTO；不冒充 DeveloperRelease 面板已移植。
- 实时带命中的端到端验收及启动阶段帧率对照尚未完成；不以离线战报或构建通过替代。

原生共享内存 ABI 不变。修改仅在桌面集成副本；原安装和此前抓包解析成果保持原样。单元测试、构建与未来实机验收分开记录。

## 重新启停与通知修复

- 主窗口 v9 的空读数允许 `dpsTime.effectiveMode=pending`，代表等待插件计时，不代表已经切换到现实时间；非空读数不允许使用 pending。抓包模式仍固定 real-time。
- 通知宿主将内部 status 规范为 v1 协议支持的 info。相同内容的未过期通知复用 id，不重置倒计时、不增加 revision；过期或实际内容变化仍产生新通知。
- 通知窗口忽略已卸载或已被后续请求取代的快照/关闭响应，旧通知不能覆盖新通知。通知格式错误与通知服务错误使用独立文案，不再误报成 Rust 桥接不可用。
- 回归覆盖三次独立采集的时钟首序号 1/41/901；同一输入现实时间为 10 秒、插件扣除时停后为 6 秒。这是离线回归，不是游戏内启停验收。

## 逐击属性、状态与暴击详情

- Capture v1 的 criticalKnown/critical/criticalSource、attacker/victim Attributes/Effects 直接绑定到当前 hit，不从暴击倍率或当前角色面板推算。未知暴击为 null；数值零与未知严格区分。受击时交换角色/敌方状态摘要，方向未知则不猜角色归属。
- 原始字段使用有界类型解析：每侧最多 64 个属性、512 个状态，单击超出 128 KiB 或畸形结构会拒绝该响应；同一采集最多保留 16,384 份 / 64 MiB 明细。采集保留预算超出会显式标为 budget_exceeded，保留伤害、暴击和状态摘要，不静默丢伤害。状态摘要含正面、负面和其他状态数，不是新增/移除数。
- 每击不可变 Arc 快照随 Hit 在历史与 JSON 导出/回放中保留。列表 v9 仅推送摘要，不在高频 Channel 复制完整属性/状态。get_main_dps_hit_snapshot 只允许战斗明细窗口调用，以源 hit 索引、时间身份及不可复用的进程内记录引用核对（重新导入重新分配，原生 capture/hit key 仍作为元数据保留），锁内只做 O(1) 查找和 Arc 引用复制；详情复制/序列化在锁外。
- 前端新增暴击、逐击快照列及列宽/显隐设置。点击详情按需读取冻结记录，属性保留原始单位和精度，不擅自换算百分比；状态列表初始 50 条，显式加载更多。先前没有记录这些字段的历史不会从当前状态补齐。
- ui-preview.html?view=hit-details 使用明确标注的合成数据，只验证显示与交互，不代表游戏内字段覆盖已验收。
