#pragma once

#include <Windows.h>

#include <stdint.h>

#ifdef __cplusplus
extern "C"
{
#define NTE_MODS_PLUGIN_NOEXCEPT noexcept
#else
#define NTE_MODS_PLUGIN_NOEXCEPT
#endif

// Stable return values for NteModsPluginInitialize. The caller must invoke the
// export from a normal target-process thread after LoadLibrary/manual-map
// initialization has returned and the Windows loader lock is not held.
typedef enum NteModsPluginInitializeStatus
{
    NTE_MODS_PLUGIN_INIT_STARTED = 0,
    NTE_MODS_PLUGIN_INIT_ALREADY_RUNNING = 1,
    NTE_MODS_PLUGIN_INIT_NOT_GAME_HOST = 2,
    NTE_MODS_PLUGIN_INIT_IN_PROGRESS = 3,
    NTE_MODS_PLUGIN_INIT_FAILED = 4,
} NteModsPluginInitializeStatus;

// `reserved` must be null. The parameter makes the export ABI-identical to a
// Win32 LPTHREAD_START_ROUTINE so a verified remote caller can use
// CreateRemoteThread without an adapter thunk.
__declspec(dllexport) uint32_t WINAPI NteModsPluginInitialize(void* reserved)
    NTE_MODS_PLUGIN_NOEXCEPT;
__declspec(dllexport) BOOL WINAPI NteModsPluginShutdown(void)
    NTE_MODS_PLUGIN_NOEXCEPT;

#ifdef __cplusplus
}
#endif

#undef NTE_MODS_PLUGIN_NOEXCEPT
