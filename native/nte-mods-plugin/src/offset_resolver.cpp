#include "offset_resolver.hpp"

#include "memory_access.hpp"
#include "offset_signatures.hpp"
#include "signature_policy.hpp"

#include <Windows.h>

#include <cstddef>
#include <cstdint>

namespace nte::mods::offsets
{
	namespace
	{
		constexpr size_t MAX_IMAGE_SECTIONS = 96;
		constexpr size_t MAX_APPEND_NAME_WINDOW = 0x140;
		constexpr size_t NOT_FOUND = static_cast<size_t>(-1);
		constexpr size_t MAX_IMAGE_SIZE = 0x40000000;
		constexpr size_t DEFAULT_VIEWPORT_TICK_INDEX = 100;
		constexpr size_t DEFAULT_PROCESS_EVENT_INDEX = 0x4C;

		struct KnownOffsetProfile
		{
			size_t image_size;
			size_t append_name_offset;
			size_t gworld_offset;
			size_t viewport_tick_index;
			size_t process_event_index;
		};

		// SDK snapshots for the currently supported CN, CN test, and Global builds.
		// Image size is the stable fallback key; PE checksum is retained only in the
		// resolved diagnostics and never blocks signature/profile location.
		constexpr KnownOffsetProfile KNOWN_OFFSET_PROFILES[]{
			{ 0x1000C000, 0x0161C020, 0x0EAAADB0, 100, 0x4C },
			{ 0x1064D000, 0x0164A940, 0x0F071DB0, 100, 0x4C },
			{ 0x1000E000, 0x0161BAE0, 0x0EAAADB0, 100, 0x4C },
		};

		constexpr uint8_t APPEND_NAME_PROLOGUE_BYTES[]{
			0x48, 0x89, 0x5C, 0x24, 0x00, 0x48, 0x89, 0x74,
			0x24, 0x00, 0x57, 0x48, 0x83, 0xEC, 0x00,
		};
		constexpr uint8_t APPEND_NAME_PROLOGUE_MASK[]{
			0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0xFF,
			0xFF, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00,
		};
		constexpr signature::BytePattern APPEND_NAME_PROLOGUE{
			APPEND_NAME_PROLOGUE_BYTES,
			APPEND_NAME_PROLOGUE_MASK,
			sizeof(APPEND_NAME_PROLOGUE_BYTES),
		};

		ResolvedOffsets resolved_offsets{};
		bool resolution_attempted = false;
		bool resolution_succeeded = false;

