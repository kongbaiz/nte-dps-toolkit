#include "nte/loader/Injection/ShimPayload.h"

#include <Windows.h>
#include <guiddef.h>
#include <ntsecapi.h>

#include "injector.h" // Simple-Manual-Map-Injector (MIT, vendored at third_party/manualmap)

#include <algorithm>
#include <array>
#include <cstdint>
#include <cstdio>
#include <fstream>
#include <limits>
#include <memory>

namespace {

using UniqueHandle = std::unique_ptr<void, decltype(&CloseHandle)>;

// RAII 失败保护: guard 自己持有进程句柄, 任何未 Disarm 的
// 退出路径都会 TerminateProcess 并关闭句柄。
class SuspendedProcessGuard {
public:
    explicit SuspendedProcessGuard(HANDLE process) : process_(process) {}
    ~SuspendedProcessGuard() {
        if (process_ != nullptr) {
            if (armed_) TerminateProcess(process_, 1);
            CloseHandle(process_);
        }
    }
    void Disarm() { armed_ = false; }

private:
    HANDLE process_ = nullptr;
    bool armed_ = true;
};

// 把 initParams 写入目标进程, 返回远程指针（调用方负责 VirtualFreeEx）。
void* WriteRemoteInitParams(HANDLE process, const nte::loader::ShimInitParams& params) {
    void* remote = VirtualAllocEx(process, nullptr, sizeof(nte::loader::ShimInitParams),
                                  MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    if (!remote) return nullptr;
    SIZE_T written = 0;
    if (!WriteProcessMemory(process, remote, &params, sizeof(params), &written) ||
        written != sizeof(params)) {
        VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        return nullptr;
    }
    return remote;
}

// 用 16 字节系统随机数（RtlGenRandom, advapi32）生成文件名后缀,
// 并通过 CREATE_NEW 原子排他创建; 碰撞时重新生成。
std::optional<std::filesystem::path> WriteResource101Temp(const void* data, DWORD size) {
    wchar_t directory[32768]{};
    const DWORD length = GetTempPathW(static_cast<DWORD>(std::size(directory)), directory);
    if (!length || length >= std::size(directory)) return std::nullopt;

    for (int attempt = 0; attempt < 32; ++attempt) {
        GUID guid{};
        if (RtlGenRandom(&guid, sizeof(guid)) == FALSE) return std::nullopt;
        wchar_t name[128]{};
        swprintf_s(name, L"nte_shim_%08lX%04X%04X%02X%02X%02X%02X%02X%02X%02X%02X.dll",
                   guid.Data1, guid.Data2, guid.Data3,
                   guid.Data4[0], guid.Data4[1], guid.Data4[2], guid.Data4[3],
                   guid.Data4[4], guid.Data4[5], guid.Data4[6], guid.Data4[7]);
        const std::filesystem::path destination = std::filesystem::path(directory) / name;
        if (destination.wstring().size() >= nte::loader::kShimPathCapacity) continue; // 理论不会发生
        // CREATE_NEW: 文件已存在即失败, 绝不覆盖既有文件/重解析点
        HANDLE raw = CreateFileW(destination.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                                 FILE_ATTRIBUTE_NORMAL, nullptr);
        if (raw == INVALID_HANDLE_VALUE) {
            const DWORD error = GetLastError();
            if (error == ERROR_FILE_EXISTS || error == ERROR_ALREADY_EXISTS) continue;
            return std::nullopt;
        }
        UniqueHandle file(raw, &CloseHandle);
        const char* bytes = static_cast<const char*>(data);
        DWORD remaining = size;
        bool ok = true;
        while (remaining > 0 && ok) {
            const DWORD chunk = (std::min)(remaining, static_cast<DWORD>(4096));
            DWORD written = 0;
            ok = WriteFile(file.get(), bytes, chunk, &written, nullptr) && written == chunk;
            bytes += written;
            remaining -= written;
        }
        if (!ok) {
            std::error_code ec;
            std::filesystem::remove(destination, ec);
            return std::nullopt;
        }
        return destination;
    }
    return std::nullopt;
}

} // namespace

namespace nte::loader {

std::optional<std::vector<std::uint8_t>> ShimPayload::ReadFileBytes(const std::filesystem::path& path) {
	std::error_code error;
	const auto size = std::filesystem::file_size(path, error);
	if (error || size == 0 || size > kMaxPayloadDllBytes ||
		size > static_cast<std::uintmax_t>((std::numeric_limits<std::size_t>::max)())) {
		return std::nullopt;
	}
	std::ifstream stream(path, std::ios::binary);
	if (!stream) return std::nullopt;
	std::vector<std::uint8_t> bytes(static_cast<std::size_t>(size));
	if (!stream.read(reinterpret_cast<char*>(bytes.data()),
		static_cast<std::streamsize>(bytes.size()))) {
		return std::nullopt;
	}
	return bytes;
}

std::optional<std::filesystem::path> ShimPayload::ExtractResource101() {
    HMODULE module = GetModuleHandleW(nullptr);
    HRSRC resourceInfo = FindResourceW(module, MAKEINTRESOURCEW(101), RT_RCDATA);
    if (!resourceInfo) return std::nullopt;
    HGLOBAL resource = LoadResource(module, resourceInfo);
    const DWORD size = SizeofResource(module, resourceInfo);
    const void* data = resource ? LockResource(resource) : nullptr;
    if (!data || !size) return std::nullopt;
    return WriteResource101Temp(data, size);
}

InjectionResult ShimPayload::InjectWithManualMap(
	DWORD pid, const std::vector<std::uint8_t>& dllBytes,
	const ShimInitParams* initParams) {
    UniqueHandle process(OpenProcess(0x43A, FALSE, pid), &CloseHandle);
    if (!process) return InjectionResult::Failed;

    void* remoteParams = nullptr;
    if (initParams != nullptr) {
        remoteParams = WriteRemoteInitParams(process.get(), *initParams);
        if (!remoteParams) return InjectionResult::Failed;
    }

	const ManualMapResult result = ManualMapDll(
		process.get(), const_cast<BYTE*>(dllBytes.data()), dllBytes.size(),
        /*ClearHeader*/ true, /*ClearNonNeededSections*/ true, /*AdjustProtections*/ true,
        /*SEHExceptionSupport*/ true, DLL_PROCESS_ATTACH, remoteParams);

	if (remoteParams && result != ManualMapResult::TimedOut)
		VirtualFreeEx(process.get(), remoteParams, 0, MEM_RELEASE);
	switch (result) {
	case ManualMapResult::Success:
		return InjectionResult::Success;
	case ManualMapResult::TimedOut:
		return InjectionResult::TimedOut;
	default:
		return InjectionResult::Failed;
	}
}

bool ShimPayload::SpawnWithManualMap(const std::filesystem::path& launcherPath,
                                     const std::vector<std::uint8_t>& shimBytes,
                                     const ShimInitParams& initParams,
                                     DWORD* outPid) {
    if (outPid) *outPid = 0;

    STARTUPINFOW startup{};
    startup.cb = sizeof(startup);
    PROCESS_INFORMATION info{};
    if (!CreateProcessW(launcherPath.c_str(), nullptr, nullptr, nullptr, FALSE,
                        CREATE_SUSPENDED, nullptr, nullptr, &startup, &info)) {
        return false;
    }
    // 失败保护: guard 独占进程句柄, 任何失败路径都会终止挂起的子进程
    SuspendedProcessGuard guard(info.hProcess);
    UniqueHandle thread(info.hThread, &CloseHandle);

    void* remoteParams = WriteRemoteInitParams(info.hProcess, initParams);
    if (!remoteParams) return false;

	const ManualMapResult result = ManualMapDll(
        info.hProcess, const_cast<BYTE*>(shimBytes.data()), shimBytes.size(),
        /*ClearHeader*/ true, /*ClearNonNeededSections*/ true, /*AdjustProtections*/ true,
        /*SEHExceptionSupport*/ true, DLL_PROCESS_ATTACH, remoteParams);
	if (result != ManualMapResult::TimedOut)
		VirtualFreeEx(info.hProcess, remoteParams, 0, MEM_RELEASE);
	if (result != ManualMapResult::Success) return false;

    if (ResumeThread(thread.get()) == static_cast<DWORD>(-1)) {
        return false;
    }
    guard.Disarm();
    if (outPid) *outPid = info.dwProcessId;
    return true;
}

bool ShimPayload::WaitForShimReady(DWORD pid, std::uint64_t sessionNonce, DWORD timeoutMs) {
    wchar_t markerPath[MAX_PATH]{};
    const DWORD len = GetTempPathW(static_cast<DWORD>(std::size(markerPath)), markerPath);
    if (!len || len >= std::size(markerPath)) return false;
    wchar_t name[64]{};
	swprintf_s(name, kShimReadyMarkerFormat, pid,
		static_cast<unsigned long long>(sessionNonce));
    wcscat_s(markerPath, name);
    const std::filesystem::path marker(markerPath);
    const ULONGLONG deadline = GetTickCount64() + timeoutMs;
    for (;;) {
        std::error_code ec;
        if (std::filesystem::exists(marker, ec)) {
            // 一次性确认: 移除标记, 避免 PID 复用时的假阳性
            std::filesystem::remove(marker, ec);
            return true;
        }
        if (timeoutMs == 0 || GetTickCount64() >= deadline) return false;
        Sleep(50);
    }
}

bool ShimPayload::WaitForInternalDllLoaded(DWORD pid, std::uint64_t sessionNonce,
	DWORD timeoutMs) {
    wchar_t markerPath[MAX_PATH]{};
    const DWORD len = GetTempPathW(static_cast<DWORD>(std::size(markerPath)), markerPath);
    if (!len || len >= std::size(markerPath)) return false;
    wchar_t name[64]{};
	swprintf_s(name, kInternalLoadedMarkerFormat, pid,
		static_cast<unsigned long long>(sessionNonce));
    wcscat_s(markerPath, name);
    const std::filesystem::path marker(markerPath);
    const ULONGLONG deadline = GetTickCount64() + timeoutMs;
    for (;;) {
        std::error_code ec;
        if (std::filesystem::exists(marker, ec)) {
            std::filesystem::remove(marker, ec);
            return true;
        }
        if (timeoutMs == 0 || GetTickCount64() >= deadline) return false;
        Sleep(50);
    }
}

bool ShimPayload::ValidateInternalDll(const std::filesystem::path& path) {
	std::error_code error;
	if (!std::filesystem::is_regular_file(path, error)) return false;
	const auto bytes = ReadFileBytes(path);
	return bytes.has_value() &&
		ValidateManualMapImage(bytes->data(), bytes->size());
}

void ShimPayload::RemoveShimFile(const std::filesystem::path& shimPath) {
    // manual map 后文件从未被映射, 直接删除即可。
    std::error_code ignored;
    std::filesystem::remove(shimPath, ignored);
    // 历史 sidecar 残留（旧协议）, 一并清理
    std::filesystem::remove(std::filesystem::path(shimPath.wstring() + L".path.txt"), ignored);
}

} // namespace nte::loader
