#include "nte/loader/Injection/ShimInjectionStrategy.h"

#include "nte/loader/Diagnostics/Logger.h"
#include "nte/loader/Injection/ShimPayload.h"
#include "nte/loader/Platform/LauncherLocator.h"
#include "nte/loader/Platform/ProcessLocator.h"

#include <Windows.h>
#include <ntsecapi.h>

#include <atomic>
#include <filesystem>
#include <map>
#include <memory>
#include <optional>
#include <set>
#include <utility>

namespace nte::loader {

// 需要注入 shim 的启动器进程名集合（按 PID 去重, 每进程注入一次）:
//   - 国服（NTELauncher 目录）: NTEGame.exe 是常驻启动器核心, 负责拉起游戏,
//     NTELauncher.exe 只是短命引导壳（偶尔能抓到, 保留无妨）;
//   - 国际服: NTEGlobalLauncher.exe 是引导壳, NTEGlobalGame.exe 是常驻核心;
//   - NTEBrowser.exe 只是 CEF 界面, 不创建游戏进程, 无需注入。
// 监控循环与 dry-run 都使用该集合, 保证“游戏启动器打开后”能被发现并注入 shim。
inline constexpr const wchar_t* kLauncherProcessNames[] = {
    //L"NTEGlobalLauncher.exe", L"NTELauncher.exe",
	L"NTEGame.exe", L"NTEGlobalGame.exe"
};

namespace {
	std::atomic_bool stopRequested{false};

	BOOL WINAPI ConsoleControlHandler(DWORD type) {
		if (type == CTRL_C_EVENT || type == CTRL_BREAK_EVENT ||
			type == CTRL_CLOSE_EVENT || type == CTRL_SHUTDOWN_EVENT) {
			stopRequested.store(true, std::memory_order_release);
			return TRUE;
		}
		return FALSE;
	}

	class ConsoleControlRegistration {
	public:
		ConsoleControlRegistration() {
			stopRequested.store(false, std::memory_order_release);
			registered_ = SetConsoleCtrlHandler(ConsoleControlHandler, TRUE) != FALSE;
		}
		~ConsoleControlRegistration() {
			if (registered_) SetConsoleCtrlHandler(ConsoleControlHandler, FALSE);
		}
	private:
		bool registered_ = false;
	};

	class ExternalStopControl {
	public:
		explicit ExternalStopControl(const LoaderConfig& config) {
			if (!config.stopEventName.empty()) {
				stopEvent_ = OpenEventW(SYNCHRONIZE, FALSE, config.stopEventName.c_str());
				valid_ = stopEvent_ != nullptr;
			}
			if (valid_ && config.ownerProcessId != 0) {
				ownerProcess_ = OpenProcess(SYNCHRONIZE, FALSE, config.ownerProcessId);
				valid_ = ownerProcess_ != nullptr;
			}
		}
		~ExternalStopControl() {
			if (ownerProcess_) CloseHandle(ownerProcess_);
			if (stopEvent_) CloseHandle(stopEvent_);
		}
		bool valid() const { return valid_; }
		bool signaled() const {
			return (stopEvent_ && WaitForSingleObject(stopEvent_, 0) == WAIT_OBJECT_0) ||
				(ownerProcess_ && WaitForSingleObject(ownerProcess_, 0) == WAIT_OBJECT_0);
		}
	private:
		HANDLE stopEvent_ = nullptr;
		HANDLE ownerProcess_ = nullptr;
		bool valid_ = true;
	};

	class TemporaryShimFile {
	public:
		explicit TemporaryShimFile(std::filesystem::path path) : path_(std::move(path)) {}
		~TemporaryShimFile() { ShimPayload::RemoveShimFile(path_); }
		const std::filesystem::path& path() const { return path_; }
	private:
		std::filesystem::path path_;
	};

