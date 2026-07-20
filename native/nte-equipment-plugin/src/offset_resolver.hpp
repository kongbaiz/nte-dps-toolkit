#pragma once

#include <cstddef>
#include <cstdint>

namespace nte::equipment::offsets
{
	struct ResolvedOffsets
	{
		uintptr_t append_name_address;
		uintptr_t gworld_address;
	};

	bool Initialize();
	const ResolvedOffsets* Get();

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
} // namespace nte::equipment::offsets
