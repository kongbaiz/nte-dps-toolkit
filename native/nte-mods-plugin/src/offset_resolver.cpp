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
		constexpr size_t FNAME_CURRENT_BLOCK_OFFSET = 0x08;
		constexpr size_t FNAME_CURRENT_CURSOR_OFFSET = 0x0C;
		constexpr size_t FNAME_BLOCKS_OFFSET = 0x10;
		constexpr uint32_t MAX_FNAME_BLOCKS = 0x2000;
		constexpr uint32_t MAX_FNAME_BLOCK_BYTES = 0x20000;
		constexpr uint32_t MAX_FNAME_LENGTH = 0x3FF;
		constexpr uint32_t MAX_FNAME_ANCHOR_BLOCKS = 16;
		constexpr uint32_t ELEMENTS_PER_CHUNK = 0x10000;
		constexpr size_t FUOBJECT_ITEM_SIZE = 0x18;
		constexpr size_t UOBJECT_CLASS_OFFSET = 0x10;
		constexpr size_t UOBJECT_INDEX_OFFSET = 0x0C;
		constexpr size_t UOBJECT_NAME_OFFSET = 0x18;
		constexpr size_t WORLD_GAME_INSTANCE_OFFSET = 0x230;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_VIEWPORT_OFFSET = 0x78;
		constexpr size_t VIEWPORT_WORLD_OFFSET = 0x78;
		constexpr size_t VIEWPORT_GAME_INSTANCE_OFFSET = 0x80;
		constexpr ULONGLONG DYNAMIC_RESOLUTION_RETRY_MS = 1000;

		struct KnownOffsetProfile
		{
			size_t image_size;
			uint32_t image_checksum;
			size_t append_name_offset;
			size_t fname_pool_offset;
			size_t gobjects_offset;
			size_t gworld_offset;
			size_t viewport_tick_index;
			size_t process_event_index;
		};

		// SDK snapshots for the currently supported CN, CN test, and Global builds.
		// Image size is the stable fallback key; PE checksum is retained only in the
		// resolved diagnostics and never blocks signature/profile location.
		constexpr KnownOffsetProfile KNOWN_OFFSET_PROFILES[]{
			{ 0x1000C000, 0, 0x0161C020, 0, 0, 0x0EAAADB0, 100, 0x4C },
			{ 0x1064D000, 0, 0x0164A940, 0, 0, 0x0F071DB0, 100, 0x4C },
			{ 0x1000E000, 0, 0x0161BAE0, 0, 0, 0x0EAAADB0, 100, 0x4C },
			// CN 1.3.5 (2026-08-13): exact PE fingerprint and offsets verified
			// by the live semantic resolver before promoting this fast profile.
			{ 0x1066A000, 0x0FDCF5DD, 0x016491C0, 0x0F3CB500,
				0x0F4AF680, 0x0F658B98, 100, 0x4C },
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

		struct GObjectsInfo
		{
			uintptr_t address;
			uintptr_t chunks;
			uint32_t max_elements;
			uint32_t num_elements;
			uint32_t max_chunks;
			uint32_t num_chunks;
		};

		ResolvedOffsets resolved_offsets{};
		detail::SectionView cached_sections[MAX_IMAGE_SECTIONS]{};
		size_t cached_section_count = 0;
		GObjectsInfo cached_gobjects{};
		uintptr_t cached_image_base = 0;
		size_t cached_image_size = 0;
		uint32_t cached_image_checksum = 0;
		bool static_resolution_attempted = false;
		bool static_resolution_succeeded = false;
		bool resolution_succeeded = false;
		ULONGLONG next_dynamic_resolution_tick = 0;

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

		template <typename T>
		T LoadUnaligned(const uint8_t* bytes)
		{
			T value{};
			auto* output = reinterpret_cast<uint8_t*>(&value);
			for (size_t index = 0; index < sizeof(T); ++index)
				output[index] = bytes[index];
			return value;
		}

		bool IsCanonicalPointer(uintptr_t value)
		{
			return value >= 0x10000 && value < 0x0000800000000000ULL &&
				(value & (alignof(void*) - 1)) == 0;
		}

		bool EqualsWideAscii(
			const wchar_t* value,
			size_t value_length,
			const char* expected)
		{
			if (value == nullptr || expected == nullptr)
				return false;
			size_t index = 0;
			for (; index < value_length && expected[index] != '\0'; ++index)
			{
				if (value[index] != static_cast<unsigned char>(expected[index]))
					return false;
			}
			return index == value_length && expected[index] == '\0';
		}

		bool MatchesAscii(
			const uint8_t* bytes,
			size_t available,
			const char* expected,
			size_t expected_length)
		{
			if (bytes == nullptr || expected == nullptr || expected_length > available)
				return false;
			for (size_t index = 0; index < expected_length; ++index)
			{
				if (bytes[index] != static_cast<uint8_t>(expected[index]))
					return false;
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

		int64_t ReadSignedDisplacement(const uint8_t* bytes);
		bool AddDisplacement(
			uintptr_t instruction_end,
			int64_t displacement,
			uintptr_t& result);

		bool TryRipRelativeTarget(
			const uint8_t* code,
			size_t size,
			uintptr_t instruction_address,
			uintptr_t& target)
		{
			target = 0;
			if (code == nullptr || size < 6)
				return false;

			size_t opcode_offset = 0;
			if ((code[0] & 0xF0) == 0x40)
				opcode_offset = 1;
			if (size < opcode_offset + 6)
				return false;

			const uint8_t opcode = code[opcode_offset];
			if (opcode != 0x8B && opcode != 0x89 && opcode != 0x8D &&
				opcode != 0x3B && opcode != 0x39)
				return false;
			const uint8_t modrm = code[opcode_offset + 1];
			if ((modrm & 0xC7) != 0x05)
				return false;

			const size_t instruction_size = opcode_offset + 6;
			if (instruction_address > UINTPTR_MAX - instruction_size)
				return false;
			return AddDisplacement(
				instruction_address + instruction_size,
				ReadSignedDisplacement(code + opcode_offset + 2),
				target);
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

		void RecordCandidate(uintptr_t value, uintptr_t& candidate, size_t& count);

		bool FindAsciiNameIndex(
			uintptr_t fname_pool,
			const char* expected,
			size_t expected_length,
			int32_t& result)
		{
			result = -1;
			uint32_t current_block = 0;
			uint32_t current_cursor = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool),
					FNAME_CURRENT_BLOCK_OFFSET,
					current_block) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool),
					FNAME_CURRENT_CURSOR_OFFSET,
					current_cursor) ||
				current_block >= MAX_FNAME_BLOCKS || current_cursor == 0 ||
				current_cursor >= MAX_FNAME_BLOCK_BYTES)
				return false;

			const uint32_t block_count = current_block + 1 < MAX_FNAME_ANCHOR_BLOCKS
				? current_block + 1
				: MAX_FNAME_ANCHOR_BLOCKS;
			for (uint32_t block_index = 0; block_index < block_count; ++block_index)
			{
				uintptr_t block = 0;
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(fname_pool),
						FNAME_BLOCKS_OFFSET + block_index * sizeof(uintptr_t),
						block) || !IsCanonicalPointer(block))
					return false;

				const size_t limit = block_index == current_block
					? current_cursor
					: MAX_FNAME_BLOCK_BYTES;
				if (limit < sizeof(uint16_t) + expected_length ||
					!IsReadableRange(reinterpret_cast<const void*>(block), limit))
					continue;

				const auto* bytes = reinterpret_cast<const uint8_t*>(block);
				for (size_t offset = 0;
					offset + sizeof(uint16_t) + expected_length <= limit;
					offset += 2)
				{
					const uint16_t header = LoadUnaligned<uint16_t>(bytes + offset);
					if ((header & 1) != 0 || ((header >> 6) & MAX_FNAME_LENGTH) != expected_length ||
						!MatchesAscii(
							bytes + offset + sizeof(uint16_t),
							limit - offset - sizeof(uint16_t),
							expected,
							expected_length))
						continue;

					const uint32_t comparison_index =
						(block_index << 16) | static_cast<uint32_t>(offset / 2);
					if (result >= 0 && static_cast<uint32_t>(result) != comparison_index)
						return false;
					result = static_cast<int32_t>(comparison_index);
				}
			}
			return result >= 0;
		}

		bool IsFNamePoolCandidate(uintptr_t address)
		{
			wchar_t decoded[16]{};
			size_t decoded_length = 0;
			int32_t byte_property_index = -1;
			return detail::DecodeNameFromPool(
					address,
					0,
					0,
					decoded,
					_countof(decoded),
					decoded_length) &&
				EqualsWideAscii(decoded, decoded_length, "None") &&
				FindAsciiNameIndex(
					address,
					"ByteProperty",
					12,
					byte_property_index);
		}

		bool ReadGObjectsInfo(uintptr_t address, GObjectsInfo& result)
		{
			result = {};
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(address), 0, result.chunks) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(address), 0x10, result.max_elements) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(address), 0x14, result.num_elements) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(address), 0x18, result.max_chunks) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(address), 0x1C, result.num_chunks))
				return false;
			result.address = address;
			return IsCanonicalPointer(result.chunks) &&
				result.num_elements >= 10000 &&
				result.num_elements <= result.max_elements &&
				result.max_elements <= 30000000 &&
				result.num_chunks >= 1 &&
				result.num_chunks <= result.max_chunks &&
				result.max_chunks <= 8192 &&
				static_cast<uint64_t>(result.num_chunks) * ELEMENTS_PER_CHUNK >=
					result.num_elements;
		}

		bool IsGObjectsCandidate(
			uintptr_t address,
			uintptr_t fname_pool,
			int32_t object_name_index,
			GObjectsInfo& result)
		{
			GObjectsInfo candidate{};
			if (!ReadGObjectsInfo(address, candidate))
				return false;

			uintptr_t first_chunk = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(candidate.chunks),
					0,
					first_chunk) || !IsCanonicalPointer(first_chunk) ||
				!IsReadableRange(
					reinterpret_cast<const void*>(first_chunk),
					FUOBJECT_ITEM_SIZE * 32))
				return false;

			int matching_indexes = 0;
			bool has_object_anchor = false;
			for (int32_t index = 0; index < 32; ++index)
			{
				uintptr_t object = 0;
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(first_chunk),
						static_cast<size_t>(index) * FUOBJECT_ITEM_SIZE,
						object) || !IsCanonicalPointer(object))
					continue;

				int32_t internal_index = -1;
				int32_t name_index = -1;
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(object),
						UOBJECT_INDEX_OFFSET,
						internal_index) ||
					!memory::ReadValue(
						reinterpret_cast<const void*>(object),
						UOBJECT_NAME_OFFSET,
						name_index))
					continue;
				if (internal_index == index)
					++matching_indexes;
				if (name_index == object_name_index)
					has_object_anchor = true;
			}

			static_cast<void>(fname_pool);
			if (matching_indexes < 4 || !has_object_anchor)
				return false;
			result = candidate;
			return true;
		}

		bool FindSemanticAnchors(
			const detail::SectionView* sections,
			size_t section_count,
			uintptr_t& fname_pool,
			GObjectsInfo& gobjects)
		{
			fname_pool = 0;
			gobjects = {};
			size_t fname_pool_count = 0;
			for (size_t section_index = 0; section_index < section_count; ++section_index)
			{
				const detail::SectionView& section = sections[section_index];
				if (!section.writable || section.executable || section.bytes == nullptr ||
					section.size < 0x20)
					continue;
				for (size_t offset = 0; offset <= section.size - 0x20; offset += 8)
				{
					const uint8_t* bytes = section.bytes + offset;
					const uint32_t current_block = LoadUnaligned<uint32_t>(bytes + 0x08);
					const uint32_t current_cursor = LoadUnaligned<uint32_t>(bytes + 0x0C);
					const uintptr_t block0 = LoadUnaligned<uintptr_t>(bytes + 0x10);
					if (current_block < MAX_FNAME_BLOCKS && current_cursor > 0 &&
						current_cursor < MAX_FNAME_BLOCK_BYTES && IsCanonicalPointer(block0))
					{
						const uintptr_t address = section.virtual_address + offset;
						if (IsFNamePoolCandidate(address))
							RecordCandidate(address, fname_pool, fname_pool_count);
					}
				}
			}
			if (fname_pool_count != 1)
				return false;

			int32_t object_name_index = -1;
			if (!FindAsciiNameIndex(fname_pool, "Object", 6, object_name_index))
				return false;

			size_t gobjects_count = 0;
			for (size_t section_index = 0; section_index < section_count; ++section_index)
			{
				const detail::SectionView& section = sections[section_index];
				if (!section.writable || section.executable || section.bytes == nullptr ||
					section.size < 0x20)
					continue;
				for (size_t offset = 0; offset <= section.size - 0x20; offset += 8)
				{
					GObjectsInfo candidate{};
					const uintptr_t address = section.virtual_address + offset;
					if (!IsGObjectsCandidate(
							address,
							fname_pool,
							object_name_index,
							candidate))
						continue;
					if (gobjects_count == 0)
					{
						gobjects = candidate;
						gobjects_count = 1;
					}
					else if (gobjects.address != candidate.address)
					{
						gobjects_count = 2;
					}
				}
			}
			return gobjects_count == 1;
		}

		bool IsGObjectsMember(
			const GObjectsInfo& info,
			uintptr_t object,
			int32_t internal_index)
		{
			if (internal_index < 0 ||
				static_cast<uint32_t>(internal_index) >= info.num_elements)
				return false;
			const uint32_t chunk_index =
				static_cast<uint32_t>(internal_index) / ELEMENTS_PER_CHUNK;
			const uint32_t item_index =
				static_cast<uint32_t>(internal_index) % ELEMENTS_PER_CHUNK;
			if (chunk_index >= info.num_chunks)
				return false;
			uintptr_t chunk = 0;
			uintptr_t stored_object = 0;
			return memory::ReadValue(
					reinterpret_cast<const void*>(info.chunks),
					static_cast<size_t>(chunk_index) * sizeof(uintptr_t),
					chunk) && IsCanonicalPointer(chunk) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(chunk),
					static_cast<size_t>(item_index) * FUOBJECT_ITEM_SIZE,
					stored_object) && stored_object == object;
		}

		bool FindWorldClass(
			uintptr_t fname_pool,
			const GObjectsInfo& info,
			uintptr_t& world_class)
		{
			world_class = 0;
			int32_t world_name_index = -1;
			int32_t class_name_index = -1;
			if (!FindAsciiNameIndex(fname_pool, "World", 5, world_name_index) ||
				!FindAsciiNameIndex(fname_pool, "Class", 5, class_name_index))
				return false;

			uintptr_t first_chunk = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(info.chunks), 0, first_chunk) ||
				!IsCanonicalPointer(first_chunk))
				return false;
			const uint32_t count = info.num_elements < ELEMENTS_PER_CHUNK
				? info.num_elements
				: ELEMENTS_PER_CHUNK;
			if (!IsReadableRange(
				reinterpret_cast<const void*>(first_chunk),
				static_cast<size_t>(count) * FUOBJECT_ITEM_SIZE))
				return false;

			for (uint32_t index = 0; index < count; ++index)
			{
				uintptr_t object = 0;
				int32_t name_index = -1;
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(first_chunk),
						static_cast<size_t>(index) * FUOBJECT_ITEM_SIZE,
						object) || !IsCanonicalPointer(object) ||
					!memory::ReadValue(
						reinterpret_cast<const void*>(object),
						UOBJECT_NAME_OFFSET,
						name_index) || name_index != world_name_index)
					continue;

				uintptr_t object_class = 0;
				int32_t object_class_name = -1;
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(object),
						UOBJECT_CLASS_OFFSET,
						object_class) || !IsCanonicalPointer(object_class) ||
					!memory::ReadValue(
						reinterpret_cast<const void*>(object_class),
						UOBJECT_NAME_OFFSET,
						object_class_name) || object_class_name != class_name_index)
					continue;
				if (world_class != 0 && world_class != object)
					return false;
				world_class = object;
			}
			return world_class != 0;
		}

		bool IsConsistentWorldCandidate(uintptr_t world)
		{
			uintptr_t game_instance = 0;
			uintptr_t local_players = 0;
			int32_t local_player_count = 0;
			int32_t local_player_capacity = 0;
			uintptr_t local_player = 0;
			uintptr_t viewport = 0;
			uintptr_t viewport_world = 0;
			uintptr_t viewport_game_instance = 0;
			return memory::ReadValue(
					reinterpret_cast<const void*>(world),
					WORLD_GAME_INSTANCE_OFFSET,
					game_instance) && IsCanonicalPointer(game_instance) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(game_instance),
					GAME_INSTANCE_LOCAL_PLAYERS_OFFSET,
					local_players) && IsCanonicalPointer(local_players) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(game_instance),
					GAME_INSTANCE_LOCAL_PLAYERS_OFFSET + sizeof(uintptr_t),
					local_player_count) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(game_instance),
					GAME_INSTANCE_LOCAL_PLAYERS_OFFSET + sizeof(uintptr_t) + sizeof(int32_t),
					local_player_capacity) &&
				local_player_count >= 1 && local_player_count <= local_player_capacity &&
				local_player_capacity <= 64 &&
				memory::ReadValue(
					reinterpret_cast<const void*>(local_players), 0, local_player) &&
				IsCanonicalPointer(local_player) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(local_player),
					LOCAL_PLAYER_VIEWPORT_OFFSET,
					viewport) && IsCanonicalPointer(viewport) &&
				memory::ReadValue(
					reinterpret_cast<const void*>(viewport),
					VIEWPORT_WORLD_OFFSET,
					viewport_world) && viewport_world == world &&
				memory::ReadValue(
					reinterpret_cast<const void*>(viewport),
					VIEWPORT_GAME_INSTANCE_OFFSET,
					viewport_game_instance) && viewport_game_instance == game_instance;
		}

		bool ResolveSemanticGWorld(
			const detail::SectionView* sections,
			size_t section_count,
			uintptr_t fname_pool,
			const GObjectsInfo& gobjects,
			uintptr_t& gworld)
		{
			gworld = 0;
			uintptr_t world_class = 0;
			if (!FindWorldClass(fname_pool, gobjects, world_class))
				return false;
			uintptr_t candidates[8]{};
			size_t candidate_count = 0;
			bool candidate_overflow = false;
			for (size_t section_index = 0; section_index < section_count; ++section_index)
			{
				const detail::SectionView& section = sections[section_index];
				if (!section.writable || section.executable || section.bytes == nullptr ||
					section.size < sizeof(uintptr_t))
					continue;
				for (size_t offset = 0;
					offset <= section.size - sizeof(uintptr_t);
					offset += sizeof(uintptr_t))
				{
					const uintptr_t object = LoadUnaligned<uintptr_t>(section.bytes + offset);
					if (!IsCanonicalPointer(object))
						continue;
					uintptr_t object_class = 0;
					int32_t internal_index = -1;
					if (!memory::ReadValue(
							reinterpret_cast<const void*>(object),
							UOBJECT_CLASS_OFFSET,
							object_class) || object_class != world_class ||
						!memory::ReadValue(
							reinterpret_cast<const void*>(object),
							UOBJECT_INDEX_OFFSET,
							internal_index) ||
						!IsGObjectsMember(gobjects, object, internal_index) ||
						!IsConsistentWorldCandidate(object))
						continue;
					const uintptr_t candidate = section.virtual_address + offset;
					bool duplicate = false;
					for (size_t index = 0; index < candidate_count; ++index)
					{
						if (candidates[index] == candidate)
							duplicate = true;
					}
					if (!duplicate)
					{
						if (candidate_count >= _countof(candidates))
							candidate_overflow = true;
						else
							candidates[candidate_count++] = candidate;
					}
				}
			}
			if (candidate_overflow || candidate_count == 0)
				return false;
			if (candidate_count == 1)
			{
				gworld = candidates[0];
				return true;
			}

			size_t reference_count = 0;
			return detail::SelectUniqueGWorldCandidateByCodeReferences(
				sections,
				section_count,
				candidates,
				candidate_count,
				gworld,
				reference_count);
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
					(profile.image_checksum != 0 &&
						profile.image_checksum != image_checksum) ||
					profile.append_name_offset >= image_size ||
					profile.fname_pool_offset >= image_size ||
					profile.gobjects_offset >= image_size ||
					profile.gworld_offset >= image_size)
					continue;

				const uintptr_t append_name = image_base + profile.append_name_offset;
				const uintptr_t fname_pool = profile.fname_pool_offset == 0
					? 0
					: image_base + profile.fname_pool_offset;
				const uintptr_t gobjects = profile.gobjects_offset == 0
					? 0
					: image_base + profile.gobjects_offset;
				const uintptr_t gworld = image_base + profile.gworld_offset;
				if (!IsExecutableCodeAddress(append_name, sections, section_count) ||
					(fname_pool != 0 &&
						!IsWritableDataAddress(fname_pool, sections, section_count)) ||
					(gobjects != 0 &&
						!IsWritableDataAddress(gobjects, sections, section_count)) ||
					!IsWritableDataAddress(gworld, sections, section_count))
					return false;

				result = {
					append_name,
					fname_pool,
					gobjects,
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
		bool SelectUniqueGWorldCandidateByCodeReferences(
			const SectionView* sections,
			size_t section_count,
			const uintptr_t* candidates,
			size_t candidate_count,
			uintptr_t& selected,
			size_t& reference_count)
		{
			selected = 0;
			reference_count = 0;
			if (sections == nullptr || section_count == 0 || candidates == nullptr ||
				candidate_count < 2 || candidate_count > 8)
				return false;

			size_t counts[8]{};
			for (size_t section_index = 0; section_index < section_count; ++section_index)
			{
				const SectionView& section = sections[section_index];
				if (!section.executable || section.bytes == nullptr || section.size < 6)
					continue;
				for (size_t offset = 0; offset + 6 <= section.size; ++offset)
				{
					if (offset != 0 &&
						(section.bytes[offset - 1] & 0xF0) == 0x40)
						continue;
					uintptr_t target = 0;
					if (!TryRipRelativeTarget(
							section.bytes + offset,
							section.size - offset,
							section.virtual_address + offset,
							target))
						continue;
					for (size_t candidate_index = 0;
						candidate_index < candidate_count;
						++candidate_index)
					{
						if (target == candidates[candidate_index])
							++counts[candidate_index];
					}
				}
			}

			size_t winner = candidate_count;
			for (size_t index = 0; index < candidate_count; ++index)
			{
				if (counts[index] > reference_count)
				{
					winner = index;
					reference_count = counts[index];
				}
				else if (counts[index] == reference_count && counts[index] != 0)
				{
					winner = candidate_count;
				}
			}
			if (winner >= candidate_count || reference_count == 0)
			{
				selected = 0;
				return false;
			}
			selected = candidates[winner];
			return true;
		}

		bool DecodeNameFromPool(
			uintptr_t fname_pool_address,
			int32_t comparison_index,
			uint32_t number,
			wchar_t* output,
			size_t output_capacity,
			size_t& output_length)
		{
			output_length = 0;
			if (fname_pool_address == 0 || comparison_index < 0 ||
				output == nullptr || output_capacity == 0)
				return false;

			uint32_t current_block = 0;
			uint32_t current_cursor = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					FNAME_CURRENT_BLOCK_OFFSET,
					current_block) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					FNAME_CURRENT_CURSOR_OFFSET,
					current_cursor) ||
				current_block >= MAX_FNAME_BLOCKS || current_cursor == 0 ||
				current_cursor >= MAX_FNAME_BLOCK_BYTES)
				return false;

			const uint32_t raw_index = static_cast<uint32_t>(comparison_index);
			const uint32_t block_index = raw_index >> 16;
			const size_t entry_offset =
				static_cast<size_t>(raw_index & 0xFFFF) * 2;
			if (block_index > current_block || block_index >= MAX_FNAME_BLOCKS ||
				entry_offset > MAX_FNAME_BLOCK_BYTES - sizeof(uint16_t))
				return false;

			uintptr_t block = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					FNAME_BLOCKS_OFFSET + block_index * sizeof(uintptr_t),
					block) || !IsCanonicalPointer(block))
				return false;

			uint16_t header = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(block), entry_offset, header))
				return false;
			const bool wide = (header & 1) != 0;
			const size_t length = (header >> 6) & MAX_FNAME_LENGTH;
			if (length == 0 || length > MAX_FNAME_LENGTH)
				return false;
			const size_t byte_length = length * (wide ? sizeof(wchar_t) : 1);
			if (entry_offset + sizeof(uint16_t) > MAX_FNAME_BLOCK_BYTES ||
				byte_length > MAX_FNAME_BLOCK_BYTES - entry_offset - sizeof(uint16_t))
				return false;
			const uintptr_t text_address =
				block + entry_offset + sizeof(uint16_t);
			if (!IsReadableRange(
				reinterpret_cast<const void*>(text_address), byte_length))
				return false;

			wchar_t suffix[11]{};
			size_t suffix_length = 0;
			if (number != 0)
			{
				uint32_t value = number - 1;
				do
				{
					suffix[suffix_length++] =
						static_cast<wchar_t>(L'0' + value % 10);
					value /= 10;
				} while (value != 0 && suffix_length < _countof(suffix));
			}
			const size_t required = length + (suffix_length == 0 ? 0 : suffix_length + 1);
			if (required >= output_capacity)
				return false;

			if (wide)
			{
				const auto* text = reinterpret_cast<const wchar_t*>(text_address);
				for (size_t index = 0; index < length; ++index)
					output[index] = text[index];
			}
			else
			{
				const auto* text = reinterpret_cast<const uint8_t*>(text_address);
				for (size_t index = 0; index < length; ++index)
					output[index] = static_cast<wchar_t>(text[index]);
			}
			output_length = length;
			if (suffix_length != 0)
			{
				output[output_length++] = L'_';
				for (size_t index = suffix_length; index > 0; --index)
					output[output_length++] = suffix[index - 1];
			}
			output[output_length] = L'\0';
			return true;
		}

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
				0,
				0,
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
		if (resolution_succeeded)
			return true;

		if (!static_resolution_attempted)
		{
			static_resolution_attempted = true;
			cached_image_base = memory::ImageBase();
			if (cached_image_base == 0)
				return false;

			IMAGE_DOS_HEADER dos_header{};
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(cached_image_base),
					0,
					dos_header) ||
				dos_header.e_magic != IMAGE_DOS_SIGNATURE ||
				dos_header.e_lfanew <= 0 || dos_header.e_lfanew > 0x100000)
				return false;

			IMAGE_NT_HEADERS64 nt_headers{};
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(cached_image_base),
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

			cached_image_size = nt_headers.OptionalHeader.SizeOfImage;
			cached_image_checksum = nt_headers.OptionalHeader.CheckSum;
			for (size_t index = 0;
				index < nt_headers.FileHeader.NumberOfSections;
				++index)
			{
				IMAGE_SECTION_HEADER section_header{};
				if (!memory::ReadValue(
						reinterpret_cast<const void*>(cached_image_base),
						section_table_offset + index * sizeof(IMAGE_SECTION_HEADER),
						section_header))
					return false;

				const size_t virtual_address = section_header.VirtualAddress;
				size_t virtual_size = section_header.Misc.VirtualSize;
				if (virtual_size == 0)
					virtual_size = section_header.SizeOfRawData;
				if (virtual_size == 0 || virtual_address >= cached_image_size)
					continue;
				if (virtual_size > cached_image_size - virtual_address)
					virtual_size = cached_image_size - virtual_address;

				const uintptr_t address = cached_image_base + virtual_address;
				const bool executable =
					(section_header.Characteristics & IMAGE_SCN_MEM_EXECUTE) != 0;
				const bool writable =
					(section_header.Characteristics & IMAGE_SCN_MEM_WRITE) != 0;
				const bool readable = IsReadableRange(
					reinterpret_cast<const void*>(address), virtual_size);
				if (executable && !readable)
					return false;
				cached_sections[cached_section_count++] = {
					readable ? reinterpret_cast<const uint8_t*>(address) : nullptr,
					virtual_size,
					address,
					executable,
					writable,
				};
			}

			ResolvedOffsets known_profile{};
			if (ResolveKnownProfile(
					cached_sections,
					cached_section_count,
					cached_image_base,
					cached_image_size,
					cached_image_checksum,
					known_profile))
			{
				resolved_offsets = known_profile;
				static_resolution_succeeded = true;
			}
			else
			{
				uintptr_t fname_pool = 0;
				if (FindSemanticAnchors(
						cached_sections,
						cached_section_count,
						fname_pool,
						cached_gobjects))
				{
					resolved_offsets = {
						0,
						fname_pool,
						cached_gobjects.address,
						0,
						cached_image_size,
						cached_image_checksum,
						DEFAULT_VIEWPORT_TICK_INDEX,
						DEFAULT_PROCESS_EVENT_INDEX,
						ResolutionSource::Semantic,
					};
					static_resolution_succeeded = true;
				}
				else
				{
					ResolvedOffsets candidate{};
					if (detail::ResolveInSections(
							cached_sections,
							cached_section_count,
							cached_image_base,
							cached_image_size,
							candidate))
					{
						candidate.image_checksum = cached_image_checksum;
						resolved_offsets = candidate;
						static_resolution_succeeded = true;
						resolution_succeeded = true;
						return true;
					}
					next_dynamic_resolution_tick =
						GetTickCount64() + DYNAMIC_RESOLUTION_RETRY_MS;
				}
			}
		}

		if (!static_resolution_succeeded)
		{
			const ULONGLONG now = GetTickCount64();
			if (now < next_dynamic_resolution_tick)
				return false;
			next_dynamic_resolution_tick = now + DYNAMIC_RESOLUTION_RETRY_MS;
			uintptr_t fname_pool = 0;
			if (!FindSemanticAnchors(
					cached_sections,
					cached_section_count,
					fname_pool,
					cached_gobjects))
				return false;
			resolved_offsets = {
				0,
				fname_pool,
				cached_gobjects.address,
				0,
				cached_image_size,
				cached_image_checksum,
				DEFAULT_VIEWPORT_TICK_INDEX,
				DEFAULT_PROCESS_EVENT_INDEX,
				ResolutionSource::Semantic,
			};
			static_resolution_succeeded = true;
			next_dynamic_resolution_tick = 0;
		}
		const ULONGLONG now = GetTickCount64();
		if (now < next_dynamic_resolution_tick)
			return false;
		next_dynamic_resolution_tick = now + DYNAMIC_RESOLUTION_RETRY_MS;
		if (resolved_offsets.source == ResolutionSource::KnownProfile &&
			resolved_offsets.gworld_address != 0)
		{
			uintptr_t world = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(resolved_offsets.gworld_address),
					0,
					world) || !IsCanonicalPointer(world) ||
				!IsConsistentWorldCandidate(world))
				return false;
			resolution_succeeded = true;
			return true;
		}

		uintptr_t gworld = 0;
		if (!ResolveSemanticGWorld(
				cached_sections,
				cached_section_count,
				resolved_offsets.fname_pool_address,
				cached_gobjects,
				gworld))
			return false;
		resolved_offsets.gworld_address = gworld;
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
