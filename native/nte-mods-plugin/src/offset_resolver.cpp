#include "offset_resolver.hpp"

#include "find_offsets_scanner.hpp"
#include "memory_access.hpp"

#include <Windows.h>

namespace nte::mods::offsets
{
	namespace
	{
		constexpr ULONGLONG RESOLUTION_RETRY_MS = 1000;

		ResolvedOffsets resolved_offsets{};
		volatile LONG resolution_state = 0;
		volatile LONG64 next_retry_tick = 0;
	}

	bool Initialize(void* cancellation_event)
	{
		if (InterlockedCompareExchange(&resolution_state, 0, 0) == 2)
			return true;

		const ULONGLONG now = GetTickCount64();
		const ULONGLONG retry_at = static_cast<ULONGLONG>(
			InterlockedCompareExchange64(&next_retry_tick, 0, 0));
		if (now < retry_at)
			return false;
		if (InterlockedCompareExchange(&resolution_state, 1, 0) != 0)
			return false;

		ResolvedOffsets candidate{};
		wchar_t error[256]{};
		const bool resolved = find_offsets::ResolveCurrentProcess(
			candidate, error, _countof(error), cancellation_event);
		if (resolved)
		{
			resolved_offsets = candidate;
			MemoryBarrier();
			InterlockedExchange(&resolution_state, 2);
			return true;
		}

		InterlockedExchange64(
			&next_retry_tick,
			static_cast<LONG64>(GetTickCount64() + RESOLUTION_RETRY_MS));
		InterlockedExchange(&resolution_state, 0);
		return false;
	}

	const ResolvedOffsets* Get()
	{
		return InterlockedCompareExchange(&resolution_state, 0, 0) == 2
			? &resolved_offsets
			: nullptr;
	}

	namespace detail
	{
		bool DecodeNameFromPool(
			uintptr_t fname_pool_address,
			int32_t comparison_index,
			uint32_t number,
			wchar_t* output,
			size_t output_capacity,
			size_t& output_length)
		{
			constexpr size_t current_block_offset = 0x08;
			constexpr size_t current_cursor_offset = 0x0C;
			constexpr size_t blocks_offset = 0x10;
			constexpr uint32_t max_blocks = 0x2000;
			constexpr uint32_t max_block_bytes = 0x20000;
			constexpr uint32_t max_name_length = 0x3FF;
			output_length = 0;
			if (fname_pool_address == 0 || comparison_index < 0 ||
				output == nullptr || output_capacity == 0)
				return false;

			uint32_t current_block = 0;
			uint32_t current_cursor = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					current_block_offset,
					current_block) ||
				!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					current_cursor_offset,
					current_cursor) ||
				current_block >= max_blocks || current_cursor == 0 ||
				current_cursor >= max_block_bytes)
				return false;

			const uint32_t raw_index = static_cast<uint32_t>(comparison_index);
			const uint32_t block_index = raw_index >> 16;
			const size_t entry_offset =
				static_cast<size_t>(raw_index & 0xFFFF) * 2;
			if (block_index > current_block || block_index >= max_blocks ||
				entry_offset > max_block_bytes - sizeof(uint16_t))
				return false;

			uintptr_t block = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(fname_pool_address),
					blocks_offset + block_index * sizeof(uintptr_t),
					block) || block < 0x10000 || block >= 0x0000800000000000ULL)
				return false;

			uint16_t header = 0;
			if (!memory::ReadValue(
					reinterpret_cast<const void*>(block), entry_offset, header))
				return false;
			const bool wide = (header & 1) != 0;
			const size_t length = (header >> 6) & max_name_length;
			if (length == 0 || length > max_name_length)
				return false;
			const size_t byte_length = length * (wide ? sizeof(wchar_t) : 1);
			if (byte_length >
				max_block_bytes - entry_offset - sizeof(uint16_t))
				return false;
			const uintptr_t text_address =
				block + entry_offset + sizeof(uint16_t);
			if (!memory::IsReadableRange(
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
			const size_t required =
				length + (suffix_length == 0 ? 0 : suffix_length + 1);
			if (required >= output_capacity)
				return false;

			if (wide)
			{
				const auto* text =
					reinterpret_cast<const wchar_t*>(text_address);
				for (size_t index = 0; index < length; ++index)
					output[index] = text[index];
			}
			else
			{
				const auto* text =
					reinterpret_cast<const uint8_t*>(text_address);
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
	} // namespace detail
} // namespace nte::mods::offsets
