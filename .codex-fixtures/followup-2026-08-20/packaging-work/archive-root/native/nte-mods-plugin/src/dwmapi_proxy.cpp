#include "dwmapi_proxy.hpp"

#include "obfuscated_string.hpp"
#include "plugin_runtime.hpp"

#include <Windows.h>

#include <array>
#include <cstdint>

extern "C" uintptr_t mProcs[];
// This import is bound by the Windows loader through the DWM API-set before
// this proxy's DllMain runs. Taking its address identifies the real system
// dwmapi image even when this local proxy has the same base filename.
extern "C" __declspec(dllimport) HRESULT WINAPI DwmFlush();

namespace
{
struct ExportDescriptor
{
    uint16_t ordinal;
    const char* name;
};

constexpr ExportDescriptor exports[]{
#include "dwmapi_exports.inc"
};
constexpr size_t EXPORT_COUNT = sizeof(exports) / sizeof(exports[0]);

INIT_ONCE proxy_initialize_once = INIT_ONCE_STATIC_INIT;

[[noreturn]] void FailFastProxyInitialization()
{
	RaiseFailFastException(nullptr, nullptr, 0);
	__assume(false);
}

BOOL CALLBACK InitializeDwmapiProxyOnce(
	PINIT_ONCE,
	PVOID,
	PVOID*)
{
	HMODULE system_dwmapi = nullptr;
	if (!GetModuleHandleExW(
			GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
				GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
			reinterpret_cast<LPCWSTR>(&DwmFlush),
			&system_dwmapi))
		return FALSE;

	std::array<uintptr_t, EXPORT_COUNT> resolved{};
	for (size_t index = 0; index < EXPORT_COUNT; ++index)
	{
		const auto& entry = exports[index];
		resolved[index] = reinterpret_cast<uintptr_t>(GetProcAddress(
			system_dwmapi,
			entry.name != nullptr
				? entry.name
				: MAKEINTRESOURCEA(entry.ordinal)));
	}

	for (size_t index = 0; index < EXPORT_COUNT; ++index)
		mProcs[index] = resolved[index];

	return TRUE;
}
} // namespace

extern "C" uintptr_t mProcs[EXPORT_COUNT]{};

extern "C" uintptr_t ResolveDwmapiExport(size_t index)
{
	if (index >= EXPORT_COUNT ||
		!InitOnceExecuteOnce(
			&proxy_initialize_once,
			InitializeDwmapiProxyOnce,
			nullptr,
			nullptr) ||
		mProcs[index] == 0)
		FailFastProxyInitialization();
	return mProcs[index];
}