	std::uint64_t GenerateSessionNonce() {
		std::uint64_t nonce = 0;
		if (RtlGenRandom(&nonce, sizeof(nonce)) == FALSE || nonce == 0) {
			nonce = (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32) ^
				GetTickCount64();
		}
		return nonce == 0 ? 1 : nonce;
	}

	bool StopOrTimeout(ULONGLONG startedAt, unsigned timeoutSeconds,
		const ExternalStopControl& externalStop) {
		if (stopRequested.load(std::memory_order_acquire)) return true;
		if (externalStop.signaled()) return true;
		if (timeoutSeconds == 0) return false;
		return GetTickCount64() - startedAt >=
			static_cast<ULONGLONG>(timeoutSeconds) * 1000ULL;
	}

bool IsProcessAlive(DWORD pid) {
    using Handle = std::unique_ptr<void, decltype(&CloseHandle)>;
	Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid), &CloseHandle);
	if (!process) {
		// Access-denied/temporary probe failures are not proof of process exit;
		// retain the PID to avoid repeated injection attempts.
		return GetLastError() != ERROR_INVALID_PARAMETER;
	}
    DWORD code = 0;
    return GetExitCodeProcess(process.get(), &code) && code == STILL_ACTIVE;
}

// 启动器/游戏关闭后, 从记录集合移除已消失的 PID, 使下次打开时能重新注入/确认。
void PruneDeadPids(std::set<DWORD>& pids) {
    for (auto it = pids.begin(); it != pids.end();) {
        if (IsProcessAlive(*it)) {
            ++it;
        } else {
            it = pids.erase(it);
        }
    }
}

// Detect a launcher without exposing the user's full installation path.
void LogLauncherDetected(Logger& logger, const ProcessInfo& process) {
	std::wstring line = L"launcher detected: " + process.imageName +
		L" (PID " + std::to_wstring(process.pid) + L")";
	logger.Write(LogLevel::Info, line);
}

bool IsPathWithin(const std::filesystem::path& candidate,
	const std::filesystem::path& root) {
	std::error_code error;
	const auto normalizedCandidate = std::filesystem::weakly_canonical(candidate, error);
	if (error) return false;
	const auto normalizedRoot = std::filesystem::weakly_canonical(root, error);
	if (error) return false;
	auto candidateIt = normalizedCandidate.begin();
	for (auto rootIt = normalizedRoot.begin(); rootIt != normalizedRoot.end();
		++rootIt, ++candidateIt) {
		if (candidateIt == normalizedCandidate.end() ||
			_wcsicmp(rootIt->c_str(), candidateIt->c_str()) != 0) {
			return false;
		}
	}
	return true;
}

bool TerminateInjectedLauncher(Logger& logger, DWORD pid,
	const std::filesystem::path& trustedLauncherRoot,
	const std::set<DWORD>& injectedPids) {
	using Handle = std::unique_ptr<void, decltype(&CloseHandle)>;
	Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE |
		SYNCHRONIZE, FALSE, pid), &CloseHandle);
	if (!process) {
		const DWORD error = GetLastError();
		if (error == ERROR_INVALID_PARAMETER) return true;
		logger.Write(LogLevel::Error, L"could not open injected launcher PID " +
			std::to_wstring(pid) + L" for shutdown");
		return false;
	}

	std::wstring executablePath(32768, L'\0');
	DWORD pathLength = static_cast<DWORD>(executablePath.size());
	if (!QueryFullProcessImageNameW(process.get(), 0, executablePath.data(), &pathLength) ||
		pathLength == 0) {
		logger.Write(LogLevel::Error, L"could not verify injected launcher PID " +
			std::to_wstring(pid) + L" before shutdown");
		return false;
	}
	executablePath.resize(pathLength);
	const std::filesystem::path executable(executablePath);
	if (!ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
			pid, executable.filename().wstring(), injectedPids) ||
		!IsPathWithin(executable, trustedLauncherRoot)) {
		logger.Write(LogLevel::Warning, L"injected launcher PID " +
			std::to_wstring(pid) + L" no longer matches the trusted target; shutdown skipped");
		return false;
	}

	if (!TerminateProcess(process.get(), ERROR_CANCELLED)) {
		const DWORD error = GetLastError();
		if (error == ERROR_ACCESS_DENIED &&
			WaitForSingleObject(process.get(), 0) == WAIT_OBJECT_0) {
			return true;
		}
		logger.Write(LogLevel::Error, L"failed to stop injected launcher PID " +
			std::to_wstring(pid));
		return false;
	}
	if (WaitForSingleObject(process.get(), 5000) != WAIT_OBJECT_0) {
		logger.Write(LogLevel::Error, L"injected launcher PID " +
			std::to_wstring(pid) + L" did not stop in time");
		return false;
	}
	logger.Write(LogLevel::Info, L"stopped injected launcher PID " +
		std::to_wstring(pid));
	return true;
}

