#include "nte/loader/Platform/LauncherLocator.h"

#include <Windows.h>

#include <array>
#include <string>

namespace {

std::optional<std::filesystem::path> ExistingExecutable(const std::filesystem::path& path) {
    std::error_code error;
    if (!path.empty() && std::filesystem::is_regular_file(path, error)) return path;
    return std::nullopt;
}

std::optional<std::filesystem::path> ReadUninstallLocation(HKEY root, REGSAM view) {
    constexpr wchar_t base[] = L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    HKEY uninstall = nullptr;
    if (RegOpenKeyExW(root, base, 0, KEY_READ | view, &uninstall) != ERROR_SUCCESS) return std::nullopt;

    DWORD index = 0;
    wchar_t child[256]{};
    DWORD childLength = static_cast<DWORD>(std::size(child));
    while (RegEnumKeyExW(uninstall, index++, child, &childLength, nullptr, nullptr, nullptr, nullptr) == ERROR_SUCCESS) {
        HKEY entry = nullptr;
        if (RegOpenKeyExW(uninstall, child, 0, KEY_READ | view, &entry) == ERROR_SUCCESS) {
            // RegGetValueW + RRF_RT_REG_SZ: 类型限定并保证 NUL 结尾,
            // 避免 RegQueryValueExW 对异常/未终止值做越界读取。
            wchar_t displayName[512]{};
            DWORD displayBytes = sizeof(displayName);
            const LSTATUS displayStatus = RegGetValueW(entry, nullptr, L"DisplayName", RRF_RT_REG_SZ,
                                                       nullptr, displayName, &displayBytes);
            if (displayStatus == ERROR_SUCCESS &&
                (wcsstr(displayName, L"Neverness To Everness") || wcsstr(displayName, L"NTE"))) {
                wchar_t location[32768]{};
                DWORD locationBytes = sizeof(location);
                const LSTATUS locationStatus = RegGetValueW(entry, nullptr, L"InstallLocation",
                                                            RRF_RT_REG_SZ, nullptr, location,
                                                            &locationBytes);
                if (locationStatus == ERROR_SUCCESS) {
                    for (const auto* name : {L"NTEGlobalLauncher.exe", L"NTELauncher.exe"}) {
                        if (auto found = ExistingExecutable(std::filesystem::path(location) / name)) {
                            RegCloseKey(entry);
                            RegCloseKey(uninstall);
                            return found;
                        }
                    }
                }
                // 名称匹配但路径无效/无启动器: 继续枚举其余卸载项, 不提前返回
            }
            RegCloseKey(entry);
        }
        childLength = static_cast<DWORD>(std::size(child));
    }
    RegCloseKey(uninstall);
    return std::nullopt;
}

} // namespace

namespace nte::loader {

std::optional<std::filesystem::path> LauncherLocator::Locate(
    const std::filesystem::path& overridePath) const {
    if (auto found = ExistingExecutable(overridePath)) return found;

	// 运行中进程的路径不能作为信任根：任意程序都能使用相同文件名。
	// 仅信任用户显式 override、已知安装根或 HKLM 卸载信息。
    constexpr std::array roots{
        L"C:\\Program Files\\Neverness To Everness",
        L"C:\\Program Files (x86)\\Neverness To Everness",
        L"D:\\Program Files\\Neverness To Everness",
        L"D:\\Neverness To Everness",
        L"E:\\Neverness To Everness",
    };
    for (const auto* root : roots) {
        for (const auto* image : {L"NTEGlobalLauncher.exe", L"NTELauncher.exe"}) {
            if (auto found = ExistingExecutable(std::filesystem::path(root) / image)) return found;
        }
    }

    if (auto found = ReadUninstallLocation(HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY)) return found;
    return ReadUninstallLocation(HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY);
}

} // namespace nte::loader
