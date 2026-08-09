#include "plugin_runtime.hpp"

#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"
#include "shadow_vtable_hook.hpp"
#include "signature_policy.hpp"
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
		constexpr size_t VIEWPORT_TICK_SCAN_RADIUS = 4;
		constexpr size_t VIEWPORT_TICK_CODE_WINDOW = 0x90;
		constexpr size_t MAX_PROCESS_EVENT_HOOKS = 16;
		constexpr size_t MAX_PROCESS_EVENT_CLASS_HOOKS = 4;
		constexpr size_t MAX_PROCESS_EVENT_SUBSCRIPTIONS = 32;
		constexpr size_t PROCESS_EVENT_QUEUE_CAPACITY = 32;
		constexpr size_t MAX_PROCESS_EVENT_PROGRAMS = 16;
		constexpr size_t MAX_PROCESS_EVENT_ARRAY_ELEMENTS = 32;
		constexpr size_t MAX_PROCESS_EVENT_ARRAY_ELEMENT_SIZE = 0x1000;
		constexpr DWORD VIEWPORT_BOOTSTRAP_RETRY_MS = 250;
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mods Plugin";
		constexpr wchar_t LEGACY_MOD_WORKSPACE_REGISTRY_KEY[] =
			L"Software\\NTE DPS Tool\\Mod Loader";
		constexpr wchar_t MOD_WORKSPACE_REGISTRY_VALUE[] = L"Workspace";

		constexpr uint8_t STACK_ALLOC_LARGE_BYTES[]{
			0x48, 0x81, 0xEC, 0x00, 0x00, 0x00, 0x00,
		};
		constexpr uint8_t STACK_ALLOC_LARGE_MASK[]{
			0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00,
		};
		constexpr signature::BytePattern STACK_ALLOC_LARGE{
			STACK_ALLOC_LARGE_BYTES,
			STACK_ALLOC_LARGE_MASK,
			sizeof(STACK_ALLOC_LARGE_BYTES),
		};
		constexpr uint8_t STACK_ALLOC_SMALL_BYTES[]{
			0x48, 0x83, 0xEC, 0x00,
		};
		constexpr uint8_t STACK_ALLOC_SMALL_MASK[]{
			0xFF, 0xFF, 0xFF, 0x00,
		};
		constexpr signature::BytePattern STACK_ALLOC_SMALL{
			STACK_ALLOC_SMALL_BYTES,
			STACK_ALLOC_SMALL_MASK,
			sizeof(STACK_ALLOC_SMALL_BYTES),
		};
		constexpr uint8_t VIEWPORT_TICK_VCALL_BYTES[]{
			0xFF, 0x90, 0x80, 0x01, 0x00, 0x00,
		};
		constexpr uint8_t VIEWPORT_TICK_VCALL_MASK[]{
			0xFF, 0xF8, 0xFF, 0xFF, 0xFF, 0xFF,
		};
		constexpr signature::BytePattern VIEWPORT_TICK_VCALL{
			VIEWPORT_TICK_VCALL_BYTES,
			VIEWPORT_TICK_VCALL_MASK,
			sizeof(VIEWPORT_TICK_VCALL_BYTES),
		};

		struct LocalPlayerArray
		{
			void** data;
			int32_t count;
			int32_t capacity;
		};

		using ViewportTick = void(__fastcall*)(void*, float);
		using ProcessEvent = void(__fastcall*)(void*, void*, void*);

		struct ProcessEventHookEntry
		{
			void* object;
			ProcessEvent original;
			nte::hook::ShadowVTableHook hook;
		};

		struct ProcessEventClassHookEntry
		{
			void** vtable;
			ProcessEvent original;
		};

		struct ProcessEventSubscription
		{
			uint32_t program_index;
			void* object;
			void** class_vtable;
			void* function;
			uint16_t params_size;
			uint16_t array_element_size;
			uint16_t array_value_offset;
			bool class_wide;
		};

		struct ProcessEventQueue
		{
			std::array<ProcessEventRecord, PROCESS_EVENT_QUEUE_CAPACITY> events;
			size_t first;
			size_t count;
		};

		struct ProcessEventArray
		{
			const uint8_t* data;
			int32_t count;
			int32_t capacity;
		};
		static_assert(sizeof(ProcessEventArray) == 16);

		constinit nte::hook::ShadowVTableHook viewport_hooks[2];
		size_t active_viewport_hook_index = 0;
		PVOID volatile hooked_viewport = nullptr;
		PVOID volatile original_viewport_tick = nullptr;
		volatile LONG ipc_dispatch_in_progress = 0;
		HANDLE runtime_stop_event = nullptr;
		HANDLE runtime_thread = nullptr;
		SRWLOCK process_event_lock = SRWLOCK_INIT;
		constinit std::array<
			ProcessEventHookEntry,
			MAX_PROCESS_EVENT_HOOKS> process_event_hooks{};
		constinit size_t process_event_hook_count = 0;
		constinit std::array<
			ProcessEventClassHookEntry,
			MAX_PROCESS_EVENT_CLASS_HOOKS> process_event_class_hooks{};
		constinit size_t process_event_class_hook_count = 0;
		constinit std::array<
			ProcessEventSubscription,
			MAX_PROCESS_EVENT_SUBSCRIPTIONS> process_event_subscriptions{};
		constinit size_t process_event_subscription_count = 0;
		constinit std::array<
			ProcessEventQueue,
			MAX_PROCESS_EVENT_PROGRAMS> process_event_queues{};

		static_assert(sizeof(LocalPlayerArray) == 16);

		bool InstallViewportHook(void* viewport);

		bool ReplaceProcessEventVTableEntry(
			void** vtable,
			void* expected,
			void* replacement)
		{
			const auto* resolved = offsets::Get();
			if (resolved == nullptr || resolved->process_event_index >= 4096 ||
				!memory::IsReadableRange(
					vtable,
					(resolved->process_event_index + 1) * sizeof(void*)))
				return false;
			auto* slot = reinterpret_cast<PVOID volatile*>(
				vtable + resolved->process_event_index);
			DWORD old_protection = 0;
			if (!VirtualProtect(
					const_cast<PVOID*>(slot),
					sizeof(void*),
					PAGE_READWRITE,
					&old_protection))
				return false;
			const bool replaced =
				InterlockedCompareExchangePointer(
					slot,
					replacement,
					expected) == expected;
			DWORD restored_protection = 0;
			VirtualProtect(
				const_cast<PVOID*>(slot),
				sizeof(void*),
				old_protection,
				&restored_protection);
			return replaced;
		}

		bool MatchesProcessEventSubscription(
			const ProcessEventSubscription& subscription,
			void* object,
			void** vtable,
			void* function)
		{
			return subscription.function == function &&
				(subscription.class_wide
					? subscription.class_vtable == vtable
					: subscription.object == object);
		}

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
			if (!memory::IsExecutableAddress(address) ||
				!memory::IsReadableRange(address, VIEWPORT_TICK_CODE_WINDOW))
				return false;

			const auto* code = static_cast<const uint8_t*>(address);
			const bool has_stack_frame =
				signature::Find(
					code,
					VIEWPORT_TICK_CODE_WINDOW,
					0,
					32,
					STACK_ALLOC_LARGE) != VIEWPORT_TICK_CODE_WINDOW ||
				signature::Find(
					code,
					VIEWPORT_TICK_CODE_WINDOW,
					0,
					32,
					STACK_ALLOC_SMALL) != VIEWPORT_TICK_CODE_WINDOW;
			return has_stack_frame &&
				signature::Find(
					code,
					VIEWPORT_TICK_CODE_WINDOW,
					0,
					VIEWPORT_TICK_CODE_WINDOW,
					VIEWPORT_TICK_VCALL) != VIEWPORT_TICK_CODE_WINDOW;
		}

		bool ResolveViewportTickIndex(
			void** vtable,
			size_t preferred_index,
			size_t& result)
		{
			const size_t begin = preferred_index > VIEWPORT_TICK_SCAN_RADIUS
				? preferred_index - VIEWPORT_TICK_SCAN_RADIUS
				: 0;
			if (preferred_index > SIZE_MAX - VIEWPORT_TICK_SCAN_RADIUS)
				return false;
			const size_t end = preferred_index + VIEWPORT_TICK_SCAN_RADIUS;
			if (!memory::IsReadableRange(vtable, (end + 1) * sizeof(void*)))
				return false;

			const auto* resolved = offsets::Get();
			if (resolved == nullptr)
				return false;
			const bool known_image_profile =
				offsets::IsKnownImageProfile(
					resolved->image_size,
					resolved->image_checksum);
			const bool preferred_index_is_valid =
				memory::IsExecutableAddress(vtable[preferred_index]) &&
				memory::IsReadableRange(vtable[preferred_index], 16);
			if (nte::hook::ShouldPreferKnownViewportTick(
					known_image_profile,
					preferred_index_is_valid))
			{
				// The current live build has two semantically similar entries in
				// the bounded scan window. The verified image profile is the
				// authoritative tie-breaker for its SDK vtable index.
				result = preferred_index;
				return true;
			}

			const signature::SelectionResult selection =
				signature::SelectUniqueIndex(
					begin,
					end,
					[&](size_t index)
					{
						return IsExpectedViewportTick(vtable[index]);
					},
					result);
			if (selection == signature::SelectionResult::Unique)
				return true;
			if (selection == signature::SelectionResult::Ambiguous)
				return false;

			return false;
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

		void __fastcall HookedProcessEvent(
			void* object,
			void* function,
			void* params)
		{
			ProcessEvent original = nullptr;
			void** object_vtable = nullptr;
			memory::ReadValue(object, 0, object_vtable);
			AcquireSRWLockShared(&process_event_lock);
			for (size_t index = 0; index < process_event_hooks.size(); ++index)
			{
				if (process_event_hooks[index].object == object)
				{
					original = process_event_hooks[index].original;
					break;
				}
			}
			if (original == nullptr)
			{
				for (const ProcessEventClassHookEntry& hook :
					process_event_class_hooks)
				{
					if (hook.vtable == object_vtable)
					{
						original = hook.original;
						break;
					}
				}
			}
			ReleaseSRWLockShared(&process_event_lock);
			if (original == nullptr)
				return;

			AcquireSRWLockExclusive(&process_event_lock);
			for (size_t index = 0;
				index < process_event_subscription_count;
				++index)
			{
				const ProcessEventSubscription& subscription =
					process_event_subscriptions[index];
				if (!MatchesProcessEventSubscription(
						subscription,
						object,
						object_vtable,
						function) ||
					subscription.program_index >= process_event_queues.size() ||
					(subscription.params_size != 0 &&
						!memory::IsReadableRange(
							params,
							subscription.params_size)))
					continue;

				auto enqueue = [&](uint64_t captured_u64)
				{
					ProcessEventQueue& queue =
						process_event_queues[subscription.program_index];
					if (queue.count == queue.events.size())
					{
						queue.first = (queue.first + 1) % queue.events.size();
						--queue.count;
					}
					ProcessEventRecord& event =
						queue.events[(queue.first + queue.count) % queue.events.size()];
					event = {};
					event.object = object;
					event.function = function;
					event.params_size = subscription.params_size;
					for (size_t byte = 0; byte < subscription.params_size; ++byte)
						event.params[byte] =
							static_cast<const uint8_t*>(params)[byte];
					event.captured_u64 = captured_u64;
					++queue.count;
				};

				if (subscription.array_element_size == 0)
				{
					enqueue(0);
					continue;
				}

				ProcessEventArray array{};
				if (subscription.params_size < sizeof(array))
					continue;
				for (size_t byte = 0; byte < sizeof(array); ++byte)
					reinterpret_cast<uint8_t*>(&array)[byte] =
						static_cast<const uint8_t*>(params)[byte];
				if (array.count < 0 ||
					array.count > static_cast<int32_t>(
						MAX_PROCESS_EVENT_ARRAY_ELEMENTS) ||
					array.capacity < array.count)
					continue;
				for (int32_t element_index = 0;
					element_index < array.count;
					++element_index)
				{
					const size_t offset =
						static_cast<size_t>(element_index) *
							subscription.array_element_size +
						subscription.array_value_offset;
					uint64_t captured_u64 = 0;
					if (!memory::ReadValue(array.data, offset, captured_u64))
						break;
					enqueue(captured_u64);
				}
			}
			ReleaseSRWLockExclusive(&process_event_lock);
			original(object, function, params);
		}

		bool InstallViewportHook(void* viewport)
		{
			if (viewport == CurrentHookedViewport() &&
				viewport_hooks[active_viewport_hook_index].IsInstalled())
				return true;

			if (!memory::IsReadableRange(viewport, sizeof(void*)))
				return false;

			auto** vtable = *reinterpret_cast<void***>(viewport);
			const auto* resolved = offsets::Get();
			size_t viewport_tick_index = 0;
			if (resolved == nullptr ||
				!ResolveViewportTickIndex(
					vtable,
					resolved->viewport_tick_index,
					viewport_tick_index))
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: unsupported viewport Tick vtable.\n")
					.c_str());
				return false;
			}

			const auto candidate_tick = reinterpret_cast<ViewportTick>(
				vtable[viewport_tick_index]);
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
				viewport_tick_index,
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
					runtime::Reset();
					RestoreViewportHook();
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

	namespace
	{
	bool WatchProcessEventCapture(
		uint32_t program_index,
		void* object,
		void* function,
		uint64_t array_element_size,
		uint64_t array_value_offset,
		bool class_wide)
	{
		if (program_index >= process_event_queues.size() ||
			!memory::IsReadableRange(object, sizeof(void*)) ||
			!memory::IsReadableRange(function, 0xBA))
			return false;
		const auto* resolved = offsets::Get();
		if (resolved == nullptr || resolved->process_event_index >= 4096)
			return false;
		const size_t process_event_index = resolved->process_event_index;
		if ((array_element_size == 0 && array_value_offset != 0) ||
			array_element_size > MAX_PROCESS_EVENT_ARRAY_ELEMENT_SIZE ||
			(array_element_size != 0 &&
				(array_value_offset > array_element_size ||
					sizeof(uint64_t) >
						array_element_size - array_value_offset)))
			return false;

		auto** vtable = *reinterpret_cast<void***>(object);
		if (!memory::IsReadableRange(
				vtable,
				(process_event_index + 1) * sizeof(void*)) ||
			!memory::IsExecutableAddress(vtable[process_event_index]))
			return false;
		uint16_t params_size = 0;
		if (!ReflectedFunctionParamSize(function, params_size) ||
			params_size > PROCESS_EVENT_PARAM_CAPACITY ||
			(array_element_size != 0 &&
				params_size < sizeof(ProcessEventArray)))
			return false;

		AcquireSRWLockExclusive(&process_event_lock);
		for (size_t index = 0;
			index < process_event_subscription_count;
			++index)
		{
			const ProcessEventSubscription& subscription =
				process_event_subscriptions[index];
			if (subscription.program_index == program_index &&
				subscription.class_wide == class_wide &&
				(class_wide
					? subscription.class_vtable == vtable
					: subscription.object == object) &&
				subscription.function == function)
			{
				const bool same_capture =
					subscription.array_element_size == array_element_size &&
					subscription.array_value_offset == array_value_offset;
				ReleaseSRWLockExclusive(&process_event_lock);
				return same_capture;
			}
		}
		if (process_event_subscription_count ==
			process_event_subscriptions.size())
		{
			ReleaseSRWLockExclusive(&process_event_lock);
			return false;
		}

		if (class_wide)
		{
			ProcessEventClassHookEntry* target_hook = nullptr;
			for (ProcessEventClassHookEntry& hook :
				process_event_class_hooks)
			{
				if (hook.vtable == vtable)
				{
					target_hook = &hook;
					break;
				}
			}
			if (target_hook == nullptr)
			{
				if (process_event_class_hook_count ==
					process_event_class_hooks.size())
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				for (ProcessEventClassHookEntry& candidate :
					process_event_class_hooks)
				{
					if (candidate.vtable == nullptr)
					{
						target_hook = &candidate;
						break;
					}
				}
				const auto original = reinterpret_cast<ProcessEvent>(
					vtable[process_event_index]);
				if (target_hook == nullptr ||
					!ReplaceProcessEventVTableEntry(
						vtable,
						reinterpret_cast<void*>(original),
						reinterpret_cast<void*>(&HookedProcessEvent)))
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				target_hook->vtable = vtable;
				target_hook->original = original;
				++process_event_class_hook_count;
			}
		}
		else
		{
			ProcessEventHookEntry* target_hook = nullptr;
			for (size_t index = 0; index < process_event_hooks.size(); ++index)
			{
				if (process_event_hooks[index].object == object &&
					process_event_hooks[index].hook.IsInstalled())
				{
					target_hook = &process_event_hooks[index];
					break;
				}
			}
			if (target_hook == nullptr)
			{
				if (process_event_hook_count == process_event_hooks.size())
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				for (ProcessEventHookEntry& candidate : process_event_hooks)
				{
					if (candidate.object == nullptr)
					{
						target_hook = &candidate;
						break;
					}
				}
				if (target_hook == nullptr)
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				target_hook->hook.Remove();
				target_hook->object = object;
				target_hook->original =
					reinterpret_cast<ProcessEvent>(vtable[process_event_index]);
				if (!target_hook->hook.Install(
						object,
						process_event_index,
						reinterpret_cast<void*>(&HookedProcessEvent)) ||
					target_hook->hook.OriginalFunction() !=
						reinterpret_cast<void*>(target_hook->original))
				{
					target_hook->hook.Remove();
					target_hook->object = nullptr;
					target_hook->original = nullptr;
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				++process_event_hook_count;
			}
		}

		process_event_subscriptions[process_event_subscription_count++] = {
			program_index,
			object,
			class_wide ? vtable : nullptr,
			function,
			params_size,
			static_cast<uint16_t>(array_element_size),
			static_cast<uint16_t>(array_value_offset),
			class_wide,
		};
		ReleaseSRWLockExclusive(&process_event_lock);
		return true;
	}
	} // namespace

	bool WatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function)
	{
		return WatchProcessEventCapture(
			program_index,
			object,
			function,
			0,
			0,
			false);
	}

	bool WatchProcessEventArrayU64(
		uint32_t program_index,
		void* object,
		void* function,
		uint64_t element_size,
		uint64_t value_offset)
	{
		return WatchProcessEventCapture(
			program_index,
			object,
			function,
			element_size,
			value_offset,
			false);
	}

	bool WatchProcessEventClassArrayU64(
		uint32_t program_index,
		void* object,
		void* function,
		uint64_t element_size,
		uint64_t value_offset)
	{
		return WatchProcessEventCapture(
			program_index,
			object,
			function,
			element_size,
			value_offset,
			true);
	}

	bool UnwatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function)
	{
		AcquireSRWLockExclusive(&process_event_lock);
		bool removed = false;
		ProcessEventSubscription removed_subscription{};
		for (size_t index = 0;
			index < process_event_subscription_count;
			++index)
		{
			const ProcessEventSubscription& subscription =
				process_event_subscriptions[index];
			if (subscription.program_index != program_index ||
				subscription.object != object ||
				subscription.function != function)
				continue;
			removed_subscription = subscription;
			process_event_subscriptions[index] =
				process_event_subscriptions[
					--process_event_subscription_count];
			removed = true;
			break;
		}

		if (removed)
		{
			bool hook_subscribed = false;
			for (size_t index = 0;
				index < process_event_subscription_count;
				++index)
			{
				const ProcessEventSubscription& subscription =
					process_event_subscriptions[index];
				if (removed_subscription.class_wide
					? subscription.class_wide &&
						subscription.class_vtable ==
							removed_subscription.class_vtable
					: !subscription.class_wide &&
						subscription.object == object)
				{
					hook_subscribed = true;
					break;
				}
			}
			if (!hook_subscribed && removed_subscription.class_wide)
			{
				for (ProcessEventClassHookEntry& hook :
					process_event_class_hooks)
				{
					if (hook.vtable != removed_subscription.class_vtable)
						continue;
					ReplaceProcessEventVTableEntry(
						hook.vtable,
						reinterpret_cast<void*>(&HookedProcessEvent),
						reinterpret_cast<void*>(hook.original));
					hook.vtable = nullptr;
					hook.original = nullptr;
					--process_event_class_hook_count;
					break;
				}
			}
			if (!hook_subscribed && !removed_subscription.class_wide)
			{
				for (ProcessEventHookEntry& hook : process_event_hooks)
				{
					if (hook.object != object)
						continue;
					hook.hook.Remove();
					hook.object = nullptr;
					hook.original = nullptr;
					--process_event_hook_count;
					break;
				}
			}
		}
		ReleaseSRWLockExclusive(&process_event_lock);
		return removed;
	}

	bool PopProcessEvent(
		uint32_t program_index,
		ProcessEventRecord& event)
	{
		if (program_index >= process_event_queues.size())
			return false;

		AcquireSRWLockExclusive(&process_event_lock);
		ProcessEventQueue& queue = process_event_queues[program_index];
		if (queue.count == 0)
		{
			ReleaseSRWLockExclusive(&process_event_lock);
			return false;
		}
		event = queue.events[queue.first];
		queue.first = (queue.first + 1) % queue.events.size();
		--queue.count;
		ReleaseSRWLockExclusive(&process_event_lock);
		return true;
	}

	void ResetProcessEventWatches()
	{
		AcquireSRWLockExclusive(&process_event_lock);
		for (ProcessEventHookEntry& hook : process_event_hooks)
		{
			hook.hook.Remove();
			hook.object = nullptr;
			hook.original = nullptr;
		}
		for (ProcessEventClassHookEntry& hook : process_event_class_hooks)
		{
			if (hook.vtable != nullptr && hook.original != nullptr)
				ReplaceProcessEventVTableEntry(
					hook.vtable,
					reinterpret_cast<void*>(&HookedProcessEvent),
					reinterpret_cast<void*>(hook.original));
			hook.vtable = nullptr;
			hook.original = nullptr;
		}
		process_event_hook_count = 0;
		process_event_class_hook_count = 0;
		process_event_subscription_count = 0;
		for (ProcessEventQueue& queue : process_event_queues)
		{
			queue.first = 0;
			queue.count = 0;
		}
		ReleaseSRWLockExclusive(&process_event_lock);
	}

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
			else if (!OpenRuntimePresence())
			{
				DebugLog(NTE_OBFUSCATE_STRING(
					L"NTE Mods plugin: failed to publish runtime presence.\n")
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
		runtime::Reset();
		RestoreViewportHook();
		CloseIpc();
		CloseRuntimePresence();
	}
} // namespace nte::mods
