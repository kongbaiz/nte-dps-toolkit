// ShimGlobals.h — nte_shim.dll: 全局状态与共享常量。
#pragma once

#include <Windows.h>

#include <cstddef>
#include <cstdint>
#include <vector>

namespace nte::shim {

// ── 常量 ────────────────────────────────────────────────────────────────────

// 环境变量协议, 由外层 loader 写入（后备通道; 主通道是 manual map 的
// lpReserved 参数, 见 ShimInitParams）
inline constexpr const wchar_t kEnvInternalDllPath[] = L"NTE_MOD_LOADER_DLL_PATH";

// 握手标记文件名格式（%TEMP%\nte_ready_<pid>_<nonce> /
// nte_loaded_<pid>_<nonce>）。
// 用文件代替命名事件: 本环境（沙箱/会话隔离）下同名事件对象在不同进程间
// 不可见, 而 %TEMP% 同用户共享。会话 nonce 用于防止 PID 复用或
// 多个 loader 实例之间误认旧标记。
// 注意: 必须与 core/include/nte/loader/Injection/ShimPayload.h 保持一致。
inline constexpr const wchar_t kShimReadyMarkerFormat[] = L"nte_ready_%lu_%016llX";
inline constexpr const wchar_t kInternalLoadedMarkerFormat[] = L"nte_loaded_%lu_%016llX";
inline constexpr const wchar_t kHookReadyEventFormat[] =
	L"Local\\nte-mod-loader-hook-%lu";

// 缓冲区容量
inline constexpr size_t kPathCapacity = 260; // WCHAR（路径缓冲区）
inline constexpr std::uint64_t kMaxDllBytes = 64ULL * 1024ULL * 1024ULL;
inline constexpr DWORD kShimReadyTimeoutMs = 10000;

// 注入目标名（子串匹配, 不区分路径边界）
inline constexpr const wchar_t kTargetHtGame[] = L"HTGame.exe";
inline constexpr const wchar_t kTargetNteGlobalGame[] = L"NTEGlobalGame.exe";
inline constexpr const wchar_t kTargetNteGame[] = L"NTEGame.exe";

// manual map 注入时经 DllMain 的 lpReserved 传入的初始化参数
// （必须与 core/include/nte/loader/Injection/ShimPayload.h 保持一致）。
struct ShimInitParams {
	wchar_t payloadDllPath[kPathCapacity]; // --dll 指定的注入 DLL
	wchar_t shimSelfPath[kPathCapacity];   // shim 自身临时文件路径（读 self 字节用）
	std::uint64_t sessionNonce;
};

// ── 全局状态 ─────────────────────────────────────────────────────────────────

// 当前 shim 模块句柄
extern HINSTANCE g_hModule;

// shim 自身完整路径（manual map 下无法用 GetModuleFileNameW 取得,
// 由 ShimInitParams.shimSelfPath 传入）
extern wchar_t g_shimPath[kPathCapacity];

// payload DLL 完整路径（manual map 注入 HTGame 用）
extern wchar_t g_internalDllPath[kPathCapacity];

// manual map 传入的初始化参数副本（UserDllMain 从 lpReserved 拷入）
extern ShimInitParams g_initParams;

// shim 自身 raw 文件字节（UserDllMain/StartAddress 从 shimSelfPath 读入,
// 供 CreateProcessW hook 向子进程传播 manual map 用; 读取后临时文件即可删除）
extern std::vector<uint8_t> g_selfBytes;

// 由 MinHook 填写的“原 CreateProcessW”（trampoline 起点）, 供 detour 使用
// 定义于 CreateProcessHook.cpp
extern void* g_originalCreateProcessW;

// ── 注入模式 ────────────────────────────────────────────────────────────────
enum class InjectionMode {
    None = 0,    // 普通子进程, 原样放行
    Shim = 1,    // NTEGlobalGame.exe / NTEGame.exe → manual map 注入 shim 自身
    Internal = 2 // HTGame.exe → manual map 注入 payload（--dll 指定）
};

} // namespace nte::shim
