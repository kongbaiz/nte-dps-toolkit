# NTE Equipment Plugin

`nte-equipment-plugin` 是 NTE DPS Toolkit 的原生 Windows x64 伴随模块。它构建为
`dwmapi.dll` 代理，在游戏进程内自动定位所需地址，并通过本机命名管道处理装备请求。

本目录只保留编译和运行需要的源码、IPC 头文件及 Visual Studio 工程，不依赖
vcpkg 或客户端 SDK。

## 构建

要求：

- Visual Studio 2022，安装“使用 C++ 的桌面开发”工作负载；
- Windows 10/11 SDK；
- MSVC v143 x64 工具集。

```powershell
# 在“Developer PowerShell for VS 2022”中执行
msbuild .\nte-equipment-plugin.sln /t:Clean,Build /p:Configuration=Release /p:Platform=x64 /m
```

原始输出：`x64\Release\dwmapi.dll`。工程的构建后步骤会将它自动同步到仓库根目录的
`plugins\dwmapi.dll`，主程序只从该目录读取待安装插件。

## 通过 GUI 安装

Windows GUI 发布压缩包会把 `plugins` 目录放在 `nte-dps-tool.exe` 同级。在“控制台 → 空幕”中启用
“游戏内装备插件”后，程序会展示风险与加载原理，并锁定启用按钮 5 秒；取消按钮和
`Esc` 可立即关闭弹窗。
确认时必须先关闭 `HTGame.exe`；若同时检测到国服与国际服，需先选择客户端。
程序随后将 DLL 写入所选客户端的
`Client\WindowsNoEditor\HT\Binaries\Win64` 目录。游戏启动时会把它作为
`dwmapi.dll` 代理加载，插件再通过本机命名管道处理装备请求。

关闭该选项会删除本工具安装的 DLL 与归属标记。程序不会覆盖未带归属标记的其他
`dwmapi.dll`，也不会删除安装后被外部修改的文件；这两种情况都会提示用户手动
处理。游戏目录改动可能触发完整性或反作弊检查，启用前应阅读并接受 GUI 中的
完整风险声明。

公开的固定 IPC 布局位于 `include\nte_equipment_ipc.h`。其余实现均保留在 DLL
内部，不额外导出装备函数。
