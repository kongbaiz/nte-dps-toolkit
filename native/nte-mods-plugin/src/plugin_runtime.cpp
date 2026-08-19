#include "plugin_runtime.hpp"

#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"
#include "sdk_cache.hpp"
#include "shadow_vtable_hook.hpp"
#include "signature_policy.hpp"
#include "viewport_hook_policy.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>

namespace nte::mods
{
	namespace
	{
		constexpr size_t WORLD_GAME_INSTANCE_OFFSET = 0x230;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_VIEWPORT_OFFSET = 0x78;
		constexpr size_t VIEWPORT_WORLD_OFFSET = 0x78;
		constexpr size_t VIEWPORT_GAME_INSTANCE_OFFSET = 0x80;
		constexpr size_t VIEWPORT_TICK_SCAN_RADIUS = 8;
		constexpr size_t VIEWPORT_TICK_CODE_WINDOW = 0x90;
		constexpr size_t MAX_VIEWPORT_HOOK_RECORDS = 64;
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
			void** original_vtable;
			ProcessEvent original;
			nte::hook::ShadowVTableHook hook;
		};

		struct ProcessEventClassHookEntry
		{
			void** vtable;
			ProcessEvent original;
			bool installed;
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

		constinit std::array<
			nte::hook::ShadowVTableHook,
			MAX_VIEWPORT_HOOK_RECORDS> viewport_hooks{};
		constinit std::array<
			nte::hook::ViewportOriginalBinding,
			MAX_VIEWPORT_HOOK_RECORDS> viewport_original_bindings{};
		volatile LONG viewport_hook_record_count = 0;
		volatile LONG active_viewport_hook_index = -1;
		PVOID volatile hooked_viewport = nullptr;
		volatile LONG ipc_dispatch_in_progress = 0;
		HANDLE runtime_stop_event = nullptr;
		HANDLE runtime_thread = nullptr;
		HANDLE sdk_cache_thread = nullptr;
		sdk_cache::WorkerContext sdk_cache_worker{};
		SRWLOCK process_event_lock = SRWLOCK_INIT;
		constinit std::array<
			ProcessEventHookEntry,
			MAX_PROCESS_EVENT_HOOKS> process_event_hooks{};
		// Published object/original bindings are immutable. A dispatch can have
		// captured a ProcessEvent detour before the vtable is restored, so reset and
		// unwatch retire the hook but keep its binding for late dispatches.
		constinit size_t process_event_hook_binding_count = 0;
		constinit std::array<
			ProcessEventClassHookEntry,
			MAX_PROCESS_EVENT_CLASS_HOOKS> process_event_class_hooks{};
		constinit size_t process_event_class_hook_binding_count = 0;
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

		size_t PublishedViewportHookRecordCount()
		{
			const LONG count = InterlockedCompareExchange(
				&viewport_hook_record_count, 0, 0);
			return count > 0 &&
				static_cast<size_t>(count) <= viewport_original_bindings.size()
				? static_cast<size_t>(count)
				: 0;
		}

		ViewportTick OriginalViewportTickFor(const void* viewport)
		{
			return reinterpret_cast<ViewportTick>(const_cast<void*>(
				nte::hook::FindViewportOriginal(
					viewport_original_bindings.data(),
					PublishedViewportHookRecordCount(),
					viewport)));
		}

