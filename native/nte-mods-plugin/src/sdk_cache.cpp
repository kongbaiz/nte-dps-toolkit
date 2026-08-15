#include "sdk_cache.hpp"

#include "dumper7_sdk.hpp"

#include <bcrypt.h>
#include <compressapi.h>

#include <algorithm>
#include <array>
#include <exception>
#include <limits>
#include <string>
#include <system_error>
#include <vector>

#pragma comment(lib, "Cabinet.lib")

namespace nte::mods::sdk_cache
{
	namespace
	{
		constexpr uint64_t MAX_EXECUTABLE_BYTES = 2ULL * 1024 * 1024 * 1024;
		constexpr uint64_t MAX_GENERATED_SDK_BYTES = 8ULL * 1024 * 1024 * 1024;
		constexpr uint64_t MAX_PACKAGE_BYTES = 10ULL * 1024 * 1024 * 1024;
		constexpr size_t MAX_GENERATED_SDK_FILES = 100000;
		constexpr size_t MAX_PACKAGE_PATH_BYTES = 4096;
		constexpr size_t MAX_READ_SDK_FILE_BYTES = 1024ULL * 1024 * 1024;
		constexpr DWORD HASH_READ_CHUNK = 1024 * 1024;
		constexpr DWORD PACKAGE_CHUNK_BYTES = 4 * 1024 * 1024;
		constexpr DWORD MAX_COMPRESSED_CHUNK_BYTES = PACKAGE_CHUNK_BYTES * 2 + 65536;
		constexpr DWORD OFFSET_RETRY_MS = 1000;
		constexpr wchar_t PACKAGE_FILE_NAME[] = L"NTE_SDK.bin";
		constexpr wchar_t LEGACY_SDK_DIRECTORY_NAME[] = L"NTE_SDK";
		constexpr wchar_t LEGACY_CHECKSUM_FILE_NAME[] = L"NTE_SDK.checksum";
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mods Plugin";
		constexpr wchar_t LEGACY_MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mod Loader";
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_VALUE[] = L"Workspace";
		constexpr std::array<uint8_t, 8> PACKAGE_MAGIC{
			'N', 'T', 'E', 'S', 'D', 'K', '0', '2',
		};
		constexpr uint32_t PACKAGE_VERSION = 2;
		constexpr uint32_t PACKAGE_ALGORITHM = COMPRESS_ALGORITHM_XPRESS_HUFF;
		constexpr std::array<const char*, 5> REQUIRED_SDK_FILES{
			"SDK.hpp",
			"PropertyFixup.hpp",
			"UnrealContainers.hpp",
			"SDK/Basic.hpp",
			"SDK/Basic.cpp",
		};

		struct SourceFile
		{
			std::filesystem::path path;
			std::string relative_path;
			uint64_t size;
		};

		void SetError(
			const wchar_t* message,
			wchar_t* output,
			size_t capacity) noexcept
		{
			if (output == nullptr || capacity == 0)
				return;
			output[0] = L'\0';
			if (message == nullptr)
				return;
			size_t index = 0;
			while (index + 1 < capacity && message[index] != L'\0')
			{
				output[index] = message[index];
				++index;
			}
			output[index] = L'\0';
		}

		void SetSystemError(
			const wchar_t* operation,
			DWORD code,
			wchar_t* output,
			size_t capacity) noexcept
		{
			wchar_t buffer[256]{};
			if (swprintf_s(
					buffer,
					L"%s failed with Windows error %lu",
					operation,
					static_cast<unsigned long>(code)) < 0)
				SetError(operation, output, capacity);
			else
				SetError(buffer, output, capacity);
		}

		bool ModulePath(
			HMODULE module,
			std::filesystem::path& result,
			wchar_t* error,
			size_t error_capacity)
		{
			std::vector<wchar_t> buffer(512);
			while (buffer.size() <= 32768)
			{
				const DWORD length = GetModuleFileNameW(
					module,
					buffer.data(),
					static_cast<DWORD>(buffer.size()));
				if (length == 0)
				{
					SetSystemError(
						L"GetModuleFileNameW",
						GetLastError(),
						error,
						error_capacity);
					return false;
				}
				if (length < buffer.size() - 1)
				{
					result = std::filesystem::path(buffer.data(), buffer.data() + length);
					return true;
				}
				buffer.resize(buffer.size() * 2);
			}
			SetError(L"module path exceeds 32768 characters", error, error_capacity);
			return false;
		}

		bool ReadWorkspaceFromRegistryKey(
			const wchar_t* registry_key,
			std::filesystem::path& workspace) noexcept
		{
			try
			{
				DWORD value_type = 0;
				DWORD byte_length = 0;
				LSTATUS status = RegGetValueW(
					HKEY_CURRENT_USER,
					registry_key,
					MOD_WORKSPACE_REGISTRY_VALUE,
					RRF_RT_REG_SZ,
					&value_type,
					nullptr,
					&byte_length);
				if (status != ERROR_SUCCESS || value_type != REG_SZ ||
					byte_length < 2 * sizeof(wchar_t) ||
					byte_length > 32768 * sizeof(wchar_t) ||
					byte_length % sizeof(wchar_t) != 0)
					return false;
				std::vector<wchar_t> buffer(byte_length / sizeof(wchar_t));
				status = RegGetValueW(
					HKEY_CURRENT_USER,
					registry_key,
					MOD_WORKSPACE_REGISTRY_VALUE,
					RRF_RT_REG_SZ,
					&value_type,
					buffer.data(),
					&byte_length);
				if (status != ERROR_SUCCESS || value_type != REG_SZ ||
					buffer.empty() || buffer.back() != L'\0')
					return false;
				workspace = std::filesystem::path(buffer.data());
				return workspace.is_absolute() && !workspace.empty();
			}
			catch (...)
			{
				return false;
			}
		}

