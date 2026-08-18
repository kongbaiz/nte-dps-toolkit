#include "nte/loader/Diagnostics/Logger.h"

#include <iostream>

namespace nte::loader {

void Logger::Write(LogLevel level, std::wstring_view message) {
    std::scoped_lock lock(mutex_);
    const wchar_t* prefix = L"[INFO] ";
    if (level == LogLevel::Warning) prefix = L"[WARN] ";
    if (level == LogLevel::Error) prefix = L"[ERROR] ";
    std::wcout << prefix << message << std::endl; // endl 立即 flush: 常驻模式下必须实时可见
}

} // namespace nte::loader
