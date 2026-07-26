#include "dwmapi_proxy.hpp"
#include "plugin_runtime.hpp"

#include <Windows.h>

extern "C" int _fltused = 0;
extern "C" __declspec(dllexport) const char NteModsPluginSignature[] =
	"NTE_DPS_TOOL_MODS_PLUGIN_V1";

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID reserved)
{
	if (reason == DLL_PROCESS_ATTACH)
	{
		if (!InitializeDwmapiProxy())
			return FALSE;
		DisableThreadLibraryCalls(module);
		nte::mods::StartPluginRuntime(module);
	}
	else if (reason == DLL_PROCESS_DETACH && reserved == nullptr)
	{
		nte::mods::StopPluginRuntime();
		ShutdownDwmapiProxy();
	}

	return TRUE;
}
