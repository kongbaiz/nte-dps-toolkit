#include "dwmapi_proxy.hpp"

#include "obfuscated_string.hpp"
#include "plugin_runtime.hpp"

#include <Windows.h>

#include <array>
#include <cstdint>

extern "C" uintptr_t mProcs[];

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
	HMODULE pinned_module = nullptr;
	if (!GetModuleHandleExW(
			GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
				GET_MODULE_HANDLE_EX_FLAG_PIN,
			reinterpret_cast<LPCWSTR>(&mProcs[0]),
			&pinned_module))
		return FALSE;

	wchar_t system_directory[MAX_PATH]{};
	const UINT length = GetSystemDirectoryW(system_directory, MAX_PATH);
	if (length == 0 || length >= MAX_PATH)
		return FALSE;

	const auto suffix = NTE_OBFUSCATE_STRING(L"\\dwmapi.dll");
	const size_t suffix_length = suffix.size() - 1;
	if (length + suffix_length >= MAX_PATH)
		return FALSE;
	for (size_t index = 0; index <= suffix_length; ++index)
		system_directory[length + index] = suffix.c_str()[index];

	HMODULE library = LoadLibraryW(system_directory);
	if (library == nullptr)
		return FALSE;

	std::array<uintptr_t, EXPORT_COUNT> resolved{};
	for (size_t index = 0; index < EXPORT_COUNT; ++index)
	{
		const auto& entry = exports[index];
		resolved[index] = reinterpret_cast<uintptr_t>(GetProcAddress(
			library,
			entry.name != nullptr
				? entry.name
				: MAKEINTRESOURCEA(entry.ordinal)));
	}

	for (size_t index = 0; index < EXPORT_COUNT; ++index)
		mProcs[index] = resolved[index];

	// The module is pinned before workers start, so an explicit FreeLibrary
	// cannot unmap their code. Process termination owns final teardown.
	nte::mods::StartPluginRuntime(pinned_module);
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
