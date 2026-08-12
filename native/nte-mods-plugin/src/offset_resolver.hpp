#pragma once

#include <cstddef>
#include <cstdint>

namespace nte::mods::offsets
{
	enum class ResolutionSource : uint8_t
	{
		None,
		Signature,
		KnownProfile,
	};

	struct ResolvedOffsets
	{
		uintptr_t append_name_address;
		uintptr_t gworld_address;
		size_t image_size;
		uint32_t image_checksum;
		size_t viewport_tick_index;
		size_t process_event_index;
		ResolutionSource source;
	};

	bool Initialize();
	const ResolvedOffsets* Get();
	bool IsKnownImageProfile(size_t image_size, uint32_t image_checksum);

	namespace detail
	{
		struct SectionView
		{
			const uint8_t* bytes;
			size_t size;
			uintptr_t virtual_address;
			bool executable;
			bool writable;
		};

		bool ResolveInSections(
			const SectionView* sections,
			size_t section_count,
			uintptr_t image_base,
			size_t image_size,
			ResolvedOffsets& result);
	} // namespace detail
} // namespace nte::mods::offsets
