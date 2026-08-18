#pragma once

#include "nte/loader/Config/LoaderConfig.h"

namespace nte::loader {

class LoaderApp final {
public:
    int Run(const LoaderConfig& config);
};

} // namespace nte::loader
