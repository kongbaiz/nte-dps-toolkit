#pragma once

#include "nte/loader/Config/LoaderConfig.h"

namespace nte::loader {

class Logger;
class ProcessLocator;

struct StrategyContext {
    const LoaderConfig& config;
    Logger& logger;
    const ProcessLocator& processes;
};

class IInjectionStrategy {
public:
    virtual ~IInjectionStrategy() = default;
    virtual int Execute(const StrategyContext& context) = 0;
};

} // namespace nte::loader
