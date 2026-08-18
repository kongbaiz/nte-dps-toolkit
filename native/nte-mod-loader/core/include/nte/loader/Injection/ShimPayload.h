#pragma once

#include <Windows.h>

#include <cstdint>
#include <filesystem>
#include <optional>
#include <vector>

namespace nte::loader {

// shim 端路径协议上限（与 shim 工程的 kPathCapacity = 260 一致）。
// 超过该长度时 shim 会拒绝, 因此 loader 必须在写入协议前显式拒绝。
inline constexpr size_t kShimPathCapacity = 260;
inline constexpr std::uintmax_t kMaxPayloadDllBytes = 64ULL * 1024ULL * 1024ULL;

// 等待 shim 握手/内部 DLL 确认的超时（毫秒）。
inline constexpr DWORD kShimReadyTimeoutMs = 10000;

// 握手标记文件名格式（%TEMP%\nte_ready_<pid>_<nonce> /
// nte_loaded_<pid>_<nonce>）。
// 用文件代替命名事件: 本环境（沙箱/会话隔离）下同名事件对象在不同进程间
// 不可见, 而 %TEMP% 同用户共享。必须与 shim/include/shim/ShimGlobals.h
// 保持一致。
inline constexpr const wchar_t kShimReadyMarkerFormat[] = L"nte_ready_%lu_%016llX";
inline constexpr const wchar_t kInternalLoadedMarkerFormat[] = L"nte_loaded_%lu_%016llX";

// manual map 注入时通过 DllMain 的 lpReserved 传给 shim 的初始化参数
// （在目标进程内分配, 由 shim 的 UserDllMain 拷走）。取代旧的
// NTE_MOD_LOADER_DLL_PATH 环境变量 + <shim>.path.txt sidecar 协议。
// 必须与 shim/include/shim/ShimGlobals.h 保持一致。
struct ShimInitParams {
	wchar_t payloadDllPath[kShimPathCapacity]; // --dll 指定的注入 DLL
	wchar_t shimSelfPath[kShimPathCapacity];   // shim 自身临时文件路径（读 self 字节用）
	std::uint64_t sessionNonce;                // scopes ready/loaded handshakes
};

enum class InjectionResult {
	Success,
	Failed,
	TimedOut,
};

class ShimPayload final {
public:
    // 读取 DLL 文件字节（manual map 的输入; 文件读完后即可删除, 不会被映射占用）。
    static std::optional<std::vector<std::uint8_t>> ReadFileBytes(const std::filesystem::path& path);

    // 把资源 101（nte_shim.dll）落盘为临时文件。落盘是必要的: 传播链上
    // 每个 shim 实例需要从自身文件读取 raw 字节以继续向子进程传播。
    static std::optional<std::filesystem::path> ExtractResource101();

    // 把 dllBytes 手工映射进 pid 进程（Simple-Manual-Map-Injector 语义:
    // 重定位/导入/TLS/x64 SEH, 清 PE 头）。initParams 非空时在目标进程分配
    // 并写入, 作为 DllMain 的 lpReserved 传给被映射的 shim。
    // Success 表示映射完成且 DllMain 已执行；TimedOut 表示远程线程
    // 仍可能运行，调用方不得在同一 PID 上重试或释放其参数。
    static InjectionResult InjectWithManualMap(
		DWORD pid, const std::vector<std::uint8_t>& dllBytes,
		const ShimInitParams* initParams = nullptr);

    // CREATE_SUSPENDED 创建 launcherPath, manual map 预装 shim（DllMain 以
    // initParams 为 lpReserved）后恢复主线程。失败时终止挂起的子进程。
    static bool SpawnWithManualMap(const std::filesystem::path& launcherPath,
                                   const std::vector<std::uint8_t>& shimBytes,
                                   const ShimInitParams& initParams,
                                   DWORD* outPid = nullptr);

    // 等待 pid 进程内 shim 写入“已加载且 hook 已启用”会话标记。
	static bool WaitForShimReady(DWORD pid, std::uint64_t sessionNonce,
	                             DWORD timeoutMs = kShimReadyTimeoutMs);

    // 等待 pid 进程内 payload DLL 注入确认标记（manual map 完成后由 shim 写入）。
	static bool WaitForInternalDllLoaded(DWORD pid, std::uint64_t sessionNonce,
	                                     DWORD timeoutMs = kShimReadyTimeoutMs);

    // 校验 payload 存在且为合法的 x64 PE32+ DLL。
    static bool ValidateInternalDll(const std::filesystem::path& path);

    // manual map 后临时文件无映射占用, 直接删除（含历史 sidecar 残留）。
    static void RemoveShimFile(const std::filesystem::path& shimPath);
};

} // namespace nte::loader
