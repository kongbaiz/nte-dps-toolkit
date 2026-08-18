#pragma once

#include <Windows.h>

#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace nte::loader {

struct ProcessInfo {
    DWORD pid{};
    std::wstring imageName;
};

class ProcessLocator final {
public:
    std::vector<ProcessInfo> Enumerate() const;
	std::vector<ProcessInfo> FindAll(std::wstring_view imageName) const;
	std::optional<ProcessInfo> FindFirst(std::wstring_view imageName) const;
	std::optional<std::filesystem::path> ExecutablePath(DWORD pid) const;
	DWORD LastEnumerationError() const { return lastEnumerationError_; }

private:
	mutable DWORD lastEnumerationError_ = ERROR_SUCCESS;
};

} // namespace nte::loader
