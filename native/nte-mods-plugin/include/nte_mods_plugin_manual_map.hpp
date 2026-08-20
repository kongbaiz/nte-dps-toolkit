#pragma once

#include <cstddef>
#include <cstdint>

#define NTE_MODS_PLUGIN_IMAGE_SIGNATURE "NTE_DPS_TOOL_MODS_PLUGIN_V1"

namespace nte::mods::manual_map
{
inline constexpr uintptr_t ATTACH_SENTINEL =
    UINT64_C(0x4E54454D4D415001);
inline constexpr char IMAGE_SIGNATURE[] = NTE_MODS_PLUGIN_IMAGE_SIGNATURE;

inline void* AttachSentinel() noexcept
{
    return reinterpret_cast<void*>(ATTACH_SENTINEL);
}

inline bool IsExplicitAttach(const void* reserved) noexcept
{
    return reserved == AttachSentinel();
}

inline bool HasImageSignature(
    const std::uint8_t* bytes,
    std::size_t size) noexcept
{
    if (bytes == nullptr || size < sizeof(IMAGE_SIGNATURE))
        return false;

    for (std::size_t offset = 0;
         offset <= size - sizeof(IMAGE_SIGNATURE);
         ++offset)
    {
        std::size_t index = 0;
        while (index < sizeof(IMAGE_SIGNATURE) &&
               bytes[offset + index] ==
                   static_cast<std::uint8_t>(IMAGE_SIGNATURE[index]))
            ++index;
        if (index == sizeof(IMAGE_SIGNATURE))
            return true;
    }
    return false;
}

inline void* SelectReservedParameter(
    const std::uint8_t* bytes,
    std::size_t size,
    void* existing_reserved) noexcept
{
    // ShimInitParams and custom DLL semantics remain untouched. Only an image
    // carrying the NTE Mods Plugin signature receives the explicit marker.
    if (existing_reserved != nullptr || !HasImageSignature(bytes, size))
        return existing_reserved;
    return AttachSentinel();
}
} // namespace nte::mods::manual_map
