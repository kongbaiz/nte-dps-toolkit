#pragma once

#include <cstddef>
#include <cstdint>

// Called by the assembly forwarding thunks on their first invocation. DllMain
// remains minimal; callers must still avoid invoking forwarded exports from a
// DllMain/TLS callback because Windows has no documented loader-lock query.
extern "C" uintptr_t ResolveDwmapiExport(size_t index);
