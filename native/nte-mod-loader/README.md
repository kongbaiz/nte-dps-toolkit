# nte-mod-loader

常驻式游戏 Mod 注入加载器。监控游戏启动器进程，通过 **manual map 注入**（Simple-Manual-Map-Injector 语义）把 shim 注入启动器，再由 shim 的 `CreateProcessW` hook 在游戏进程（HTGame.exe）创建时注入用户指定的 payload DLL，并确认注入成功。

## 组成

| 目录 | 内容 |
|------|------|
| `app/`   | 控制台入口（`nte-mod-loader.exe`），内嵌 shim 为资源 101 |
| `core/`  | 核心静态库：进程监控、启动器定位、配置、注入、日志 |
| `shim/`  | `nte_shim.dll`：注入到启动器的 CreateProcessW hook 载荷 |
| `tests/` | 单元/集成测试（`nte-mod-loader-tests.exe`） |
| `third_party/` | vendored 依赖：MinHook (BSD-2-Clause)、Simple-Manual-Map-Injector (MIT) |

## 构建

```powershell
& 'C:\Program Files\Microsoft Visual Studio\2022\Community\MSBuild\Current\Bin\MSBuild.exe' `
  '.\native\nte-mod-loader\nte-mod-loader.sln' `
  /m /t:Rebuild /p:Configuration=Release /p:Platform=x64 /v:minimal /nologo
```

产物输出到 `bin\Release\`（`nte-mod-loader.exe`、`nte_shim.dll`、`nte-mod-loader-tests.exe`）。

## 使用

发布包中 Loader 由 NTE 控制台直接管理，不需要放进游戏目录。目录结构固定为：

```text
NTE-DPS-TOOL/
├── nte-dps-tool.exe
├── nte-mod-loader.exe
└── plugins/
    └── dwmapi.dll
```

`nte-mod-loader` 是代理加载无效时使用的备用方式。在「Mod 工坊」中把加载方式切换
到 **Mod Loader（备用）** 后打开开关即可启动；该开关不会把 `dwmapi.dll` 复制到
`HTGame.exe` 旁边。开发者单独运行 Loader 时也应保持上述
相对目录，或显式传入 `--dll <path>`。

```powershell
# 常驻监控: 启动器打开 → 注入 shim → 游戏创建 → 注入 payload DLL
.\nte-mod-loader.exe
```

Release 可执行文件内嵌 `requireAdministrator` manifest，双击或从终端启动时会由 Windows 显示 UAC 提权。

- `--dll <path>`：指定注入到 HTGame.exe 的 payload DLL（默认 `<exe 目录>\plugins\dwmapi.dll`，也可用环境变量 `NTE_MOD_LOADER_DLL` 覆盖）
- `--dry-run`：预览模式，不做任何进程/文件变更
- `--once`：完成一轮探测即退出
- `--monitor-timeout <seconds>`：监控超时；`0` 表示持续监控
- `--stop-event <name>` / `--owner-pid <pid>`：控制台管理会话使用的内部生命周期参数
- 环境变量：`NTE_MOD_LOADER_LAUNCHER`（启动器路径覆盖）、`NTE_MOD_LOADER_SPAWN_LAUNCHER=1`（spawn 官方启动器）、`NTE_MOD_LOADER_MONITOR_TIMEOUT`（监控超时秒数，默认 120；`0` 表示持续监控）

## 工作方式

1. 启动后静默，仅输出 `[INFO] NTE Mod Loader started`
2. 常驻监控 `NTEGlobalLauncher.exe` / `NTELauncher.exe` / `NTEGame.exe` / `NTEGlobalGame.exe`
3. 检测到受信安装根内的启动器 → 输出进程名和 PID → manual map 注入 shim；临时文件在本轮监控结束后删除
4. shim hook `kernel32!CreateProcessW`；HTGame.exe 创建时 manual map 注入 payload，写带会话 nonce 的 `%TEMP%\nte_loaded_<pid>_<nonce>` 标记
5. loader 只接受当前会话标记，确认后输出 payload 文件名和游戏 PID
6. 监控期间启动器/游戏重开会自动重新注入；控制台停止事件、控制台进程退出、Ctrl+C 或监控超时会先终止本次会话已注入 shim 的启动器，再结束 loader

## 测试

```powershell
.\bin\Release\nte-mod-loader-tests.exe
```

预期输出 `ALL_TESTS_PASSED`。
