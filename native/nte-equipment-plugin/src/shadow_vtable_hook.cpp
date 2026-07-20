#include "shadow_vtable_hook.hpp"

#include <cstdint>

namespace nte::hook
{
namespace
{
struct MemoryRange
{
    uintptr_t begin{};
    uintptr_t end{};

    bool Contains(const void* address, size_t size) const
    {
        if (address == nullptr || size == 0)
            return false;

        const uintptr_t start = reinterpret_cast<uintptr_t>(address);
        return start >= begin && start <= end && size <= end - start;
    }
};

bool QueryReadableRange(const void* address, MemoryRange& range)
{
    if (address == nullptr)
        return false;

    MEMORY_BASIC_INFORMATION memory{};
    if (VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
        memory.State != MEM_COMMIT ||
        (memory.Protect & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
        return false;

    const uintptr_t region_start = reinterpret_cast<uintptr_t>(memory.BaseAddress);
    if (memory.RegionSize > UINTPTR_MAX - region_start)
        return false;

    const uintptr_t region_end = region_start + memory.RegionSize;
    range = {region_start, region_end};
    return range.Contains(address, 1);
}

bool IsReadableRange(const void* address, size_t size)
{
    MemoryRange range{};
    return QueryReadableRange(address, range) && range.Contains(address, size);
}

bool IsWritableAddress(const void* address)
{
    MEMORY_BASIC_INFORMATION memory{};
    if (address == nullptr ||
        VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
        memory.State != MEM_COMMIT ||
        (memory.Protect & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
        return false;

    switch (memory.Protect & 0xFF)
    {
    case PAGE_READWRITE:
    case PAGE_WRITECOPY:
    case PAGE_EXECUTE_READWRITE:
    case PAGE_EXECUTE_WRITECOPY:
        return true;
    default:
        return false;
    }
}

bool QueryExecutableImageRange(const void* address, MemoryRange& range)
{
    MEMORY_BASIC_INFORMATION memory{};
    if (address == nullptr ||
        VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
        memory.State != MEM_COMMIT || memory.Type != MEM_IMAGE ||
        (memory.Protect & PAGE_GUARD) != 0)
        return false;

    const DWORD protection = memory.Protect & 0xFF;
    if (protection != PAGE_EXECUTE && protection != PAGE_EXECUTE_READ &&
        protection != PAGE_EXECUTE_READWRITE &&
        protection != PAGE_EXECUTE_WRITECOPY)
        return false;

    const uintptr_t region_start =
        reinterpret_cast<uintptr_t>(memory.BaseAddress);
    if (memory.RegionSize > UINTPTR_MAX - region_start)
        return false;

    range = {region_start, region_start + memory.RegionSize};
    return range.Contains(address, 1);
}

bool IsExecutableAddress(const void* address)
{
    MemoryRange range{};
    return QueryExecutableImageRange(address, range);
}

size_t CountVTableEntries(void** vtable, size_t maximum)
{
    MemoryRange vtable_range{};
    MemoryRange executable_range{};
    size_t count = 0;
    while (count < maximum)
    {
        auto** entry = vtable + count;
        if ((!vtable_range.Contains(entry, sizeof(*entry)) &&
             !QueryReadableRange(entry, vtable_range)) ||
            !vtable_range.Contains(entry, sizeof(*entry)))
            break;

        void* function = *entry;
        if (!executable_range.Contains(function, 1) &&
            !QueryExecutableImageRange(function, executable_range))
            break;

        ++count;
    }
    return count == maximum ? 0 : count;
}
} // namespace

bool ShadowVTableHook::Install(void* object, size_t index, void* detour)
{
    if (object == nullptr || detour == nullptr ||
        index >= MAX_VTABLE_ENTRIES ||
        InterlockedCompareExchange(&installed_, 0, 0) != 0 ||
        !IsReadableRange(object, sizeof(void*)) || !IsWritableAddress(object) ||
        !IsExecutableAddress(detour))
        return false;

    auto** original_vtable = *reinterpret_cast<void***>(object);
    if (original_vtable == nullptr ||
        !IsReadableRange(original_vtable, (index + 1) * sizeof(void*)))
        return false;

    const size_t entry_count =
        CountVTableEntries(original_vtable, MAX_VTABLE_ENTRIES);
    if (entry_count <= index)
        return false;

    constexpr size_t prefix_entries = 1;
    const size_t allocation_entries = prefix_entries + entry_count;
    if (allocation_entries > SIZE_MAX / sizeof(void*))
        return false;

    const size_t allocation_size = allocation_entries * sizeof(void*);
    auto** allocation = static_cast<void**>(VirtualAlloc(
        nullptr,
        allocation_size,
        MEM_RESERVE | MEM_COMMIT,
        PAGE_READWRITE));
    if (allocation == nullptr)
        return false;

    // Preserve the MSVC complete-object-locator slot immediately before the
    // address point. Builds without RTTI still retain the preceding value.
    allocation[0] = IsReadableRange(original_vtable - 1, sizeof(void*))
                        ? original_vtable[-1]
                        : nullptr;
    auto** shadow_vtable = allocation + prefix_entries;
    for (size_t entry = 0; entry < entry_count; ++entry)
        shadow_vtable[entry] = original_vtable[entry];
    const void* original_function = shadow_vtable[index];
    shadow_vtable[index] = detour;

    DWORD old_protection = 0;
    if (!VirtualProtect(
            allocation, allocation_size, PAGE_READONLY, &old_protection))
    {
        VirtualFree(allocation, 0, MEM_RELEASE);
        return false;
    }

    auto* object_vtable_slot = reinterpret_cast<PVOID volatile*>(object);
    if (InterlockedCompareExchangePointer(
            object_vtable_slot, shadow_vtable, original_vtable) !=
        original_vtable)
    {
        VirtualFree(allocation, 0, MEM_RELEASE);
        return false;
    }

    object_vtable_slot_ = object_vtable_slot;
    original_vtable_ = original_vtable;
    shadow_allocation_ = allocation;
    shadow_vtable_ = shadow_vtable;
    original_function_ = const_cast<void*>(original_function);
    entry_count_ = entry_count;
    InterlockedExchange(&installed_, 1);
    return true;
}

void ShadowVTableHook::Remove() noexcept
{
    InterlockedExchange(&installed_, 0);

    if (object_vtable_slot_ != nullptr && shadow_vtable_ != nullptr &&
        original_vtable_ != nullptr &&
        IsReadableRange(
            const_cast<PVOID*>(object_vtable_slot_), sizeof(void*)) &&
        IsWritableAddress(const_cast<PVOID*>(object_vtable_slot_)))
    {
        InterlockedCompareExchangePointer(
            object_vtable_slot_, original_vtable_, shadow_vtable_);
    }

    if (shadow_allocation_ != nullptr)
        VirtualFree(shadow_allocation_, 0, MEM_RELEASE);

    object_vtable_slot_ = nullptr;
    original_vtable_ = nullptr;
    shadow_allocation_ = nullptr;
    shadow_vtable_ = nullptr;
    original_function_ = nullptr;
    entry_count_ = 0;
}

void* ShadowVTableHook::OriginalFunction() const noexcept
{
    return original_function_;
}

bool ShadowVTableHook::IsInstalled() const noexcept
{
    return InterlockedCompareExchange(
        const_cast<volatile LONG*>(&installed_), 0, 0) != 0;
}
} // namespace nte::hook
