// CreateProcessHook.cpp — CreateProcessW detour。
//
// 匹配顺序（只匹配 lpApplicationName 或命令行首 token 的精确文件名）:
//   1. "HTGame.exe"                                      → payload
//   2. "NTEGlobalGame.exe" / "NTEGame.exe"             → Shim
// 命中后强制 CREATE_SUSPENDED 创建子进程, manual map 注入（Simple-Manual-
// Map-Injector 语义）, 并按调用者原始 suspend 语义决定是否 ResumeThread。
// 注入失败不会改变返回值; HTGame 分支在 manual map 成功后写
// %TEMP%\nte_loaded_<pid>_<nonce> 标记, 让外层 launcher 能分辨“payload 已注入”与
// “进程在但注入未生效”。
#include "shim/CreateProcessHook.h"

#include "shim/DllMain.h"
#include "shim/ManualMapLoader.h"
#include "shim/ProcessTarget.h"
#include "shim/ShimGlobals.h"

#include <fstream>
#include <vector>

namespace nte::shim {

// detour 使用的“原 CreateProcessW”（MinHook 通过 MH_CreateHookApi 的 ppOriginal
// 填入 trampoline 起点）
void* g_originalCreateProcessW = nullptr;

namespace {

InjectionMode DetectCreatedProcessMode(HANDLE process) {
	wchar_t path[32768]{};
	DWORD size = static_cast<DWORD>(std::size(path));
	if (!QueryFullProcessImageNameW(process, 0, path, &size) || size == 0)
		return InjectionMode::None;
	return ModeForExecutableName(ExecutableName(path, nullptr));
}

std::vector<uint8_t> ReadFileBytes(const wchar_t* path) {
	std::ifstream stream(path, std::ios::binary);
	if (!stream) return {};
	stream.seekg(0, std::ios::end);
	const auto size = stream.tellg();
	if (size <= 0 || static_cast<std::uint64_t>(size) > kMaxDllBytes) return {};
	stream.seekg(0, std::ios::beg);
	std::vector<uint8_t> bytes(static_cast<std::size_t>(size));
	if (!stream.read(reinterpret_cast<char*>(bytes.data()), size)) return {};
	return bytes;
}

// 写当前会话 loaded 标记（payload manual map 成功后调用）
void SignalInternalLoaded(DWORD htgPid) {
	WriteMarkerFile(kInternalLoadedMarkerFormat, htgPid, g_initParams.sessionNonce);
}

} // namespace

// DetourCreateProcessW
BOOL WINAPI DetourCreateProcessW(
    LPCWSTR lpApplicationName,
    LPWSTR lpCommandLine,
    LPSECURITY_ATTRIBUTES lpProcessAttributes,
    LPSECURITY_ATTRIBUTES lpThreadAttributes,
    BOOL bInheritHandles,
    DWORD dwCreationFlags,
    LPVOID lpEnvironment,
    LPCWSTR lpCurrentDirectory,
    LPSTARTUPINFOW lpStartupInfo,
    LPPROCESS_INFORMATION lpProcessInformation)
{
    using OriginalCreateProcessW = BOOL(WINAPI*)(
        LPCWSTR, LPWSTR, LPSECURITY_ATTRIBUTES, LPSECURITY_ATTRIBUTES, BOOL, DWORD,
        LPVOID, LPCWSTR, LPSTARTUPINFOW, LPPROCESS_INFORMATION);
    auto original = reinterpret_cast<OriginalCreateProcessW>(g_originalCreateProcessW);

    const InjectionMode mode = DetectMode(lpApplicationName, lpCommandLine);
    if (mode == InjectionMode::None) {
        // 普通子进程: 原样放行
        return original(lpApplicationName, lpCommandLine, lpProcessAttributes,
                        lpThreadAttributes, bInheritHandles, dwCreationFlags, lpEnvironment,
                        lpCurrentDirectory, lpStartupInfo, lpProcessInformation);
    }

    // 命中: 保存原始 suspend 语义, 强制 suspended 创建
    const DWORD originalSuspend = dwCreationFlags & CREATE_SUSPENDED;
    dwCreationFlags |= CREATE_SUSPENDED;

    // 把 payload 路径写回当前进程环境块, 让继承环境的子进程都能解析到
    // （manual map 参数通道为主, 环境变量仅后备）
    if (g_internalDllPath[0] != L'\0') {
        SetEnvironmentVariableW(kEnvInternalDllPath, g_internalDllPath);
    }

    const BOOL result = original(lpApplicationName, lpCommandLine, lpProcessAttributes,
                                 lpThreadAttributes, bInheritHandles, dwCreationFlags,
                                 lpEnvironment, lpCurrentDirectory, lpStartupInfo,
                                 lpProcessInformation);
	if (!result) return FALSE;

	if (lpProcessInformation != nullptr) {
		// The parsed command only decides whether suspension is needed. The
		// created process path is authoritative before any remote write.
		if (DetectCreatedProcessMode(lpProcessInformation->hProcess) != mode) {
			if (originalSuspend == 0) ResumeThread(lpProcessInformation->hThread);
			return TRUE;
		}
        if (mode == InjectionMode::Shim) {
            // NTEGlobalGame.exe / NTEGame.exe: manual map 注入 shim 自身, 继续二级传播。
            // g_selfBytes 在 shim 初始化时从自身临时文件读入; 传播参数里带上
            // payload 路径（shimSelfPath 指向的临时文件可能已被 launcher 删除,
            // 被传播的 shim 读不到 self 字节也没关系——它不需要继续传播 shim）。
            if (!g_selfBytes.empty()) {
                ShimInitParams params{};
				wcsncpy_s(params.payloadDllPath, g_internalDllPath, _TRUNCATE);
				wcsncpy_s(params.shimSelfPath, g_initParams.shimSelfPath, _TRUNCATE);
				params.sessionNonce = g_initParams.sessionNonce;
                InjectByManualMap(lpProcessInformation->hProcess, g_selfBytes, &params);
            }
        } else {
            // HTGame.exe: manual map 注入 payload（--dll 指定的 DLL）。
            // 文件由用户持有, 持久存在, 每次注入时读取 raw 字节。
            if (g_internalDllPath[0] != L'\0') {
                const auto payloadBytes = ReadFileBytes(g_internalDllPath);
                if (!payloadBytes.empty()) {
                    const bool mapped = InjectByManualMap(lpProcessInformation->hProcess,
                                                          payloadBytes, nullptr);
                    if (mapped) {
                        // manual map 完成: 置位确认事件, 供外层 launcher 判断
                        SignalInternalLoaded(lpProcessInformation->dwProcessId);
                    }
                }
            }
        }
        // 调用者原本没有要求 suspended → 恢复主线程
        if (originalSuspend == 0) {
            ResumeThread(lpProcessInformation->hThread);
        }
    }
    return TRUE;
}

} // namespace nte::shim
