#pragma once

#include <cstddef>
#include <cstdint>

// Called by the assembly forwarding thunks on their first invocation. The real
// module is already loader-bound through an API-set anchor, so this path never
// loads a DLL, starts a worker, or pins this proxy. A third-party DllMain/TLS
// callback can therefore resolve and forward without entering loader work.
extern "C" uintptr_t ResolveDwmapiExport(size_t index);