		bool ReadModWorkspace(std::filesystem::path& workspace) noexcept
		{
			if (ReadWorkspaceFromRegistryKey(MOD_WORKSPACE_REGISTRY_KEY, workspace))
				return true;
			return ReadWorkspaceFromRegistryKey(
				LEGACY_MOD_WORKSPACE_REGISTRY_KEY,
				workspace);
		}

		bool ComputeSha256(
			const std::filesystem::path& path,
			std::array<uint8_t, SHA256_SIZE>& digest,
			wchar_t* error,
			size_t error_capacity)
		{
			HANDLE file = CreateFileW(
				path.c_str(),
				GENERIC_READ,
				FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
				nullptr,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
				nullptr);
			if (file == INVALID_HANDLE_VALUE)
			{
				SetSystemError(
					L"CreateFileW(executable)",
					GetLastError(),
					error,
					error_capacity);
				return false;
			}

			LARGE_INTEGER size{};
			if (!GetFileSizeEx(file, &size))
			{
				const DWORD code = GetLastError();
				CloseHandle(file);
				SetSystemError(L"GetFileSizeEx", code, error, error_capacity);
				return false;
			}
			if (size.QuadPart <= 0 ||
				static_cast<uint64_t>(size.QuadPart) > MAX_EXECUTABLE_BYTES)
			{
				CloseHandle(file);
				SetError(
					L"executable size is outside the 1..2 GiB checksum budget",
					error,
					error_capacity);
				return false;
			}

			BCRYPT_ALG_HANDLE algorithm = nullptr;
			BCRYPT_HASH_HANDLE hash = nullptr;
			std::vector<uint8_t> hash_object;
			bool succeeded = false;
			do
			{
				if (BCryptOpenAlgorithmProvider(
						&algorithm,
						BCRYPT_SHA256_ALGORITHM,
						nullptr,
						0) < 0)
				{
					SetError(
						L"BCryptOpenAlgorithmProvider(SHA-256) failed",
						error,
						error_capacity);
					break;
				}
				DWORD object_length = 0;
				DWORD returned = 0;
				if (BCryptGetProperty(
						algorithm,
						BCRYPT_OBJECT_LENGTH,
						reinterpret_cast<PUCHAR>(&object_length),
						sizeof(object_length),
						&returned,
						0) < 0 ||
					returned != sizeof(object_length) || object_length == 0 ||
					object_length > 1024 * 1024)
				{
					SetError(
						L"BCrypt SHA-256 object length is invalid",
						error,
						error_capacity);
					break;
				}
				hash_object.resize(object_length);
				if (BCryptCreateHash(
						algorithm,
						&hash,
						hash_object.data(),
						static_cast<ULONG>(hash_object.size()),
						nullptr,
						0,
						0) < 0)
				{
					SetError(L"BCryptCreateHash(SHA-256) failed", error, error_capacity);
					break;
				}

				std::vector<uint8_t> buffer(HASH_READ_CHUNK);
				for (;;)
				{
					DWORD bytes_read = 0;
					if (!ReadFile(
							file,
							buffer.data(),
							static_cast<DWORD>(buffer.size()),
							&bytes_read,
							nullptr))
					{
						SetSystemError(
							L"ReadFile(executable)",
							GetLastError(),
							error,
							error_capacity);
						break;
					}
					if (bytes_read == 0)
					{
						if (BCryptFinishHash(
								hash,
								digest.data(),
								static_cast<ULONG>(digest.size()),
								0) < 0)
							SetError(
								L"BCryptFinishHash(SHA-256) failed",
								error,
								error_capacity);
						else
							succeeded = true;
						break;
					}
					if (BCryptHashData(hash, buffer.data(), bytes_read, 0) < 0)
					{
						SetError(L"BCryptHashData(SHA-256) failed", error, error_capacity);
						break;
					}
				}
			} while (false);

			if (hash != nullptr)
				BCryptDestroyHash(hash);
			if (algorithm != nullptr)
				BCryptCloseAlgorithmProvider(algorithm, 0);
			CloseHandle(file);
			return succeeded;
		}

		void FormatChecksum(
			const std::array<uint8_t, SHA256_SIZE>& digest,
			std::array<char, SHA256_HEX_SIZE + 1>& output) noexcept
		{
			constexpr char HEX[] = "0123456789abcdef";
			for (size_t index = 0; index < digest.size(); ++index)
			{
				output[index * 2] = HEX[digest[index] >> 4];
				output[index * 2 + 1] = HEX[digest[index] & 0x0F];
			}
			output[SHA256_HEX_SIZE] = '\0';
		}

		bool HasRequiredSdkFiles(const std::filesystem::path& sdk)
		{
			std::error_code error;
			if (std::filesystem::is_symlink(sdk, error) || error ||
				!std::filesystem::is_directory(sdk, error) || error)
				return false;
			const std::filesystem::path required[]{
				sdk / L"SDK.hpp",
				sdk / L"PropertyFixup.hpp",
				sdk / L"UnrealContainers.hpp",
				sdk / L"SDK" / L"Basic.hpp",
				sdk / L"SDK" / L"Basic.cpp",
			};
			for (const std::filesystem::path& path : required)
			{
				if (std::filesystem::is_symlink(path, error) || error ||
					!std::filesystem::is_regular_file(path, error) || error ||
					std::filesystem::file_size(path, error) == 0 || error)
					return false;
			}
			return true;
		}

		bool IsSafePackagePath(const std::string& path)
		{
			if (path.empty() || path.size() > MAX_PACKAGE_PATH_BYTES ||
				path.front() == '/' || path.back() == '/' ||
				path.find('\\') != std::string::npos ||
				path.find('\0') != std::string::npos)
				return false;
			size_t begin = 0;
			while (begin < path.size())
			{
				const size_t end = path.find('/', begin);
				const size_t length =
					(end == std::string::npos ? path.size() : end) - begin;
				if (length == 0 ||
					(length == 1 && path[begin] == '.') ||
					(length == 2 && path[begin] == '.' && path[begin + 1] == '.'))
					return false;
				if (end == std::string::npos)
					break;
				begin = end + 1;
			}
			return true;
		}

