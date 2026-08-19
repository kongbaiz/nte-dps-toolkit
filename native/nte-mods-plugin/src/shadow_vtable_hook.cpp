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

bool TryReadPointer(void* const* address, void*& result)
{
    __try
    {
        result = *address;
        return true;
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        result = nullptr;
        return false;
    }
}

bool TryCompareExchangePointer(
    PVOID volatile* slot,
    void* exchange,
    void* comparand,
    void*& observed)
{
    __try
    {
        observed = InterlockedCompareExchangePointer(slot, exchange, comparand);
        return true;
    }
    __except (EXCEPTION_EXECUTE_HANDLER)
    {
        observed = nullptr;
        return false;
    }
}

bool QueryExecutableRange(
    const void* address,
    bool allow_private,
    MemoryRange& range)
{
    MEMORY_BASIC_INFORMATION memory{};
    if (address == nullptr ||
        VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
        memory.State != MEM_COMMIT ||
        (memory.Type != MEM_IMAGE &&
         !(allow_private && memory.Type == MEM_PRIVATE)) ||
        (memory.Protect & PAGE_GUARD) != 0)
        return false;

    // manual-map 的 detour 代码页是 MEM_PRIVATE；游戏原始 vtable
    // 条目仍必须位于 MEM_IMAGE，不得因适配 detour 而放宽原函数边界。
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
    return QueryExecutableRange(address, true, range);
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

        void* function = nullptr;
        if (!TryReadPointer(entry, function))
            break;
        if (!executable_range.Contains(function, 1) &&
            !QueryExecutableRange(function, false, executable_range))
            break;

        ++count;
    }
    return count == maximum ? 0 : count;
}
} // namespace

bool ShadowVTableHook::Install(
    void* object,
    size_t index,
    void* detour,
    void** expected_vtable,
    void* expected_original)
{
    if (object == nullptr || detour == nullptr ||
        expected_vtable == nullptr || expected_original == nullptr ||
        index >= MAX_VTABLE_ENTRIES ||
        retired_allocation_count_ >= MAX_RETIRED_ALLOCATIONS ||
        InterlockedCompareExchange(&installed_, 0, 0) != 0 ||
        !IsReadableRange(object, sizeof(void*)) || !IsWritableAddress(object) ||
        !IsExecutableAddress(detour))
        return false;

    void** original_vtable = expected_vtable;
    if (!IsReadableRange(original_vtable, (index + 1) * sizeof(void*)))
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
    void* complete_object_locator = nullptr;
    if (IsReadableRange(original_vtable - 1, sizeof(void*)))
        TryReadPointer(original_vtable - 1, complete_object_locator);
    allocation[0] = complete_object_locator;
    auto** shadow_vtable = allocation + prefix_entries;
    for (size_t entry = 0; entry < entry_count; ++entry)
    {
        if (!TryReadPointer(original_vtable + entry, shadow_vtable[entry]))
        {
            VirtualFree(allocation, 0, MEM_RELEASE);
            return false;
        }
    }
    const void* original_function = shadow_vtable[index];
    if (original_function != expected_original)
    {
        VirtualFree(allocation, 0, MEM_RELEASE);
        return false;
    }
    shadow_vtable[index] = detour;

    DWORD old_protection = 0;
    if (!VirtualProtect(
            allocation, allocation_size, PAGE_READONLY, &old_protection))
    {
        VirtualFree(allocation, 0, MEM_RELEASE);
        return false;
    }

    auto* object_vtable_slot = reinterpret_cast<PVOID volatile*>(object);
    void* observed_vtable = nullptr;
    if (!TryCompareExchangePointer(
            object_vtable_slot,
            shadow_vtable,
            original_vtable,
            observed_vtable) ||
        observed_vtable != original_vtable)
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
        void* observed_vtable = nullptr;
        TryCompareExchangePointer(
            object_vtable_slot_,
            original_vtable_,
            shadow_vtable_,
            observed_vtable);
    }

    // A CPU may have read the published shadow vptr and be descheduled before
    // it loads the target entry. Restoring the object's vptr therefore does
    // not prove dispatch quiescence. Published tables are deliberately retired
    // for the remaining process lifetime; install failures above may still be
    // freed because those allocations were never published.
    if (shadow_allocation_ != nullptr)
        ++retired_allocation_count_;

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

bool ShadowVTableHook::CanInstall() const noexcept
{
    return !IsInstalled() &&
           retired_allocation_count_ < MAX_RETIRED_ALLOCATIONS;
}
} // namespace nte::hook
