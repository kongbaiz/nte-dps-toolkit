#pragma once

#include "offset_resolver.hpp"

#include <cstddef>
#include <cstdint>

namespace nte::mods::offsets::find_offsets
{
	bool ResolveCurrentProcess(
		ResolvedOffsets& result,
		wchar_t* error,
		size_t error_capacity,
		void* cancellation_event = nullptr) noexcept;

	namespace detail
	{
		bool TryRipRelativeTarget(
			const uint8_t* code,
			size_t size,
			uint64_t instruction_address,
			uint64_t& result) noexcept;

		bool ValidateResolvedOffsets(
			const ResolvedOffsets& candidate,
			uintptr_t image_base) noexcept;
	} // namespace detail
} // namespace nte::mods::offsets::find_offsets
