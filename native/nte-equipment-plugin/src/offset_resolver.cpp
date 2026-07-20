#include "offset_resolver.hpp"

#include "memory_access.hpp"

#include <Windows.h>

#include <cstddef>
#include <cstdint>

namespace nte::equipment::offsets
{
	namespace
	{
		constexpr size_t MAX_IMAGE_SECTIONS = 96;
		constexpr size_t MAX_APPEND_NAME_WINDOW = 0x140;
		constexpr size_t GWORLD_SEQUENCE_SIZE = 35;
		constexpr size_t NOT_FOUND = static_cast<size_t>(-1);
		constexpr size_t MAX_IMAGE_SIZE = 0x40000000;

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
			return size >= 15 &&
				code[0] == 0x48 && code[1] == 0x89 && code[2] == 0x5C &&
				code[3] == 0x24 && code[4] == 0x10 && code[5] == 0x48 &&
				code[6] == 0x89 && code[7] == 0x74 && code[8] == 0x24 &&
				code[9] == 0x18 && code[10] == 0x57 && code[11] == 0x48 &&
				code[12] == 0x83 && code[13] == 0xEC && code[14] == 0x20;
		}

		bool IsAppendNameFunction(const uint8_t* code, size_t size)
		{
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
			return size >= GWORLD_SEQUENCE_SIZE &&
				code[0] == 0x48 && code[1] == 0x8B && code[2] == 0x04 &&
				code[3] == 0xD0 && code[4] == 0x8B && code[5] == 0x04 &&
				code[6] == 0x01 && code[7] == 0x39 && code[8] == 0x05 &&
				code[13] == 0x7F && code[15] == 0x48 && code[16] == 0x89 &&
				code[17] == 0x1D && code[22] == 0x48 && code[23] == 0x8D &&
				code[24] == 0x05 && code[29] == 0x48 && code[30] == 0x83 &&
				code[31] == 0xC4 && code[33] == 0x5B && code[34] == 0xC3;
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

			result.append_name_address = append_name;
			result.gworld_address = gworld;
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
		if (!detail::ResolveInSections(
			sections,
			section_count,
			image_base,
			image_size,
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
} // namespace nte::equipment::offsets
