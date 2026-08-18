#pragma once

#include <filesystem>
#include <optional>

namespace nte::loader {

class LauncherLocator final {
public:
    std::optional<std::filesystem::path> Locate(const std::filesystem::path& overridePath) const;
};

} // namespace nte::loader
