#include <Windows.h>

#include "nte_mods_plugin_manual_map.hpp"
#include "nte_mods_plugin_lifecycle.h"
#include "plugin_runtime.hpp"

extern "C" __declspec(dllexport) const char NteModsPluginSignature[] =
	NTE_MODS_PLUGIN_IMAGE_SIGNATURE;

extern "C" __declspec(dllexport) uint32_t WINAPI
NteModsPluginInitialize(void* reserved) noexcept
{
	if (reserved != nullptr)
		return static_cast<uint32_t>(
			nte::mods::PluginStartResult::Failed);
	return static_cast<uint32_t>(
		nte::mods::InitializeRecordedPluginRuntime());
}

// Manual-map owners may call this export from a normal thread before unmapping.
// TRUE is deliberately strict: any published detour lineage keeps the image
// resident because foreign game-thread instruction-pointer quiescence cannot be
// proven without suspending those threads.
extern "C" __declspec(dllexport) BOOL WINAPI NteModsPluginShutdown() noexcept
{
	return nte::mods::PluginStopAllowsUnload(
		nte::mods::StopPluginRuntime()) ? TRUE : FALSE;
}

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID reserved)
{
	if (reason == DLL_PROCESS_ATTACH)
	{
		// Record the already-supplied image address before choosing the deployment
		// lifecycle. The manual mapper already owns a loader-lock-free remote
		// thread. A normal proxy load schedules a finite in-process worker whose
		// entry cannot run until this loader callback has returned.
		nte::mods::RecordPluginModule(module);
		// The vendored manual mapper invokes this entry on its dedicated remote
		// thread after imports, TLS, and unwind registration, outside the OS loader
		// lock. Its private marker preserves that explicit initialization contract.
		if (nte::mods::manual_map::IsExplicitAttach(reserved))
			nte::mods::InitializeRecordedPluginRuntime();
		else
			nte::mods::ScheduleRecordedPluginRuntimeInitialization();
	}
	return TRUE;
}
