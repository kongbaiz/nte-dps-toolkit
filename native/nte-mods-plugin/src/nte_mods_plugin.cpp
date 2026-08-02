#include "dwmapi_proxy.hpp"
#include "plugin_runtime.hpp"

#include <Windows.h>

extern "C" int _fltused = 0;
extern "C" __declspec(dllexport) const char NteModsPluginSignature[] =
	"NTE_DPS_TOOL_MODS_PLUGIN_V1";

extern "C" EXCEPTION_DISPOSITION __cdecl __C_specific_handler(
	EXCEPTION_RECORD* exception_record,
	void* establisher_frame,
	CONTEXT* context_record,
	DISPATCHER_CONTEXT* dispatcher_context)
{
	using SystemSpecificHandler = EXCEPTION_DISPOSITION(__cdecl*)(
		EXCEPTION_RECORD*,
		void*,
		CONTEXT*,
		DISPATCHER_CONTEXT*);
	const HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
	const auto handler = ntdll == nullptr
		? nullptr
		: reinterpret_cast<SystemSpecificHandler>(
			GetProcAddress(ntdll, "__C_specific_handler"));
	return handler == nullptr
		? ExceptionContinueSearch
		: handler(
			exception_record,
			establisher_frame,
			context_record,
			dispatcher_context);
}

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
