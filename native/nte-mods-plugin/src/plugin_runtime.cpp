#include "plugin_runtime.hpp"

#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"
#include "shadow_vtable_hook.hpp"
#include "viewport_hook_policy.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	namespace
	{
		constexpr size_t WORLD_GAME_INSTANCE_OFFSET = 0x230;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_VIEWPORT_OFFSET = 0x78;
		constexpr size_t VIEWPORT_WORLD_OFFSET = 0x78;
		constexpr size_t VIEWPORT_GAME_INSTANCE_OFFSET = 0x80;
		constexpr size_t VIEWPORT_TICK_INDEX = 100;
		constexpr DWORD VIEWPORT_BOOTSTRAP_RETRY_MS = 250;
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mods Plugin";
		constexpr wchar_t LEGACY_MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mod Loader";
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_VALUE[] = L"Workspace";

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
		PVOID volatile hooked_viewport = nullptr;
		PVOID volatile original_viewport_tick = nullptr;
		volatile LONG ipc_dispatch_in_progress = 0;
		HANDLE runtime_stop_event = nullptr;
		HANDLE runtime_thread = nullptr;

		static_assert(sizeof(LocalPlayerArray) == 16);

		bool InstallViewportHook(void* viewport);

		void* CurrentHookedViewport()
		{
			return InterlockedCompareExchangePointer(
				&hooked_viewport, nullptr, nullptr);
		}

		ViewportTick CurrentOriginalViewportTick()
		{
			return reinterpret_cast<ViewportTick>(
				InterlockedCompareExchangePointer(
					&original_viewport_tick, nullptr, nullptr));
		}

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
			const auto original_tick = CurrentOriginalViewportTick();
			if (original_tick == nullptr)
				return;
			original_tick(viewport, delta_seconds);

			if (InterlockedCompareExchange(
					&ipc_dispatch_in_progress, 1, 0) != 0)
				return;

			if (viewport != CurrentHookedViewport())
			{
				InterlockedExchange(&ipc_dispatch_in_progress, 0);
				return;
			}

			runtime::ExecuteViewportTickPrograms(viewport);
			InterlockedExchange(&ipc_dispatch_in_progress, 0);
		}

		bool InstallViewportHook(void* viewport)
		{
			if (viewport == CurrentHookedViewport() &&
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
					L"NTE Mods plugin: unsupported viewport Tick vtable.\n")
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

			const auto previous_tick = CurrentOriginalViewportTick();
			InterlockedExchangePointer(
				&original_viewport_tick,
				reinterpret_cast<void*>(candidate_tick));
			if (!viewport_hooks[target_hook_index].Install(
				viewport,
				VIEWPORT_TICK_INDEX,
				reinterpret_cast<void*>(&HookedViewportTick)) ||
				viewport_hooks[target_hook_index].OriginalFunction() !=
				reinterpret_cast<void*>(candidate_tick))
			{
				viewport_hooks[target_hook_index].Remove();
				InterlockedExchangePointer(
					&original_viewport_tick,
					reinterpret_cast<void*>(previous_tick));
				return false;
			}

			if (had_active_hook)
				viewport_hooks[active_viewport_hook_index].Remove();
			active_viewport_hook_index = target_hook_index;
			InterlockedExchangePointer(&hooked_viewport, viewport);

			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: viewport Tick hook installed.\n")
				.c_str());
			return true;
		}

		void RestoreViewportHook()
		{
			viewport_hooks[0].Remove();
			viewport_hooks[1].Remove();
			active_viewport_hook_index = 0;
			InterlockedExchangePointer(&hooked_viewport, nullptr);
			InterlockedExchangePointer(&original_viewport_tick, nullptr);
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

		bool ReadModWorkspaceFromKey(
			const wchar_t* registry_key,
			std::array<wchar_t, MAX_PATH>& workspace)
		{
			DWORD value_type = 0;
			DWORD byte_length = static_cast<DWORD>(
				workspace.size() * sizeof(wchar_t));
			const LSTATUS status = RegGetValueW(
				HKEY_CURRENT_USER,
				registry_key,
				MOD_WORKSPACE_REGISTRY_VALUE,
				RRF_RT_REG_SZ,
				&value_type,
				workspace.data(),
				&byte_length);
			if (status != ERROR_SUCCESS || value_type != REG_SZ ||
				byte_length < 2 * sizeof(wchar_t) ||
				byte_length > workspace.size() * sizeof(wchar_t) ||
				byte_length % sizeof(wchar_t) != 0)
				return false;

			const size_t length =
				byte_length / sizeof(wchar_t);
			if (workspace[length - 1] != L'\0')
				return false;
			const bool drive_path =
				length >= 4 &&
				((workspace[0] >= L'A' && workspace[0] <= L'Z') ||
					(workspace[0] >= L'a' && workspace[0] <= L'z')) &&
				workspace[1] == L':' &&
				(workspace[2] == L'\\' || workspace[2] == L'/');
			const bool unc_path =
				length >= 4 && workspace[0] == L'\\' &&
				workspace[1] == L'\\';
			return drive_path || unc_path;
		}

		bool ReadModWorkspace(
			std::array<wchar_t, MAX_PATH>& workspace)
		{
			if (ReadModWorkspaceFromKey(
				MOD_WORKSPACE_REGISTRY_KEY, workspace))
				return true;
			return ReadModWorkspaceFromKey(
				LEGACY_MOD_WORKSPACE_REGISTRY_KEY, workspace);
		}

		DWORD WINAPI WatchModWorkspace(void*)
		{
			bool offsets_initialized = false;
			for (;;)
			{
				std::array<wchar_t, MAX_PATH> workspace{};
				if (ReadModWorkspace(workspace))
				{
					const runtime::ReloadResult reload =
						runtime::ReloadEnabledPrograms(workspace.data());
					if (reload == runtime::ReloadResult::Error)
					{
						DebugLog(NTE_OBFUSCATE_STRING(
							L"NTE Mods plugin: failed to reload programs.\n")
							.c_str());
					}
					else if (reload == runtime::ReloadResult::Changed &&
						(runtime::EnabledCapabilities() &
							runtime::CAPABILITY_IPC) == 0)
					{
						CloseIpc();
					}
				}
				else if (runtime::HasViewportTickPrograms() ||
					CurrentHookedViewport() != nullptr)
				{
					RestoreViewportHook();
					runtime::Reset();
					CloseIpc();
				}

				if (runtime::HasViewportTickPrograms())
				{
					if (!offsets_initialized)
					{
						offsets_initialized = offsets::Initialize();
						if (offsets_initialized)
						{
							DebugLog(NTE_OBFUSCATE_STRING(
								L"NTE Mods plugin: offsets resolved.\n")
								.c_str());
						}
					}
					if (offsets_initialized)
					{
						if (auto* viewport = ResolveViewport())
							InstallViewportHook(viewport);
					}
				}
				else if (CurrentHookedViewport() != nullptr)
				{
					RestoreViewportHook();
					CloseIpc();
				}

				if (WaitForSingleObject(
						runtime_stop_event,
						VIEWPORT_BOOTSTRAP_RETRY_MS) != WAIT_TIMEOUT)
					return 0;
			}
		}
	} // namespace

	void StartPluginRuntime(HMODULE module)
	{
		static_cast<void>(module);
		if (IsGameExecutableHost())
		{
			runtime_stop_event = CreateEventW(
				nullptr, TRUE, FALSE, nullptr);
			if (runtime_stop_event == nullptr)
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: failed to create runtime stop event.\n")
					.c_str());
				return;
			}
			runtime_thread = CreateThread(
				nullptr, 0, WatchModWorkspace, nullptr, 0, nullptr);
			if (runtime_thread == nullptr)
			{
				CloseHandle(runtime_stop_event);
				runtime_stop_event = nullptr;
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: failed to start runtime watcher.\n")
					.c_str());
			}
		}
	}

	void StopPluginRuntime()
	{
		if (runtime_stop_event != nullptr)
			SetEvent(runtime_stop_event);
		if (runtime_thread != nullptr)
		{
			WaitForSingleObject(runtime_thread, INFINITE);
			CloseHandle(runtime_thread);
			runtime_thread = nullptr;
		}
		if (runtime_stop_event != nullptr)
		{
			CloseHandle(runtime_stop_event);
			runtime_stop_event = nullptr;
		}
		RestoreViewportHook();
		CloseIpc();
		runtime::Reset();
	}
} // namespace nte::mods
