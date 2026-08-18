// ManualMapLoader.h — manual map 注入器（替代原 Early-Bird APC LoadLibraryW）。
#pragma once

#include <Windows.h>

#include <cstdint>
#include <vector>

namespace nte::shim {

struct ShimInitParams;

// 把 dllBytes 手工映射进 childProcess（Simple-Manual-Map-Injector 语义:
// 重定位/导入/TLS/x64 SEH, 清 PE 头与 .rsrc/.reloc 等）。initParams 非空时
// 在目标进程分配并写入, 作为被映射 DLL 的 DllMain lpReserved（shim 传播时
// 用它传递 payload 路径）。返回 true 表示映射完成且 DllMain 已执行。
bool InjectByManualMap(HANDLE childProcess, const std::vector<std::uint8_t>& dllBytes,
                       const ShimInitParams* initParams = nullptr);

} // namespace nte::shim
