#include <Windows.h>

#include "nte_mods_plugin_manual_map.hpp"
#include "plugin_runtime.hpp"

extern "C" __declspec(dllexport) const char NteModsPluginSignature[] =
	NTE_MODS_PLUGIN_IMAGE_SIGNATURE;

BOOL WINAPI DllMain(HINSTANCE module, DWORD reason, LPVOID reserved)
{
	// The OS loader always reaches this entry without the private marker, so its
	// loader-lock path performs no worker or library operation. The vendored
	// manual-map shellcode invokes this same entry on its dedicated remote thread
	// and supplies the marker only after imports, TLS, and unwind registration.
	if (reason == DLL_PROCESS_ATTACH &&
		nte::mods::manual_map::IsExplicitAttach(reserved))
		nte::mods::StartPluginRuntime(module);
	return TRUE;
}
