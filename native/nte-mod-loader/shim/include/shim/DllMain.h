// DllMain.h — 入口与初始化工作线程的声明。
#pragma once

#include <Windows.h>
#include <cstdint>

namespace nte::shim {

// 写 %TEMP% 会话握手标记文件（ready/loaded + PID + nonce）
void WriteMarkerFile(const wchar_t* format, DWORD pid, std::uint64_t sessionNonce);

// StartAddress — 初始化工作线程（payload 路径恢复 + hook 初始化）
DWORD WINAPI StartAddress(LPVOID lpParameter);

// UserDllMain — 用户 DllMain
BOOL WINAPI UserDllMain(HINSTANCE hinstDLL, DWORD fdwReason, LPVOID lpReserved);

} // namespace nte::shim
