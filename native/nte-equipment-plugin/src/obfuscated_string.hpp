#pragma once

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::obfuscation
{
	void DecodeString(
		const char* encrypted,
		char* output,
		size_t count,
		uint8_t seed) noexcept;
	void DecodeString(
		const wchar_t* encrypted,
		wchar_t* output,
		size_t count,
		uint8_t seed) noexcept;

	template <typename Character>
	constexpr Character MaskAt(size_t index, uint8_t seed) noexcept
	{
		uint32_t value = static_cast<uint32_t>(seed) + 0x6Du +
			static_cast<uint32_t>(index) * 0x3Du;
		value ^= value >> 3;
		return static_cast<Character>((value & 0x7Fu) + 1u);
	}

	template <typename Character, size_t Size>
	class DecodedString
	{
	public:
		const Character* c_str() const noexcept
		{
			return value_.data();
		}

		constexpr size_t size() const noexcept
		{
			return Size;
		}

	private:
		template <typename, size_t, uint8_t>
		friend class EncryptedString;

		std::array<Character, Size> value_{};
	};

	template <typename Character, size_t Size, uint8_t Seed>
	class EncryptedString
	{
	public:
		consteval explicit EncryptedString(const Character (&value)[Size])
		{
			for (size_t index = 0; index < Size; ++index)
				encrypted_[index] = value[index] ^ MaskAt<Character>(index, Seed);
		}

		DecodedString<Character, Size> Decode() const noexcept
		{
			DecodedString<Character, Size> output;
			DecodeString(
				encrypted_.data(), output.value_.data(), Size, Seed);
			return output;
		}

	private:
		std::array<Character, Size> encrypted_{};
	};

	template <uint8_t Seed, typename Character, size_t Size>
	consteval auto MakeEncrypted(const Character (&value)[Size])
	{
		return EncryptedString<Character, Size, Seed>(value);
	}
} // namespace nte::obfuscation

#define NTE_OBFUSCATE_STRING(value)                                            \
	[]() {                                                                     \
		static constexpr auto encrypted =                                        \
			::nte::obfuscation::MakeEncrypted<                                    \
				static_cast<uint8_t>((__LINE__ * 131u + __COUNTER__ * 17u) & 0xFFu) \
			>(value);                                                             \
		return encrypted.Decode();                                               \
	}()
