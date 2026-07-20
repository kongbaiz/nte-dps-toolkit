#include "dwmapi_proxy.hpp"
#include "plugin_runtime.hpp"

#include <Windows.h>

extern "C" int _fltused = 0;

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID reserved)
{
	if (reason == DLL_PROCESS_ATTACH)
	{
		if (!InitializeDwmapiProxy())
			return FALSE;
		DisableThreadLibraryCalls(module);
		nte::equipment::StartPluginRuntime();
	}
	else if (reason == DLL_PROCESS_DETACH && reserved == nullptr)
	{
		nte::equipment::StopPluginRuntime();
		ShutdownDwmapiProxy();
	}

	return TRUE;
}