		bool IsReadableProtection(DWORD protection)
		{
			if ((protection & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
				return false;

			switch (protection & 0xFF)
			{
			case PAGE_READONLY:
			case PAGE_READWRITE:
			case PAGE_WRITECOPY:
			case PAGE_EXECUTE_READ:
			case PAGE_EXECUTE_READWRITE:
			case PAGE_EXECUTE_WRITECOPY:
				return true;
			default:
				return false;
			}
		}

		bool IsReadableRange(const void* address, size_t size)
		{
			if (address == nullptr || size == 0)
				return false;

			const uintptr_t start = reinterpret_cast<uintptr_t>(address);
			if (size > UINTPTR_MAX - start)
				return false;
			const uintptr_t end = start + size;

			for (uintptr_t cursor = start; cursor < end;)
			{
				MEMORY_BASIC_INFORMATION region{};
				if (VirtualQuery(
					reinterpret_cast<const void*>(cursor),
					&region,
					sizeof(region)) != sizeof(region) ||
					region.State != MEM_COMMIT ||
					!IsReadableProtection(region.Protect))
					return false;

				const uintptr_t region_start =
					reinterpret_cast<uintptr_t>(region.BaseAddress);
				if (region.RegionSize > UINTPTR_MAX - region_start)
					return false;
				const uintptr_t region_end = region_start + region.RegionSize;
				if (cursor < region_start || cursor >= region_end)
					return false;
				cursor = region_end < end ? region_end : end;
			}

			return true;
		}

		template <typename Matcher>
		size_t FindInstruction(
			const uint8_t* code,
			size_t size,
			size_t begin,
			size_t end,
			size_t instruction_size,
			Matcher matches)
		{
			if (code == nullptr || begin > size)
				return NOT_FOUND;
			if (end > size)
				end = size;
			if (instruction_size > end || begin > end - instruction_size)
				return NOT_FOUND;

			for (size_t offset = begin;
				offset <= end - instruction_size;
				++offset)
			{
				if (matches(code + offset))
					return offset;
			}
			return NOT_FOUND;
		}

		bool IsAppendNamePrologue(const uint8_t* code, size_t size)
		{
			return signature::Matches(code, size, APPEND_NAME_PROLOGUE);
		}

		bool IsAppendNameFunction(const uint8_t* code, size_t size)
		{
			// Prefer the current live signature; retain the structural matcher below
			// for supported builds whose relocations or prologue layout differ.
			if (signature::Matches(
				code, size, detail::APPEND_NAME_LIVE_SIGNATURE))
				return true;

			if (!IsAppendNamePrologue(code, size))
				return false;

			const size_t seed_load = FindInstruction(
				code, size, 15, 56, 8, [](const uint8_t* value)
				{
					return value[0] == 0x48 && value[1] == 0x8B &&
						value[2] == 0xFA && value[3] == 0x8B &&
						value[4] == 0x19 && value[5] == 0x48 &&
						value[6] == 0x8B && value[7] == 0xF1;
				});
			if (seed_load == NOT_FOUND)
				return false;

			const size_t packed_name_split = FindInstruction(
				code, size, seed_load + 8, 112, 8, [](const uint8_t* value)
				{
					return value[0] == 0x8B && value[1] == 0xCB &&
						value[2] == 0x0F && value[3] == 0xB7 &&
						value[4] == 0xC3 && value[5] == 0xC1 &&
						value[6] == 0xE9 && value[7] == 0x10;
				});
			if (packed_name_split == NOT_FOUND)
				return false;

			const size_t entry_load = FindInstruction(
				code, size, packed_name_split + 8, 152, 12, [](const uint8_t* value)
				{
					return value[0] == 0x48 && value[1] == 0xC1 &&
						value[2] == 0xE8 && value[3] == 0x20 &&
						value[4] == 0x8D && value[5] == 0x1C &&
						value[6] == 0x00 && value[7] == 0x48 &&
						value[8] == 0x03 && value[9] == 0x5C &&
						value[10] == 0xCA && value[11] == 0x10;
				});
			if (entry_load == NOT_FOUND)
				return false;

			const size_t header_decode = FindInstruction(
				code, size, entry_load + 12, 176, 9, [](const uint8_t* value)
				{
					return value[0] == 0x48 && value[1] == 0x8B &&
						value[2] == 0xCF && value[3] == 0x0F &&
						value[4] == 0xB7 && value[5] == 0x13 &&
						value[6] == 0xC1 && value[7] == 0xEA &&
						value[8] == 0x06;
				});
			if (header_decode == NOT_FOUND)
				return false;

			const size_t number_check = FindInstruction(
				code, size, header_decode + 9, 192, 4, [](const uint8_t* value)
				{
					return value[0] == 0x83 && value[1] == 0x7E &&
						value[2] == 0x04 && value[3] == 0x00;
				});
			if (number_check == NOT_FOUND)
				return false;

			const size_t underscore_append = FindInstruction(
				code, size, number_check + 4, 256, 5, [](const uint8_t* value)
				{
					return value[0] == 0xBA && value[1] == 0x5F &&
						value[2] == 0x00 && value[3] == 0x00 &&
						value[4] == 0x00;
				});
			if (underscore_append == NOT_FOUND)
				return false;

			const size_t number_load = FindInstruction(
				code, size, underscore_append + 5, 280, 3, [](const uint8_t* value)
				{
					return value[0] == 0x8B && value[1] == 0x56 &&
						value[2] == 0x04;
				});
			if (number_load == NOT_FOUND)
				return false;

			const size_t decrement = FindInstruction(
				code, size, number_load + 3, 296, 2, [](const uint8_t* value)
				{
					return value[0] == 0xFF && value[1] == 0xCA;
				});
			if (decrement == NOT_FOUND)
				return false;

			return FindInstruction(
				code, size, decrement + 2, MAX_APPEND_NAME_WINDOW, 6,
				[](const uint8_t* value)
				{
					return value[0] == 0x48 && value[1] == 0x83 &&
						value[2] == 0xC4 && value[4] == 0x5F &&
						value[5] == 0xE9;
				}) != NOT_FOUND;
		}

		bool IsGWorldSequence(const uint8_t* code, size_t size)
		{
			return size >= detail::GWORLD_SEQUENCE_SIZE &&
				signature::Matches(code, size, detail::GWORLD_SEQUENCE);
		}

		int64_t ReadSignedDisplacement(const uint8_t* bytes)
		{
			const uint32_t value =
				static_cast<uint32_t>(bytes[0]) |
				(static_cast<uint32_t>(bytes[1]) << 8) |
				(static_cast<uint32_t>(bytes[2]) << 16) |
				(static_cast<uint32_t>(bytes[3]) << 24);
			return value <= INT32_MAX
				? static_cast<int64_t>(value)
				: static_cast<int64_t>(value) - 0x100000000LL;
		}

		bool AddDisplacement(
			uintptr_t instruction_end,
			int64_t displacement,
			uintptr_t& result)
		{
			if (displacement >= 0)
			{
				const uintptr_t value = static_cast<uintptr_t>(displacement);
				if (value > UINTPTR_MAX - instruction_end)
					return false;
				result = instruction_end + value;
				return true;
			}

			const uintptr_t magnitude = static_cast<uintptr_t>(-displacement);
			if (magnitude > instruction_end)
				return false;
			result = instruction_end - magnitude;
			return true;
		}

		bool IsWritableDataAddress(
			uintptr_t address,
			const detail::SectionView* sections,
			size_t section_count)
		{
			if ((address & (alignof(void*) - 1)) != 0)
				return false;

			for (size_t index = 0; index < section_count; ++index)
			{
				const detail::SectionView& section = sections[index];
				if (!section.writable || section.executable || section.size < sizeof(void*))
					continue;
				if (section.virtual_address > UINTPTR_MAX - section.size)
					continue;
				const uintptr_t end = section.virtual_address + section.size;
				if (address >= section.virtual_address &&
					address <= end - sizeof(void*))
					return true;
			}
			return false;
		}

		bool IsExecutableCodeAddress(
			uintptr_t address,
			const detail::SectionView* sections,
			size_t section_count)
		{
			for (size_t index = 0; index < section_count; ++index)
			{
				const detail::SectionView& section = sections[index];
				if (!section.executable || section.bytes == nullptr || section.size == 0 ||
					section.virtual_address > UINTPTR_MAX - section.size)
					continue;
				const uintptr_t end = section.virtual_address + section.size;
				if (address >= section.virtual_address && address < end)
					return IsReadableRange(reinterpret_cast<const void*>(address), 1);
			}
			return false;
		}

		bool ResolveKnownProfile(
			const detail::SectionView* sections,
			size_t section_count,
			uintptr_t image_base,
			size_t image_size,
			uint32_t image_checksum,
			ResolvedOffsets& result)
		{
			for (const KnownOffsetProfile& profile : KNOWN_OFFSET_PROFILES)
			{
				if (profile.image_size != image_size ||
					profile.append_name_offset >= image_size ||
					profile.gworld_offset >= image_size)
					continue;

				const uintptr_t append_name = image_base + profile.append_name_offset;
				const uintptr_t gworld = image_base + profile.gworld_offset;
				if (!IsExecutableCodeAddress(append_name, sections, section_count) ||
					!IsWritableDataAddress(gworld, sections, section_count))
					return false;

				result = {
					append_name,
					gworld,
					image_size,
					image_checksum,
					profile.viewport_tick_index,
					profile.process_event_index,
					ResolutionSource::KnownProfile,
				};
				return true;
			}
			return false;
		}

		void RecordCandidate(uintptr_t value, uintptr_t& candidate, size_t& count)
		{
			if (count == 0)
			{
				candidate = value;
				count = 1;
			}
			else if (candidate != value)
			{
				count = 2;
			}
		}
	} // namespace

	namespace detail
	{
		bool ResolveInSections(
			const SectionView* sections,
			size_t section_count,
			uintptr_t image_base,
			size_t image_size,
			ResolvedOffsets& result)
		{
			result = {};
			if (sections == nullptr || section_count == 0 || image_size == 0 ||
				image_size > UINTPTR_MAX - image_base)
				return false;

			const uintptr_t image_end = image_base + image_size;
			uintptr_t append_name = 0;
			uintptr_t gworld = 0;
			size_t append_name_count = 0;
			size_t gworld_count = 0;

			for (size_t section_index = 0;
				section_index < section_count;
				++section_index)
			{
				const SectionView& section = sections[section_index];
				if (!section.executable || section.bytes == nullptr || section.size == 0 ||
					section.virtual_address < image_base ||
					section.virtual_address >= image_end ||
					section.size > image_end - section.virtual_address)
					continue;

				for (size_t offset = 0; offset < section.size; ++offset)
				{
					if (section.bytes[offset] != 0x48)
						continue;

					const size_t remaining = section.size - offset;
					const uint8_t* code = section.bytes + offset;
					const uintptr_t address = section.virtual_address + offset;

					if (append_name_count < 2 && IsAppendNameFunction(code, remaining))
						RecordCandidate(address, append_name, append_name_count);

					if (gworld_count >= 2 || !IsGWorldSequence(code, remaining) ||
						address > UINTPTR_MAX - 22)
						continue;

					uintptr_t target = 0;
					if (!AddDisplacement(
						address + 22,
						ReadSignedDisplacement(code + 18),
						target) ||
						target < image_base || target >= image_end ||
						!IsWritableDataAddress(target, sections, section_count))
						continue;
					RecordCandidate(target, gworld, gworld_count);
				}
			}

			if (append_name_count != 1 || gworld_count != 1)
				return false;

			result = {
				append_name,
				gworld,
				image_size,
				0,
				DEFAULT_VIEWPORT_TICK_INDEX,
				DEFAULT_PROCESS_EVENT_INDEX,
				ResolutionSource::Signature,
			};
			return true;
		}
	} // namespace detail

	bool Initialize()
	{
		if (resolution_attempted)
			return resolution_succeeded;
		resolution_attempted = true;

		const uintptr_t image_base = memory::ImageBase();
		if (image_base == 0)
			return false;

		IMAGE_DOS_HEADER dos_header{};
		if (!memory::ReadValue(
			reinterpret_cast<const void*>(image_base),
			0,
			dos_header) ||
			dos_header.e_magic != IMAGE_DOS_SIGNATURE ||
			dos_header.e_lfanew <= 0 || dos_header.e_lfanew > 0x100000)
			return false;

		IMAGE_NT_HEADERS64 nt_headers{};
		if (!memory::ReadValue(
			reinterpret_cast<const void*>(image_base),
			static_cast<size_t>(dos_header.e_lfanew),
			nt_headers) ||
			nt_headers.Signature != IMAGE_NT_SIGNATURE ||
			nt_headers.FileHeader.Machine != IMAGE_FILE_MACHINE_AMD64 ||
			nt_headers.OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC ||
			nt_headers.OptionalHeader.SizeOfImage == 0 ||
			nt_headers.OptionalHeader.SizeOfImage > MAX_IMAGE_SIZE ||
			nt_headers.FileHeader.NumberOfSections == 0 ||
			nt_headers.FileHeader.NumberOfSections > MAX_IMAGE_SECTIONS)
			return false;

		const size_t section_table_offset =
			static_cast<size_t>(dos_header.e_lfanew) + sizeof(DWORD) +
			sizeof(IMAGE_FILE_HEADER) + nt_headers.FileHeader.SizeOfOptionalHeader;
		const size_t section_table_size =
			static_cast<size_t>(nt_headers.FileHeader.NumberOfSections) *
			sizeof(IMAGE_SECTION_HEADER);
		if (section_table_offset > nt_headers.OptionalHeader.SizeOfHeaders ||
			section_table_size >
			nt_headers.OptionalHeader.SizeOfHeaders - section_table_offset)
			return false;

		detail::SectionView sections[MAX_IMAGE_SECTIONS]{};
		size_t section_count = 0;
		const size_t image_size = nt_headers.OptionalHeader.SizeOfImage;
		for (size_t index = 0;
			index < nt_headers.FileHeader.NumberOfSections;
			++index)
		{
			IMAGE_SECTION_HEADER section_header{};
			if (!memory::ReadValue(
				reinterpret_cast<const void*>(image_base),
				section_table_offset + index * sizeof(IMAGE_SECTION_HEADER),
				section_header))
				return false;

			const size_t virtual_address = section_header.VirtualAddress;
			size_t virtual_size = section_header.Misc.VirtualSize;
			if (virtual_size == 0)
				virtual_size = section_header.SizeOfRawData;
			if (virtual_size == 0 || virtual_address >= image_size)
				continue;
			if (virtual_size > image_size - virtual_address)
				virtual_size = image_size - virtual_address;

			const uintptr_t address = image_base + virtual_address;
			const bool executable =
				(section_header.Characteristics & IMAGE_SCN_MEM_EXECUTE) != 0;
			if (executable && !IsReadableRange(
				reinterpret_cast<const void*>(address), virtual_size))
				return false;

			sections[section_count++] = {
				executable ? reinterpret_cast<const uint8_t*>(address) : nullptr,
				virtual_size,
				address,
				executable,
				(section_header.Characteristics & IMAGE_SCN_MEM_WRITE) != 0,
			};
		}

		ResolvedOffsets candidate{};
		const bool resolved_by_signature = detail::ResolveInSections(
			sections,
			section_count,
			image_base,
			image_size,
			candidate);
		if (resolved_by_signature)
		{
			candidate.image_checksum = nt_headers.OptionalHeader.CheckSum;
		}
		else if (!ResolveKnownProfile(
				sections,
				section_count,
				image_base,
				image_size,
				nt_headers.OptionalHeader.CheckSum,
				candidate))
			return false;

		resolved_offsets = candidate;
		resolution_succeeded = true;
		return true;
	}

	const ResolvedOffsets* Get()
	{
		return resolution_succeeded ? &resolved_offsets : nullptr;
	}

	bool IsKnownImageProfile(size_t image_size, uint32_t)
	{
		for (const KnownOffsetProfile& profile : KNOWN_OFFSET_PROFILES)
		{
			if (profile.image_size == image_size)
				return true;
		}
		return false;
	}
} // namespace nte::mods::offsets
