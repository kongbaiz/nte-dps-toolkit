#pragma once

#include <cstdint>

namespace nte::hook
{
struct ViewportGenerationToken
{
    std::uint64_t value;
};

constexpr ViewportGenerationToken NextViewportGeneration(
    ViewportGenerationToken current) noexcept
{
    ++current.value;
    if (current.value == 0)
        ++current.value;
    return current;
}

struct ViewportSessionIdentity
{
    const void* world;
    const void* game_instance;
    const void* local_player;
    const void* viewport;
    std::uint32_t host_thread_id;
};

constexpr bool IsCompleteViewportSession(
    const ViewportSessionIdentity& identity) noexcept
{
    return identity.world != nullptr && identity.game_instance != nullptr &&
           identity.local_player != nullptr && identity.viewport != nullptr &&
           identity.host_thread_id != 0;
}

constexpr bool IsSameViewportSession(
    const ViewportSessionIdentity& left,
    const ViewportSessionIdentity& right) noexcept
{
    return left.world == right.world &&
           left.game_instance == right.game_instance &&
           left.local_player == right.local_player &&
           left.viewport == right.viewport &&
           left.host_thread_id == right.host_thread_id;
}

constexpr bool CanPublishViewportSnapshot(
    std::uint64_t requested_epoch,
    std::uint64_t current_epoch,
    const ViewportSessionIdentity& first,
    const ViewportSessionIdentity& confirmed) noexcept
{
    return requested_epoch != 0 && requested_epoch == current_epoch &&
           IsCompleteViewportSession(first) &&
           IsSameViewportSession(first, confirmed);
}

constexpr bool CanDispatchViewportGeneration(
    std::uint64_t binding_generation,
    std::uint64_t active_generation,
    std::uint32_t binding_thread_id,
    std::uint32_t current_thread_id,
    const void* binding_viewport,
    const void* dispatched_viewport) noexcept
{
    return binding_generation != 0 &&
           binding_generation == active_generation &&
           binding_thread_id != 0 &&
           binding_thread_id == current_thread_id &&
           binding_viewport != nullptr &&
           binding_viewport == dispatched_viewport;
}
} // namespace nte::hook
