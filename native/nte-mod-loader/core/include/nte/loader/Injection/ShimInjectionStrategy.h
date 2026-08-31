#pragma once

#include "nte/loader/Config/LoaderConfig.h"

#include <Windows.h>

#include <set>
#include <string_view>

namespace nte::loader {

class Logger;
class ProcessLocator;

struct StrategyContext {
    const LoaderConfig& config;
    Logger& logger;
    const ProcessLocator& processes;
};

class ShimInjectionStrategy {
public:
    int Execute(const StrategyContext& context);

    static std::wstring TemporaryFilePattern();
    static bool ShouldTerminateInjectedLauncher(
        DWORD pid, std::wstring_view imageName, const std::set<DWORD>& injectedPids);
};

} // namespace nte::loader
