#include "plugin_runtime.hpp"

#include "equipment_rpc.hpp"
#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"
#include "shadow_vtable_hook.hpp"
#include "viewport_hook_policy.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::equipment
{
	namespace
	{
		constexpr size_t WORLD_GAME_INSTANCE_OFFSET = 0x230;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_CONTROLLER_OFFSET = 0x30;
		constexpr size_t LOCAL_PLAYER_VIEWPORT_OFFSET = 0x78;
		constexpr size_t VIEWPORT_WORLD_OFFSET = 0x78;
		constexpr size_t VIEWPORT_GAME_INSTANCE_OFFSET = 0x80;
		constexpr size_t CONTROLLER_PLAYER_STATE_OFFSET = 0x2D0;
		constexpr size_t VIEWPORT_TICK_INDEX = 100;
		constexpr DWORD VIEWPORT_BOOTSTRAP_RETRY_MS = 250;
		constexpr uint32_t VIEWPORT_REBIND_CHECK_TICKS = 120;

		constexpr std::array<uint8_t, 22> VIEWPORT_TICK_PREFIX{
			0x4C, 0x89, 0x74, 0x24, 0x20, 0x55, 0x48, 0x8D, 0x6C, 0x24, 0xD0,
			0x48, 0x81, 0xEC, 0x30, 0x01, 0x00, 0x00, 0x4C, 0x8B, 0xF1, 0xE8,
		};
		constexpr std::array<uint8_t, 12> VIEWPORT_TICK_SUFFIX{
			0x49, 0x8B, 0x06, 0x49, 0x8B, 0xCE, 0xFF, 0x90, 0x80, 0x01, 0x00, 0x00,
		};

		struct LocalPlayerArray
		{
			void** data;
			int32_t count;
			int32_t capacity;
		};

		using ViewportTick = void(__fastcall*)(void*, float);

		constinit nte::hook::ShadowVTableHook viewport_hooks[2];
		size_t active_viewport_hook_index = 0;
		void* hooked_viewport = nullptr;
		ViewportTick original_viewport_tick = nullptr;
		uint32_t viewport_rebind_tick_count = 0;
		bool ipc_dispatch_in_progress = false;

		static_assert(sizeof(LocalPlayerArray) == 16);

		bool InstallViewportHook(void* viewport);

		bool BytesEqual(const void* left, const void* right, size_t size)
		{
			const auto* left_bytes = static_cast<const uint8_t*>(left);
			const auto* right_bytes = static_cast<const uint8_t*>(right);
			for (size_t index = 0; index < size; ++index)
			{
				if (left_bytes[index] != right_bytes[index])
					return false;
			}
			return true;
		}

		bool EqualsAsciiCaseInsensitive(const wchar_t* left, const wchar_t* right)
		{
			for (;; ++left, ++right)
			{
				wchar_t left_value = *left;
				wchar_t right_value = *right;
				if (left_value >= L'A' && left_value <= L'Z')
					left_value += L'a' - L'A';
				if (right_value >= L'A' && right_value <= L'Z')
					right_value += L'a' - L'A';
				if (left_value != right_value)
					return false;
				if (left_value == L'\0')
					return true;
			}
		}


		void DebugLog(const wchar_t* message)
		{
		#if defined(_DEBUG)
			OutputDebugStringW(message);
		#else
			static_cast<void>(message);
		#endif
		}

		void* ResolveLocalPlayer(const void* game_instance)
		{
			LocalPlayerArray local_players{};
			if (!memory::ReadValue(
				game_instance,
				GAME_INSTANCE_LOCAL_PLAYERS_OFFSET,
				local_players) ||
				local_players.data == nullptr || local_players.count < 1 ||
				local_players.capacity < local_players.count ||
				!memory::IsReadableRange(local_players.data, sizeof(*local_players.data)))
				return nullptr;

			return local_players.data[0];
		}

		void* ResolveViewport()
		{
			const auto* resolved = offsets::Get();
			if (resolved == nullptr)
				return nullptr;
			auto* world = memory::ReadPointer<void>(
				reinterpret_cast<const void*>(resolved->gworld_address), 0);
			auto* game_instance = memory::ReadPointer<void>(
				world, WORLD_GAME_INSTANCE_OFFSET);
			auto* local_player = ResolveLocalPlayer(game_instance);
			auto* viewport = memory::ReadPointer<void>(
				local_player, LOCAL_PLAYER_VIEWPORT_OFFSET);
			auto* viewport_world = memory::ReadPointer<void>(
				viewport, VIEWPORT_WORLD_OFFSET);
			auto* viewport_game_instance = memory::ReadPointer<void>(
				viewport, VIEWPORT_GAME_INSTANCE_OFFSET);
			return nte::hook::IsConsistentViewportChain(
				world,
				game_instance,
				viewport,
				viewport_world,
				viewport_game_instance)
				? viewport
				: nullptr;
		}

		void* ResolvePlayerState(void* viewport)
		{
			auto* game_instance = memory::ReadPointer<void>(
				viewport, VIEWPORT_GAME_INSTANCE_OFFSET);
			auto* local_player = ResolveLocalPlayer(game_instance);
			auto* player_controller = memory::ReadPointer<void>(
				local_player, LOCAL_PLAYER_CONTROLLER_OFFSET);
			return memory::ReadPointer<void>(
				player_controller, CONTROLLER_PLAYER_STATE_OFFSET);
		}

		bool IsExpectedViewportTick(const void* address)
		{
			constexpr size_t CALL_DISPLACEMENT_SIZE = 4;
			constexpr size_t suffix_offset =
				VIEWPORT_TICK_PREFIX.size() + CALL_DISPLACEMENT_SIZE;
			constexpr size_t signature_size =
				suffix_offset + VIEWPORT_TICK_SUFFIX.size();

			if (!memory::IsExecutableAddress(address) ||
				!memory::IsReadableRange(address, signature_size))
				return false;

			const auto* code = static_cast<const uint8_t*>(address);
			return BytesEqual(
				code,
				VIEWPORT_TICK_PREFIX.data(),
				VIEWPORT_TICK_PREFIX.size()) &&
				BytesEqual(
					code + suffix_offset,
					VIEWPORT_TICK_SUFFIX.data(),
					VIEWPORT_TICK_SUFFIX.size());
		}

		void __fastcall HookedViewportTick(
			void* viewport,
			float delta_seconds)
		{
			original_viewport_tick(viewport, delta_seconds);

			if (ipc_dispatch_in_progress)
				return;

			ipc_dispatch_in_progress = true;
			if (++viewport_rebind_tick_count >= VIEWPORT_REBIND_CHECK_TICKS)
			{
				viewport_rebind_tick_count = 0;
				if (auto* resolved_viewport = ResolveViewport();
					nte::hook::ShouldRebindViewport(
						resolved_viewport, hooked_viewport))
				{
					InstallViewportHook(resolved_viewport);
				}
			}

			if (viewport != hooked_viewport)
			{
				ipc_dispatch_in_progress = false;
				return;
			}

			if (!IsEquipmentRpcCacheReady())
			{
				const EquipmentContext context{ ResolvePlayerState(viewport) };
				PrepareEquipmentRpcCache(&context);
			}
			PumpLiveIpc(viewport, ResolvePlayerState);
			ipc_dispatch_in_progress = false;
		}

		bool InstallViewportHook(void* viewport)
		{
			if (viewport == hooked_viewport &&
				viewport_hooks[active_viewport_hook_index].IsInstalled())
				return true;

			if (!memory::IsReadableRange(viewport, sizeof(void*)))
				return false;

			auto** vtable = *reinterpret_cast<void***>(viewport);
			if (!memory::IsReadableRange(
				vtable, (VIEWPORT_TICK_INDEX + 1) * sizeof(void*)) ||
				!IsExpectedViewportTick(vtable[VIEWPORT_TICK_INDEX]))
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE equipment plugin: unsupported viewport Tick vtable.\n")
					.c_str());
				return false;
			}

			const auto candidate_tick = reinterpret_cast<ViewportTick>(
				vtable[VIEWPORT_TICK_INDEX]);
			const bool had_active_hook =
				viewport_hooks[active_viewport_hook_index].IsInstalled();
			const size_t target_hook_index = had_active_hook
				? 1 - active_viewport_hook_index
				: active_viewport_hook_index;
			viewport_hooks[target_hook_index].Remove();

			const auto previous_tick = original_viewport_tick;
			original_viewport_tick = candidate_tick;
			if (!viewport_hooks[target_hook_index].Install(
				viewport,
				VIEWPORT_TICK_INDEX,
				reinterpret_cast<void*>(&HookedViewportTick)) ||
				viewport_hooks[target_hook_index].OriginalFunction() !=
				reinterpret_cast<void*>(candidate_tick))
			{
				viewport_hooks[target_hook_index].Remove();
				original_viewport_tick = previous_tick;
				return false;
			}

			if (had_active_hook)
				viewport_hooks[active_viewport_hook_index].Remove();
			active_viewport_hook_index = target_hook_index;
			hooked_viewport = viewport;
			viewport_rebind_tick_count = 0;

			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE equipment plugin: viewport Tick hook installed.\n")
				.c_str());
			return true;
		}

		void RestoreViewportHook()
		{
			viewport_hooks[0].Remove();
			viewport_hooks[1].Remove();
			active_viewport_hook_index = 0;
			hooked_viewport = nullptr;
			original_viewport_tick = nullptr;
			viewport_rebind_tick_count = 0;
		}

		bool IsGameExecutableHost()
		{
			std::array<wchar_t, MAX_PATH> path{};
			const DWORD length = GetModuleFileNameW(
				nullptr, path.data(), static_cast<DWORD>(path.size()));
			if (length == 0 || length == path.size())
				return false;

			const wchar_t* filename = path.data();
			for (const wchar_t* cursor = path.data(); *cursor != L'\0'; ++cursor)
			{
				if (*cursor == L'\\' || *cursor == L'/')
					filename = cursor + 1;
			}
			return EqualsAsciiCaseInsensitive(
				filename,
				NTE_OBFUSCATE_STRING(L"HTGame.exe").c_str());
		}

		DWORD WINAPI BootstrapViewportHook(void*)
		{
			if (!offsets::Initialize())
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE equipment plugin: automatic offset resolution failed.\n")
					.c_str());
				return ERROR_NOT_FOUND;
			}
			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE equipment plugin: offsets resolved.\n").c_str());

			for (;;)
			{
				if (auto* viewport = ResolveViewport())
				{
					if (InstallViewportHook(viewport))
						return 0;
				}
				Sleep(VIEWPORT_BOOTSTRAP_RETRY_MS);
			}
		}
	} // namespace

	void StartPluginRuntime()
	{
		if (IsGameExecutableHost())
		{
			if (HANDLE thread = CreateThread(
				nullptr, 0, BootstrapViewportHook, nullptr, 0, nullptr))
				CloseHandle(thread);
			else
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE equipment plugin: failed to start viewport bootstrap.\n")
					.c_str());
		}
	}

	void StopPluginRuntime()
	{
		RestoreViewportHook();
		CloseIpc();
	}
} // namespace nte::equipment
