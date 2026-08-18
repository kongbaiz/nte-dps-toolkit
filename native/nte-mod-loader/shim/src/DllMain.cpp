// DllMain.cpp — nte_shim.dll 入口与初始化工作线程。
//
// 工作线程（StartAddress）:
//   1) 从 manual map 的 lpReserved（ShimInitParams）恢复 payload 路径与
//      shim 自身路径; 环境变量 NTE_MOD_LOADER_DLL_PATH 仅作后备
//   2) 读取 shim 自身 raw 字节到 g_selfBytes（供 CreateProcessW hook
//      向子进程传播 manual map 用; 读完后临时文件即可删除）
//   3) 初始化 MinHook, hook kernel32!CreateProcessW 并启用
//   4) 置位 PID 作用域握手标记文件
//
// hook 引擎采用官方 MinHook（third_party/minhook, BSD-2-Clause）; 注入方式为
// Simple-Manual-Map-Injector（third_party/manualmap, MIT）。
#include "shim/DllMain.h"

#include "shim/CreateProcessHook.h"
#include "shim/ShimGlobals.h"

#include <MinHook.h>

#include <cstring>
#include <fstream>
#include <wchar.h>

namespace nte::shim {

// ── 全局状态 ─────────────────────────────────────────────────────────────────
HINSTANCE g_hModule = nullptr;
wchar_t g_shimPath[kPathCapacity] = {};
wchar_t g_internalDllPath[kPathCapacity] = {};
ShimInitParams g_initParams{};
std::vector<uint8_t> g_selfBytes;
HANDLE g_hookReadyEvent = nullptr;

// ── 握手标记文件 ─────────────────────────────────────────────────────────────
void WriteMarkerFile(const wchar_t* format, DWORD pid, std::uint64_t sessionNonce) {
    wchar_t path[MAX_PATH]{};
    const DWORD len = GetTempPathW(static_cast<DWORD>(std::size(path)), path);
    if (!len || len >= std::size(path)) return;
    wchar_t name[64]{};
	swprintf_s(name, format, pid, static_cast<unsigned long long>(sessionNonce));
    wcscat_s(path, name);
    HANDLE file = CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                              FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return;
    DWORD written = 0;
    WriteFile(file, "1", 1, &written, nullptr);
    CloseHandle(file);
}

namespace {

// 从文件读取 raw 字节（shim 自身传播用）。
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

} // namespace

// ── StartAddress — 初始化工作线程 ────────────────────────────────────────────
DWORD WINAPI StartAddress(LPVOID) {
    // 1) payload 路径: manual map 参数优先, 环境变量后备
    if (g_initParams.payloadDllPath[0] != L'\0') {
        wcsncpy_s(g_internalDllPath, g_initParams.payloadDllPath, _TRUNCATE);
    } else {
        GetEnvironmentVariableW(kEnvInternalDllPath,
                                g_internalDllPath, kPathCapacity);
    }
    if (g_internalDllPath[0] != L'\0') {
        // 写回环境变量（供 shim 传播后的子进程继承, 后备通道）
        SetEnvironmentVariableW(kEnvInternalDllPath, g_internalDllPath);
    }

    // 2) shim 自身路径 + raw 字节（manual map 下 GetModuleFileNameW 拿不到,
    //    必须由 launcher 通过 ShimInitParams.shimSelfPath 传入）
    if (g_initParams.shimSelfPath[0] != L'\0') {
        wcsncpy_s(g_shimPath, g_initParams.shimSelfPath, _TRUNCATE);
    } else {
        GetModuleFileNameW(g_hModule, g_shimPath, kPathCapacity); // 普通加载兜底
    }
    if (g_shimPath[0] != L'\0') {
        // 读取自身 raw 字节供传播; 读完后临时文件即可被 launcher 删除
        g_selfBytes = ReadFileBytes(g_shimPath);
    }

	// 3) Per-session ownership and readiness. Instruction bytes are not an
	// ownership protocol: third-party hooks may use the same jump encoding.
	wchar_t hookEventName[128]{};
	swprintf_s(hookEventName, kHookReadyEventFormat, GetCurrentProcessId());
	SetLastError(ERROR_SUCCESS);
	g_hookReadyEvent = CreateEventW(nullptr, TRUE, FALSE, hookEventName);
	if (g_hookReadyEvent == nullptr) return 0;
	if (GetLastError() == ERROR_ALREADY_EXISTS) {
		const DWORD wait = WaitForSingleObject(g_hookReadyEvent, kShimReadyTimeoutMs);
		CloseHandle(g_hookReadyEvent);
		g_hookReadyEvent = nullptr;
		if (wait == WAIT_OBJECT_0) {
			WriteMarkerFile(kShimReadyMarkerFormat, GetCurrentProcessId(),
				g_initParams.sessionNonce);
		}
		return 0;
	}

    // 4) 初始化 MinHook 并 hook kernel32!CreateProcessW。
    //    ppOriginal 返回 trampoline 起点, 即 detour 调用的“原函数”。
	if (MH_Initialize() != MH_OK) {
		CloseHandle(g_hookReadyEvent);
		g_hookReadyEvent = nullptr;
		return 0;
    }
	if (MH_CreateHookApi(L"kernel32", "CreateProcessW",
                         reinterpret_cast<LPVOID>(&DetourCreateProcessW),
		                 &g_originalCreateProcessW) != MH_OK) {
		MH_Uninitialize();
		CloseHandle(g_hookReadyEvent);
		g_hookReadyEvent = nullptr;
		return 0;
    }
	if (MH_EnableHook(MH_ALL_HOOKS) != MH_OK) {
		MH_RemoveHook(MH_ALL_HOOKS);
		MH_Uninitialize();
		CloseHandle(g_hookReadyEvent);
		g_hookReadyEvent = nullptr;
		return 0;
    }

    // 5) 握手: 通知 launcher “shim 已加载且 hook 已启用”。
    //    %TEMP%\nte_ready_<pid>_<nonce> 标记文件（命名事件在本环境跨进程不可见,
    //    见 ShimGlobals.h 注释）。
	SetEvent(g_hookReadyEvent);
	WriteMarkerFile(kShimReadyMarkerFormat, GetCurrentProcessId(),
		g_initParams.sessionNonce);
    return 0;
}

// ── 用户 DllMain ─────────────────────────────────────────────────────────────
BOOL WINAPI UserDllMain(HINSTANCE hinstDLL, DWORD fdwReason, LPVOID lpReserved) {
	if (fdwReason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(hinstDLL);
        g_hModule = hinstDLL;
        // manual map 注入器把 ShimInitParams 作为 lpReserved 传入（目标进程
        // 内分配的指针, 可直接读）
        if (lpReserved != nullptr) {
            const auto* params = static_cast<const ShimInitParams*>(lpReserved);
			wcsncpy_s(g_initParams.payloadDllPath, params->payloadDllPath, _TRUNCATE);
			wcsncpy_s(g_initParams.shimSelfPath, params->shimSelfPath, _TRUNCATE);
			g_initParams.sessionNonce = params->sessionNonce;
        }
		HANDLE hThread = CreateThread(nullptr, 0, &StartAddress, nullptr, 0, nullptr);
		if (hThread != nullptr) CloseHandle(hThread);
	} else if (fdwReason == DLL_PROCESS_DETACH && lpReserved == nullptr) {
		MH_DisableHook(MH_ALL_HOOKS);
		MH_RemoveHook(MH_ALL_HOOKS);
		MH_Uninitialize();
		if (g_hookReadyEvent != nullptr) {
			CloseHandle(g_hookReadyEvent);
			g_hookReadyEvent = nullptr;
		}
	}
    return TRUE;
}

} // namespace nte::shim

// 标准 CRT DllMain 入口
BOOL APIENTRY DllMain(HMODULE hModule, DWORD ul_reason_for_call, LPVOID lpReserved) {
    return nte::shim::UserDllMain(static_cast<HINSTANCE>(hModule),
                                  ul_reason_for_call, lpReserved);
}
