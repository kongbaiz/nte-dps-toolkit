#pragma once

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
} // namespace nte::hook
