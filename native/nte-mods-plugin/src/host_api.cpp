#include "host_api.hpp"

#include "memory_access.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	namespace
	{
		constexpr uint32_t NATIVE_FUNCTION_FLAG = 0x400;
		constexpr size_t PROCESS_EVENT_INDEX = 0x4C;
		// UE5 UFunction stores ParmsSize before ReturnValueOffset at 0xB8.
		constexpr size_t FUNCTION_PARAM_SIZE_OFFSET = 0xB6;
		constexpr uint64_t FUNCTION_CAST_FLAG = 0x0000000000080000;
		constexpr uint8_t PAUSED_GAME_TYPE_PLAY_SKILL_VIDEO = 2;
		constexpr uint8_t PAUSED_GAME_TYPE_ULTRA_PASSIVE_EFFECT = 3;
		constexpr uint8_t PAUSED_GAME_TYPE_JIN_EFFECT = 4;
		constexpr uint32_t COMBAT_CLOCK_RELEVANT_PAUSE_MASK =
			(1u << PAUSED_GAME_TYPE_PLAY_SKILL_VIDEO) |
			(1u << PAUSED_GAME_TYPE_ULTRA_PASSIVE_EFFECT) |
			(1u << PAUSED_GAME_TYPE_JIN_EFFECT);

		struct UeName
		{
			int32_t comparison_index;
			uint32_t number;
		};

		struct UeClass;

		struct UeObject
		{
			void** vtable;
			uint32_t flags;
			int32_t index;
			UeClass* object_class;
			UeName name;
			UeObject* outer;
		};

		struct UeField : UeObject
		{
			UeField* next;
		};

		struct UeStruct : UeField
		{
			uint8_t base_chain[0x10];
			UeStruct* super;
			UeField* children;
			void* child_properties;
			int32_t size;
			int16_t minimum_alignment;
			uint8_t padding[0x52];
		};

		struct UeClass : UeStruct
		{
			uint8_t padding_b0[0x28];
			uint64_t cast_flags;
			uint8_t padding_e0[0x120];
		};

		struct UeFunction : UeStruct
		{
			uint32_t function_flags;
			uint8_t padding_b4[0x2C];
		};

		struct UeStringBuffer
		{
			wchar_t* data;
			int32_t count;
			int32_t capacity;
		};

		struct UeArrayView
		{
			void* data;
			int32_t count;
			int32_t capacity;
		};

		struct UeItemNetId
		{
			uint32_t slot;
			uint32_t serial;
		};

		struct UeEquipPlaceData
		{
			UeItemNetId equipment;
			int32_t row;
			int32_t column;
		};

		struct SingleItemParams
		{
			UeItemNetId item;
		};

		struct TwoItemParams
		{
			UeItemNetId first;
			UeItemNetId second;
		};

		struct PositionedItemParams
		{
			UeItemNetId character;
			UeItemNetId equipment;
			int32_t row;
			int32_t column;
		};

		struct OneKeyParams
		{
			UeItemNetId character;
			UeArrayView placements;
			UeItemNetId core;
		};

		struct ItemBooleanParams
		{
			UeItemNetId item;
			uint8_t state;
			uint8_t padding[3];
		};

		enum class EquipmentFunction : size_t
		{
			EquipOneKey,
			UnequipAll,
			EquipModule,
			UnequipModule,
			EquipCore,
			UnequipCore,
			MoveModuleToCharacter,
			MoveCoreToCharacter,
			SetItemDiscarded,
			SetItemLocked,
			Count,
		};

		struct DecodedUeName
		{
			wchar_t buffer[256];
			int32_t first;
			int32_t length;
		};

		struct EquipmentFunctionCache
		{
			UeClass* player_state_class;
			std::array<
				UeFunction*,
				static_cast<size_t>(EquipmentFunction::Count)> functions;
			bool initialized;
		};

		struct GamePauseFunctionCache
		{
			UeClass* player_controller_class;
			UeFunction* is_game_paused_by_type;
		};

		struct SdkFunctionCache
		{
			UeClass* object_class;
			std::array<UeFunction*, 12> functions;
		};

		struct IsGamePausedByTypeParams
		{
			uint8_t paused_type;
			uint8_t return_value;
		};

		struct PointerReturnParams
		{
			void* return_value;
		};

		struct BoolReturnParams
		{
			uint8_t return_value;
		};

		struct Int32ReturnParams
		{
			int32_t return_value;
		};

		struct FloatReturnParams
		{
			float return_value;
		};

		struct BoolFloatReturnParams
		{
			uint8_t argument;
			uint8_t padding[3];
			float return_value;
		};

		using AppendName = void(__fastcall*)(const UeName*, UeStringBuffer&);
		using ProcessEvent = void(__fastcall*)(
			const UeObject*, UeFunction*, void*);

		constinit EquipmentFunctionCache function_cache{};
		constinit GamePauseFunctionCache game_pause_function_cache{};
		constinit SdkFunctionCache player_controller_sdk_cache{};
		constinit SdkFunctionCache ability_character_sdk_cache{};
		constinit std::array<
			NteCombatClockTransition,
			NTE_COMBAT_CLOCK_HISTORY_SIZE> combat_clock_history{};
		constinit uint32_t combat_clock_history_count = 0;
		constinit uint32_t combat_clock_history_next = 0;
		constinit uint64_t next_combat_clock_sequence = 1;

		static_assert(sizeof(PluginContext) == 16);
		static_assert(sizeof(NteModsStatus) == 4);
		static_assert(sizeof(UeName) == 8);
		static_assert(sizeof(UeObject) == 0x28);
		static_assert(offsetof(UeObject, object_class) == 0x10);
		static_assert(offsetof(UeObject, name) == 0x18);
		static_assert(offsetof(UeField, next) == 0x28);
		static_assert(offsetof(UeStruct, super) == 0x40);
		static_assert(offsetof(UeStruct, children) == 0x48);
		static_assert(sizeof(UeStruct) == 0xB0);
		static_assert(offsetof(UeClass, cast_flags) == 0xD8);
		static_assert(offsetof(UeFunction, function_flags) == 0xB0);
		static_assert(sizeof(UeItemNetId) == 8);
		static_assert(sizeof(UeEquipPlaceData) == 16);
		static_assert(sizeof(UeArrayView) == 16);
		static_assert(sizeof(SingleItemParams) == 8);
		static_assert(sizeof(TwoItemParams) == 16);
		static_assert(sizeof(PositionedItemParams) == 24);
		static_assert(sizeof(OneKeyParams) == 32);
		static_assert(sizeof(ItemBooleanParams) == 12);
		static_assert(sizeof(IsGamePausedByTypeParams) == 2);
		static_assert(sizeof(PointerReturnParams) == 8);
		static_assert(sizeof(BoolReturnParams) == 1);
		static_assert(sizeof(Int32ReturnParams) == 4);
		static_assert(sizeof(FloatReturnParams) == 4);
		static_assert(sizeof(BoolFloatReturnParams) == 8);
		static_assert(static_cast<size_t>(SdkReadApi::CharacterSlomoMilli) + 1 == 12);

		uint64_t CurrentFileTime100ns()
		{
			FILETIME timestamp{};
			GetSystemTimePreciseAsFileTime(&timestamp);
			return (static_cast<uint64_t>(timestamp.dwHighDateTime) << 32) |
				timestamp.dwLowDateTime;
		}

		bool IsValidItemId(const NteItemNetId* item)
		{
			return item != nullptr && (item->slot != 0 || item->serial != 0);
		}

		bool IsValidGridPosition(int32_t row, int32_t column)
		{
			return row >= NTE_EQUIPMENT_GRID_MIN && row <= NTE_EQUIPMENT_GRID_MAX &&
				column >= NTE_EQUIPMENT_GRID_MIN && column <= NTE_EQUIPMENT_GRID_MAX;
		}

		NteModsStatus ValidateContext(const PluginContext* context)
		{
			if (context == nullptr)
				return NTE_MODS_STATUS_INVALID_CONTEXT;

			if (context->player_state == nullptr)
				return NTE_MODS_STATUS_INVALID_PLAYER_STATE;

			return NTE_MODS_STATUS_DRY_RUN_OK;
		}

		NteModsStatus ValidateCharacterArgument(
			const PluginContext* context,
			const NteItemNetId* character)
		{
			const NteModsStatus context_status = ValidateContext(context);
			if (context_status != NTE_MODS_STATUS_DRY_RUN_OK)
				return context_status;

			if (!IsValidItemId(character))
				return NTE_MODS_STATUS_INVALID_ITEM_ID;

			return NTE_MODS_STATUS_DRY_RUN_OK;
		}

		NteModsStatus ValidateItemArgument(
			const PluginContext* context,
			const NteItemNetId* item)
		{
			const NteModsStatus context_status = ValidateContext(context);
			if (context_status != NTE_MODS_STATUS_DRY_RUN_OK)
				return context_status;

			if (!IsValidItemId(item))
				return NTE_MODS_STATUS_INVALID_ITEM_ID;

			return NTE_MODS_STATUS_DRY_RUN_OK;
		}

		bool DecodeName(const UeName& name, DecodedUeName& decoded)
		{
			decoded = {};
			UeStringBuffer output{
				decoded.buffer,
				0,
				static_cast<int32_t>(_countof(decoded.buffer)) };
			const auto* resolved = offsets::Get();
			if (resolved == nullptr)
				return false;
			const auto append_name = reinterpret_cast<AppendName>(
				resolved->append_name_address);
			if (!memory::IsExecutableAddress(reinterpret_cast<const void*>(append_name)))
				return false;

			append_name(&name, output);
			if (output.count < 0 || output.count > output.capacity)
				return false;

			decoded.length = output.count;
			if (decoded.length > 0 &&
				decoded.buffer[decoded.length - 1] == L'\0')
				--decoded.length;
			for (int32_t index = 0; index < decoded.length; ++index)
			{
				if (decoded.buffer[index] == L'/')
					decoded.first = index + 1;
			}
			return true;
		}

		bool DecodedNameEquals(
			const DecodedUeName& decoded,
			const char* expected)
		{
			int32_t index = decoded.first;
			while (*expected != '\0' && index < decoded.length)
			{
				if (decoded.buffer[index] !=
					static_cast<unsigned char>(*expected))
					return false;
				++index;
				++expected;
			}
			return *expected == '\0' && index == decoded.length;
		}

		bool HashDecodedName(const DecodedUeName& decoded, uint64_t& hash)
		{
			if (decoded.first >= decoded.length)
				return false;

			hash = 0xcbf29ce484222325ull;
			for (int32_t index = decoded.first; index < decoded.length; ++index)
			{
				wchar_t character = decoded.buffer[index];
				if (character > 0x7f)
					return false;
				if (character >= L'A' && character <= L'Z')
					character += L'a' - L'A';
				hash ^= static_cast<uint8_t>(character);
				hash *= 0x100000001b3ull;
			}
			return true;
		}

		bool NameEquals(const UeName& name, const char* expected)
		{
			DecodedUeName decoded{};
			return DecodeName(name, decoded) &&
				DecodedNameEquals(decoded, expected);
		}

		UeFunction* FindFunction(
			UeClass* object_class,
			const char* owner_class_name,
			const char* function_name)
		{
			for (auto* current = static_cast<UeStruct*>(object_class);
				current != nullptr;
				current = current->super)
			{
				if (!memory::IsReadableRange(current, sizeof(UeStruct)))
					return nullptr;
				if (!NameEquals(current->name, owner_class_name))
					continue;

				for (UeField* field = current->children;
					field != nullptr;
					field = field->next)
				{
					if (!memory::IsReadableRange(field, sizeof(UeField)))
						return nullptr;

					uint64_t cast_flags = 0;
					if (!memory::ReadValue(
						field->object_class,
						offsetof(UeClass, cast_flags),
						cast_flags) ||
						(cast_flags & FUNCTION_CAST_FLAG) == 0)
						continue;
					if (NameEquals(field->name, function_name))
						return reinterpret_cast<UeFunction*>(field);
				}
				return nullptr;
			}
			return nullptr;
		}

		void ResolveGamePauseFunctions(UeClass* object_class)
		{
			if (game_pause_function_cache.player_controller_class == object_class)
				return;

			game_pause_function_cache = {
				object_class,
				FindFunction(
					object_class,
					NTE_OBFUSCATE_STRING("HTPlayerController").c_str(),
					NTE_OBFUSCATE_STRING("IsGamePausedByType").c_str()),
			};
		}

		bool QueryGamePausedByType(
			UeObject* player_controller,
			ProcessEvent process_event,
			uint8_t paused_type,
			bool& paused)
		{
			if (game_pause_function_cache.is_game_paused_by_type == nullptr)
				return false;

			IsGamePausedByTypeParams params{ paused_type, 0 };
			process_event(
				player_controller,
				game_pause_function_cache.is_game_paused_by_type,
				&params);
			paused = params.return_value != 0;
			return true;
		}

		bool ReadRelevantGamePauseMask(
			UeObject* player_controller,
			uint32_t& pause_type_mask)
		{
			if (!memory::IsReadableRange(player_controller, sizeof(UeObject)) ||
				player_controller->object_class == nullptr ||
				!memory::IsReadableRange(
					player_controller->vtable,
					(PROCESS_EVENT_INDEX + 1) * sizeof(void*)))
				return false;

			ResolveGamePauseFunctions(player_controller->object_class);
			if (game_pause_function_cache.is_game_paused_by_type == nullptr)
				return false;

			const auto process_event = reinterpret_cast<ProcessEvent>(
				player_controller->vtable[PROCESS_EVENT_INDEX]);
			if (!memory::IsExecutableAddress(
				reinterpret_cast<const void*>(process_event)))
				return false;

			pause_type_mask = 0;
			constexpr std::array<uint8_t, 3> PAUSE_TYPES{
				PAUSED_GAME_TYPE_PLAY_SKILL_VIDEO,
				PAUSED_GAME_TYPE_ULTRA_PASSIVE_EFFECT,
				PAUSED_GAME_TYPE_JIN_EFFECT,
			};
			for (const uint8_t paused_type : PAUSE_TYPES)
			{
				bool paused = false;
				if (!QueryGamePausedByType(
					player_controller, process_event, paused_type, paused))
					return false;
				if (paused)
					pause_type_mask |= 1u << paused_type;
			}
			return true;
		}

		bool IsPlayerControllerApi(SdkReadApi api)
		{
			return api == SdkReadApi::PlayerCharacter ||
				api == SdkReadApi::PlayerState ||
				api == SdkReadApi::GamePaused;
		}

		UeFunction* FindSdkFunctionWithName(
			UeClass* object_class,
			SdkReadApi api,
			const char* function_name)
		{
			if (IsPlayerControllerApi(api))
			{
				const auto owner_name =
					NTE_OBFUSCATE_STRING("HTPlayerController");
				return FindFunction(
					object_class, owner_name.c_str(), function_name);
			}
			const auto owner_name =
				NTE_OBFUSCATE_STRING("HTAbilityCharacter");
			return FindFunction(object_class, owner_name.c_str(), function_name);
		}

		UeFunction* FindSdkFunction(UeClass* object_class, SdkReadApi api)
		{
			switch (api)
			{
			case SdkReadApi::PlayerCharacter:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetPlayerCharacter");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::PlayerState:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetHTPlayerState");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::GamePaused:
			{
				const auto name = NTE_OBFUSCATE_STRING("IsGamePaused");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::AttackTarget:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetAttackTarget");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CurrentWeapon:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetCurrentWeapon");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterLevel:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetCharacterLevel");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterHpMilli:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetHP");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterHpMaxMilli:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetHPMax");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterIsAlive:
			{
				const auto name = NTE_OBFUSCATE_STRING("CharacterIsAlive");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterIsDead:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetIsDead");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterIsControlled:
			{
				const auto name =
					NTE_OBFUSCATE_STRING("GetIsControlledCharacter");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			case SdkReadApi::CharacterSlomoMilli:
			{
				const auto name = NTE_OBFUSCATE_STRING("GetSlomoValue");
				return FindSdkFunctionWithName(
					object_class, api, name.c_str());
			}
			}
			return nullptr;
		}

		UeFunction* ResolveSdkFunction(UeClass* object_class, SdkReadApi api)
		{
			SdkFunctionCache& cache = IsPlayerControllerApi(api)
				? player_controller_sdk_cache
				: ability_character_sdk_cache;
			if (cache.object_class != object_class)
				cache = { object_class, {} };

			const size_t index = static_cast<size_t>(api);
			if (cache.functions[index] == nullptr)
				cache.functions[index] = FindSdkFunction(object_class, api);
			return cache.functions[index];
		}

		bool InvokeSdkFunction(
			UeObject* object,
			SdkReadApi api,
			void* params)
		{
			if (!memory::IsReadableRange(object, sizeof(UeObject)) ||
				object->object_class == nullptr ||
				!memory::IsReadableRange(
					object->vtable,
					(PROCESS_EVENT_INDEX + 1) * sizeof(void*)))
				return false;

			UeFunction* function = ResolveSdkFunction(object->object_class, api);
			if (function == nullptr ||
				!memory::IsReadableRange(
					function,
					offsetof(UeFunction, function_flags) + sizeof(uint32_t)))
				return false;

			const auto process_event = reinterpret_cast<ProcessEvent>(
				object->vtable[PROCESS_EVENT_INDEX]);
			if (!memory::IsExecutableAddress(
				reinterpret_cast<const void*>(process_event)))
				return false;

			const uint32_t original_flags = function->function_flags;
			function->function_flags |= NATIVE_FUNCTION_FLAG;
			process_event(object, function, params);
			function->function_flags = original_flags;
			return true;
		}

		bool FloatToMilli(float value, uint64_t& output)
		{
			constexpr float MAX_MILLI_INPUT = 9.0e15f;
			if (value != value ||
				value < -MAX_MILLI_INPUT || value > MAX_MILLI_INPUT)
				return false;
			output = static_cast<uint64_t>(
				static_cast<int64_t>(value * 1000.0f));
			return true;
		}

		void RecordCombatClockTransition(
			uint32_t pause_type_mask,
			uint32_t state_flags)
		{
			NteCombatClockTransition& transition =
				combat_clock_history[combat_clock_history_next];
			transition = {
				next_combat_clock_sequence++,
				CurrentFileTime100ns(),
				pause_type_mask,
				0,
				state_flags,
				0,
			};
			combat_clock_history_next =
				(combat_clock_history_next + 1) % NTE_COMBAT_CLOCK_HISTORY_SIZE;
			if (combat_clock_history_count < NTE_COMBAT_CLOCK_HISTORY_SIZE)
				++combat_clock_history_count;
		}

		EquipmentFunction IdentifyEquipmentFunction(const UeName& name)
		{
			DecodedUeName decoded{};
			if (!DecodeName(name, decoded))
				return EquipmentFunction::Count;

			const auto one_key =
				NTE_OBFUSCATE_STRING("ServerEquipmentInlayOneKey");
			if (DecodedNameEquals(decoded, one_key.c_str()))
				return EquipmentFunction::EquipOneKey;
			const auto unequip_all =
				NTE_OBFUSCATE_STRING("ServerEquipmentClear");
			if (DecodedNameEquals(decoded, unequip_all.c_str()))
				return EquipmentFunction::UnequipAll;
			const auto equip_module =
				NTE_OBFUSCATE_STRING("ServerEquipmentInlay");
			if (DecodedNameEquals(decoded, equip_module.c_str()))
				return EquipmentFunction::EquipModule;
			const auto unequip_module =
				NTE_OBFUSCATE_STRING("ServerEquipmentErase");
			if (DecodedNameEquals(decoded, unequip_module.c_str()))
				return EquipmentFunction::UnequipModule;
			const auto equip_core =
				NTE_OBFUSCATE_STRING("ServerEquipmentInlayCore");
			if (DecodedNameEquals(decoded, equip_core.c_str()))
				return EquipmentFunction::EquipCore;
			const auto unequip_core =
				NTE_OBFUSCATE_STRING("ServerEquipmentEraseCore");
			if (DecodedNameEquals(decoded, unequip_core.c_str()))
				return EquipmentFunction::UnequipCore;
			const auto move_module = NTE_OBFUSCATE_STRING(
				"ServerEquipmentEraseAndInlayToOther");
			if (DecodedNameEquals(decoded, move_module.c_str()))
				return EquipmentFunction::MoveModuleToCharacter;
			const auto move_core = NTE_OBFUSCATE_STRING(
				"ServerEquipmentCoreEraseAndInlayToOther");
			if (DecodedNameEquals(decoded, move_core.c_str()))
				return EquipmentFunction::MoveCoreToCharacter;
			const auto set_discarded =
				NTE_OBFUSCATE_STRING("ServerEquipmentItemDiscard");
			if (DecodedNameEquals(decoded, set_discarded.c_str()))
				return EquipmentFunction::SetItemDiscarded;
			const auto set_locked =
				NTE_OBFUSCATE_STRING("ServerEquipmentItemLocked");
			if (DecodedNameEquals(decoded, set_locked.c_str()))
				return EquipmentFunction::SetItemLocked;
			return EquipmentFunction::Count;
		}

		bool BuildFunctionCache(UeClass* object_class)
		{
			EquipmentFunctionCache candidate{};
			candidate.player_state_class = object_class;
			size_t function_count = 0;

			for (auto* current = static_cast<UeStruct*>(object_class);
				current != nullptr;
				current = current->super)
			{
				if (!memory::IsReadableRange(current, sizeof(UeStruct)))
					return false;
				if (!NameEquals(
					current->name,
					NTE_OBFUSCATE_STRING("HTPlayerState").c_str()))
					continue;

				for (UeField* field = current->children;
					field != nullptr;
					field = field->next)
				{
					if (!memory::IsReadableRange(field, sizeof(UeField)))
						return false;

					uint64_t cast_flags = 0;
					if (!memory::ReadValue(
						field->object_class,
						offsetof(UeClass, cast_flags),
						cast_flags) ||
						(cast_flags & FUNCTION_CAST_FLAG) == 0)
						continue;

					const EquipmentFunction function =
						IdentifyEquipmentFunction(field->name);
					if (function == EquipmentFunction::Count)
						continue;

					auto& cached = candidate.functions[
						static_cast<size_t>(function)];
					if (cached == nullptr)
					{
						cached = reinterpret_cast<UeFunction*>(field);
						if (++function_count == candidate.functions.size())
							break;
					}
				}
				break;
			}

			candidate.initialized = true;
			function_cache = candidate;
			return true;
		}

		UeFunction* ResolveFunction(
			UeClass* object_class,
			EquipmentFunction function)
		{
			if (!function_cache.initialized ||
				function_cache.player_state_class != object_class)
			{
				if (!BuildFunctionCache(object_class))
					return nullptr;
			}
			return function_cache.functions[static_cast<size_t>(function)];
		}

		UeItemNetId ToUeItemId(const NteItemNetId& item)
		{
			return UeItemNetId{ item.slot, item.serial };
		}

		NteModsStatus Dispatch(
			const PluginContext& context,
			EquipmentFunction function_id,
			void* params)
		{
			auto* player_state = static_cast<UeObject*>(context.player_state);
			if (!memory::IsReadableRange(player_state, sizeof(UeObject)) ||
				player_state->object_class == nullptr ||
				!memory::IsReadableRange(
					player_state->vtable,
					(PROCESS_EVENT_INDEX + 1) * sizeof(void*)))
				return NTE_MODS_STATUS_INVALID_PLAYER_STATE;

			UeFunction* function = ResolveFunction(
				player_state->object_class, function_id);
			if (function == nullptr)
				return NTE_MODS_STATUS_FUNCTION_NOT_FOUND;
			if (!memory::IsReadableRange(
				function, offsetof(UeFunction, function_flags) + sizeof(uint32_t)))
				return NTE_MODS_STATUS_FUNCTION_NOT_FOUND;

			const auto process_event = reinterpret_cast<ProcessEvent>(
				player_state->vtable[PROCESS_EVENT_INDEX]);
			if (!memory::IsExecutableAddress(reinterpret_cast<const void*>(process_event)))
				return NTE_MODS_STATUS_INVALID_PLAYER_STATE;

			const auto original_flags = function->function_flags;
			function->function_flags |= NATIVE_FUNCTION_FLAG;
			process_event(player_state, function, params);
			function->function_flags = original_flags;

			return NTE_MODS_STATUS_RPC_DISPATCHED;
		}
	} // namespace

	bool IsEquipmentRpcCacheReady()
	{
		return function_cache.initialized;
	}

	bool IsEquipmentRpcCacheReadyFor(const PluginContext* context)
	{
		if (ValidateContext(context) != NTE_MODS_STATUS_DRY_RUN_OK)
			return false;

		auto* player_state = static_cast<UeObject*>(context->player_state);
		return memory::IsReadableRange(player_state, sizeof(UeObject)) &&
			player_state->object_class != nullptr &&
			function_cache.initialized &&
			function_cache.player_state_class == player_state->object_class;
	}

	void PrepareEquipmentRpcCache(const PluginContext* context)
	{
		if (ValidateContext(context) != NTE_MODS_STATUS_DRY_RUN_OK)
			return;

		auto* player_state = static_cast<UeObject*>(context->player_state);
		if (!memory::IsReadableRange(player_state, sizeof(UeObject)) ||
			player_state->object_class == nullptr)
			return;

		if (!function_cache.initialized ||
			function_cache.player_state_class != player_state->object_class)
			BuildFunctionCache(player_state->object_class);
	}

	uint64_t SampleCombatClockState(void* player_controller)
	{
		uint32_t pause_type_mask = 0;
		uint32_t state_flags = 0;
		if (player_controller != nullptr &&
			ReadRelevantGamePauseMask(
				static_cast<UeObject*>(player_controller), pause_type_mask))
			state_flags |= NTE_COMBAT_CLOCK_PAUSE_VALID;
		return (static_cast<uint64_t>(state_flags) << 32) | pause_type_mask;
	}

	void ForwardCombatClockState(
		uint32_t pause_type_mask,
		uint32_t state_flags)
	{
		if ((pause_type_mask & ~COMBAT_CLOCK_RELEVANT_PAUSE_MASK) != 0 ||
			(state_flags & ~NTE_COMBAT_CLOCK_PAUSE_VALID) != 0 ||
			(state_flags == 0 && pause_type_mask != 0))
			return;
		RecordCombatClockTransition(pause_type_mask, state_flags);
	}

	bool InvokeSdkReadApi(
		void* object,
		SdkReadApi api,
		uint64_t argument,
		uint64_t& result)
	{
		auto* ue_object = static_cast<UeObject*>(object);
		result = 0;
		switch (api)
		{
		case SdkReadApi::PlayerCharacter:
		case SdkReadApi::PlayerState:
		case SdkReadApi::AttackTarget:
		case SdkReadApi::CurrentWeapon:
		{
			if (argument != 0)
				return false;
			PointerReturnParams params{};
			if (!InvokeSdkFunction(ue_object, api, &params))
				return false;
			result = reinterpret_cast<uint64_t>(params.return_value);
			return true;
		}
		case SdkReadApi::GamePaused:
		case SdkReadApi::CharacterIsAlive:
		case SdkReadApi::CharacterIsDead:
		case SdkReadApi::CharacterIsControlled:
		{
			if (argument != 0)
				return false;
			BoolReturnParams params{};
			if (!InvokeSdkFunction(ue_object, api, &params))
				return false;
			result = params.return_value != 0;
			return true;
		}
		case SdkReadApi::CharacterLevel:
		{
			if (argument != 0)
				return false;
			Int32ReturnParams params{};
			if (!InvokeSdkFunction(ue_object, api, &params))
				return false;
			result = static_cast<uint64_t>(
				static_cast<int64_t>(params.return_value));
			return true;
		}
		case SdkReadApi::CharacterHpMilli:
		case SdkReadApi::CharacterSlomoMilli:
		{
			if (argument != 0)
				return false;
			FloatReturnParams params{};
			return InvokeSdkFunction(ue_object, api, &params) &&
				FloatToMilli(params.return_value, result);
		}
		case SdkReadApi::CharacterHpMaxMilli:
		{
			if (argument > 1)
				return false;
			BoolFloatReturnParams params{
				static_cast<uint8_t>(argument),
				{},
				0.0f,
			};
			return InvokeSdkFunction(ue_object, api, &params) &&
				FloatToMilli(params.return_value, result);
		}
		}
		return false;
	}

	bool ReadNameHash(
		const void* object,
		uint64_t offset,
		uint64_t& result)
	{
		result = 0;
		UeName name{};
		if (!memory::ReadValue(
				object,
				static_cast<size_t>(offset),
				name) ||
			name.comparison_index < 0)
			return false;
		DecodedUeName decoded{};
		return DecodeName(name, decoded) &&
			HashDecodedName(decoded, result);
	}

	bool FindReflectedFunction(
		void* object,
		const char* owner_class_name,
		const char* function_name,
		void*& result)
	{
		result = nullptr;
		auto* ue_object = static_cast<UeObject*>(object);
		if (!memory::IsReadableRange(ue_object, sizeof(UeObject)) ||
			ue_object->object_class == nullptr ||
			owner_class_name == nullptr ||
			function_name == nullptr)
			return false;
		result = FindFunction(
			ue_object->object_class,
			owner_class_name,
			function_name);
		return result != nullptr;
	}

	bool ReflectedFunctionParamSize(
		void* function,
		uint16_t& size)
	{
		size = 0;
		auto* ue_function = static_cast<UeFunction*>(function);
		if (!memory::IsReadableRange(
				ue_function,
				FUNCTION_PARAM_SIZE_OFFSET + sizeof(uint16_t)))
			return false;
		uint64_t cast_flags = 0;
		return memory::ReadValue(
				ue_function->object_class,
				offsetof(UeClass, cast_flags),
				cast_flags) &&
			(cast_flags & FUNCTION_CAST_FLAG) != 0 &&
			memory::ReadValue(
				ue_function,
				FUNCTION_PARAM_SIZE_OFFSET,
				size);
	}

	bool InvokeReflectedFunction(
		void* object,
		void* function,
		void* params,
		uint16_t params_size)
	{
		auto* ue_object = static_cast<UeObject*>(object);
		auto* ue_function = static_cast<UeFunction*>(function);
		if (!memory::IsReadableRange(ue_object, sizeof(UeObject)) ||
			ue_object->object_class == nullptr ||
			!memory::IsReadableRange(
				ue_object->vtable,
				(PROCESS_EVENT_INDEX + 1) * sizeof(void*)) ||
			(params_size != 0 &&
				!memory::IsReadableRange(params, params_size)))
			return false;

		uint16_t reflected_params_size = 0;
		if (!ReflectedFunctionParamSize(
				ue_function,
				reflected_params_size) ||
			reflected_params_size != params_size)
			return false;

		const auto process_event = reinterpret_cast<ProcessEvent>(
			ue_object->vtable[PROCESS_EVENT_INDEX]);
		if (!memory::IsExecutableAddress(
			reinterpret_cast<const void*>(process_event)))
			return false;

		const uint32_t original_flags = ue_function->function_flags;
		ue_function->function_flags |= NATIVE_FUNCTION_FLAG;
		process_event(
			ue_object,
			ue_function,
			params_size == 0 ? nullptr : params);
		ue_function->function_flags = original_flags;
		return true;
	}

	uint32_t CopyCombatClockTransitions(
		NteCombatClockTransition* output,
		uint32_t capacity)
	{
		if (output == nullptr || capacity == 0)
			return 0;

		const uint32_t copy_count =
			capacity < combat_clock_history_count
				? capacity
				: combat_clock_history_count;
		const uint32_t first =
			(combat_clock_history_next +
				NTE_COMBAT_CLOCK_HISTORY_SIZE - copy_count) %
			NTE_COMBAT_CLOCK_HISTORY_SIZE;
		for (uint32_t index = 0; index < copy_count; ++index)
		{
			output[index] = combat_clock_history[
				(first + index) % NTE_COMBAT_CLOCK_HISTORY_SIZE];
		}
		return copy_count;
	}

	NteModsStatus EquipOneKey(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteEquipmentPlacement* placements,
		uint32_t placement_count,
		const NteItemNetId* core)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;

		if (placement_count > NTE_EQUIPMENT_MAX_PLACEMENTS)
			return NTE_MODS_STATUS_TOO_MANY_PLACEMENTS;
		if (placement_count == 0)
			return NTE_MODS_STATUS_EMPTY_LOADOUT;
		if (placements == nullptr)
			return NTE_MODS_STATUS_INVALID_PLACEMENT_BUFFER;
		if (!IsValidItemId(core))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;

		std::array<UeEquipPlaceData, NTE_EQUIPMENT_MAX_PLACEMENTS>
			sdk_placements{};
		for (uint32_t index = 0; index < placement_count; ++index)
		{
			const NteEquipmentPlacement& placement = placements[index];
			if (!IsValidItemId(&placement.equipment))
				return NTE_MODS_STATUS_INVALID_ITEM_ID;
			if (!IsValidGridPosition(placement.row, placement.column))
				return NTE_MODS_STATUS_INVALID_GRID_POSITION;

			sdk_placements[index] = UeEquipPlaceData{
				ToUeItemId(placement.equipment), placement.row, placement.column };
		}

		OneKeyParams params{};
		params.character = ToUeItemId(*character);
		params.placements = UeArrayView{
			sdk_placements.data(),
			static_cast<int32_t>(placement_count),
			static_cast<int32_t>(placement_count) };
		params.core = ToUeItemId(*core);

		return Dispatch(
			*context,
			EquipmentFunction::EquipOneKey,
			&params);
	}

	NteModsStatus UnequipAll(
		const PluginContext* context,
		const NteItemNetId* character)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;

		SingleItemParams params{};
		params.item = ToUeItemId(*character);
		return Dispatch(
			*context,
			EquipmentFunction::UnequipAll,
			&params);
	}

	NteModsStatus EquipModule(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;
		if (!IsValidGridPosition(row, column))
			return NTE_MODS_STATUS_INVALID_GRID_POSITION;

		PositionedItemParams params{};
		params.character = ToUeItemId(*character);
		params.equipment = ToUeItemId(*equipment);
		params.row = row;
		params.column = column;
		return Dispatch(
			*context,
			EquipmentFunction::EquipModule,
			&params);
	}

	NteModsStatus UnequipModule(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*equipment);
		return Dispatch(
			*context,
			EquipmentFunction::UnequipModule,
			&params);
	}

	NteModsStatus EquipCore(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			EquipmentFunction::EquipCore,
			&params);
	}

	NteModsStatus UnequipCore(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			EquipmentFunction::UnequipCore,
			&params);
	}

	NteModsStatus MoveModuleToCharacter(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;
		if (!IsValidGridPosition(row, column))
			return NTE_MODS_STATUS_INVALID_GRID_POSITION;

		PositionedItemParams params{};
		params.character = ToUeItemId(*character);
		params.equipment = ToUeItemId(*equipment);
		params.row = row;
		params.column = column;
		return Dispatch(
			*context,
			EquipmentFunction::MoveModuleToCharacter,
			&params);
	}

	NteModsStatus MoveCoreToCharacter(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteModsStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_MODS_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			EquipmentFunction::MoveCoreToCharacter,
			&params);
	}

	NteModsStatus SetItemDiscarded(
		const PluginContext* context,
		const NteItemNetId* item,
		uint32_t discarded)
	{
		const NteModsStatus argument_status = ValidateItemArgument(context, item);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (discarded > 1)
			return NTE_MODS_STATUS_INVALID_BOOLEAN_VALUE;

		ItemBooleanParams params{};
		params.item = ToUeItemId(*item);
		params.state = static_cast<uint8_t>(discarded);
		return Dispatch(
			*context,
			EquipmentFunction::SetItemDiscarded,
			&params);
	}

	NteModsStatus SetItemLocked(
		const PluginContext* context,
		const NteItemNetId* item,
		uint32_t locked)
	{
		const NteModsStatus argument_status = ValidateItemArgument(context, item);
		if (argument_status != NTE_MODS_STATUS_DRY_RUN_OK)
			return argument_status;
		if (locked > 1)
			return NTE_MODS_STATUS_INVALID_BOOLEAN_VALUE;

		ItemBooleanParams params{};
		params.item = ToUeItemId(*item);
		params.state = static_cast<uint8_t>(locked);
		return Dispatch(
			*context,
			EquipmentFunction::SetItemLocked,
			&params);
	}
} // namespace nte::mods
