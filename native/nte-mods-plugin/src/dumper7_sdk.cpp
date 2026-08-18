#include "dumper7_sdk.hpp"

#include "Generators/CppGenerator.h"
#include "Generators/Generator.h"
#include "Settings.h"

#include <Windows.h>

#include <array>
#include <climits>
#include <cstdio>
#include <exception>
#include <string>

namespace nte::mods::dumper7
{
		namespace
		{
		// thread_local 在 manual map 注入下不可靠: loader 不会为手动映射的
		// 模块初始化 _tls_index, TlsGetValue(0) 读到的是其他 DLL 的 TLS 槽,
		// 取消检测会被垃圾句柄误触发。SDK 生成只在 worker 线程内执行,
		// 普通全局即可。
		HANDLE sdk_stop_event = nullptr;

		bool IsSdkCancellationRequested() noexcept
		{
			return sdk_stop_event != nullptr &&
				WaitForSingleObject(sdk_stop_event, 0) == WAIT_OBJECT_0;
		}

		struct GenerateContext
		{
			const offsets::ResolvedOffsets* resolved;
			const std::filesystem::path* staging_root;
			const char* checksum_hex;
			HANDLE stop_event;
			wchar_t* error;
			size_t error_capacity;
		};

		class CancellationScope
		{
		public:
			explicit CancellationScope(HANDLE event) noexcept
				: previous_(sdk_stop_event)
			{
				sdk_stop_event = event;
				Settings::NteRuntimeOffsets::CancellationRequested =
					&IsSdkCancellationRequested;
			}

			~CancellationScope()
			{
				sdk_stop_event = previous_;
			}

			CancellationScope(const CancellationScope&) = delete;
			CancellationScope& operator=(const CancellationScope&) = delete;

		private:
			HANDLE previous_;
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

		void SetUtf8Error(
			const char* message,
			wchar_t* output,
			size_t capacity) noexcept
		{
			if (output == nullptr || capacity == 0)
				return;
			output[0] = L'\0';
			if (message == nullptr)
				return;
			const size_t bounded_capacity = capacity > static_cast<size_t>(INT_MAX)
				? static_cast<size_t>(INT_MAX)
				: capacity;
			const int output_size = static_cast<int>(bounded_capacity);
			if (MultiByteToWideChar(
					CP_UTF8,
					0,
					message,
					-1,
					output,
					output_size) == 0)
				output[0] = L'\0';
			output[bounded_capacity - 1] = L'\0';
		}

		bool ToRva(uintptr_t address, uintptr_t image_base, int32& output) noexcept
		{
			if (address < image_base)
				return false;
			const uintptr_t delta = address - image_base;
			if (delta == 0 || delta > static_cast<uintptr_t>(INT32_MAX))
				return false;
			output = static_cast<int32>(delta);
			return true;
		}

		bool GenerateImpl(GenerateContext& context)
		{
			if (context.resolved == nullptr || context.staging_root == nullptr ||
				context.staging_root->empty() || context.checksum_hex == nullptr)
			{
				SetError(L"Dumper-7 SDK generation arguments are invalid", context.error, context.error_capacity);
				return false;
			}
			for (size_t index = 0; index < 64; ++index)
			{
				const char value = context.checksum_hex[index];
				if (!((value >= '0' && value <= '9') ||
					(value >= 'a' && value <= 'f')))
				{
					SetError(L"Dumper-7 checksum is not a lowercase SHA-256 value", context.error, context.error_capacity);
					return false;
				}
			}
			if (context.checksum_hex[64] != '\0' ||
				context.resolved->source != offsets::ResolutionSource::FindOffsets)
			{
				SetError(L"Dumper-7 requires a complete find_offsets result", context.error, context.error_capacity);
				return false;
			}
			const uintptr_t image_base = reinterpret_cast<uintptr_t>(
				GetModuleHandleW(nullptr));
			if (image_base == 0)
			{
				SetError(L"Dumper-7 could not resolve the executable image base", context.error, context.error_capacity);
				return false;
			}

			int32 gobjects = 0;
			int32 append_name = 0;
			int32 fname_pool = 0;
			int32 gworld = 0;
			int32 process_event = 0;
			if (!ToRva(context.resolved->gobjects_address, image_base, gobjects) ||
				!ToRva(context.resolved->append_name_address, image_base, append_name) ||
				!ToRva(context.resolved->fname_pool_address, image_base, fname_pool) ||
				!ToRva(context.resolved->gworld_address, image_base, gworld) ||
				!ToRva(context.resolved->process_event_address, image_base, process_event) ||
				context.resolved->process_event_index > INT32_MAX ||
				context.resolved->viewport_tick_index > INT32_MAX)
			{
				SetError(L"find_offsets returned a Dumper-7 offset outside the supported RVA range", context.error, context.error_capacity);
				return false;
			}

			Settings::NteRuntimeOffsets::OffsetGObjects = gobjects;
			Settings::NteRuntimeOffsets::OffsetAppendString = append_name;
			Settings::NteRuntimeOffsets::OffsetGNames = fname_pool;
			Settings::NteRuntimeOffsets::OffsetGWorld = gworld;
			Settings::NteRuntimeOffsets::OffsetProcessEvent = process_event;
			Settings::NteRuntimeOffsets::IndexProcessEvent =
				static_cast<int32>(context.resolved->process_event_index);
			Settings::NteRuntimeOffsets::IndexViewportTick =
				static_cast<int32>(context.resolved->viewport_tick_index);
			Settings::Config::SDKNamespaceName = "SDK";
			Settings::Generator::GameName = "NTE";
			Settings::Generator::GameVersion.assign(
				context.checksum_hex,
				context.checksum_hex + 16);
			Settings::Generator::SDKGenerationPath = *context.staging_root;

			CancellationScope cancellation(context.stop_event);
			Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
			Generator::InitEngineCore();
			Generator::InitInternal();
			Generator::Generate<CppGenerator>();
			Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
			return true;
		}

		int GenerateNoexcept(GenerateContext& context) noexcept
		{
			try
			{
				return GenerateImpl(context) ? 1 : 0;
			}
			catch (const std::exception& exception)
			{
				SetUtf8Error(exception.what(), context.error, context.error_capacity);
				return 0;
			}
			catch (...)
			{
				SetError(L"Dumper-7 SDK generation raised an unknown C++ exception", context.error, context.error_capacity);
				return 0;
			}
		}

		int GenerateGuarded(GenerateContext& context) noexcept
		{
			__try
			{
				return GenerateNoexcept(context);
			}
			__except (EXCEPTION_EXECUTE_HANDLER)
			{
				wchar_t message[128]{};
				swprintf_s(
					message,
					L"Dumper-7 SDK generation stopped on structured exception 0x%08lX",
					static_cast<unsigned long>(GetExceptionCode()));
				SetError(message, context.error, context.error_capacity);
				return 0;
			}
		}
	} // namespace

	bool GenerateCppSdk(
		const offsets::ResolvedOffsets& resolved,
		const std::filesystem::path& staging_root,
		const char* checksum_hex,
		HANDLE stop_event,
		wchar_t* error,
		size_t error_capacity) noexcept
	{
		if (error != nullptr && error_capacity != 0)
			error[0] = L'\0';
		GenerateContext context{
			&resolved,
			&staging_root,
			checksum_hex,
			stop_event,
			error,
			error_capacity,
		};
		return GenerateGuarded(context) == 1;
	}
} // namespace nte::mods::dumper7
