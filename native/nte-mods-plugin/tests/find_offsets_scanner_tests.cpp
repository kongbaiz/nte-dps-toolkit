#include "../src/find_offsets_scanner.hpp"

#include <Windows.h>

#include <array>
#include <cstdint>
#include <cstdio>
#include <cstring>

namespace
{
	template <typename T, size_t Size>
	void Store(std::array<uint8_t, Size>& buffer, size_t offset, T value)
	{
		std::memcpy(buffer.data() + offset, &value, sizeof(value));
	}

	bool DecodeNamePoolFixture()
	{
		std::array<uint8_t, 0x300> pool{};
		std::array<uint8_t, 0x300> block{};
		Store<uint32_t>(pool, 0x08, 0);
		Store<uint32_t>(pool, 0x0C, 0x100);
		Store<uintptr_t>(
			pool,
			0x10,
			reinterpret_cast<uintptr_t>(block.data()));
		Store<uint16_t>(block, 0, static_cast<uint16_t>(4 << 6));
		std::memcpy(block.data() + 2, "None", 4);

		wchar_t output[32]{};
		size_t length = 0;
		if (!nte::mods::offsets::detail::DecodeNameFromPool(
				reinterpret_cast<uintptr_t>(pool.data()),
				0,
				0,
				output,
				_countof(output),
				length) ||
			length != 4 || std::wcscmp(output, L"None") != 0)
			return false;

		Store<uint16_t>(block, 2, static_cast<uint16_t>(3 << 6));
		std::memcpy(block.data() + 4, "Foo", 3);
		return nte::mods::offsets::detail::DecodeNameFromPool(
			reinterpret_cast<uintptr_t>(pool.data()),
			1,
			2,
			output,
			_countof(output),
			length) && length == 5 && std::wcscmp(output, L"Foo_1") == 0;
	}

	bool RipRelativeFixture()
	{
		constexpr uint64_t instruction = 0x140001000;
		std::array<uint8_t, 7> code{
			0x48, 0x8B, 0x05, 0x20, 0x00, 0x00, 0x00,
		};
		uint64_t target = 0;
		if (!nte::mods::offsets::find_offsets::detail::TryRipRelativeTarget(
				code.data(), code.size(), instruction, target) ||
			target != instruction + code.size() + 0x20)
			return false;
		code[1] = 0x90;
		return !nte::mods::offsets::find_offsets::detail::TryRipRelativeTarget(
			code.data(), code.size(), instruction, target);
	}

	bool ResolvedSetValidationFixture()
	{
		constexpr uintptr_t image_base = 0x140000000;
		nte::mods::offsets::ResolvedOffsets candidate{
			image_base + 0x1000,
			image_base + 0x2000,
			image_base + 0x3000,
			image_base + 0x4000,
			image_base + 0x5000,
			0x10000,
			100,
			0x4C,
			nte::mods::offsets::ResolutionSource::FindOffsets,
		};
		if (!nte::mods::offsets::find_offsets::detail::ValidateResolvedOffsets(
				candidate, image_base))
			return false;
		candidate.process_event_address = image_base + candidate.image_size;
		if (nte::mods::offsets::find_offsets::detail::ValidateResolvedOffsets(
				candidate, image_base))
			return false;
		candidate.process_event_address = image_base + 0x5000;
		candidate.source = nte::mods::offsets::ResolutionSource::None;
		return !nte::mods::offsets::find_offsets::detail::ValidateResolvedOffsets(
			candidate, image_base);
	}

	bool CancellationFixture()
	{
		HANDLE event = CreateEventW(nullptr, TRUE, TRUE, nullptr);
		if (event == nullptr)
			return false;
		nte::mods::offsets::ResolvedOffsets result{};
		wchar_t error[128]{};
		const bool resolved =
			nte::mods::offsets::find_offsets::ResolveCurrentProcess(
				result, error, _countof(error), event);
		CloseHandle(event);
		return !resolved && std::wcsstr(error, L"cancelled") != nullptr;
	}
}

int main()
{
	const bool rip_relative = RipRelativeFixture();
	const bool resolved_set = ResolvedSetValidationFixture();
	const bool name_pool = DecodeNamePoolFixture();
	const bool cancellation = CancellationFixture();
	std::printf(
		"FIND_OFFSETS_SCANNER_TEST rip_relative=%s resolved_set=%s name_pool=%s cancellation=%s known_profiles=absent\n",
		rip_relative ? "true" : "false",
		resolved_set ? "true" : "false",
		name_pool ? "true" : "false",
		cancellation ? "true" : "false");
	return rip_relative && resolved_set && name_pool && cancellation ? 0 : 1;
}