class InjectedLauncherCleanup final {
public:
	InjectedLauncherCleanup(Logger& logger, const std::set<DWORD>& injectedPids,
		std::optional<std::filesystem::path> trustedLauncherRoot)
		: logger_(logger), injectedPids_(injectedPids),
		  trustedLauncherRoot_(std::move(trustedLauncherRoot)) {}

	~InjectedLauncherCleanup() {
		if (!armed_ || injectedPids_.empty()) return;
		if (!trustedLauncherRoot_) {
			logger_.Write(LogLevel::Error,
				L"trusted launcher root unavailable; injected launcher shutdown skipped");
			return;
		}
		for (const DWORD pid : injectedPids_) {
			TerminateInjectedLauncher(logger_, pid, *trustedLauncherRoot_, injectedPids_);
		}
	}

	void Disarm() { armed_ = false; }

private:
	Logger& logger_;
	const std::set<DWORD>& injectedPids_;
	std::optional<std::filesystem::path> trustedLauncherRoot_;
	bool armed_ = true;
};

} // namespace

bool ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
	DWORD pid, std::wstring_view imageName, const std::set<DWORD>& injectedPids) {
	if (injectedPids.find(pid) == injectedPids.end()) return false;
	const std::wstring processName(imageName);
	for (const auto* launcherName : kLauncherProcessNames) {
		if (_wcsicmp(processName.c_str(), launcherName) == 0) return true;
	}
	return false;
}

