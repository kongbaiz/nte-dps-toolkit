#include "plugin_runtime.hpp"

#include "ipc_transport.hpp"
#include "memory_access.hpp"
#include "mod_runtime.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"
#include "sdk_cache.hpp"
#include "shadow_vtable_hook.hpp"
#include "signature_policy.hpp"
#include "viewport_generation_policy.hpp"
#include "viewport_hook_policy.hpp"
#include "vtable_patch_policy.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <utility>

namespace nte::mods
{
	namespace
	{
		constexpr size_t WORLD_GAME_INSTANCE_OFFSET = 0x230;
		constexpr size_t GAME_INSTANCE_LOCAL_PLAYERS_OFFSET = 0x38;
		constexpr size_t LOCAL_PLAYER_VIEWPORT_OFFSET = 0x78;
		constexpr size_t LOCAL_PLAYER_CONTROLLER_OFFSET = 0x30;
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
		constexpr DWORD VIEWPORT_HOST_STOP_TIMEOUT_MS = 1000;
		constexpr DWORD RUNTIME_WORKER_STOP_TIMEOUT_MS = 5000;
		constexpr DWORD RUNTIME_DISPATCH_DRAIN_TIMEOUT_MS = 1000;
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

		struct ViewportSessionSnapshot
		{
			hook::ViewportSessionIdentity identity;
			void* player_controller;
			uint64_t dispatch_epoch;
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
		alignas(8) volatile LONG64 active_viewport_generation = 0;
		alignas(8) volatile LONG64 next_viewport_generation = 0;
		alignas(8) volatile LONG64 viewport_dispatch_epoch = 1;
		PVOID volatile viewport_host_window = nullptr;
		volatile LONG viewport_host_thread_id = 0;
		volatile LONG viewport_host_enabled = 0;
		volatile LONG viewport_host_callback_in_progress = 0;
		volatile LONG viewport_host_stop_requested = 0;
		HANDLE viewport_host_stopped_event = nullptr;
		constinit hook::ViewportSessionIdentity active_viewport_identity{};
		PVOID volatile recorded_plugin_module = nullptr;
		volatile LONG proxy_initialization_scheduled = 0;
		volatile LONG ipc_dispatch_in_progress = 0;
		volatile LONG runtime_stopping = 1;
		volatile LONG active_runtime_detours = 0;
		volatile LONG runtime_detour_ever_published = 0;
		enum class PluginLifecycleState
		{
			NeverStarted,
			Starting,
			Running,
			Stopping,
			Stopped,
			FailedClosed,
		};
		SRWLOCK runtime_lifecycle_lock = SRWLOCK_INIT;
		PluginLifecycleState runtime_lifecycle_state =
			PluginLifecycleState::NeverStarted;
		PluginStopResult last_stop_result = PluginStopResult::UnloadSafe;
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
		constinit bool process_event_vtable_healthy = true;

		class RuntimeDetourScope
		{
		public:
			RuntimeDetourScope() noexcept
			{
				InterlockedIncrement(&active_runtime_detours);
			}

			~RuntimeDetourScope()
			{
				InterlockedDecrement(&active_runtime_detours);
			}

			bool AllowsRuntimeWork() const noexcept
			{
				return InterlockedCompareExchange(
					&runtime_stopping, 0, 0) == 0;
			}
		};

		void MarkRuntimeDetourPublished() noexcept
		{
			InterlockedExchange(&runtime_detour_ever_published, 1);
		}

		static_assert(sizeof(LocalPlayerArray) == 16);

		template <size_t Index>
		void __fastcall HookedViewportTick(void* viewport, float delta_seconds);

		template <size_t... Indices>
		constexpr std::array<ViewportTick, sizeof...(Indices)>
		MakeViewportTickDetours(std::index_sequence<Indices...>) noexcept
		{
			return {&HookedViewportTick<Indices>...};
		}

		constexpr auto viewport_tick_detours = MakeViewportTickDetours(
			std::make_index_sequence<MAX_VIEWPORT_HOOK_RECORDS>{});

		bool InstallViewportHook(const ViewportSessionSnapshot& snapshot);
		void DebugLog(const wchar_t* message);

		void* TryCompareExchangeProcessEventSlot(
			PVOID volatile* slot,
			void* exchange,
			void* comparand) noexcept
		{
			__try
			{
				return InterlockedCompareExchangePointer(
					slot, exchange, comparand);
			}
			__except (EXCEPTION_EXECUTE_HANDLER)
			{
				return nullptr;
			}
		}

