#include "dwmapi_proxy.hpp"

#include "obfuscated_string.hpp"

#include <Windows.h>

#include <cstdint>

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

HMODULE original_dwmapi = nullptr;
} // namespace

extern "C" uintptr_t mProcs[EXPORT_COUNT]{};

bool InitializeDwmapiProxy()
{
    wchar_t system_directory[MAX_PATH]{};
    const UINT length = GetSystemDirectoryW(system_directory, MAX_PATH);
    if (length == 0 || length >= MAX_PATH)
        return false;

    const auto suffix = NTE_OBFUSCATE_STRING(L"\\dwmapi.dll");
    const size_t suffix_length = suffix.size() - 1;
    if (length + suffix_length >= MAX_PATH)
        return false;
    for (size_t index = 0; index <= suffix_length; ++index)
        system_directory[length + index] = suffix.c_str()[index];

    original_dwmapi = LoadLibraryW(system_directory);
    if (original_dwmapi == nullptr)
        return false;

    for (size_t index = 0; index < EXPORT_COUNT; ++index)
    {
        const auto& entry = exports[index];
        mProcs[index] = reinterpret_cast<uintptr_t>(GetProcAddress(
            original_dwmapi,
            entry.name != nullptr ? entry.name : MAKEINTRESOURCEA(entry.ordinal)));
    }
    return true;
}

void ShutdownDwmapiProxy()
{
    if (original_dwmapi != nullptr)
        FreeLibrary(original_dwmapi);
    original_dwmapi = nullptr;
}