		bool PathToPackageString(
			const std::filesystem::path& path,
			std::string& result)
		{
			if (path.empty() || path.is_absolute())
				return false;
			const std::u8string utf8 = path.lexically_normal().generic_u8string();
			result.clear();
			result.reserve(utf8.size());
			for (const char8_t value : utf8)
				result.push_back(static_cast<char>(value));
			return IsSafePackagePath(result);
		}

		bool RelativeUtf8Path(
			const std::filesystem::path& root,
			const std::filesystem::path& path,
			std::string& result)
		{
			std::error_code error;
			const std::filesystem::path relative =
				std::filesystem::relative(path, root, error);
			return !error && PathToPackageString(relative, result);
		}

		bool CollectSourceFiles(
			const std::filesystem::path& sdk,
			std::vector<SourceFile>& files,
			uint64_t& total_bytes,
			wchar_t* error,
			size_t error_capacity)
		{
			files.clear();
			total_bytes = 0;
			std::error_code fs_error;
			for (std::filesystem::recursive_directory_iterator iterator(
					sdk,
					std::filesystem::directory_options::skip_permission_denied,
					fs_error), end;
				!fs_error && iterator != end;
				iterator.increment(fs_error))
			{
				if (iterator->is_symlink(fs_error) || fs_error)
				{
					SetError(L"generated SDK contains a symlink", error, error_capacity);
					return false;
				}
				if (!iterator->is_regular_file(fs_error))
				{
					if (fs_error)
						break;
					continue;
				}
				if (files.size() == MAX_GENERATED_SDK_FILES)
				{
					SetError(L"generated SDK contains too many files", error, error_capacity);
					return false;
				}
				const uintmax_t file_size = iterator->file_size(fs_error);
				if (fs_error || file_size > MAX_GENERATED_SDK_BYTES ||
					static_cast<uint64_t>(file_size) > MAX_GENERATED_SDK_BYTES - total_bytes)
				{
					SetError(L"generated SDK exceeds its byte budget", error, error_capacity);
					return false;
				}
				std::string relative_path;
				if (!RelativeUtf8Path(sdk, iterator->path(), relative_path))
				{
					SetError(L"generated SDK contains an invalid relative path", error, error_capacity);
					return false;
				}
				files.push_back(SourceFile{
					iterator->path(),
					std::move(relative_path),
					static_cast<uint64_t>(file_size),
				});
				total_bytes += static_cast<uint64_t>(file_size);
			}
			if (fs_error || files.empty() || total_bytes == 0)
			{
				SetError(L"generated SDK could not be enumerated", error, error_capacity);
				return false;
			}
			std::sort(
				files.begin(),
				files.end(),
				[](const SourceFile& left, const SourceFile& right)
				{
					return left.relative_path < right.relative_path;
				});
			return true;
		}

		bool WriteExact(
			HANDLE file,
			const void* data,
			size_t size,
			wchar_t* error,
			size_t error_capacity)
		{
			const auto* bytes = static_cast<const uint8_t*>(data);
			while (size != 0)
			{
				const DWORD chunk = static_cast<DWORD>(std::min<size_t>(
					size,
					std::numeric_limits<DWORD>::max()));
				DWORD written = 0;
				if (!WriteFile(file, bytes, chunk, &written, nullptr) || written != chunk)
				{
					SetSystemError(L"WriteFile(SDK package)", GetLastError(), error, error_capacity);
					return false;
				}
				bytes += written;
				size -= written;
			}
			return true;
		}

		bool ReadExact(
			HANDLE file,
			void* data,
			size_t size,
			wchar_t* error,
			size_t error_capacity)
		{
			auto* bytes = static_cast<uint8_t*>(data);
			while (size != 0)
			{
				const DWORD chunk = static_cast<DWORD>(std::min<size_t>(
					size,
					std::numeric_limits<DWORD>::max()));
				DWORD read = 0;
				if (!ReadFile(file, bytes, chunk, &read, nullptr) || read != chunk)
				{
					SetError(L"SDK package is truncated", error, error_capacity);
					return false;
				}
				bytes += read;
				size -= read;
			}
			return true;
		}

		bool HashCurrentRange(
			HANDLE file,
			uint64_t byte_count,
			std::array<uint8_t, SHA256_SIZE>& digest,
			wchar_t* error,
			size_t error_capacity)
		{
			if (file == INVALID_HANDLE_VALUE || byte_count == 0)
			{
				SetError(L"SDK package integrity range is invalid", error, error_capacity);
				return false;
			}

			BCRYPT_ALG_HANDLE algorithm = nullptr;
			BCRYPT_HASH_HANDLE hash = nullptr;
			std::vector<uint8_t> hash_object;
			bool succeeded = false;
			do
			{
				if (BCryptOpenAlgorithmProvider(
						&algorithm,
						BCRYPT_SHA256_ALGORITHM,
						nullptr,
						0) < 0)
				{
					SetError(L"BCryptOpenAlgorithmProvider(SDK package SHA-256) failed", error, error_capacity);
					break;
				}
				DWORD object_length = 0;
				DWORD returned = 0;
				if (BCryptGetProperty(
						algorithm,
						BCRYPT_OBJECT_LENGTH,
						reinterpret_cast<PUCHAR>(&object_length),
						sizeof(object_length),
						&returned,
						0) < 0 ||
					returned != sizeof(object_length) || object_length == 0 ||
					object_length > 1024 * 1024)
				{
					SetError(L"SDK package SHA-256 object length is invalid", error, error_capacity);
					break;
				}
				hash_object.resize(object_length);
				if (BCryptCreateHash(
						algorithm,
						&hash,
						hash_object.data(),
						static_cast<ULONG>(hash_object.size()),
						nullptr,
						0,
						0) < 0)
				{
					SetError(L"BCryptCreateHash(SDK package SHA-256) failed", error, error_capacity);
					break;
				}

				std::vector<uint8_t> buffer(HASH_READ_CHUNK);
				uint64_t remaining = byte_count;
				while (remaining != 0)
				{
					const DWORD chunk = static_cast<DWORD>(std::min<uint64_t>(remaining, buffer.size()));
					DWORD bytes_read = 0;
					if (!ReadFile(file, buffer.data(), chunk, &bytes_read, nullptr) || bytes_read != chunk)
					{
						SetError(L"SDK package integrity payload is truncated", error, error_capacity);
						break;
					}
					if (BCryptHashData(hash, buffer.data(), bytes_read, 0) < 0)
					{
						SetError(L"BCryptHashData(SDK package SHA-256) failed", error, error_capacity);
						break;
					}
					remaining -= bytes_read;
				}
				if (remaining != 0)
					break;
				if (BCryptFinishHash(
						hash,
						digest.data(),
						static_cast<ULONG>(digest.size()),
						0) < 0)
				{
					SetError(L"BCryptFinishHash(SDK package SHA-256) failed", error, error_capacity);
					break;
				}
				succeeded = true;
			} while (false);

			if (hash != nullptr)
				BCryptDestroyHash(hash);
			if (algorithm != nullptr)
				BCryptCloseAlgorithmProvider(algorithm, 0);
			return succeeded;
		}

