#pragma once

#include <cstddef>

namespace nte::hook
{
constexpr bool IsConsistentViewportChain(
    const void* world,
    const void* game_instance,
    const void* viewport,
    const void* viewport_world,
    const void* viewport_game_instance) noexcept
{
    return world != nullptr && game_instance != nullptr && viewport != nullptr &&
           viewport_world == world && viewport_game_instance == game_instance;
}

constexpr bool ShouldRebindViewport(
    const void* resolved_viewport,
    const void* hooked_viewport) noexcept
{
    return resolved_viewport != nullptr && hooked_viewport != nullptr &&
           resolved_viewport != hooked_viewport;
}

constexpr bool ShouldPreferKnownViewportTick(
    bool known_image_profile,
    bool preferred_index_is_valid) noexcept
{
    return known_image_profile && preferred_index_is_valid;
}

template <typename IsSemanticCandidate>
constexpr bool SelectPreferredSemanticViewportTick(
    size_t preferred_index,
    size_t begin,
    size_t end,
    size_t max_drift,
    IsSemanticCandidate&& is_semantic_candidate,
    size_t& result) noexcept
{
    if (begin > end || preferred_index < begin || preferred_index > end)
        return false;

    // An exact semantic match at the long-lived UE vtable slot is stronger
    // than the generic body signature, which intentionally matches several
    // neighbouring viewport functions on current builds.
    if (is_semantic_candidate(preferred_index))
    {
        result = preferred_index;
        return true;
    }

    size_t best_distance = static_cast<size_t>(-1);
    size_t best_index = 0;
    bool tied = false;
    for (size_t index = begin; index <= end; ++index)
    {
        if (index == preferred_index || !is_semantic_candidate(index))
            continue;
        const size_t distance = index > preferred_index
            ? index - preferred_index
            : preferred_index - index;
        if (distance > max_drift)
            continue;
        if (distance < best_distance)
        {
            best_distance = distance;
            best_index = index;
            tied = false;
        }
        else if (distance == best_distance)
        {
            tied = true;
        }
    }
    if (best_distance == static_cast<size_t>(-1) || tied)
        return false;
    result = best_index;
    return true;
}
} // namespace nte::hook
