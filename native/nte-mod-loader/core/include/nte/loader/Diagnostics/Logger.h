#pragma once

#include <mutex>
#include <string_view>

namespace nte::loader {

enum class LogLevel { Info, Warning, Error };

class Logger final {
public:
    void Write(LogLevel level, std::wstring_view message);

private:
    std::mutex mutex_;
};

} // namespace nte::loader