		template <typename T>
		bool WriteScalar(
			HANDLE file,
			T value,
			wchar_t* error,
			size_t error_capacity)
		{
			return WriteExact(file, &value, sizeof(value), error, error_capacity);
		}

		template <typename T>
		bool ReadScalar(
			HANDLE file,
			T& value,
			wchar_t* error,
			size_t error_capacity)
		{
			return ReadExact(file, &value, sizeof(value), error, error_capacity);
		}

		bool SeekForward(
			HANDLE file,
			uint32_t bytes,
			wchar_t* error,
			size_t error_capacity)
		{
			LARGE_INTEGER distance{};
			distance.QuadPart = bytes;
			if (!SetFilePointerEx(file, distance, nullptr, FILE_CURRENT))
			{
				SetSystemError(L"SetFilePointerEx(SDK package)", GetLastError(), error, error_capacity);
				return false;
			}
			return true;
		}

		bool CompressChunk(
			const uint8_t* input,
			size_t input_size,
			std::vector<uint8_t>& output,
			wchar_t* error,
			size_t error_capacity)
		{
			COMPRESSOR_HANDLE compressor = nullptr;
			if (!CreateCompressor(PACKAGE_ALGORITHM, nullptr, &compressor))
			{
				SetSystemError(L"CreateCompressor", GetLastError(), error, error_capacity);
				return false;
			}
			SIZE_T required = 0;
			const BOOL sized = Compress(
				compressor,
				input,
				input_size,
				nullptr,
				0,
				&required);
			const DWORD sizing_error = GetLastError();
			if (sized || sizing_error != ERROR_INSUFFICIENT_BUFFER || required == 0 ||
				required > MAX_COMPRESSED_CHUNK_BYTES)
			{
				CloseCompressor(compressor);
				SetError(L"XPRESS-Huffman compressed chunk size is invalid", error, error_capacity);
				return false;
			}
			output.resize(required);
			SIZE_T compressed_size = 0;
			const BOOL compressed = Compress(
				compressor,
				input,
				input_size,
				output.data(),
				output.size(),
				&compressed_size);
			const DWORD compression_error = compressed ? ERROR_SUCCESS : GetLastError();
			CloseCompressor(compressor);
			if (!compressed || compressed_size == 0 || compressed_size > output.size())
			{
				if (!compressed)
					SetSystemError(
						L"Compress(XPRESS-Huffman)",
						compression_error,
						error,
						error_capacity);
				else
					SetError(L"XPRESS-Huffman output size is invalid", error, error_capacity);
				return false;
			}
			output.resize(compressed_size);
			return true;
		}

		bool DecompressChunk(
			const uint8_t* input,
			size_t input_size,
			uint8_t* output,
			size_t output_size,
			wchar_t* error,
			size_t error_capacity)
		{
			DECOMPRESSOR_HANDLE decompressor = nullptr;
			if (!CreateDecompressor(PACKAGE_ALGORITHM, nullptr, &decompressor))
			{
				SetSystemError(L"CreateDecompressor", GetLastError(), error, error_capacity);
				return false;
			}
			SIZE_T decompressed_size = 0;
			const BOOL decompressed = Decompress(
				decompressor,
				input,
				input_size,
				output,
				output_size,
				&decompressed_size);
			const DWORD decompression_error = decompressed ? ERROR_SUCCESS : GetLastError();
			CloseDecompressor(decompressor);
			if (!decompressed || decompressed_size != output_size)
			{
				if (!decompressed)
					SetSystemError(
						L"Decompress(XPRESS-Huffman)",
						decompression_error,
						error,
						error_capacity);
				else
					SetError(L"XPRESS-Huffman decompressed size is invalid", error, error_capacity);
				return false;
			}
			return true;
		}

