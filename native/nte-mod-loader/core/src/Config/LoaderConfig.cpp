#include "nte/loader/Config/LoaderConfig.h"

#include <Windows.h>

#include <cwchar>
#include <cerrno>
#include <limits>

namespace {

std::wstring ReadEnvironment(const wchar_t* name) {
    const DWORD required = GetEnvironmentVariableW(name, nullptr, 0);
    if (required == 0) {
        return {};
    }
    std::wstring value(required, L'\0');
    const DWORD written = GetEnvironmentVariableW(name, value.data(), required);
    if (written == 0) {
        return {};
    }
    value.resize(written);
    return value;
}

unsigned ReadUnsignedEnvironment(const wchar_t* name, unsigned fallback) {
    const auto text = ReadEnvironment(name);
    if (text.empty()) return fallback;
    wchar_t* end = nullptr;
    const auto value = std::wcstoul(text.c_str(), &end, 10);
    if (end == text.c_str() || *end != L'\0' || value > (std::numeric_limits<unsigned>::max)()) {
        return fallback;
    }
    return static_cast<unsigned>(value);
}

bool ParseUnsigned(const std::wstring& text, unsigned& value) {
	if (text.empty()) return false;
	errno = 0;
	wchar_t* end = nullptr;
	const auto parsed = std::wcstoul(text.c_str(), &end, 10);
	if (errno == ERANGE || end == text.c_str() || *end != L'\0' ||
		parsed > (std::numeric_limits<unsigned>::max)()) {
		return false;
	}
	value = static_cast<unsigned>(parsed);
	return true;
}

bool IsManagedStopEventName(const std::wstring& name) {
	constexpr const wchar_t* prefix = L"Local\\NTE-DPS-TOOL-ModLoader-";
	constexpr std::size_t prefixLength = 29;
	if (name.size() != prefixLength + 16 || name.rfind(prefix, 0) != 0) return false;
	for (std::size_t index = prefixLength; index < name.size(); ++index) {
		const wchar_t unit = name[index];
		if (!((unit >= L'0' && unit <= L'9') || (unit >= L'a' && unit <= L'f'))) {
			return false;
		}
	}
	return true;
}

} // namespace

namespace nte::loader {

LoaderConfig LoaderConfig::Parse(const std::vector<std::wstring>& arguments,
                                     const std::filesystem::path& executableDirectory) {
    LoaderConfig config;
	config.payloadDll = executableDirectory / L"plugins" / L"dwmapi.dll";
    // 默认即执行模式（--execute 已取消, 不再需要显式开关）; --dry-run 用于预览
    config.dryRun = false;
    bool dllExplicit = false;
	bool timeoutExplicit = false;

    for (std::size_t i = 0; i < arguments.size(); ++i) {
        const auto& argument = arguments[i];
        if (argument == L"--dry-run") {
            config.dryRun = true;
        } else if (argument == L"--once") {
            config.oneShot = true;
		} else if (argument == L"--monitor-timeout") {
			unsigned timeout = 0;
			if (i + 1 >= arguments.size() || !ParseUnsigned(arguments[++i], timeout)) {
				config.controlArgumentsValid = false;
			} else {
				config.monitorTimeoutSeconds = timeout;
				timeoutExplicit = true;
			}
		} else if (argument == L"--stop-event") {
			if (i + 1 >= arguments.size() ||
				!IsManagedStopEventName(arguments[++i])) {
				config.controlArgumentsValid = false;
			} else {
				config.stopEventName = arguments[i];
			}
		} else if (argument == L"--owner-pid") {
			unsigned ownerProcessId = 0;
			if (i + 1 >= arguments.size() ||
				!ParseUnsigned(arguments[++i], ownerProcessId) || ownerProcessId == 0) {
				config.controlArgumentsValid = false;
			} else {
				config.ownerProcessId = ownerProcessId;
			}
        } else if (argument == L"--dll" && i + 1 < arguments.size()) {
            // 自定义注入 DLL: --dll <完整路径>（不校验存在性, 交由
            // LoaderApp 在开始进程监控前统一校验）
            config.payloadDll = arguments[++i];
            dllExplicit = true;
        }
    }

    // 环境变量覆盖（优先级: --dll 命令行 > NTE_MOD_LOADER_DLL > 默认）
    if (!dllExplicit) {
        const auto dllOverride = ReadEnvironment(L"NTE_MOD_LOADER_DLL");
        if (!dllOverride.empty()) {
            config.payloadDll = dllOverride;
        }
    }

    const auto overridePath = ReadEnvironment(L"NTE_MOD_LOADER_LAUNCHER");
    if (!overridePath.empty()) {
        config.launcherOverride = overridePath;
    }
    config.spawnLauncher = ReadEnvironment(L"NTE_MOD_LOADER_SPAWN_LAUNCHER") == L"1";
	if (!timeoutExplicit) {
		config.monitorTimeoutSeconds =
			ReadUnsignedEnvironment(L"NTE_MOD_LOADER_MONITOR_TIMEOUT", 120);
	}
    return config;
}

} // namespace nte::loader