		hook::ProtectedPointerPatchResult ReplaceProcessEventVTableEntry(
			void** vtable,
			void* expected,
			void* replacement)
		{
			const auto* resolved = offsets::Get();
			if (resolved == nullptr || resolved->process_event_index >= 4096 ||
				!memory::IsReadableRange(
					vtable,
					(resolved->process_event_index + 1) * sizeof(void*)))
			{
				return {
					hook::ProtectedPointerPatchCode::TargetInvalid,
					hook::ProtectedPointerSlotState::Unknown,
					true,
					false,
				};
			}
			auto* slot = reinterpret_cast<PVOID volatile*>(
				vtable + resolved->process_event_index);
			if ((reinterpret_cast<uintptr_t>(slot) % alignof(void*)) != 0 ||
				!memory::IsImageRange(
					const_cast<PVOID*>(slot), sizeof(void*)))
			{
				return {
					hook::ProtectedPointerPatchCode::TargetInvalid,
					hook::ProtectedPointerSlotState::Unknown,
					true,
					false,
				};
			}
			DWORD old_protection = 0;
			return hook::ReplaceProtectedPointer(
				expected,
				replacement,
				[&]
				{
					return VirtualProtect(
						const_cast<PVOID*>(slot),
						sizeof(void*),
						PAGE_READWRITE,
						&old_protection) != FALSE;
				},
				[&]
				{
					DWORD restored_protection = 0;
					return VirtualProtect(
						const_cast<PVOID*>(slot),
						sizeof(void*),
						old_protection,
						&restored_protection) != FALSE;
				},
				[&](void* exchange, void* comparand)
				{
					return TryCompareExchangeProcessEventSlot(
						slot, exchange, comparand);
				});
		}

		void FailProcessEventVTableIntegrity()
		{
			if (!process_event_vtable_healthy)
				return;
			process_event_vtable_healthy = false;
			process_event_subscription_count = 0;
			for (ProcessEventQueue& queue : process_event_queues)
			{
				queue.first = 0;
				queue.count = 0;
			}
			DebugLog(L"ProcessEvent vtable patch integrity failed.\n");
		}