		bool WritePackageTemp(
			const std::filesystem::path& package,
			const std::filesystem::path& sdk,
			const CacheContext& context,
			HANDLE stop_event,
			wchar_t* error,
			size_t error_capacity)
		{
			std::vector<SourceFile> files;
			uint64_t total_bytes = 0;
			if (!CollectSourceFiles(sdk, files, total_bytes, error, error_capacity))
				return false;

			HANDLE output = CreateFileW(
				package.c_str(),
				GENERIC_READ | GENERIC_WRITE,
				0,
				nullptr,
				CREATE_NEW,
				FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
				nullptr);
			if (output == INVALID_HANDLE_VALUE)
			{
				SetSystemError(L"CreateFileW(SDK package temp)", GetLastError(), error, error_capacity);
				return false;
			}

			bool succeeded = false;
			do
			{
				const uint32_t file_count = static_cast<uint32_t>(files.size());
				const uint32_t reserved = 0;
				if (!WriteExact(output, PACKAGE_MAGIC.data(), PACKAGE_MAGIC.size(), error, error_capacity) ||
					!WriteScalar(output, PACKAGE_VERSION, error, error_capacity) ||
					!WriteScalar(output, PACKAGE_ALGORITHM, error, error_capacity) ||
					!WriteExact(output, context.checksum.data(), context.checksum.size(), error, error_capacity) ||
					!WriteScalar(output, file_count, error, error_capacity) ||
					!WriteScalar(output, reserved, error, error_capacity) ||
					!WriteScalar(output, total_bytes, error, error_capacity))
					break;

				std::vector<uint8_t> raw(PACKAGE_CHUNK_BYTES);
				std::vector<uint8_t> compressed;
				bool all_files_succeeded = true;
				for (const SourceFile& source : files)
				{
					if (stop_event != nullptr &&
						WaitForSingleObject(stop_event, 0) != WAIT_TIMEOUT)
					{
						SetError(L"SDK package compression was cancelled", error, error_capacity);
						all_files_succeeded = false;
						break;
					}
					const uint32_t path_length = static_cast<uint32_t>(source.relative_path.size());
					const uint64_t chunk_count_64 =
						(source.size + PACKAGE_CHUNK_BYTES - 1) / PACKAGE_CHUNK_BYTES;
					if (chunk_count_64 > std::numeric_limits<uint32_t>::max())
					{
						SetError(L"SDK package file has too many chunks", error, error_capacity);
						all_files_succeeded = false;
						break;
					}
					const uint32_t chunk_count = static_cast<uint32_t>(chunk_count_64);
					if (!WriteScalar(output, path_length, error, error_capacity) ||
						!WriteScalar(output, chunk_count, error, error_capacity) ||
						!WriteScalar(output, source.size, error, error_capacity) ||
						!WriteExact(output, source.relative_path.data(), source.relative_path.size(), error, error_capacity))
					{
						all_files_succeeded = false;
						break;
					}

					HANDLE input = CreateFileW(
						source.path.c_str(),
						GENERIC_READ,
						FILE_SHARE_READ,
						nullptr,
						OPEN_EXISTING,
						FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
						nullptr);
					if (input == INVALID_HANDLE_VALUE)
					{
						SetSystemError(L"CreateFileW(generated SDK file)", GetLastError(), error, error_capacity);
						all_files_succeeded = false;
						break;
					}

					bool file_succeeded = true;
					uint64_t remaining = source.size;
					for (uint32_t chunk_index = 0; chunk_index < chunk_count; ++chunk_index)
					{
						if (stop_event != nullptr &&
							WaitForSingleObject(stop_event, 0) != WAIT_TIMEOUT)
						{
							SetError(L"SDK package compression was cancelled", error, error_capacity);
							file_succeeded = false;
							break;
						}
						const DWORD raw_size = static_cast<DWORD>(std::min<uint64_t>(
							remaining,
							PACKAGE_CHUNK_BYTES));
						if (raw_size == 0 ||
							!ReadExact(input, raw.data(), raw_size, error, error_capacity) ||
							!CompressChunk(raw.data(), raw_size, compressed, error, error_capacity))
						{
							file_succeeded = false;
							break;
						}
						const uint32_t compressed_size = static_cast<uint32_t>(compressed.size());
						if (!WriteScalar(output, raw_size, error, error_capacity) ||
							!WriteScalar(output, compressed_size, error, error_capacity) ||
							!WriteExact(output, compressed.data(), compressed.size(), error, error_capacity))
						{
							file_succeeded = false;
							break;
						}
						remaining -= raw_size;
					}
					CloseHandle(input);
					if (!file_succeeded || remaining != 0)
					{
						if (file_succeeded)
							SetError(L"generated SDK file changed while packaging", error, error_capacity);
						all_files_succeeded = false;
						break;
					}
				}
				if (!all_files_succeeded)
					break;

				LARGE_INTEGER zero{};
				LARGE_INTEGER payload_position{};
				if (!SetFilePointerEx(output, zero, &payload_position, FILE_CURRENT) ||
					payload_position.QuadPart <= 0 ||
					static_cast<uint64_t>(payload_position.QuadPart) > MAX_PACKAGE_BYTES - SHA256_SIZE)
				{
					SetError(L"SDK package payload exceeds its byte budget", error, error_capacity);
					break;
				}
				if (!FlushFileBuffers(output))
				{
					SetSystemError(L"FlushFileBuffers(SDK package payload)", GetLastError(), error, error_capacity);
					break;
				}
				LARGE_INTEGER begin{};
				if (!SetFilePointerEx(output, begin, nullptr, FILE_BEGIN))
				{
					SetSystemError(L"SetFilePointerEx(SDK package begin)", GetLastError(), error, error_capacity);
					break;
				}
				std::array<uint8_t, SHA256_SIZE> integrity{};
				if (!HashCurrentRange(
						output,
						static_cast<uint64_t>(payload_position.QuadPart),
						integrity,
						error,
						error_capacity))
					break;
				LARGE_INTEGER payload_end{};
				payload_end.QuadPart = payload_position.QuadPart;
				if (!SetFilePointerEx(output, payload_end, nullptr, FILE_BEGIN) ||
					!WriteExact(output, integrity.data(), integrity.size(), error, error_capacity))
				{
					SetError(L"SDK package integrity trailer could not be written", error, error_capacity);
					break;
				}
				if (!FlushFileBuffers(output))
				{
					SetSystemError(L"FlushFileBuffers(SDK package)", GetLastError(), error, error_capacity);
					break;
				}
				succeeded = true;
			} while (false);

			CloseHandle(output);
			if (!succeeded)
			{
				std::error_code remove_error;
				std::filesystem::remove(package, remove_error);
			}
			return succeeded;
		}

