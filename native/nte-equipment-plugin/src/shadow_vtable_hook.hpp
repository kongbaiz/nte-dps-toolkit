#pragma once

#include <Windows.h>

#include <cstddef>

namespace nte::hook
{
// Replaces one virtual function for one object instance. The original vtable
// and every executable image section remain unchanged.
class ShadowVTableHook
{
public:
    constexpr ShadowVTableHook() noexcept = default;
    ShadowVTableHook(const ShadowVTableHook&) = delete;
    ShadowVTableHook& operator=(const ShadowVTableHook&) = delete;
    bool Install(void* object, size_t index, void* detour);
    void Remove() noexcept;

    void* OriginalFunction() const noexcept;
    bool IsInstalled() const noexcept;

private:
    static constexpr size_t MAX_VTABLE_ENTRIES = 4096;

    PVOID volatile* object_vtable_slot_{};
    void** original_vtable_{};
    void** shadow_allocation_{};
    void** shadow_vtable_{};
    void* original_function_{};
    size_t entry_count_{};
    volatile LONG installed_{};
};
} // namespace nte::hook
