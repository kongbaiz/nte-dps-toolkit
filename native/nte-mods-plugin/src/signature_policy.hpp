#pragma once

#include <cstddef>
#include <cstdint>

namespace nte::signature
{
	enum class SelectionResult : uint8_t
	{
		None,
		Unique,
		Ambiguous,
	};

	struct BytePattern
	{
		const uint8_t* bytes;
		const uint8_t* mask;
		size_t size;
	};

	constexpr bool Matches(
		const uint8_t* input,
		size_t input_size,
		const BytePattern& pattern) noexcept
	{
		if (input == nullptr || pattern.bytes == nullptr ||
			pattern.mask == nullptr || pattern.size == 0 ||
			input_size < pattern.size)
			return false;

		for (size_t index = 0; index < pattern.size; ++index)
		{
			if ((input[index] & pattern.mask[index]) !=
				(pattern.bytes[index] & pattern.mask[index]))
				return false;
		}
		return true;
	}

	constexpr size_t Find(
		const uint8_t* input,
		size_t input_size,
		size_t begin,
		size_t end,
		const BytePattern& pattern) noexcept
	{
		if (input == nullptr || begin > input_size ||
			pattern.size == 0 || pattern.size > input_size)
			return input_size;
		if (end > input_size)
			end = input_size;
		if (begin > end || pattern.size > end - begin)
			return input_size;

		for (size_t offset = begin; offset <= end - pattern.size; ++offset)
		{
			if (Matches(input + offset, input_size - offset, pattern))
				return offset;
		}
		return input_size;
	}

	template <typename Matcher>
	constexpr SelectionResult SelectUniqueIndex(
		size_t begin,
		size_t end,
		Matcher matches,
		size_t& result) noexcept
	{
		if (begin > end)
			return SelectionResult::None;

		bool found = false;
		for (size_t index = begin;; ++index)
		{
			if (matches(index))
			{
				if (found)
					return SelectionResult::Ambiguous;
				result = index;
				found = true;
			}
			if (index == end)
				break;
		}
		return found ? SelectionResult::Unique : SelectionResult::None;
	}
} // namespace nte::signature