		bool ReadPackage(
			const CacheContext& context,
			const std::string* requested_path,
			std::vector<uint8_t>* requested_contents,
			bool validate_required,
			wchar_t* error,
			size_t error_capacity)
		{
			HANDLE package = CreateFileW(
				context.package_file.c_str(),
				GENERIC_READ,
				FILE_SHARE_READ | FILE_SHARE_DELETE,
				nullptr,
				OPEN_EXISTING,
				FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
				nullptr);
			if (package == INVALID_HANDLE_VALUE)
				return false;

			LARGE_INTEGER package_size{};
			if (!GetFileSizeEx(package, &package_size) ||
				package_size.QuadPart <= static_cast<LONGLONG>(SHA256_SIZE) ||
				static_cast<uint64_t>(package_size.QuadPart) > MAX_PACKAGE_BYTES)
			{
				CloseHandle(package);
				return false;
			}

			bool succeeded = false;
			do
			{
				const uint64_t payload_size =
					static_cast<uint64_t>(package_size.QuadPart) - SHA256_SIZE;
				std::array<uint8_t, SHA256_SIZE> actual_integrity{};
				std::array<uint8_t, SHA256_SIZE> expected_integrity{};
				if (!HashCurrentRange(
						package,
						payload_size,
						actual_integrity,
						error,
						error_capacity) ||
					!ReadExact(
						package,
						expected_integrity.data(),
						expected_integrity.size(),
						error,
						error_capacity))
					break;
				if (actual_integrity != expected_integrity)
				{
					SetError(L"SDK package integrity check failed", error, error_capacity);
					break;
				}
				LARGE_INTEGER begin{};
				if (!SetFilePointerEx(package, begin, nullptr, FILE_BEGIN))
				{
					SetSystemError(L"SetFilePointerEx(SDK package begin)", GetLastError(), error, error_capacity);
					break;
				}

				std::array<uint8_t, PACKAGE_MAGIC.size()> magic{};
				uint32_t version = 0;
				uint32_t algorithm = 0;
				std::array<uint8_t, SHA256_SIZE> checksum{};
				uint32_t file_count = 0;
				uint32_t reserved = 0;
				uint64_t declared_total = 0;
				if (!ReadExact(package, magic.data(), magic.size(), error, error_capacity) ||
					!ReadScalar(package, version, error, error_capacity) ||
					!ReadScalar(package, algorithm, error, error_capacity) ||
					!ReadExact(package, checksum.data(), checksum.size(), error, error_capacity) ||
					!ReadScalar(package, file_count, error, error_capacity) ||
					!ReadScalar(package, reserved, error, error_capacity) ||
					!ReadScalar(package, declared_total, error, error_capacity))
					break;
				if (magic != PACKAGE_MAGIC || version != PACKAGE_VERSION ||
					algorithm != PACKAGE_ALGORITHM || checksum != context.checksum ||
					reserved != 0 || file_count == 0 ||
					file_count > MAX_GENERATED_SDK_FILES ||
					declared_total == 0 || declared_total > MAX_GENERATED_SDK_BYTES)
					break;

				std::array<bool, REQUIRED_SDK_FILES.size()> required_seen{};
				bool requested_seen = false;
				bool parse_valid = true;
				uint64_t parsed_total = 0;
				std::vector<uint8_t> compressed;
				std::vector<uint8_t> raw;
				for (uint32_t file_index = 0; file_index < file_count; ++file_index)
				{
					uint32_t path_length = 0;
					uint32_t chunk_count = 0;
					uint64_t file_size = 0;
					if (!ReadScalar(package, path_length, error, error_capacity) ||
						!ReadScalar(package, chunk_count, error, error_capacity) ||
						!ReadScalar(package, file_size, error, error_capacity) ||
						path_length == 0 || path_length > MAX_PACKAGE_PATH_BYTES ||
						file_size > MAX_GENERATED_SDK_BYTES ||
						file_size > MAX_GENERATED_SDK_BYTES - parsed_total)
					{
						parse_valid = false;
						break;
					}
					const uint64_t expected_chunks =
						(file_size + PACKAGE_CHUNK_BYTES - 1) / PACKAGE_CHUNK_BYTES;
					if (expected_chunks != chunk_count)
					{
						parse_valid = false;
						break;
					}
					std::string path(path_length, '\0');
					if (!ReadExact(package, path.data(), path.size(), error, error_capacity) ||
						!IsSafePackagePath(path))
					{
						parse_valid = false;
						break;
					}

					int required_index = -1;
					for (size_t index = 0; index < REQUIRED_SDK_FILES.size(); ++index)
					{
						if (path == REQUIRED_SDK_FILES[index])
						{
							if (required_seen[index])
							{
								required_index = -2;
								break;
							}
							required_seen[index] = true;
							required_index = static_cast<int>(index);
							break;
						}
					}
					if (required_index == -2)
					{
						parse_valid = false;
						break;
					}
					const bool requested = requested_path != nullptr && path == *requested_path;
					if (requested && requested_seen)
					{
						parse_valid = false;
						break;
					}
					if (requested)
					{
						requested_seen = true;
						if (requested_contents == nullptr || file_size > MAX_READ_SDK_FILE_BYTES)
						{
							parse_valid = false;
							break;
						}
						requested_contents->clear();
						requested_contents->reserve(static_cast<size_t>(file_size));
					}
					const bool decompress_file =
						requested || (validate_required && required_index >= 0);
					uint64_t file_parsed = 0;
					for (uint32_t chunk_index = 0; chunk_index < chunk_count; ++chunk_index)
					{
						uint32_t raw_size = 0;
						uint32_t compressed_size = 0;
						if (!ReadScalar(package, raw_size, error, error_capacity) ||
							!ReadScalar(package, compressed_size, error, error_capacity) ||
							raw_size == 0 || raw_size > PACKAGE_CHUNK_BYTES ||
							compressed_size == 0 || compressed_size > MAX_COMPRESSED_CHUNK_BYTES ||
							raw_size > file_size - file_parsed)
						{
							parse_valid = false;
							break;
						}
						if (decompress_file)
						{
							compressed.resize(compressed_size);
							raw.resize(raw_size);
							if (!ReadExact(package, compressed.data(), compressed.size(), error, error_capacity) ||
								!DecompressChunk(
									compressed.data(),
									compressed.size(),
									raw.data(),
									raw.size(),
									error,
									error_capacity))
							{
								parse_valid = false;
								break;
							}
							if (requested)
								requested_contents->insert(
									requested_contents->end(), raw.begin(), raw.end());
						}
						else if (!SeekForward(package, compressed_size, error, error_capacity))
						{
							parse_valid = false;
							break;
						}
						file_parsed += raw_size;
					}
					if (!parse_valid || file_parsed != file_size ||
						(validate_required && required_index >= 0 && file_size == 0) ||
						(requested && requested_contents->size() != file_size))
					{
						parse_valid = false;
						break;
					}
					parsed_total += file_size;
				}
				if (!parse_valid)
					break;
				LARGE_INTEGER zero{};
				LARGE_INTEGER position{};
				if (!SetFilePointerEx(package, zero, &position, FILE_CURRENT) ||
					static_cast<uint64_t>(position.QuadPart) != payload_size ||
					parsed_total != declared_total ||
					(validate_required &&
						!std::all_of(required_seen.begin(), required_seen.end(), [](bool value) { return value; })) ||
					(requested_path != nullptr && !requested_seen))
					break;
				succeeded = true;
			} while (false);

			CloseHandle(package);
			return succeeded;
		}

