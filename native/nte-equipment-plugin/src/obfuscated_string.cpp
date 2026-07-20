#include "obfuscated_string.hpp"

namespace nte::obfuscation
{
	namespace
	{
		template <typename Character>
		void DecodeStringImpl(
			const Character* encrypted,
			Character* output,
			size_t count,
			uint8_t seed) noexcept
		{
			const volatile Character* source = encrypted;
			for (size_t index = 0; index < count; ++index)
				output[index] = source[index] ^ MaskAt<Character>(index, seed);
		}
	} // namespace

	__declspec(noinline) void DecodeString(
		const char* encrypted,
		char* output,
		size_t count,
		uint8_t seed) noexcept
	{
		DecodeStringImpl(encrypted, output, count, seed);
	}

	__declspec(noinline) void DecodeString(
		const wchar_t* encrypted,
		wchar_t* output,
		size_t count,
		uint8_t seed) noexcept
	{
		DecodeStringImpl(encrypted, output, count, seed);
	}
} // namespace nte::obfuscation
