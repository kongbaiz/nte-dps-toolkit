#include "../src/dumper7_sdk.hpp"
#include "../src/sdk_cache.hpp"

#include <Windows.h>

#include <array>
#include <cstdint>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

namespace nte::mods::offsets
{
	bool Initialize(void*) { return false; }
	const ResolvedOffsets* Get() { return nullptr; }
}

namespace nte::mods::dumper7
{
	bool GenerateCppSdk(
		const offsets::ResolvedOffsets&,
		const std::filesystem::path&,
		const char*,
		HANDLE,
		wchar_t*,
		size_t) noexcept
	{
		return false;
	}
}

namespace
{
	constexpr size_t PACKAGE_HEADER_BYTES = 64;
	constexpr size_t PACKAGE_INTEGRITY_BYTES = 32;

	void Write(const std::filesystem::path& path, const std::string& text)
	{
		std::filesystem::create_directories(path.parent_path());
		std::ofstream output(path, std::ios::binary | std::ios::trunc);
		output.write(text.data(), static_cast<std::streamsize>(text.size()));
	}

	std::string Read(const std::filesystem::path& path)
	{
		std::ifstream input(path, std::ios::binary);
		return std::string(
			std::istreambuf_iterator<char>(input),
			std::istreambuf_iterator<char>());
	}

	uint32_t ReadU32(const std::string& bytes, size_t offset)
	{
		uint32_t value = 0;
		for (size_t index = 0; index < sizeof(value); ++index)
			value |= static_cast<uint32_t>(
				static_cast<unsigned char>(bytes[offset + index])) << (index * 8);
		return value;
	}

	uint64_t ReadU64(const std::string& bytes, size_t offset)
	{
		uint64_t value = 0;
		for (size_t index = 0; index < sizeof(value); ++index)
			value |= static_cast<uint64_t>(
				static_cast<unsigned char>(bytes[offset + index])) << (index * 8);
		return value;
	}

	bool CorruptPackageMember(
		const std::filesystem::path& package_path,
		const std::string& member)
	{
		std::string package = Read(package_path);
		if (package.size() <= PACKAGE_HEADER_BYTES + PACKAGE_INTEGRITY_BYTES)
			return false;
		const size_t payload_end = package.size() - PACKAGE_INTEGRITY_BYTES;
		size_t offset = PACKAGE_HEADER_BYTES;
		while (offset < payload_end)
		{
			if (payload_end - offset < 16)
				return false;
			const uint32_t path_length = ReadU32(package, offset);
			const uint32_t chunk_count = ReadU32(package, offset + 4);
			const uint64_t file_size = ReadU64(package, offset + 8);
			offset += 16;
			if (path_length == 0 || path_length > payload_end - offset)
				return false;
			const std::string path = package.substr(offset, path_length);
			offset += path_length;
			uint64_t parsed_size = 0;
			for (uint32_t chunk_index = 0; chunk_index < chunk_count; ++chunk_index)
			{
				if (payload_end - offset < 8)
					return false;
				const uint32_t raw_size = ReadU32(package, offset);
				const uint32_t compressed_size = ReadU32(package, offset + 4);
				offset += 8;
				if (compressed_size == 0 || compressed_size > payload_end - offset)
					return false;
				if (path == member)
				{
					package[offset + compressed_size / 2] ^= static_cast<char>(0x5a);
					Write(package_path, package);
					return true;
				}
				offset += compressed_size;
				parsed_size += raw_size;
			}
			if (parsed_size != file_size)
				return false;
		}
		return false;
	}

	std::string ReadPackageFile(
		const nte::mods::sdk_cache::CacheContext& context,
		const std::filesystem::path& path)
	{
		std::vector<uint8_t> contents;
		wchar_t error[512]{};
		if (!nte::mods::sdk_cache::ReadSdkFile(
				context,
				path,
				contents,
				error,
				_countof(error)))
			return {};
		return std::string(contents.begin(), contents.end());
	}

	bool FakeGenerate(
		const nte::mods::offsets::ResolvedOffsets&,
		const std::filesystem::path& staging_root,
		const char* checksum,
		HANDLE,
		wchar_t*,
		size_t) noexcept
	{
		try
		{
			const auto sdk = staging_root / L"CppSDK";
			Write(sdk / L"SDK.hpp", "sdk");
			Write(sdk / L"PropertyFixup.hpp", "fixup");
			Write(sdk / L"UnrealContainers.hpp", "containers");
			Write(sdk / L"SDK" / L"Basic.hpp", "basic-hpp");
			Write(sdk / L"SDK" / L"Basic.cpp", "basic-cpp");
			Write(sdk / L"generation.txt", checksum);
			Write(sdk / L"SDK" / L"CompressionProbe.cpp", std::string(512 * 1024, 'A'));
			return true;
		}
		catch (...)
		{
			return false;
		}
	}

