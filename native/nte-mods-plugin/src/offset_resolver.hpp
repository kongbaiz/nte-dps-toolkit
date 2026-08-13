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
		Semantic,
	};

	struct ResolvedOffsets
	{
		uintptr_t append_name_address;
		uintptr_t fname_pool_address;
		uintptr_t gobjects_address;
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

		bool DecodeNameFromPool(
			uintptr_t fname_pool_address,
			int32_t comparison_index,
			uint32_t number,
			wchar_t* output,
			size_t output_capacity,
			size_t& output_length);

		bool SelectUniqueGWorldCandidateByCodeReferences(
			const SectionView* sections,
			size_t section_count,
			const uintptr_t* candidates,
			size_t candidate_count,
			uintptr_t& selected,
			size_t& reference_count);
	} // namespace detail
} // namespace nte::mods::offsets
