// ManualMapLoader.cpp — manual map 注入器。
//
// 使用 Simple-Manual-Map-Injector（third_party/manualmap, MIT）的 ManualMapDll:
// 直接读 DLL raw 字节 → 在目标进程分配映像 → 重定位 → 解析导入（远程
// LoadLibraryA）→ TLS → x64 SEH(RtlAddFunctionTable) → 清 PE 头。
// manual map 后文件从未被映射, 可立即删除（loader 侧 RemoveShimFile）。
#include "shim/ManualMapLoader.h"

#include "shim/ShimGlobals.h"

#include "injector.h" // Simple-Manual-Map-Injector (MIT, vendored at third_party/manualmap)

#include <cwchar>
#include <memory>

namespace nte::shim {

namespace {

using UniqueHandle = std::unique_ptr<void, decltype(&CloseHandle)>;

void* WriteRemoteBytes(HANDLE process, const void* data, SIZE_T size) {
    void* remote = VirtualAllocEx(process, nullptr, size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
    if (!remote) return nullptr;
    SIZE_T written = 0;
    if (!WriteProcessMemory(process, remote, data, size, &written) || written != size) {
        VirtualFreeEx(process, remote, 0, MEM_RELEASE);
        return nullptr;
    }
    return remote;
}

} // namespace

bool InjectByManualMap(HANDLE childProcess, const std::vector<std::uint8_t>& dllBytes,
                       const ShimInitParams* initParams) {
    if (childProcess == nullptr || dllBytes.empty()) return false;

    void* remoteParams = nullptr;
    if (initParams != nullptr) {
        remoteParams = WriteRemoteBytes(childProcess, initParams, sizeof(ShimInitParams));
        if (!remoteParams) return false;
    }

	const ManualMapResult result = ManualMapDll(
        childProcess, const_cast<BYTE*>(dllBytes.data()), dllBytes.size(),
        /*ClearHeader*/ true, /*ClearNonNeededSections*/ true, /*AdjustProtections*/ true,
        /*SEHExceptionSupport*/ true, DLL_PROCESS_ATTACH, remoteParams);

	if (remoteParams && result != ManualMapResult::TimedOut)
		VirtualFreeEx(childProcess, remoteParams, 0, MEM_RELEASE);
	return result == ManualMapResult::Success;
}

} // namespace nte::shim
