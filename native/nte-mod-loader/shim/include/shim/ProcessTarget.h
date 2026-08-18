#pragma once

#include "shim/ShimGlobals.h"

#include <cwchar>
#include <string>

namespace nte::shim {

inline std::wstring ExecutableName(LPCWSTR applicationName, LPCWSTR commandLine) {
    const wchar_t* begin = applicationName;
    const wchar_t* end = nullptr;
    if (begin != nullptr && *begin != L'\0') {
        end = begin + std::wcslen(begin);
    } else {
        begin = commandLine;
        if (begin == nullptr) return {};
        while (*begin == L' ' || *begin == L'\t') ++begin;
        if (*begin == L'"') {
            ++begin;
            end = std::wcschr(begin, L'"');
        } else {
            end = begin;
            while (*end != L'\0' && *end != L' ' && *end != L'\t') ++end;
        }
    }
    if (end == nullptr || end <= begin) return {};
    const wchar_t* name = end;
    while (name > begin && name[-1] != L'\\' && name[-1] != L'/') --name;
    return std::wstring(name, end);
}

inline InjectionMode ModeForExecutableName(const std::wstring& name) {
    if (_wcsicmp(name.c_str(), kTargetHtGame) == 0) return InjectionMode::Internal;
    if (_wcsicmp(name.c_str(), kTargetNteGlobalGame) == 0 ||
        _wcsicmp(name.c_str(), kTargetNteGame) == 0) {
        return InjectionMode::Shim;
    }
    return InjectionMode::None;
}

inline InjectionMode DetectMode(LPCWSTR applicationName, LPCWSTR commandLine) {
    return ModeForExecutableName(ExecutableName(applicationName, commandLine));
}

} // namespace nte::shim
