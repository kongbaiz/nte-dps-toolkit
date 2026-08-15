#pragma once

#include "offset_resolver.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>
#include <filesystem>
#include <vector>

namespace nte::mods::sdk_cache
{
	constexpr size_t SHA256_SIZE = 32;
	constexpr size_t SHA256_HEX_SIZE = SHA256_SIZE * 2;

	struct CacheContext
	{
		std::filesystem::path executable_path;
		std::filesystem::path plugin_directory;
		std::filesystem::path package_file;
		std::array<uint8_t, SHA256_SIZE> checksum;
		std::array<char, SHA256_HEX_SIZE + 1> checksum_hex;
	};

	using GenerateSdk = bool(*)(
		const offsets::ResolvedOffsets& resolved,
		const std::filesystem::path& staging_root,
		const char* checksum_hex,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity) noexcept;

	enum class InspectResult : uint8_t
	{
		Reusable,
		RegenerationRequired,
		Error,
	};

	enum class PublishResult : uint8_t
	{
		Generated,
		Error,
	};

	InspectResult Inspect(
		const std::filesystem::path& executable_path,
		const std::filesystem::path& plugin_directory,
		CacheContext& context,
		wchar_t* error,
		size_t error_capacity) noexcept;

	PublishResult Regenerate(
		const CacheContext& context,
		const offsets::ResolvedOffsets& resolved,
		GenerateSdk generator,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity) noexcept;

	bool ReadSdkFile(
		const CacheContext& context,
		const std::filesystem::path& relative_path,
		std::vector<uint8_t>& contents,
		wchar_t* error,
		size_t error_capacity) noexcept;

	struct WorkerContext
	{
		HMODULE plugin_module;
		HANDLE stop_event;
	};

	DWORD WINAPI RunWorker(void* context);
} // namespace nte::mods::sdk_cache
