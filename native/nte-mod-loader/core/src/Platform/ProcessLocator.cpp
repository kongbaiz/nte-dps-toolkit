#include "nte/loader/Platform/ProcessLocator.h"

#include <TlHelp32.h>

#include <cwchar>
#include <memory>

namespace nte::loader {

std::vector<ProcessInfo> ProcessLocator::Enumerate() const {
    using Snapshot = std::unique_ptr<void, decltype(&CloseHandle)>;
	Snapshot snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0), &CloseHandle);
	std::vector<ProcessInfo> processes;
	if (snapshot.get() == INVALID_HANDLE_VALUE) {
		lastEnumerationError_ = GetLastError();
		return processes;
	}

    PROCESSENTRY32W entry{};
    entry.dwSize = sizeof(entry);
	if (!Process32FirstW(snapshot.get(), &entry)) {
		lastEnumerationError_ = GetLastError();
		return processes;
	}
    do {
        processes.push_back({entry.th32ProcessID, entry.szExeFile});
	} while (Process32NextW(snapshot.get(), &entry));
	const DWORD enumerationEnd = GetLastError();
	lastEnumerationError_ = enumerationEnd == ERROR_NO_MORE_FILES
		? ERROR_SUCCESS
		: enumerationEnd;
	return processes;
}

std::optional<ProcessInfo> ProcessLocator::FindFirst(std::wstring_view imageName) const {
    const auto matches = FindAll(imageName);
    if (!matches.empty()) return matches.front();
    return std::nullopt;
}

std::vector<ProcessInfo> ProcessLocator::FindAll(std::wstring_view imageName) const {
    std::vector<ProcessInfo> matches;
    const std::wstring wanted(imageName);
    for (auto& process : Enumerate()) {
        if (_wcsicmp(process.imageName.c_str(), wanted.c_str()) == 0) {
            matches.push_back(process);
        }
    }
    return matches;
}

std::optional<std::filesystem::path> ProcessLocator::ExecutablePath(DWORD pid) const {
    using Handle = std::unique_ptr<void, decltype(&CloseHandle)>;
    Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid), &CloseHandle);
    if (!process) return std::nullopt;

    std::wstring path(32768, L'\0');
    DWORD length = static_cast<DWORD>(path.size());
    if (!QueryFullProcessImageNameW(process.get(), 0, path.data(), &length)) return std::nullopt;
    path.resize(length);
    return std::filesystem::path(path);
}

} // namespace nte::loader