		hook::ProtectedPointerPatchResult ApplyProcessEventClassPatch(
			ProcessEventClassHookEntry& entry,
			void* expected,
			void* replacement,
			bool replacement_is_installed)
		{
			const hook::ProtectedPointerPatchResult result =
				ReplaceProcessEventVTableEntry(
					entry.vtable,
					expected,
					replacement);
			switch (result.slot_state)
			{
			case hook::ProtectedPointerSlotState::Expected:
				entry.installed = !replacement_is_installed;
				break;
			case hook::ProtectedPointerSlotState::Replacement:
				entry.installed = replacement_is_installed;
				break;
			case hook::ProtectedPointerSlotState::Other:
				entry.installed = false;
				break;
			case hook::ProtectedPointerSlotState::Unknown:
				break;
			}
			if (result.RequiresFailClosed())
				FailProcessEventVTableIntegrity();
			// A failed protection restore can roll the slot back after another
			// thread already loaded the detour. That transient publication has the
			// same process-resident unload constraint as a committed installation.
			if (replacement_is_installed && result.RequiresBindingRetention())
				MarkRuntimeDetourPublished();
			return result;
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

		const hook::ViewportOriginalBinding* PublishedViewportBinding(
			size_t index)
		{
			return index < PublishedViewportHookRecordCount()
				? &viewport_original_bindings[index]
				: nullptr;
		}

		bool FindOrPublishViewportHookRecord(
			void* viewport,
			ViewportTick original,
			uint64_t generation,
			uint32_t host_thread_id,
			size_t& result)
		{
			const size_t count = PublishedViewportHookRecordCount();
			const nte::hook::ViewportOriginalBinding candidate{
				viewport,
				reinterpret_cast<void*>(original),
				generation,
				host_thread_id,
			};
			for (size_t index = 0; index < count; ++index)
			{
				const auto& binding = viewport_original_bindings[index];
				if (nte::hook::CanReuseViewportBinding(
						binding,
						candidate.viewport,
						candidate.original,
						candidate.generation,
						candidate.host_thread_id) &&
					viewport_hooks[index].CanInstall())
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

		uint64_t CurrentViewportDispatchEpoch() noexcept
		{
			return static_cast<uint64_t>(InterlockedCompareExchange64(
				&viewport_dispatch_epoch, 0, 0));
		}

		bool CaptureViewportSessionIdentity(
			hook::ViewportSessionIdentity& identity,
			void*& player_controller)
		{
			const auto* resolved = offsets::Get();
			if (resolved == nullptr)
				return false;
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
			player_controller = memory::ReadPointer<void>(
				local_player, LOCAL_PLAYER_CONTROLLER_OFFSET);
			identity = {
				world,
				game_instance,
				local_player,
				viewport,
				GetCurrentThreadId(),
			};
			return player_controller != nullptr &&
				hook::IsConsistentViewportChain(
					world,
					game_instance,
					viewport,
					viewport_world,
					viewport_game_instance);
		}

		bool CaptureViewportSessionSnapshot(
			uint64_t requested_epoch,
			ViewportSessionSnapshot& snapshot)
		{
			hook::ViewportSessionIdentity first{};
			hook::ViewportSessionIdentity confirmed{};
			void* first_controller = nullptr;
			void* confirmed_controller = nullptr;
			if (!CaptureViewportSessionIdentity(first, first_controller) ||
				!CaptureViewportSessionIdentity(confirmed, confirmed_controller))
				return false;
			const uint64_t current_epoch = CurrentViewportDispatchEpoch();
			if (!hook::CanPublishViewportSnapshot(
					requested_epoch,
					current_epoch,
					first,
					confirmed) ||
				first_controller != confirmed_controller)
				return false;
			snapshot = {confirmed, confirmed_controller, current_epoch};
			return true;
		}

		bool CaptureViewportSessionSnapshotGuarded(
			uint64_t requested_epoch,
			ViewportSessionSnapshot& snapshot) noexcept
		{
		#if defined(_MSC_VER)
			__try
			{
				return CaptureViewportSessionSnapshot(
					requested_epoch, snapshot);
			}
			__except (EXCEPTION_EXECUTE_HANDLER)
			{
				snapshot = {};
				return false;
			}
		#else
			return CaptureViewportSessionSnapshot(requested_epoch, snapshot);
		#endif
		}

		bool IsExpectedViewportTick(const void* address)
		{
			if (!memory::IsImageExecutableAddress(address) ||
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

		template <size_t Index>
		void __fastcall HookedViewportTick(
			void* viewport,
			float delta_seconds)
		{
			RuntimeDetourScope detour_scope;
			const hook::ViewportOriginalBinding* binding =
				PublishedViewportBinding(Index);
			if (binding == nullptr || binding->original == nullptr)
				return;
			const auto original_tick = reinterpret_cast<ViewportTick>(
				const_cast<void*>(binding->original));
			original_tick(viewport, delta_seconds);
			if (!detour_scope.AllowsRuntimeWork())
				return;
			const uint64_t generation = static_cast<uint64_t>(
				InterlockedCompareExchange64(
					&active_viewport_generation, 0, 0));
			if (!hook::CanDispatchViewportGeneration(
					binding->generation,
					generation,
					binding->host_thread_id,
					GetCurrentThreadId(),
					binding->viewport,
					viewport))
				return;

			const uint64_t requested_epoch = CurrentViewportDispatchEpoch();
			ViewportSessionSnapshot snapshot{};
			if (!CaptureViewportSessionSnapshotGuarded(
					requested_epoch, snapshot) ||
				!hook::IsSameViewportSession(
					snapshot.identity, active_viewport_identity))
			{
				// Invalidate any concurrently queued host snapshot. The next host
				// timer callback will bind one complete new generation.
				InterlockedIncrement64(&viewport_dispatch_epoch);
				return;
			}

			if (InterlockedCompareExchange(
					&ipc_dispatch_in_progress, 1, 0) != 0)
				return;

			if (viewport != CurrentHookedViewport() ||
				generation != static_cast<uint64_t>(
					InterlockedCompareExchange64(
						&active_viewport_generation, 0, 0)))
			{
				InterlockedExchange(&ipc_dispatch_in_progress, 0);
				return;
			}

			const runtime::ViewportTickContext context{
				generation,
				binding->host_thread_id,
				const_cast<void*>(snapshot.identity.world),
				const_cast<void*>(snapshot.identity.game_instance),
				const_cast<void*>(snapshot.identity.local_player),
				snapshot.player_controller,
				viewport,
			};
			runtime::ExecuteViewportTickPrograms(context);
			InterlockedExchange(&ipc_dispatch_in_progress, 0);
		}

		void CaptureProcessEvent(
			void* object,
			void** object_vtable,
			void* function,
			void* params)
		{
			AcquireSRWLockExclusive(&process_event_lock);
			if (!process_event_vtable_healthy)
			{
				ReleaseSRWLockExclusive(&process_event_lock);
				return;
			}
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
			RuntimeDetourScope detour_scope;
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

			if (detour_scope.AllowsRuntimeWork())
			{
				void** object_vtable = nullptr;
				memory::ReadValue(object, 0, object_vtable);
				CaptureProcessEvent(object, object_vtable, function, params);
			}
			original(object, function, params);
		}

		void __fastcall HookedProcessEventClass(
			void* object,
			void* function,
			void* params)
		{
			RuntimeDetourScope detour_scope;
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

			if (detour_scope.AllowsRuntimeWork())
				CaptureProcessEvent(object, object_vtable, function, params);
			original(object, function, params);
		}

		uint64_t AllocateViewportGeneration() noexcept
		{
			for (;;)
			{
				const LONG64 current = InterlockedCompareExchange64(
					&next_viewport_generation, 0, 0);
				const uint64_t next = hook::NextViewportGeneration(
					{static_cast<uint64_t>(current)}).value;
				if (InterlockedCompareExchange64(
						&next_viewport_generation,
						static_cast<LONG64>(next),
						current) == current)
					return next;
			}
		}

		bool InstallViewportHook(const ViewportSessionSnapshot& snapshot)
		{
			void* viewport = const_cast<void*>(snapshot.identity.viewport);
			const LONG current_active_index = InterlockedCompareExchange(
				&active_viewport_hook_index, -1, -1);
			if (viewport == CurrentHookedViewport() &&
				hook::IsSameViewportSession(
					snapshot.identity, active_viewport_identity) &&
				current_active_index >= 0 &&
				static_cast<size_t>(current_active_index) < viewport_hooks.size() &&
				viewport_hooks[current_active_index].IsInstalled())
				return true;
			if (!hook::IsCompleteViewportSession(snapshot.identity) ||
				snapshot.identity.host_thread_id != GetCurrentThreadId() ||
				snapshot.dispatch_epoch != CurrentViewportDispatchEpoch())
				return false;

			// The host window thread is also the sole hook-publication owner. It
			// retires the previous generation before reading the next object's vptr,
			// so no background thread observes or mutates a rebuilding UObject.
			InterlockedExchange64(&active_viewport_generation, 0);
			InterlockedExchangePointer(&hooked_viewport, nullptr);
			InterlockedExchange(&active_viewport_hook_index, -1);
			if (current_active_index >= 0 &&
				static_cast<size_t>(current_active_index) < viewport_hooks.size() &&
				!viewport_hooks[current_active_index].Remove())
				return false;
			active_viewport_identity = {};

			ViewportSessionSnapshot confirmed{};
			if (!CaptureViewportSessionSnapshotGuarded(
					snapshot.dispatch_epoch, confirmed) ||
				!hook::IsSameViewportSession(
					snapshot.identity, confirmed.identity) ||
				snapshot.player_controller != confirmed.player_controller)
				return false;

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
			const uint64_t generation = AllocateViewportGeneration();
			size_t target_hook_index = 0;
			if (!FindOrPublishViewportHookRecord(
					viewport,
					candidate_tick,
					generation,
					snapshot.identity.host_thread_id,
					target_hook_index))
				return false;
			if (viewport_hooks[target_hook_index].IsInstalled() &&
				!viewport_hooks[target_hook_index].Remove())
				return false;
			if (!viewport_hooks[target_hook_index].Install(
				viewport,
				viewport_tick_index,
				reinterpret_cast<void*>(viewport_tick_detours[target_hook_index]),
				vtable,
				reinterpret_cast<void*>(candidate_tick)))
			{
				(void)viewport_hooks[target_hook_index].Remove();
				return false;
			}
			MarkRuntimeDetourPublished();

			active_viewport_identity = snapshot.identity;
			InterlockedExchange(
				&active_viewport_hook_index,
				static_cast<LONG>(target_hook_index));
			InterlockedExchangePointer(&hooked_viewport, viewport);
			InterlockedExchange64(
				&active_viewport_generation,
				static_cast<LONG64>(generation));

			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: viewport Tick hook installed.\n")
				.c_str());
			return true;
		}

		bool RestoreViewportHook()
		{
			InterlockedExchange64(&active_viewport_generation, 0);
			InterlockedExchangePointer(&hooked_viewport, nullptr);
			InterlockedExchange(&active_viewport_hook_index, -1);
			active_viewport_identity = {};
			const size_t count = PublishedViewportHookRecordCount();
			bool all_removed = true;
			for (size_t index = 0; index < count; ++index)
				all_removed = viewport_hooks[index].Remove() && all_removed;
			return all_removed;
		}

		UINT_PTR ViewportHostTimerId() noexcept
		{
			return reinterpret_cast<UINT_PTR>(&viewport_dispatch_epoch);
		}

		uint64_t AdvanceViewportDispatchEpoch() noexcept
		{
			for (;;)
			{
				const LONG64 current = InterlockedCompareExchange64(
					&viewport_dispatch_epoch, 0, 0);
				const uint64_t next = hook::NextViewportGeneration(
					{static_cast<uint64_t>(current)}).value;
				if (InterlockedCompareExchange64(
						&viewport_dispatch_epoch,
						static_cast<LONG64>(next),
						current) == current)
					return next;
			}
		}

		struct HostWindowCandidate
		{
			HWND visible;
			HWND fallback;
		};

		BOOL CALLBACK FindHostWindowCallback(HWND window, LPARAM parameter)
		{
			DWORD process_id = 0;
			GetWindowThreadProcessId(window, &process_id);
			if (process_id != GetCurrentProcessId() ||
				GetWindow(window, GW_OWNER) != nullptr)
				return TRUE;

			auto& candidate = *reinterpret_cast<HostWindowCandidate*>(parameter);
			if (candidate.fallback == nullptr)
				candidate.fallback = window;
			if (IsWindowVisible(window))
			{
				candidate.visible = window;
				return FALSE;
			}
			return TRUE;
		}

		HWND FindHostWindow()
		{
			HostWindowCandidate candidate{};
			EnumWindows(
				FindHostWindowCallback,
				reinterpret_cast<LPARAM>(&candidate));
			return candidate.visible != nullptr
				? candidate.visible
				: candidate.fallback;
		}

		void CALLBACK HostViewportTimerProc(
			HWND window,
			UINT,
			UINT_PTR timer_id,
			DWORD) noexcept
		{
			if (timer_id != ViewportHostTimerId() ||
				InterlockedCompareExchange(
					&viewport_host_callback_in_progress, 1, 0) != 0)
				return;

			auto release_callback = []
			{
				InterlockedExchange(&viewport_host_callback_in_progress, 0);
			};
			DWORD process_id = 0;
			const DWORD owner_thread = GetWindowThreadProcessId(
				window, &process_id);
			const auto current_window = static_cast<HWND>(
				InterlockedCompareExchangePointer(
					&viewport_host_window, nullptr, nullptr));
			if (window != current_window || process_id != GetCurrentProcessId() ||
				owner_thread == 0 || owner_thread != GetCurrentThreadId() ||
				owner_thread != static_cast<DWORD>(InterlockedCompareExchange(
					&viewport_host_thread_id, 0, 0)))
			{
				KillTimer(window, timer_id);
				release_callback();
				return;
			}

			if (InterlockedCompareExchange(
					&viewport_host_stop_requested, 0, 0) != 0)
			{
				const bool restored = RestoreViewportHook();
				KillTimer(window, timer_id);
				InterlockedExchangePointer(&viewport_host_window, nullptr);
				InterlockedExchange(&viewport_host_thread_id, 0);
				AdvanceViewportDispatchEpoch();
				if (restored && viewport_host_stopped_event != nullptr)
					SetEvent(viewport_host_stopped_event);
				release_callback();
				return;
			}

			if (InterlockedCompareExchange(
					&viewport_host_enabled, 0, 0) == 0)
			{
				if (CurrentHookedViewport() != nullptr)
					(void)RestoreViewportHook();
				release_callback();
				return;
			}

			const uint64_t requested_epoch = CurrentViewportDispatchEpoch();
			ViewportSessionSnapshot snapshot{};
			if (CaptureViewportSessionSnapshotGuarded(
					requested_epoch, snapshot) &&
				hook::CanPublishViewportSnapshot(
					requested_epoch,
					CurrentViewportDispatchEpoch(),
					snapshot.identity,
					snapshot.identity))
			{
				(void)InstallViewportHook(snapshot);
			}
			release_callback();
		}

		bool EnsureViewportHostTimer(DWORD interval_ms)
		{
			HWND window = FindHostWindow();
			if (window == nullptr)
				return false;
			DWORD process_id = 0;
			const DWORD owner_thread = GetWindowThreadProcessId(
				window, &process_id);
			if (owner_thread == 0 || process_id != GetCurrentProcessId())
				return false;

			const auto previous = static_cast<HWND>(
				InterlockedExchangePointer(&viewport_host_window, window));
			if (previous != window)
			{
				if (previous != nullptr)
					KillTimer(previous, ViewportHostTimerId());
				InterlockedExchange(
					&viewport_host_thread_id,
					static_cast<LONG>(owner_thread));
				AdvanceViewportDispatchEpoch();
			}
			return SetTimer(
				window,
				ViewportHostTimerId(),
				interval_ms,
				HostViewportTimerProc) != 0;
		}

		bool StopViewportHostDispatch()
		{
			InterlockedExchange(&viewport_host_enabled, 0);
			InterlockedExchange(&viewport_host_stop_requested, 1);
			AdvanceViewportDispatchEpoch();
			if (viewport_host_stopped_event == nullptr)
				return CurrentHookedViewport() == nullptr;
			ResetEvent(viewport_host_stopped_event);

			HWND window = static_cast<HWND>(InterlockedCompareExchangePointer(
				&viewport_host_window, nullptr, nullptr));
			if (window == nullptr)
				return CurrentHookedViewport() == nullptr;
			if (!EnsureViewportHostTimer(1))
				return false;
			return WaitForSingleObject(
				viewport_host_stopped_event,
				VIEWPORT_HOST_STOP_TIMEOUT_MS) == WAIT_OBJECT_0;
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

		DWORD WINAPI WatchModWorkspace(void* parameter)
		{
			const HANDLE stop_event = static_cast<HANDLE>(parameter);
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
					if (runtime::Reset())
						CloseIpc();
				}

				const bool viewport_enabled =
					runtime::HasViewportTickPrograms();
				InterlockedExchange(
					&viewport_host_enabled,
					viewport_enabled ? 1 : 0);
				if (viewport_enabled)
				{
					// 偏移在成功解析后只发布一次；未就绪或解析失败时
					// Initialize 按自身节流策略重试，避免读者持有指针时改写已发布数据。
					if (offsets::Get() == nullptr)
						offsets::Initialize(stop_event);
					if (offsets::Get() != nullptr)
						(void)EnsureViewportHostTimer(
							VIEWPORT_BOOTSTRAP_RETRY_MS);
				}
				else if (CurrentHookedViewport() != nullptr)
				{
					// Only request the transition here. The timer callback executes
					// hook teardown on the same stable host window thread that owns
					// UObject resolution and publication.
					(void)EnsureViewportHostTimer(1);
					CloseIpc();
				}

				if (WaitForSingleObject(
						stop_event,
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

		uint16_t params_size = 0;
		if (!ReflectedFunctionParamSize(function, params_size) ||
			params_size > PROCESS_EVENT_PARAM_CAPACITY ||
			(array_element_size != 0 &&
				params_size < sizeof(ProcessEventArray)))
			return false;

		AcquireSRWLockExclusive(&process_event_lock);
		if (!process_event_vtable_healthy)
		{
			ReleaseSRWLockExclusive(&process_event_lock);
			return false;
		}
		// The object vtable and ProcessEvent slot are sampled while the hook lock
		// is held so Watch/Unwatch/Reset observe one linearized binding state.
		void** vtable = nullptr;
		ProcessEvent process_event_original = nullptr;
		if (!memory::ReadValue(object, 0, vtable) ||
			!memory::IsReadableRange(
				vtable,
				(process_event_index + 1) * sizeof(void*)) ||
			!memory::ReadValue(
				vtable,
				process_event_index * sizeof(void*),
				process_event_original) ||
			!memory::IsImageExecutableAddress(
				reinterpret_cast<void*>(process_event_original)))
		{
			ReleaseSRWLockExclusive(&process_event_lock);
			return false;
		}
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
						hook.installed =
							process_event_original == &HookedProcessEventClass;
						FailProcessEventVTableIntegrity();
						ReleaseSRWLockExclusive(&process_event_lock);
						return false;
					}
					target_hook = &hook;
					break;
				}
			}
			bool new_binding = false;
			if (target_hook == nullptr)
			{
				if (process_event_original == &HookedProcessEventClass ||
					process_event_original == &HookedProcessEventInstance)
				{
					FailProcessEventVTableIntegrity();
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
				new_binding = true;
			}
			if (!target_hook->installed)
			{
				const hook::ProtectedPointerPatchResult patch_result =
					ApplyProcessEventClassPatch(
						*target_hook,
						reinterpret_cast<void*>(target_hook->original),
						reinterpret_cast<void*>(&HookedProcessEventClass),
						true);
				if (new_binding && patch_result.RequiresBindingRetention())
					++process_event_class_hook_binding_count;
				if (!patch_result.Applied())
				{
					if (new_binding &&
						!patch_result.RequiresBindingRetention())
						*target_hook = {};
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
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
				(void)target_hook->hook.Remove();
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
					(void)target_hook->hook.Remove();
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
			if (target_hook->hook.IsInstalled())
				MarkRuntimeDetourPublished();
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
		size_t removal_index = process_event_subscription_count;
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
			removal_index = index;
			break;
		}
		if (removal_index == process_event_subscription_count)
		{
			ReleaseSRWLockExclusive(&process_event_lock);
			return false;
		}

		const ProcessEventSubscription removed_subscription =
			process_event_subscriptions[removal_index];
		bool hook_subscribed = false;
		for (size_t index = 0;
			index < process_event_subscription_count;
			++index)
		{
			if (index == removal_index)
				continue;
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
			ProcessEventClassHookEntry* target_hook = nullptr;
			for (ProcessEventClassHookEntry& hook :
				process_event_class_hooks)
			{
				if (hook.vtable == removed_subscription.class_vtable)
				{
					target_hook = &hook;
					break;
				}
			}
			if (target_hook == nullptr)
			{
				FailProcessEventVTableIntegrity();
				ReleaseSRWLockExclusive(&process_event_lock);
				return false;
			}
			if (target_hook->installed)
			{
				const hook::ProtectedPointerPatchResult patch_result =
					ApplyProcessEventClassPatch(
						*target_hook,
						reinterpret_cast<void*>(&HookedProcessEventClass),
						reinterpret_cast<void*>(target_hook->original),
						false);
				if (!patch_result.Applied())
				{
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
			}
		}
		if (!hook_subscribed && !removed_subscription.class_wide)
		{
			for (ProcessEventHookEntry& hook : process_event_hooks)
			{
				if (hook.object != object || !hook.hook.IsInstalled())
					continue;
				if (!hook.hook.Remove())
				{
					FailProcessEventVTableIntegrity();
					ReleaseSRWLockExclusive(&process_event_lock);
					return false;
				}
				break;
			}
		}

		process_event_subscriptions[removal_index] =
			process_event_subscriptions[--process_event_subscription_count];
		ReleaseSRWLockExclusive(&process_event_lock);
		return true;
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

	bool ResetProcessEventWatches()
	{
		AcquireSRWLockExclusive(&process_event_lock);
		bool all_removed = process_event_vtable_healthy;
		for (ProcessEventHookEntry& hook : process_event_hooks)
			all_removed = hook.hook.Remove() && all_removed;
		for (ProcessEventClassHookEntry& hook : process_event_class_hooks)
		{
			if (!hook.installed)
				continue;
			if (hook.vtable == nullptr || hook.original == nullptr)
			{
				FailProcessEventVTableIntegrity();
				all_removed = false;
				continue;
			}
			const hook::ProtectedPointerPatchResult patch_result =
				ApplyProcessEventClassPatch(
					hook,
					reinterpret_cast<void*>(&HookedProcessEventClass),
					reinterpret_cast<void*>(hook.original),
					false);
			if (!patch_result.Applied())
				all_removed = false;
		}
		process_event_subscription_count = 0;
		for (ProcessEventQueue& queue : process_event_queues)
		{
			queue.first = 0;
			queue.count = 0;
		}
		if (!all_removed)
			FailProcessEventVTableIntegrity();
		ReleaseSRWLockExclusive(&process_event_lock);
		return all_removed;
	}

	void RecordPluginModule(HMODULE module) noexcept
	{
		if (module != nullptr)
			InterlockedCompareExchangePointer(
				&recorded_plugin_module, module, nullptr);
	}

	DWORD WINAPI InitializeRecordedPluginRuntimeWorker(void*) noexcept
	{
		InitializeRecordedPluginRuntime();
		return 0;
	}

	bool ScheduleRecordedPluginRuntimeInitialization() noexcept
	{
		if (InterlockedCompareExchange(
				&proxy_initialization_scheduled, 1, 0) != 0)
			return true;

		// This helper is invoked during DLL_PROCESS_ATTACH, but it performs only
		// finite thread creation and never waits. Windows does not run the new
		// thread's entry point until all active DLL initialization callbacks have
		// returned, so the runtime starts after the loader lock is released.
		HANDLE worker = CreateThread(
			nullptr,
			0,
			InitializeRecordedPluginRuntimeWorker,
			nullptr,
			0,
			nullptr);
		if (worker == nullptr)
		{
			InterlockedExchange(&proxy_initialization_scheduled, 0);
			return false;
		}
		CloseHandle(worker);
		return true;
	}

	PluginStartResult InitializeRecordedPluginRuntime()
	{
		const auto module = static_cast<HMODULE>(
			InterlockedCompareExchangePointer(
				&recorded_plugin_module, nullptr, nullptr));
		return module != nullptr
			? StartPluginRuntime(module)
			: PluginStartResult::Failed;
	}

	PluginStartResult StartPluginRuntime(HMODULE module)
	{
		if (!IsGameExecutableHost())
			return PluginStartResult::NotGameHost;

		AcquireSRWLockExclusive(&runtime_lifecycle_lock);
		switch (runtime_lifecycle_state)
		{
		case PluginLifecycleState::Running:
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStartResult::AlreadyRunning;
		case PluginLifecycleState::Starting:
		case PluginLifecycleState::Stopping:
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStartResult::InProgress;
		case PluginLifecycleState::FailedClosed:
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStartResult::Failed;
		case PluginLifecycleState::NeverStarted:
		case PluginLifecycleState::Stopped:
			break;
		}
		runtime_lifecycle_state = PluginLifecycleState::Starting;
		ReleaseSRWLockExclusive(&runtime_lifecycle_lock);

		HANDLE stop_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
		if (stop_event == nullptr)
		{
			AcquireSRWLockExclusive(&runtime_lifecycle_lock);
			runtime_lifecycle_state = PluginLifecycleState::Stopped;
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: failed to create runtime stop event.\n")
				.c_str());
			return PluginStartResult::Failed;
		}
		HANDLE host_stopped_event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
		if (host_stopped_event == nullptr)
		{
			const bool stop_event_closed = CloseHandle(stop_event) != FALSE;
			AcquireSRWLockExclusive(&runtime_lifecycle_lock);
			if (!stop_event_closed)
				runtime_stop_event = stop_event;
			runtime_lifecycle_state = stop_event_closed
				? PluginLifecycleState::Stopped
				: PluginLifecycleState::FailedClosed;
			last_stop_result = stop_event_closed
				? PluginStopResult::UnloadSafe
				: PluginStopResult::TeardownIncomplete;
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStartResult::Failed;
		}
		viewport_host_stopped_event = host_stopped_event;
		InterlockedExchange(&viewport_host_stop_requested, 0);
		InterlockedExchange(&viewport_host_enabled, 0);

		// Presence is a required part of the Running contract. Publish it before
		// worker creation so a fixed-name collision or an unverifiable descriptor
		// fails closed without starting any runtime work.
		if (!OpenRuntimePresence())
		{
			const bool host_event_closed =
				CloseHandle(host_stopped_event) != FALSE;
			if (host_event_closed)
				viewport_host_stopped_event = nullptr;
			const bool stop_event_closed = CloseHandle(stop_event) != FALSE;
			AcquireSRWLockExclusive(&runtime_lifecycle_lock);
			if (!stop_event_closed)
				runtime_stop_event = stop_event;
			const bool resources_drained =
				host_event_closed && stop_event_closed;
			runtime_lifecycle_state = resources_drained
				? PluginLifecycleState::Stopped
				: PluginLifecycleState::FailedClosed;
			last_stop_result = resources_drained
				? PluginStopResult::UnloadSafe
				: PluginStopResult::TeardownIncomplete;
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: failed to publish runtime presence.\n")
					.c_str());
			return PluginStartResult::Failed;
		}

		HANDLE watcher = CreateThread(
			nullptr, 0, WatchModWorkspace, stop_event, 0, nullptr);
		if (watcher == nullptr)
		{
			const bool presence_closed =
				CloseRuntimePresence() == RuntimePresenceCloseResult::Closed;
			const bool stop_event_closed = CloseHandle(stop_event) != FALSE;
			AcquireSRWLockExclusive(&runtime_lifecycle_lock);
			if (!stop_event_closed)
				runtime_stop_event = stop_event;
			const bool host_event_closed =
				CloseHandle(host_stopped_event) != FALSE;
			if (host_event_closed)
				viewport_host_stopped_event = nullptr;
			const bool resources_drained = presence_closed &&
				stop_event_closed && host_event_closed;
			runtime_lifecycle_state = resources_drained
				? PluginLifecycleState::Stopped
				: PluginLifecycleState::FailedClosed;
			last_stop_result = resources_drained
				? PluginStopResult::UnloadSafe
				: PluginStopResult::TeardownIncomplete;
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: failed to start runtime watcher.\n")
				.c_str());
			return PluginStartResult::Failed;
		}

		sdk_cache_worker = { module, stop_event };
		HANDLE sdk_worker = CreateThread(
			nullptr,
			0,
			sdk_cache::RunWorker,
			&sdk_cache_worker,
			0,
			nullptr);
		if (sdk_worker == nullptr)
		{
			DebugLog(NTE_OBFUSCATE_STRING(
				L"NTE Mods plugin: failed to start SDK cache worker.\n")
				.c_str());
		}
		InterlockedExchange(&runtime_stopping, 0);
		SetIpcStopping(false);
		AcquireSRWLockExclusive(&runtime_lifecycle_lock);
		runtime_stop_event = stop_event;
		runtime_thread = watcher;
		sdk_cache_thread = sdk_worker;
		runtime_lifecycle_state = PluginLifecycleState::Running;
		last_stop_result = PluginStopResult::UnloadSafe;
		ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
		return PluginStartResult::Started;
	}

	PluginStopResult StopPluginRuntime()
	{
		AcquireSRWLockExclusive(&runtime_lifecycle_lock);
		if (runtime_lifecycle_state == PluginLifecycleState::NeverStarted)
		{
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStopResult::UnloadSafe;
		}
		if (runtime_lifecycle_state == PluginLifecycleState::Stopped)
		{
			const PluginStopResult result = last_stop_result;
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return result;
		}
		if (runtime_lifecycle_state == PluginLifecycleState::Starting ||
			runtime_lifecycle_state == PluginLifecycleState::Stopping)
		{
			ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
			return PluginStopResult::InProgress;
		}
		// Running and FailedClosed both have exactly one teardown owner. A second
		// call after a partial failure retries only the still-owned handles.
		runtime_lifecycle_state = PluginLifecycleState::Stopping;
		ReleaseSRWLockExclusive(&runtime_lifecycle_lock);

		// Publish both gates before signaling or waiting. New detours still call
		// their immutable original binding but cannot dispatch runtime/IPC work.
		InterlockedExchange(&runtime_stopping, 1);
		SetIpcStopping(true);
		const bool stop_signaled = runtime_stop_event == nullptr ||
			SetEvent(runtime_stop_event) != FALSE;

		auto stop_worker = [](HANDLE& worker)
		{
			if (worker == nullptr)
				return true;
			if (WaitForSingleObject(
					worker, RUNTIME_WORKER_STOP_TIMEOUT_MS) != WAIT_OBJECT_0)
				return false;
			if (CloseHandle(worker) == FALSE)
				return false;
			worker = nullptr;
			return true;
		};
		const bool watcher_stopped = stop_worker(runtime_thread);
		const bool sdk_worker_stopped = stop_worker(sdk_cache_thread);
		const bool viewport_succeeded = watcher_stopped &&
			StopViewportHostDispatch();
		bool stop_event_closed = runtime_stop_event == nullptr;
		if (watcher_stopped && sdk_worker_stopped &&
			runtime_stop_event != nullptr)
		{
			stop_event_closed = CloseHandle(runtime_stop_event) != FALSE;
			if (stop_event_closed)
				runtime_stop_event = nullptr;
		}

		// These resources are independent: a failure in one never skips teardown
		// of another. Each result contributes to the unload-safe decision.
		const bool reset_succeeded = runtime::Reset();
		const bool ipc_succeeded = CloseIpc() == IpcCloseResult::Closed;
		const bool presence_succeeded =
			CloseRuntimePresence() == RuntimePresenceCloseResult::Closed;
		bool viewport_event_closed = viewport_host_stopped_event == nullptr;
		if (viewport_succeeded && viewport_host_stopped_event != nullptr)
		{
			viewport_event_closed =
				CloseHandle(viewport_host_stopped_event) != FALSE;
			if (viewport_event_closed)
				viewport_host_stopped_event = nullptr;
		}

		const ULONGLONG dispatch_deadline =
			GetTickCount64() + RUNTIME_DISPATCH_DRAIN_TIMEOUT_MS;
		while (InterlockedCompareExchange(
				&active_runtime_detours, 0, 0) != 0 &&
			GetTickCount64() < dispatch_deadline)
		{
			Sleep(1);
		}
		const bool dispatch_succeeded = InterlockedCompareExchange(
			&active_runtime_detours, 0, 0) == 0;
		const bool resources_drained = stop_signaled && watcher_stopped &&
			sdk_worker_stopped && stop_event_closed && reset_succeeded &&
			viewport_succeeded && ipc_succeeded && presence_succeeded &&
			viewport_event_closed && dispatch_succeeded;

		PluginStopResult result = PluginStopResult::TeardownIncomplete;
		if (resources_drained)
		{
			// A thread may have loaded a published detour pointer immediately before
			// restoration and been descheduled before entering its counter. Without
			// suspending foreign game threads, that lineage is process-resident.
			result = InterlockedCompareExchange(
				&runtime_detour_ever_published, 0, 0) == 0
				? PluginStopResult::UnloadSafe
				: PluginStopResult::Resident;
		}

		AcquireSRWLockExclusive(&runtime_lifecycle_lock);
		last_stop_result = result;
		runtime_lifecycle_state = resources_drained
			? PluginLifecycleState::Stopped
			: PluginLifecycleState::FailedClosed;
		ReleaseSRWLockExclusive(&runtime_lifecycle_lock);
		if (!resources_drained)
			DebugLog(L"Runtime teardown incomplete; unload blocked.\n");
		return result;
	}
} // namespace nte::mods
