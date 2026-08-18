// CreateProcessHook.h — CreateProcessW detour。
#pragma once

#include <Windows.h>

namespace nte::shim {

// DetourCreateProcessW:
//  1. 同时扫描 lpApplicationName 与 lpCommandLine 做 UTF-16 子串匹配:
//       HTGame.exe        → Internal（manual map 注入 payload）
//       NTEGlobalGame.exe → Shim（manual map 注入 shim 自身）
//       NTEGame.exe       → Shim
//      其他              → 原样调用原函数
//  2. 命中时强制 CREATE_SUSPENDED, manual map 后按调用者原始 suspend 语义恢复。
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
    LPPROCESS_INFORMATION lpProcessInformation);

} // namespace nte::shim