		bool FindOrPublishViewportHookRecord(
			void* viewport,
			ViewportTick original,
			size_t& result)
		{
			const size_t count = PublishedViewportHookRecordCount();
			const nte::hook::ViewportOriginalBinding candidate{
				viewport,
				reinterpret_cast<void*>(original),
			};
			for (size_t index = 0; index < count; ++index)
			{
				const auto& binding = viewport_original_bindings[index];
				if (binding.viewport != viewport)
					continue;
				if (!nte::hook::CanReuseViewportBinding(
						binding, candidate.viewport, candidate.original))
					return false;
				if (viewport_hooks[index].CanInstall())
				{
					result = index;
					return true;
				}
			}
			if (count == viewport_original_bindings.size())
				return false;

			// The workspace worker is the only publisher. The interlocked count
			// release-publishes an immutable object/original binding before the
			// shadow vptr can make HookedViewportTick reachable for this object.
			viewport_original_bindings[count] = candidate;
			InterlockedExchange(
				&viewport_hook_record_count,
				static_cast<LONG>(count + 1));
			result = count;
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

			return memory::ReadPointer<void>(local_players.data, 0);
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
			auto* confirmed_world = memory::ReadPointer<void>(
				reinterpret_cast<const void*>(resolved->gworld_address), 0);
			auto* confirmed_game_instance = memory::ReadPointer<void>(
				confirmed_world, WORLD_GAME_INSTANCE_OFFSET);
			return world == confirmed_world &&
				game_instance == confirmed_game_instance &&
				nte::hook::IsConsistentViewportChain(
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

			std::array<uint8_t, VIEWPORT_TICK_CODE_WINDOW> code{};
			if (!memory::ReadBytes(
					address, 0, code.data(), code.size()))
				return false;
			const bool has_stack_frame =
				signature::Find(
					code.data(),
					VIEWPORT_TICK_CODE_WINDOW,
					0,
					32,
					STACK_ALLOC_LARGE) != VIEWPORT_TICK_CODE_WINDOW ||
				signature::Find(
					code.data(),
					VIEWPORT_TICK_CODE_WINDOW,
					0,
					32,
					STACK_ALLOC_SMALL) != VIEWPORT_TICK_CODE_WINDOW;
			return has_stack_frame &&
				signature::Find(
					code.data(),
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

			void* preferred_tick = nullptr;
			if (memory::ReadValue(
					vtable,
					preferred_index * sizeof(void*),
					preferred_tick) &&
				IsExpectedViewportTick(preferred_tick))
			{
				// find_offsets already selected this semantic vtable slot. Recheck
				// the same body predicate before accepting it in the hook path.
				result = preferred_index;
				return true;
			}

			return nte::hook::SelectPreferredSemanticViewportTick(
				preferred_index,
				begin,
				end,
				VIEWPORT_TICK_SCAN_RADIUS,
				[&](size_t index)
				{
					void* candidate = nullptr;
					return memory::ReadValue(
						vtable, index * sizeof(void*), candidate) &&
						IsExpectedViewportTick(candidate);
				},
				result);
		}

		void __fastcall HookedViewportTick(
			void* viewport,
			float delta_seconds)
		{
			const auto original_tick = OriginalViewportTickFor(viewport);
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

		void CaptureProcessEvent(
			void* object,
			void** object_vtable,
			void* function,
			void* params)
		{
			AcquireSRWLockExclusive(&process_event_lock);
			for (size_t index = 0;
				index < process_event_subscription_count;
				++index)
			{
				const ProcessEventSubscription& subscription =
					process_event_subscriptions[index];
				std::array<uint8_t, PROCESS_EVENT_PARAM_CAPACITY>
					captured_params{};
				if (!MatchesProcessEventSubscription(
						subscription,
						object,
						object_vtable,
						function) ||
					subscription.program_index >= process_event_queues.size() ||
					(subscription.params_size != 0 &&
						!memory::ReadBytes(
							params,
							0,
							captured_params.data(),
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
					event.params = captured_params;
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
				std::memcpy(&array, captured_params.data(), sizeof(array));
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
		}

		void __fastcall HookedProcessEventInstance(
			void* object,
			void* function,
			void* params)
		{
			ProcessEvent original = nullptr;
			AcquireSRWLockShared(&process_event_lock);
			for (const ProcessEventHookEntry& hook : process_event_hooks)
			{
				if (hook.object == object)
				{
					original = hook.original;
					break;
				}
			}
			ReleaseSRWLockShared(&process_event_lock);
			if (original == nullptr)
				return;

			void** object_vtable = nullptr;
			memory::ReadValue(object, 0, object_vtable);
			CaptureProcessEvent(object, object_vtable, function, params);
			original(object, function, params);
		}

		void __fastcall HookedProcessEventClass(
			void* object,
			void* function,
			void* params)
		{
			void** object_vtable = nullptr;
			if (!memory::ReadValue(object, 0, object_vtable))
				return;
			ProcessEvent original = nullptr;
			AcquireSRWLockShared(&process_event_lock);
			for (const ProcessEventClassHookEntry& hook :
				process_event_class_hooks)
			{
				if (hook.vtable == object_vtable)
				{
					original = hook.original;
					break;
				}
			}
			ReleaseSRWLockShared(&process_event_lock);
			if (original == nullptr)
				return;

			CaptureProcessEvent(object, object_vtable, function, params);
			original(object, function, params);
		}

		bool InstallViewportHook(void* viewport)
		{
			const LONG current_active_index = InterlockedCompareExchange(
				&active_viewport_hook_index, -1, -1);
			if (viewport == CurrentHookedViewport() &&
				current_active_index >= 0 &&
				static_cast<size_t>(current_active_index) < viewport_hooks.size() &&
				viewport_hooks[current_active_index].IsInstalled())
				return true;

			if (!memory::IsReadableRange(viewport, sizeof(void*)))
				return false;

			void** vtable = nullptr;
			if (!memory::ReadValue(viewport, 0, vtable))
				return false;
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

			ViewportTick candidate_tick = nullptr;
			if (!memory::ReadValue(
					vtable,
					viewport_tick_index * sizeof(void*),
					candidate_tick))
				return false;
			// The workspace watcher does not own Unreal object lifetime. Re-resolve
			// the complete GWorld -> game instance -> local player -> viewport chain
			// immediately before publication; ShadowVTableHook then CASes the exact
			// captured vptr, so a stale or different object fails closed.
			if (ResolveViewport() != viewport)
				return false;
			size_t target_hook_index = 0;
			if (!FindOrPublishViewportHookRecord(
					viewport, candidate_tick, target_hook_index))
				return false;
			if (viewport_hooks[target_hook_index].IsInstalled())
				viewport_hooks[target_hook_index].Remove();
			if (!viewport_hooks[target_hook_index].Install(
				viewport,
				viewport_tick_index,
				reinterpret_cast<void*>(&HookedViewportTick),
				vtable,
				reinterpret_cast<void*>(candidate_tick)))
			{
				viewport_hooks[target_hook_index].Remove();
				return false;
			}

			InterlockedExchange(
				&active_viewport_hook_index,
				static_cast<LONG>(target_hook_index));
			InterlockedExchangePointer(&hooked_viewport, viewport);
			if (current_active_index >= 0 &&
				static_cast<size_t>(current_active_index) < viewport_hooks.size() &&
				static_cast<size_t>(current_active_index) != target_hook_index)
				viewport_hooks[current_active_index].Remove();

			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: viewport Tick hook installed.\n")
				.c_str());
			return true;
		}

		void RestoreViewportHook()
		{
			InterlockedExchangePointer(&hooked_viewport, nullptr);
			InterlockedExchange(&active_viewport_hook_index, -1);
			const size_t count = PublishedViewportHookRecordCount();
			for (size_t index = 0; index < count; ++index)
				viewport_hooks[index].Remove();
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
					// 偏移在成功解析后只发布一次；未就绪或解析失败时
					// Initialize 按自身节流策略重试，避免读者持有指针时改写已发布数据。
					if (offsets::Get() == nullptr)
						offsets::Initialize(runtime_stop_event);
					if (offsets::Get() != nullptr)
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

		void** vtable = nullptr;
		if (!memory::ReadValue(object, 0, vtable))
			return false;
		ProcessEvent process_event_original = nullptr;
		if (!memory::IsReadableRange(
				vtable,
				(process_event_index + 1) * sizeof(void*)) ||
			!memory::ReadValue(
				vtable,
				process_event_index * sizeof(void*),
				process_event_original) ||
			!memory::IsExecutableAddress(
				reinterpret_cast<void*>(process_event_original)))
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
			// Permanently assign a vtable lineage to one hook mode. The distinct
			// detours keep late dispatch lookup unambiguous, while this gate also
			// prevents treating either detour as a new original during re-hooking.
			for (const ProcessEventHookEntry& hook : process_event_hooks)
			{
				if (hook.object == object || hook.original_vtable == vtable)
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
			}
			ProcessEventClassHookEntry* target_hook = nullptr;
			for (ProcessEventClassHookEntry& hook :
				process_event_class_hooks)
			{
				if (hook.vtable == vtable)
				{
					const ProcessEvent expected_slot = hook.installed
						? &HookedProcessEventClass
						: hook.original;
					if (expected_slot != process_event_original)
					{
						ReleaseSRWLockExclusive(&process_event_lock);
						return false;
					}
					target_hook = &hook;
					break;
				}
			}
			if (target_hook == nullptr)
			{
				if (process_event_original == &HookedProcessEventClass ||
					process_event_original == &HookedProcessEventInstance)
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				if (process_event_class_hook_binding_count ==
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
				if (target_hook == nullptr)
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				target_hook->vtable = vtable;
				target_hook->original = process_event_original;
				++process_event_class_hook_binding_count;
			}
			if (!target_hook->installed)
			{
				if (!ReplaceProcessEventVTableEntry(
						target_hook->vtable,
						reinterpret_cast<void*>(target_hook->original),
						reinterpret_cast<void*>(&HookedProcessEventClass)))
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				target_hook->installed = true;
			}
		}
		else
		{
			for (const ProcessEventClassHookEntry& hook :
				process_event_class_hooks)
			{
				if (hook.vtable == vtable)
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
			}
			ProcessEventHookEntry* target_hook = nullptr;
			for (ProcessEventHookEntry& hook : process_event_hooks)
			{
				if (hook.object == object && hook.hook.IsInstalled())
				{
					if (process_event_original != &HookedProcessEventInstance)
					{
						ReleaseSRWLockExclusive(&process_event_lock);
						return false;
					}
					target_hook = &hook;
					break;
				}
			}
			if (target_hook == nullptr &&
				(process_event_original == &HookedProcessEventClass ||
					process_event_original == &HookedProcessEventInstance))
			{
				ReleaseSRWLockExclusive(&process_event_lock);
				return false;
			}
			if (target_hook == nullptr)
			{
				for (ProcessEventHookEntry& hook : process_event_hooks)
				{
					if (hook.object != object)
						continue;
					if (hook.original_vtable != vtable ||
						hook.original != process_event_original)
					{
						ReleaseSRWLockExclusive(&process_event_lock);
						return false;
					}
					if (hook.hook.CanInstall())
					{
						target_hook = &hook;
						break;
					}
				}
			}
			if (target_hook == nullptr)
			{
				if (process_event_hook_binding_count == process_event_hooks.size())
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				for (ProcessEventHookEntry& candidate : process_event_hooks)
				{
					if (candidate.object == nullptr &&
						candidate.hook.CanInstall())
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
				target_hook->original_vtable = vtable;
				target_hook->original = process_event_original;
				++process_event_hook_binding_count;
			}
			if (!target_hook->hook.IsInstalled() &&
				(!target_hook->hook.Install(
						object,
						process_event_index,
						reinterpret_cast<void*>(&HookedProcessEventInstance),
					target_hook->original_vtable,
					reinterpret_cast<void*>(target_hook->original))))
				{
					target_hook->hook.Remove();
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
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
						reinterpret_cast<void*>(&HookedProcessEventClass),
						reinterpret_cast<void*>(hook.original));
					hook.installed = false;
					break;
				}
			}
			if (!hook_subscribed && !removed_subscription.class_wide)
			{
				for (ProcessEventHookEntry& hook : process_event_hooks)
				{
					if (hook.object != object || !hook.hook.IsInstalled())
						continue;
					hook.hook.Remove();
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
			hook.hook.Remove();
		for (ProcessEventClassHookEntry& hook : process_event_class_hooks)
		{
			if (hook.installed && hook.vtable != nullptr && hook.original != nullptr)
				ReplaceProcessEventVTableEntry(
					hook.vtable,
					reinterpret_cast<void*>(&HookedProcessEventClass),
					reinterpret_cast<void*>(hook.original));
			hook.installed = false;
		}
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
			if (runtime_thread != nullptr)
			{
				sdk_cache_worker = { module, runtime_stop_event };
				sdk_cache_thread = CreateThread(
					nullptr,
					0,
					sdk_cache::RunWorker,
					&sdk_cache_worker,
					0,
					nullptr);
				if (sdk_cache_thread == nullptr)
				{
					DebugLog(NTE_OBFUSCATE_STRING(
						L"NTE Mods plugin: failed to start SDK cache worker.\n")
						.c_str());
				}
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
		if (sdk_cache_thread != nullptr)
		{
			WaitForSingleObject(sdk_cache_thread, INFINITE);
			CloseHandle(sdk_cache_thread);
			sdk_cache_thread = nullptr;
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
