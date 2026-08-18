#pragma once

#include <filesystem>
#include <cstdint>
#include <string>
#include <vector>

namespace nte::loader {

struct LoaderConfig {
    bool spawnLauncher{false};
    bool dryRun{true};
    bool oneShot{false};
	bool controlArgumentsValid{true};
    unsigned monitorTimeoutSeconds{120};
	std::wstring stopEventName;
	std::uint32_t ownerProcessId{0};
	// 注入到 HTGame.exe 的自定义 DLL（默认 <exe 目录>\plugins\dwmapi.dll,
    // 可用 --dll <path> 或环境变量 NTE_MOD_LOADER_DLL 覆盖）。
    std::filesystem::path payloadDll;
    std::filesystem::path launcherOverride;

    static LoaderConfig Parse(const std::vector<std::wstring>& arguments,
                                const std::filesystem::path& executableDirectory);
};

} // namespace nte::loader
