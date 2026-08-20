#pragma once

#include <cstdint>

namespace nte::mods::ipc
{
// Tracks the one OVERLAPPED operation owned by the transport state machine.
// Callers serialize this object together with the HANDLE, event, buffers, and
// transport state; the generation prevents an old completion from being
// accepted after a pipe has been recreated.
class OperationEpoch
{
public:
	constexpr bool Begin(uint64_t generation) noexcept
	{
		if (pending_ || generation == 0)
			return false;
		pending_ = true;
		generation_ = generation;
		return true;
	}

	constexpr bool Complete(uint64_t generation) noexcept
	{
		if (!pending_ || generation_ != generation)
			return false;
		pending_ = false;
		generation_ = 0;
		return true;
	}

	constexpr bool IsPending() const noexcept
	{
		return pending_;
	}

	constexpr bool BelongsTo(uint64_t generation) const noexcept
	{
		return pending_ && generation_ == generation;
	}

	constexpr bool CanReuse() const noexcept
	{
		return !pending_;
	}

private:
	uint64_t generation_{};
	bool pending_{};
};
} // namespace nte::mods::ipc
