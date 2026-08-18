#pragma once

#include "nte/loader/Injection/IInjectionStrategy.h"

#include <Windows.h>

#include <set>
#include <string_view>

namespace nte::loader {

class ShimInjectionStrategy final : public IInjectionStrategy {
public:
    int Execute(const StrategyContext& context) override;

    static std::wstring TemporaryFilePattern();
    static bool ShouldTerminateInjectedLauncher(
        DWORD pid, std::wstring_view imageName, const std::set<DWORD>& injectedPids);
};

} // namespace nte::loader
