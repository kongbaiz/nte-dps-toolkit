# 旧 Mod 插件引用检查

检查范围：2026-09-16 当前公开应用工作树及本地 UE Tools 工作树。

## 本次移除与迁移

- 旧 native Mod 插件受 Git 管理的源码、工程和专用测试已移除。
- Loader 迁至私有 `UETools-NTE/src/mod_loader/`；App、Core、Shim、Tests
  接入 `nte_debug_tools.sln` 的 Debug、Release、DeveloperRelease 配置。
- Loader 输出到 UE Tools 的 `out/x64/<configuration>/mod_loader/`，没有加入
  公开应用或 Toolkit runtime 发布包。第三方原始许可保留。
- 公开 Rust 测试不再读取旧插件实现。IPC v7 头保留为
  `src/platform/fixtures/legacy_mods_ipc_v7.h` 冻结契约，继续验证 wire layout。
- 五项只检查已删除 C++ 实现文本的 Rust 测试、四个旧 native 测试入口已移除；
  Rust 编解码、输入边界、部署保护及生命周期行为测试保留。
- schema 生成/检查仅服务仍存在的 Rust 消费者，不再重建旧 native 目录。
- 旧 native 目录的未跟踪 `x64/` 产物、`.vs/` 缓存及 `.vcxproj.user` 因自动审批
  拦截删除而留在本机；根 `.gitignore` 显式排除这些残留，不作为源码交付。

## 仍有旧版引用

| 位置 | 引用及状态 |
| --- | --- |
| `src/platform/mods_plugin.rs` | IPC v7 客户端、`nte-mods-plugin-v7` 命名管道、运行存在标记、旧 DLL 部署与脚本资源；仍编译。 |
| `src/cli/stdio.rs`、`docs/CLI_PROTOCOL*.md` | 装备 RPC 实际使用 `ModsPluginClient`；仍需外部旧 runtime，不是 Toolkit v1。 |
| `src-tauri/src/commands/empty_curtain.rs` | 装备操作仍使用旧 `ModsPluginOperation` 类型，但提交入口明确返回 Toolkit unsupported，不再发起旧 IPC。按当前要求保留待调整函数，等待新的空幕插件接口。 |
| `src/engine/capture.rs`、Tauri `state.rs` / `commands/toolkit.rs` | 仍复用旧模块中的战斗时钟记录、错误和游戏区域类型；类型耦合不代表新 Toolkit 回退到旧 IPC。 |
| `src/platform/mod_loader.rs`、`mods_plugin_bootstrap.rs` | 保留旧 Loader 管理、`plugins/dwmapi.dll` 默认路径及旧导出 bootstrap；相关模块仍编译。手工 bootstrap 测试改用 `NTE_LEGACY_MOD_PLUGIN_DLL` 外部 DLL。 |
| `src/core/mod_studio.rs`、`src/storage/mod_scripts.rs`、`res/mod-runtime-schema.json` | 旧脚本验证、日志和事件查询实现仍保留；当前 Tauri Mod 市场入口已走 Toolkit。 |
| `src/core/update.rs`、`src/storage/update.rs`、Tauri settings/update DTO、前端 update/settings contract | 仍含旧插件版本、状态文件和更新元数据；本次未改变更新协议。 |
| 原 `plugins/` 下的版本文件、默认脚本及示例 | 已删除；Rust 不再内嵌、自动安装或替换这些脚本，测试使用内联最小样例。用户已有脚本仍按原校验与迁移规则处理；不支持的内容报错保留，不自动改写。 |
| 私有 `src/mod_loader/core/src/config/loader_config.cpp` | 默认 payload 仍为 `plugins/dwmapi.dll`，保留原 CLI 行为。 |
| 私有 `src/mod_loader/shim/include/shim/legacy_mods_manual_map.hpp` | 保留旧 DLL 镜像签名与显式 attach 标记，原逻辑及单元测试不变；不依赖旧插件源目录。 |
| `docs/releases/0.4.1.md`、网站旧功能介绍及部分历史说明 | 历史旧功能描述仍存在，不作为当前 Toolkit 能力证明。失效插件文档链接已更新。 |

## 边界

当前 `src/platform/toolkit.rs` 和 Tauri Mod 市场使用 Toolkit v1；它们没有回退到旧 v7。
移除源码不等于上述兼容客户端已退役，也不等于 Loader 已完成 Toolkit 宿主适配。
本次不改变 CLI 装备契约、用户已安装 DLL、更新协议或现有第三方许可。

战斗时钟设置已移除，旧配置不再控制新会话：抓包使用现实时间，插件默认扣除时停。
Toolkit schema 14 的暂停转换进入共用 reducer；无效或缺失的权威时钟不声称已扣除。
当前设置只读返回由数据模式推导的计时状态，保存记录及暂停查看保留原口径。

私有化范围是当前本地源码布局及现有 private-only CI。远程可见性、Git 历史和
已发布源码未修改，不能据此认定历史内容已经撤回。

## 验证记录

- UE Tools 主解决方案 Release / DeveloperRelease x64 MSBuild 均通过；两种配置的
  Loader 测试通过，各有两项依赖外部旧 DLL 的检查明确跳过。内嵌 shim 资源与产物一致。
- 原生 Python 全量初跑 182 项，出现三项失败；迁移涉及的命名和工程数量检查已修复，
  对应六项专项检查重验通过。剩余 HUD 图集复现差异已在未改动的 HEAD 输入中确认。
- 前端 lint、typecheck 和 293 项测试通过；Tauri 全量 270 项测试通过。
- Rust desktop / CLI 全量均遇到既有日文翻译缺少三项 key 的失败。
  排除此一项后，desktop 809 项单元测试及 2 项集成测试通过（12 项忽略），
  CLI 727 项单元测试及 2 项集成测试通过（8 项忽略）。
- desktop、CLI、Tauri 的 fmt / check / 严格 clippy 通过；架构、运行时安全、
  Mod schema 门禁和两个仓库的 diff whitespace 检查通过。
- 契约版本门禁仍被既有 `MOD_MARKET_SCHEMA_VERSION` 清单 4 / 实际 5 不一致阻断；
  本次 Settings Rust / TypeScript 版本均为 10。
- 未进行真实游戏注入、实时抓包或游戏内时停验收；编译和离线测试不代表这些运行成功。