int ShimInjectionStrategy::Execute(const StrategyContext& context) {
    // 启动时静默: 不输出 strategy 名称与 locator 结果, 等检测到启动器后再输出。

    std::set<DWORD> injected;      // 已注入 shim 的启动器 PID
    std::set<DWORD> checkedGames;  // 已检查过 payload DLL 加载的 HTGame PID
	std::map<DWORD, ULONGLONG> retryAfter; // 可清理失败按 PID 节流重试

    const LauncherLocator locator;
    const auto launcherPath = locator.Locate(context.config.launcherOverride);

    // payload 路径必须能放进 ShimInitParams（260 WCHAR）, 否则注入的 shim
    // 会截断路径导致后续 manual map payload 失败。
	if (context.config.payloadDll.wstring().size() >= kShimPathCapacity) {
		context.logger.Write(LogLevel::Error, L"payload DLL path is too long (>= 260 chars)");
        return 4;
    }

	if (context.config.dryRun) {
		for (const auto* name : kLauncherProcessNames) {
			const auto matches = context.processes.FindAll(name);
			if (context.processes.LastEnumerationError() != ERROR_SUCCESS) {
				context.logger.Write(LogLevel::Error, L"process enumeration failed");
				return 5;
			}
			for (const auto& process : matches) {
                context.logger.Write(LogLevel::Info, L"dry-run: would inject resource 101 into PID " +
                    std::to_wstring(process.pid));
            }
        }
        context.logger.Write(LogLevel::Info, L"dry-run: no target process was changed");
		return 0;
	}

	ConsoleControlRegistration consoleControl;
	ExternalStopControl externalStop(context.config);
	if (!externalStop.valid()) {
		context.logger.Write(LogLevel::Error, L"loader control channel could not be opened");
		return 6;
	}
	const ULONGLONG monitoringStartedAt = GetTickCount64();
	const std::uint64_t sessionNonce = GenerateSessionNonce();
	const auto extractedShim = ShimPayload::ExtractResource101();
	if (!extractedShim) {
		context.logger.Write(LogLevel::Error, L"resource 101 extraction failed");
		return 3;
	}
	TemporaryShimFile shimFile(*extractedShim);
	const auto shimBytes = ShimPayload::ReadFileBytes(shimFile.path());
	if (!shimBytes) {
		context.logger.Write(LogLevel::Error, L"could not read extracted shim file");
		return 4;
	}
	ShimInitParams initParams{};
	wcsncpy_s(initParams.payloadDllPath,
		context.config.payloadDll.wstring().c_str(), _TRUNCATE);
	wcsncpy_s(initParams.shimSelfPath,
		shimFile.path().wstring().c_str(), _TRUNCATE);
	initParams.sessionNonce = sessionNonce;
	InjectedLauncherCleanup injectedLauncherCleanup(
		context.logger, injected,
		launcherPath ? std::optional(launcherPath->parent_path()) : std::nullopt);

    if (context.config.spawnLauncher && launcherPath) {
        // manual map 预装: CREATE_SUSPENDED 创建官方启动器, 把 shim 手工映射
        // 进挂起进程（DllMain 以 ShimInitParams 为 lpReserved）, 再恢复主线程。
		context.logger.Write(LogLevel::Info, L"spawning official launcher");
		DWORD spawnedPid = 0;
		const bool spawned = ShimPayload::SpawnWithManualMap(*launcherPath, *shimBytes,
		                                                     initParams, &spawnedPid);

        if (!spawned) {
            context.logger.Write(LogLevel::Error, L"launcher spawn/manual-map failed");
            // 常驻: spawn 失败不退出, 继续监控手动打开的启动器
        } else {
            injected.insert(spawnedPid); // 避免监控循环二次注入
            // 握手确认: 只有 shim 确认“已加载且 hook 已启用”才记成功。
            // 常驻模式下握手失败不退出（NTELauncher.exe 这类短命引导壳可能来不及
            // 初始化）, 继续由监控循环注入常驻的 NTEGame.exe/NTEGlobalGame.exe。
			if (!ShimPayload::WaitForShimReady(spawnedPid, sessionNonce)) {
                context.logger.Write(LogLevel::Warning,
                    L"spawned launcher (PID " + std::to_wstring(spawnedPid) +
                    L") did not confirm shim initialization; keep monitoring");
            } else {
                context.logger.Write(LogLevel::Info,
                    L"shim preloaded into spawned launcher (PID " +
                    std::to_wstring(spawnedPid) + L"): hooks enabled");
            }
        }
    }

    // 常驻监控循环: 启动器进程出现就注入, 关闭后重开（新 PID）自动重新注入;
    // HTGame 确认后不退出, 继续监控下一局。
	for (;;) {
		if (StopOrTimeout(monitoringStartedAt, context.config.monitorTimeoutSeconds,
			externalStop)) {
			context.logger.Write(LogLevel::Info, L"monitoring stopped");
			return 0;
		}
        PruneDeadPids(injected);
        PruneDeadPids(checkedGames);
		for (auto it = retryAfter.begin(); it != retryAfter.end();) {
			if (IsProcessAlive(it->first)) ++it;
			else it = retryAfter.erase(it);
		}

        bool sawLauncher = false;
		for (const auto* name : kLauncherProcessNames) {
			const auto matches = context.processes.FindAll(name);
			if (context.processes.LastEnumerationError() != ERROR_SUCCESS) {
				context.logger.Write(LogLevel::Error, L"process enumeration failed");
				return 5;
			}
			for (const auto& process : matches) {
				sawLauncher = true;
				if (injected.find(process.pid) != injected.end()) continue;
				const auto retry = retryAfter.find(process.pid);
				if (retry != retryAfter.end() && GetTickCount64() < retry->second) continue;
				const auto executable = context.processes.ExecutablePath(process.pid);
				if (!executable || !launcherPath ||
					!IsPathWithin(*executable, launcherPath->parent_path())) {
					context.logger.Write(LogLevel::Warning,
						L"launcher candidate path was not trusted; skipping PID " +
						std::to_wstring(process.pid));
					continue;
				}
				// 当前会话标记存在时，说明该 PID 已确认 hook 就绪。
				if (ShimPayload::WaitForShimReady(process.pid, sessionNonce, 0)) {
					injected.insert(process.pid);
					retryAfter.erase(process.pid);
                    context.logger.Write(LogLevel::Info,
                        L"shim already active in PID " + std::to_wstring(process.pid) +
                        L"; skipping duplicate injection");
                    continue;
                }

                LogLauncherDetected(context.logger, process);

				const InjectionResult mapped = ShimPayload::InjectWithManualMap(
					process.pid, *shimBytes, &initParams);

				if (mapped == InjectionResult::Failed) {
					retryAfter[process.pid] = GetTickCount64() + 5000;
                    context.logger.Write(LogLevel::Error, L"manual map failed for " +
						process.imageName + L" (PID " + std::to_wstring(process.pid) +
						L"); retrying after backoff");
					continue;
                }
				// 超时时远程线程可能仍在使用映射参数，同一 PID 不重复注入。
				injected.insert(process.pid);
				retryAfter.erase(process.pid);
				if (mapped == InjectionResult::TimedOut) {
					context.logger.Write(LogLevel::Error,
						L"manual map timed out for " + process.imageName + L" (PID " +
						std::to_wstring(process.pid) + L"); retry suppressed");
					continue;
				}

                // 映射完成且 DllMain 已执行; 再等 hook 初始化握手
				if (ShimPayload::WaitForShimReady(process.pid, sessionNonce)) {
                    context.logger.Write(LogLevel::Info, L"shim injected into " +
                        process.imageName + L" (PID " + std::to_wstring(process.pid) +
                        L"): hooks enabled");
                } else {
                    context.logger.Write(LogLevel::Error,
                        L"shim mapped into PID " + std::to_wstring(process.pid) +
                        L" but hook initialization was not confirmed");
                }
            }
        }

        // 游戏确认: 不能只用“进程存在”代替注入成功。
        // 常驻模式下确认后不退出, 同一 PID 只确认一次; 游戏退出后 PID 被清理,
        // 下一局重新确认。shim 在 manual map 完成后立即置位事件（无模块轮询）。
		const auto games = context.processes.FindAll(L"HTGame.exe");
		if (context.processes.LastEnumerationError() != ERROR_SUCCESS) {
			context.logger.Write(LogLevel::Error, L"process enumeration failed");
			return 5;
		}
		for (const auto& game : games) {
			if (checkedGames.insert(game.pid).second) {
				if (ShimPayload::WaitForInternalDllLoaded(
					game.pid, sessionNonce, 30000)) {
                    context.logger.Write(LogLevel::Info,
						L"HTGame.exe (PID " + std::to_wstring(game.pid) + L"): " +
                        context.config.payloadDll.filename().wstring() + L" loaded");
                } else {
                    context.logger.Write(LogLevel::Warning,
						L"HTGame.exe (PID " + std::to_wstring(game.pid) +
                        L") observed but payload DLL load was not confirmed");
                }
            }
        }

        if (context.config.oneShot) {
            // --once: 完成一轮探测即返回
			injectedLauncherCleanup.Disarm();
            return (sawLauncher || !injected.empty()) ? 0 : 2;
        }
        Sleep(700);
    }
}

std::wstring ShimInjectionStrategy::TemporaryFilePattern() {
    return L"nte_shim_*.dll";
}

} // namespace nte::loader