		bool ValidatePackage(const CacheContext& context)
		{
			wchar_t ignored[256]{};
			return ReadPackage(context, nullptr, nullptr, true, ignored, _countof(ignored));
		}

		std::filesystem::path UniqueSibling(
			const std::filesystem::path& directory,
			const wchar_t* stem)
		{
			const unsigned long process_id = GetCurrentProcessId();
			const unsigned long long tick = GetTickCount64();
			wchar_t name[128]{};
			swprintf_s(
				name,
				L".%s.%lu.%llu",
				stem,
				process_id,
				tick);
			return directory / name;
		}

		bool IsDirectChild(
			const std::filesystem::path& parent,
			const std::filesystem::path& child)
		{
			return !parent.empty() && child.parent_path() == parent &&
				!child.filename().empty();
		}

		void RemoveLegacyCache(const std::filesystem::path& directory)
		{
			std::error_code error;
			std::filesystem::remove_all(directory / LEGACY_SDK_DIRECTORY_NAME, error);
			error.clear();
			std::filesystem::remove(directory / LEGACY_CHECKSUM_FILE_NAME, error);
		}
	} // namespace

	InspectResult InspectImpl(
		const std::filesystem::path& executable_path,
		const std::filesystem::path& plugin_directory,
		CacheContext& context,
		wchar_t* error,
		size_t error_capacity)
	{
		context = {};
		if (error != nullptr && error_capacity != 0)
			error[0] = L'\0';
		if (executable_path.empty() || plugin_directory.empty())
		{
			SetError(L"SDK cache paths are empty", error, error_capacity);
			return InspectResult::Error;
		}
		context.executable_path = executable_path;
		context.plugin_directory = plugin_directory;
		context.package_file = plugin_directory / PACKAGE_FILE_NAME;
		if (!ComputeSha256(
				context.executable_path,
				context.checksum,
				error,
				error_capacity))
			return InspectResult::Error;
		FormatChecksum(context.checksum, context.checksum_hex);
		return ValidatePackage(context)
			? InspectResult::Reusable
			: InspectResult::RegenerationRequired;
	}

	PublishResult RegenerateImpl(
		const CacheContext& context,
		const offsets::ResolvedOffsets& resolved,
		GenerateSdk generator,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity)
	{
		if (error != nullptr && error_capacity != 0)
			error[0] = L'\0';
		if (generator == nullptr ||
			!IsDirectChild(context.plugin_directory, context.package_file))
		{
			SetError(L"SDK cache regeneration arguments are invalid", error, error_capacity);
			return PublishResult::Error;
		}
		std::error_code fs_error;
		if (!std::filesystem::is_directory(context.plugin_directory, fs_error) || fs_error)
		{
			SetError(L"plugin directory is unavailable", error, error_capacity);
			return PublishResult::Error;
		}

		const std::filesystem::path staging_root =
			UniqueSibling(context.plugin_directory, L"NTE_SDK.tmp");
		const std::filesystem::path generated_sdk = staging_root / L"CppSDK";
		const std::filesystem::path package_temp =
			UniqueSibling(context.plugin_directory, L"NTE_SDK.bin.tmp");
		if (!std::filesystem::create_directory(staging_root, fs_error) || fs_error)
		{
			SetError(L"could not create SDK staging directory", error, error_capacity);
			return PublishResult::Error;
		}

		const bool generated = generator(
			resolved,
			staging_root,
			context.checksum_hex.data(),
			stop_event,
			error,
			error_capacity);
		if (!generated || !HasRequiredSdkFiles(generated_sdk))
		{
			if (generated)
				SetError(L"generated SDK is incomplete", error, error_capacity);
			std::filesystem::remove_all(staging_root, fs_error);
			return PublishResult::Error;
		}

		if (!WritePackageTemp(
				package_temp,
				generated_sdk,
				context,
				stop_event,
				error,
				error_capacity))
		{
			std::filesystem::remove_all(staging_root, fs_error);
			return PublishResult::Error;
		}

		if (!MoveFileExW(
				package_temp.c_str(),
				context.package_file.c_str(),
				MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH))
		{
			SetSystemError(L"MoveFileExW(SDK package)", GetLastError(), error, error_capacity);
			std::filesystem::remove(package_temp, fs_error);
			std::filesystem::remove_all(staging_root, fs_error);
			return PublishResult::Error;
		}

		std::filesystem::remove_all(staging_root, fs_error);
		RemoveLegacyCache(context.plugin_directory);
		return PublishResult::Generated;
	}

