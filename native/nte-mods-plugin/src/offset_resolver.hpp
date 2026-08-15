#pragma once

#include <cstddef>
#include <cstdint>

namespace nte::mods::offsets
{
	enum class ResolutionSource : uint8_t
	{
		None,
		FindOffsets,
	};

	struct ResolvedOffsets
	{
		uintptr_t append_name_address;
		uintptr_t fname_pool_address;
		uintptr_t gobjects_address;
		uintptr_t gworld_address;
		uintptr_t process_event_address;
		size_t image_size;
		size_t viewport_tick_index;
		size_t process_event_index;
		ResolutionSource source;
	};

	bool Initialize(void* cancellation_event = nullptr);
	const ResolvedOffsets* Get();

	namespace detail
	{
		bool DecodeNameFromPool(
			uintptr_t fname_pool_address,
			int32_t comparison_index,
			uint32_t number,
			wchar_t* output,
			size_t output_capacity,
			size_t& output_length);
	} // namespace detail
} // namespace nte::mods::offsets
