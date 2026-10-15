# 旧 Mod 插件引用检查

## 0.4.6 当前状态

- 当前 Mod 市场和桌面插件控制使用 Toolkit v1，采集使用 Capture pipe，不回退旧 v7。
- 已删除无调用者的 `src/platform/mod_loader.rs` 与 `src/platform/mods_plugin_bootstrap.rs`，并移除模块注册。旧 `nte-mod-loader.exe`、`plugins/dwmapi.dll` 启动与 bootstrap 不再编译进桌面端。
- 当前 Loader 是私有 UE Tools 工作区的 `src/loader/`，正式下载为 `tools/NTE-Loader.exe`。宿主为 `game/d3d12.dll`。
- 正式功能插件为 Combat、User、Training；Network、Performance、RenoDX、MCP 和 Dumper7 不作为公开市场功能。已安装的其他插件保留通用生命周期管理，不因下架而删除文件。

## 不能作为无效代码删除的引用

| 位置 | 当前用途 |
| --- | --- |
| `src/platform/mods_plugin.rs` | CLI 装备客户端及共享游戏区域、目录解析、时钟和错误类型仍有调用者。 |
| `src/platform/fixtures/legacy_mods_ipc_v7.h` | 冻结 wire-layout 测试输入，不是当前桌面运行时。 |
| `src/core/mod_studio.rs`、`src/storage/mod_scripts.rs` | 旧文件校验/读取和仍在使用的共享限制，不因 UI 退役直接删除用户文件兼容逻辑。 |
| `src/core/update.rs`、`src/storage/update.rs` | 旧插件更新元数据仍属于既有更新契约；本次不改变 updater protocol 或签名机制。 |
| 旧发布说明 | 历史记录，不作为当前已开放功能证明。 |

清理以当前调用关系为准。未删除仍有消费者的 CLI 契约、历史数据支持、第三方许可证或用户安装。构建、隔离回归及发布校验与真实游戏验收分别记录。
