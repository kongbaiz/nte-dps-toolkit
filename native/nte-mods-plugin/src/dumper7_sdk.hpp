#pragma once

#include "offset_resolver.hpp"

#include <Windows.h>

#include <cstddef>
#include <filesystem>

namespace nte::mods::dumper7
{
	bool GenerateCppSdk(
		const offsets::ResolvedOffsets& resolved,
		const std::filesystem::path& staging_root,
		const char* checksum_hex,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity) noexcept;
} // namespace nte::mods::dumper7