	bool FailGenerate(
		const nte::mods::offsets::ResolvedOffsets&,
		const std::filesystem::path&,
		const char*,
		HANDLE,
		wchar_t*,
		size_t) noexcept
	{
		return false;
	}
}

int main()
{
	namespace fs = std::filesystem;
	wchar_t name[128]{};
	swprintf_s(
		name,
		L"nte-sdk-cache-tests-%lu-%llu",
		static_cast<unsigned long>(GetCurrentProcessId()),
		static_cast<unsigned long long>(GetTickCount64()));
	const fs::path root = fs::temp_directory_path() / name;
	const fs::path plugin = root / L"plugin";
	const fs::path executable = root / L"HTGame.exe";
	fs::create_directories(plugin);
	Write(executable, "version-one");

	wchar_t error[512]{};
	nte::mods::sdk_cache::CacheContext first{};
	const auto first_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, first, error, _countof(error));
	nte::mods::offsets::ResolvedOffsets resolved{};
	const auto first_publish = nte::mods::sdk_cache::Regenerate(
		first, resolved, &FakeGenerate, nullptr, error, _countof(error));
	nte::mods::sdk_cache::CacheContext reused{};
	const auto second_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, reused, error, _countof(error));
	const std::string first_generation = ReadPackageFile(reused, L"generation.txt");
	const std::string compression_probe =
		ReadPackageFile(reused, L"SDK/CompressionProbe.cpp");
	const std::string first_package = Read(plugin / L"NTE_SDK.bin");
	const bool single_compressed_package =
		fs::is_regular_file(plugin / L"NTE_SDK.bin") &&
		!fs::exists(plugin / L"NTE_SDK") &&
		!fs::exists(plugin / L"NTE_SDK.checksum") &&
		first_package.size() < compression_probe.size();

	const bool corruption_injected = CorruptPackageMember(
		plugin / L"NTE_SDK.bin", "SDK/CompressionProbe.cpp");
	nte::mods::sdk_cache::CacheContext corrupted{};
	const auto corrupted_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, corrupted, error, _countof(error));
	Write(plugin / L"NTE_SDK.bin", first_package);
	nte::mods::sdk_cache::CacheContext restored{};
	const auto restored_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, restored, error, _countof(error));

	Write(executable, "version-two");
	nte::mods::sdk_cache::CacheContext changed{};
	const auto changed_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, changed, error, _countof(error));
	const auto failed_publish = nte::mods::sdk_cache::Regenerate(
		changed, resolved, &FailGenerate, nullptr, error, _countof(error));
	const bool failure_preserved = Read(plugin / L"NTE_SDK.bin") == first_package;
	const auto changed_publish = nte::mods::sdk_cache::Regenerate(
		changed, resolved, &FakeGenerate, nullptr, error, _countof(error));
	nte::mods::sdk_cache::CacheContext final_context{};
	const auto final_inspection = nte::mods::sdk_cache::Inspect(
		executable, plugin, final_context, error, _countof(error));
	const std::string final_generation =
		ReadPackageFile(final_context, L"generation.txt");

	const bool passed =
		first_inspection == nte::mods::sdk_cache::InspectResult::RegenerationRequired &&
		first_publish == nte::mods::sdk_cache::PublishResult::Generated &&
		second_inspection == nte::mods::sdk_cache::InspectResult::Reusable &&
		!first_generation.empty() &&
		compression_probe.size() == 512 * 1024 &&
		single_compressed_package &&
		corruption_injected &&
		corrupted_inspection == nte::mods::sdk_cache::InspectResult::RegenerationRequired &&
		restored_inspection == nte::mods::sdk_cache::InspectResult::Reusable &&
		changed_inspection == nte::mods::sdk_cache::InspectResult::RegenerationRequired &&
		failed_publish == nte::mods::sdk_cache::PublishResult::Error &&
		failure_preserved &&
		changed_publish == nte::mods::sdk_cache::PublishResult::Generated &&
		final_inspection == nte::mods::sdk_cache::InspectResult::Reusable &&
		!final_generation.empty() &&
		final_generation != first_generation;
	std::printf(
		"SDK_CACHE_TEST first=regenerate second=reuse corruption=rejected restored=reuse mismatch=regenerate failed_publish_preserved=%s final=reuse package_bytes=%zu unpacked_probe_bytes=%zu single_package=%s\n",
		failure_preserved ? "true" : "false",
		Read(plugin / L"NTE_SDK.bin").size(),
		compression_probe.size(),
		single_compressed_package ? "true" : "false");
	std::error_code cleanup_error;
	fs::remove_all(root, cleanup_error);
	return passed ? 0 : 1;
}
