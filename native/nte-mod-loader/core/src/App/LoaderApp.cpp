#include "nte/loader/App/LoaderApp.h"

#include "nte/loader/Diagnostics/Logger.h"
#include "nte/loader/Injection/IInjectionStrategy.h"
#include "nte/loader/Injection/ShimInjectionStrategy.h"
#include "nte/loader/Injection/ShimPayload.h"
#include "nte/loader/Platform/ProcessLocator.h"
#include "nte/loader/Platform/SecurityContext.h"

#include <Windows.h>

#include <chrono>
#include <filesystem>
#include <memory>

namespace nte::loader {

int LoaderApp::Run(const LoaderConfig& config) {
    Logger logger;
    ProcessLocator processes;
    // 启动时静默（review: 只输出这一行, 等检测到启动器/游戏后再输出细节）
    logger.Write(LogLevel::Info, L"NTE Mod Loader started");
	if (!config.controlArgumentsValid) {
		logger.Write(LogLevel::Error, L"invalid loader control arguments");
		return 2;
	}

    if (!config.dryRun) {
        // 先完成权限与配置校验, 再执行任何进程/文件变更
		const auto administrator = SecurityContext::Administrator();
		if (administrator == AdministratorStatus::ProbeFailed) {
			logger.Write(LogLevel::Error, L"Administrator status probe failed.");
			return 1;
		}
		if (administrator == AdministratorStatus::NotAdministrator) {
            logger.Write(LogLevel::Error, L"Administrator privileges required.");
            return 1;
        }
        if (!SecurityContext::EnableDebugPrivilege()) {
            logger.Write(LogLevel::Warning, L"SeDebugPrivilege could not be enabled");
        }
		// 在启动监控前校验注入 DLL 存在且是有界、结构完整的 x64 PE32+。
		if (!ShimPayload::ValidateInternalDll(config.payloadDll)) {
			logger.Write(LogLevel::Error,
				L"payload DLL is missing or not a valid bounded x64 DLL");
            return 1;
        }

		// 只清理超过 24 小时的普通 shim 文件。直接删除所有 nte_shim_*
		// 会破坏另一个活跃 loader 的后续传播链。
        std::error_code ignored;
        const auto temp = std::filesystem::temp_directory_path(ignored);
        if (!ignored) {
			const auto cutoff = std::filesystem::file_time_type::clock::now() -
				std::chrono::hours(24);
			std::filesystem::directory_iterator current(temp, ignored), end;
			while (!ignored && current != end) {
				const auto path = current->path();
				const auto name = path.filename().wstring();
				std::error_code entryError;
				const auto status = current->symlink_status(entryError);
				const auto written = current->last_write_time(entryError);
				if (!entryError && std::filesystem::is_regular_file(status) &&
					name.rfind(L"nte_shim_", 0) == 0 && path.extension() == L".dll" &&
					written < cutoff) {
					std::filesystem::remove(path, entryError);
				}
				current.increment(ignored);
			}
		}
	} else {
		logger.Write(LogLevel::Info, L"dry-run enabled; cleanup and injection are suppressed");
    }

    std::unique_ptr<IInjectionStrategy> strategy = std::make_unique<ShimInjectionStrategy>();

    const StrategyContext context{config, logger, processes};
    const int result = strategy->Execute(context);
    logger.Write(LogLevel::Info, L"nte-mod-loader finished");
    return result;
}

} // namespace nte::loader