	DWORD RunWorkerImpl(void* opaque)
	{
		const auto* worker = static_cast<const WorkerContext*>(opaque);
		if (worker == nullptr || worker->plugin_module == nullptr ||
			worker->stop_event == nullptr)
			return ERROR_INVALID_PARAMETER;

		std::filesystem::path executable;
		std::filesystem::path plugin_file;
		wchar_t error[512]{};
		if (!ModulePath(nullptr, executable, error, _countof(error)) ||
			!ModulePath(worker->plugin_module, plugin_file, error, _countof(error)))
		{
			OutputDebugStringW(L"NTE Mods plugin SDK cache: ");
			OutputDebugStringW(error);
			OutputDebugStringW(L"\n");
			return ERROR_INVALID_DATA;
		}
		const std::filesystem::path legacy_game_cache_directory = plugin_file.parent_path();

		std::filesystem::path plugin;
		while (WaitForSingleObject(worker->stop_event, 0) == WAIT_TIMEOUT)
		{
			if (ReadModWorkspace(plugin))
				break;
			if (WaitForSingleObject(worker->stop_event, OFFSET_RETRY_MS) != WAIT_TIMEOUT)
				return ERROR_CANCELLED;
		}
		if (plugin.empty())
			return ERROR_CANCELLED;

		CacheContext context{};
		const InspectResult inspection = InspectImpl(
			executable, plugin, context, error, _countof(error));
		if (inspection == InspectResult::Reusable)
		{
			RemoveLegacyCache(legacy_game_cache_directory);
			OutputDebugStringW(L"NTE Mods plugin: reusing checksum-matched compressed SDK package.\n");
			return ERROR_SUCCESS;
		}
		if (inspection == InspectResult::Error)
		{
			OutputDebugStringW(L"NTE Mods plugin SDK cache inspection failed: ");
			OutputDebugStringW(error);
			OutputDebugStringW(L"\n");
			return ERROR_INVALID_DATA;
		}

		while (WaitForSingleObject(worker->stop_event, 0) == WAIT_TIMEOUT)
		{
			if (offsets::Initialize(worker->stop_event))
				break;
			if (WaitForSingleObject(worker->stop_event, OFFSET_RETRY_MS) != WAIT_TIMEOUT)
				return ERROR_CANCELLED;
		}
		const offsets::ResolvedOffsets* resolved = offsets::Get();
		if (resolved == nullptr)
			return ERROR_CANCELLED;

		const PublishResult published = RegenerateImpl(
			context,
			*resolved,
			&dumper7::GenerateCppSdk,
			worker->stop_event,
			error,
			_countof(error));
		if (published != PublishResult::Generated)
		{
			OutputDebugStringW(L"NTE Mods plugin SDK generation failed: ");
			OutputDebugStringW(error);
			OutputDebugStringW(L"\n");
			return ERROR_WRITE_FAULT;
		}
		RemoveLegacyCache(legacy_game_cache_directory);
		OutputDebugStringW(L"NTE Mods plugin: generated and cached the compressed SDK package.\n");
		return ERROR_SUCCESS;
	}

	InspectResult Inspect(
		const std::filesystem::path& executable_path,
		const std::filesystem::path& plugin_directory,
		CacheContext& context,
		wchar_t* error,
		size_t error_capacity) noexcept
	{
		try
		{
			return InspectImpl(
				executable_path,
				plugin_directory,
				context,
				error,
				error_capacity);
		}
		catch (const std::filesystem::filesystem_error&)
		{
			SetError(L"SDK cache inspection raised a filesystem exception", error, error_capacity);
		}
		catch (const std::exception&)
		{
			SetError(L"SDK cache inspection raised a C++ exception", error, error_capacity);
		}
		catch (...)
		{
			SetError(L"SDK cache inspection raised an unknown exception", error, error_capacity);
		}
		return InspectResult::Error;
	}

	PublishResult Regenerate(
		const CacheContext& context,
		const offsets::ResolvedOffsets& resolved,
		GenerateSdk generator,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity) noexcept
	{
		try
		{
			return RegenerateImpl(
				context,
				resolved,
				generator,
				stop_event,
				error,
				error_capacity);
		}
		catch (const std::filesystem::filesystem_error&)
		{
			SetError(L"SDK cache generation raised a filesystem exception", error, error_capacity);
		}
		catch (const std::exception&)
		{
			SetError(L"SDK cache generation raised a C++ exception", error, error_capacity);
		}
		catch (...)
		{
			SetError(L"SDK cache generation raised an unknown exception", error, error_capacity);
		}
		return PublishResult::Error;
	}

	bool ReadSdkFile(
		const CacheContext& context,
		const std::filesystem::path& relative_path,
		std::vector<uint8_t>& contents,
		wchar_t* error,
		size_t error_capacity) noexcept
	{
		try
		{
			if (error != nullptr && error_capacity != 0)
				error[0] = L'\0';
			std::string requested;
			if (!PathToPackageString(relative_path, requested))
			{
				SetError(L"requested SDK package path is invalid", error, error_capacity);
				return false;
			}
			return ReadPackage(context, &requested, &contents, false, error, error_capacity);
		}
		catch (const std::exception&)
		{
			SetError(L"SDK package read raised a C++ exception", error, error_capacity);
		}
		catch (...)
		{
			SetError(L"SDK package read raised an unknown exception", error, error_capacity);
		}
		return false;
	}

	DWORD WINAPI RunWorker(void* context)
	{
		try
		{
			return RunWorkerImpl(context);
		}
		catch (const std::exception&)
		{
			OutputDebugStringW(L"NTE Mods plugin SDK cache worker raised a C++ exception.\n");
		}
		catch (...)
		{
			OutputDebugStringW(L"NTE Mods plugin SDK cache worker raised an unknown exception.\n");
		}
		return ERROR_UNHANDLED_EXCEPTION;
	}
} // namespace nte::mods::sdk_cache
