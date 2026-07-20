#include "equipment_rpc.hpp"

#include "memory_access.hpp"
#include "obfuscated_string.hpp"
#include "offset_resolver.hpp"

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::equipment
{
	namespace
	{
		constexpr uint32_t NATIVE_FUNCTION_FLAG = 0x400;
		constexpr size_t PROCESS_EVENT_INDEX = 0x4C;
		constexpr uint64_t FUNCTION_CAST_FLAG = 0x0000000000080000;

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

		using AppendName = void(__fastcall*)(const UeName*, UeStringBuffer&);
		using ProcessEvent = void(__fastcall*)(
			const UeObject*, UeFunction*, void*);

		static_assert(sizeof(EquipmentContext) == 8);
		static_assert(sizeof(NteEquipmentStatus) == 4);
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

		bool IsValidItemId(const NteItemNetId* item)
		{
			return item != nullptr && (item->slot != 0 || item->serial != 0);
		}

		bool IsValidGridPosition(int32_t row, int32_t column)
		{
			return row >= NTE_EQUIPMENT_GRID_MIN && row <= NTE_EQUIPMENT_GRID_MAX &&
				column >= NTE_EQUIPMENT_GRID_MIN && column <= NTE_EQUIPMENT_GRID_MAX;
		}

		NteEquipmentStatus ValidateContext(const EquipmentContext* context)
		{
			if (context == nullptr)
				return NTE_EQUIPMENT_STATUS_INVALID_CONTEXT;

			if (context->player_state == nullptr)
				return NTE_EQUIPMENT_STATUS_INVALID_PLAYER_STATE;

			return NTE_EQUIPMENT_STATUS_DRY_RUN_OK;
		}

		NteEquipmentStatus ValidateCharacterArgument(
			const EquipmentContext* context,
			const NteItemNetId* character)
		{
			const NteEquipmentStatus context_status = ValidateContext(context);
			if (context_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
				return context_status;

			if (!IsValidItemId(character))
				return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

			return NTE_EQUIPMENT_STATUS_DRY_RUN_OK;
		}

		NteEquipmentStatus ValidateItemArgument(
			const EquipmentContext* context,
			const NteItemNetId* item)
		{
			const NteEquipmentStatus context_status = ValidateContext(context);
			if (context_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
				return context_status;

			if (!IsValidItemId(item))
				return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

			return NTE_EQUIPMENT_STATUS_DRY_RUN_OK;
		}

		bool NameEquals(const UeName& name, const char* expected)
		{
			wchar_t buffer[256]{};
			UeStringBuffer output{ buffer, 0, static_cast<int32_t>(_countof(buffer)) };
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

			int32_t length = output.count;
			if (length > 0 && buffer[length - 1] == L'\0')
				--length;
			int32_t first = 0;
			for (int32_t index = 0; index < length; ++index)
			{
				if (buffer[index] == L'/')
					first = index + 1;
			}

			int32_t index = first;
			while (*expected != '\0' && index < length)
			{
				if (buffer[index] != static_cast<unsigned char>(*expected))
					return false;
				++index;
				++expected;
			}
			return *expected == '\0' && index == length;
		}

		UeFunction* FindFunction(UeClass* object_class, const char* function_name)
		{
			for (auto* current = static_cast<UeStruct*>(object_class);
				current != nullptr;
				current = current->super)
			{
				if (!memory::IsReadableRange(current, sizeof(UeStruct)))
					return nullptr;
				if (!NameEquals(
					current->name,
					NTE_OBFUSCATE_STRING("HTPlayerState").c_str()))
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
			}
			return nullptr;
		}

		UeItemNetId ToUeItemId(const NteItemNetId& item)
		{
			return UeItemNetId{ item.slot, item.serial };
		}

		NteEquipmentStatus Dispatch(
			const EquipmentContext& context,
			const char* function_name,
			void* params)
		{
			auto* player_state = static_cast<UeObject*>(context.player_state);
			if (!memory::IsReadableRange(player_state, sizeof(UeObject)) ||
				player_state->object_class == nullptr ||
				!memory::IsReadableRange(
					player_state->vtable,
					(PROCESS_EVENT_INDEX + 1) * sizeof(void*)))
				return NTE_EQUIPMENT_STATUS_INVALID_PLAYER_STATE;

			UeFunction* function = FindFunction(
				player_state->object_class, function_name);
			if (function == nullptr)
				return NTE_EQUIPMENT_STATUS_FUNCTION_NOT_FOUND;
			if (!memory::IsReadableRange(
				function, offsetof(UeFunction, function_flags) + sizeof(uint32_t)))
				return NTE_EQUIPMENT_STATUS_FUNCTION_NOT_FOUND;

			const auto process_event = reinterpret_cast<ProcessEvent>(
				player_state->vtable[PROCESS_EVENT_INDEX]);
			if (!memory::IsExecutableAddress(reinterpret_cast<const void*>(process_event)))
				return NTE_EQUIPMENT_STATUS_INVALID_PLAYER_STATE;

			const auto original_flags = function->function_flags;
			function->function_flags |= NATIVE_FUNCTION_FLAG;
			process_event(player_state, function, params);
			function->function_flags = original_flags;

			return NTE_EQUIPMENT_STATUS_RPC_DISPATCHED;
		}
	} // namespace

	NteEquipmentStatus EquipOneKey(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteEquipmentPlacement* placements,
		uint32_t placement_count,
		const NteItemNetId* core)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;

		if (placement_count > NTE_EQUIPMENT_MAX_PLACEMENTS)
			return NTE_EQUIPMENT_STATUS_TOO_MANY_PLACEMENTS;
		if (placement_count == 0)
			return NTE_EQUIPMENT_STATUS_EMPTY_LOADOUT;
		if (placements == nullptr)
			return NTE_EQUIPMENT_STATUS_INVALID_PLACEMENT_BUFFER;
		if (!IsValidItemId(core))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

		std::array<UeEquipPlaceData, NTE_EQUIPMENT_MAX_PLACEMENTS>
			sdk_placements{};
		for (uint32_t index = 0; index < placement_count; ++index)
		{
			const NteEquipmentPlacement& placement = placements[index];
			if (!IsValidItemId(&placement.equipment))
				return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;
			if (!IsValidGridPosition(placement.row, placement.column))
				return NTE_EQUIPMENT_STATUS_INVALID_GRID_POSITION;

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
			NTE_OBFUSCATE_STRING("ServerEquipmentInlayOneKey").c_str(),
			&params);
	}

	NteEquipmentStatus UnequipAll(
		const EquipmentContext* context,
		const NteItemNetId* character)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;

		SingleItemParams params{};
		params.item = ToUeItemId(*character);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentClear").c_str(),
			&params);
	}

	NteEquipmentStatus EquipModule(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;
		if (!IsValidGridPosition(row, column))
			return NTE_EQUIPMENT_STATUS_INVALID_GRID_POSITION;

		PositionedItemParams params{};
		params.character = ToUeItemId(*character);
		params.equipment = ToUeItemId(*equipment);
		params.row = row;
		params.column = column;
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentInlay").c_str(),
			&params);
	}

	NteEquipmentStatus UnequipModule(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*equipment);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentErase").c_str(),
			&params);
	}

	NteEquipmentStatus EquipCore(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentInlayCore").c_str(),
			&params);
	}

	NteEquipmentStatus UnequipCore(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentEraseCore").c_str(),
			&params);
	}

	NteEquipmentStatus MoveModuleToCharacter(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(equipment))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;
		if (!IsValidGridPosition(row, column))
			return NTE_EQUIPMENT_STATUS_INVALID_GRID_POSITION;

		PositionedItemParams params{};
		params.character = ToUeItemId(*character);
		params.equipment = ToUeItemId(*equipment);
		params.row = row;
		params.column = column;
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING(
				"ServerEquipmentEraseAndInlayToOther").c_str(),
			&params);
	}

	NteEquipmentStatus MoveCoreToCharacter(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core)
	{
		const NteEquipmentStatus argument_status = ValidateCharacterArgument(context, character);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (!IsValidItemId(core))
			return NTE_EQUIPMENT_STATUS_INVALID_ITEM_ID;

		TwoItemParams params{};
		params.first = ToUeItemId(*character);
		params.second = ToUeItemId(*core);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING(
				"ServerEquipmentCoreEraseAndInlayToOther").c_str(),
			&params);
	}

	NteEquipmentStatus SetItemDiscarded(
		const EquipmentContext* context,
		const NteItemNetId* item,
		uint32_t discarded)
	{
		const NteEquipmentStatus argument_status = ValidateItemArgument(context, item);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (discarded > 1)
			return NTE_EQUIPMENT_STATUS_INVALID_BOOLEAN_VALUE;

		ItemBooleanParams params{};
		params.item = ToUeItemId(*item);
		params.state = static_cast<uint8_t>(discarded);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentItemDiscard").c_str(),
			&params);
	}

	NteEquipmentStatus SetItemLocked(
		const EquipmentContext* context,
		const NteItemNetId* item,
		uint32_t locked)
	{
		const NteEquipmentStatus argument_status = ValidateItemArgument(context, item);
		if (argument_status != NTE_EQUIPMENT_STATUS_DRY_RUN_OK)
			return argument_status;
		if (locked > 1)
			return NTE_EQUIPMENT_STATUS_INVALID_BOOLEAN_VALUE;

		ItemBooleanParams params{};
		params.item = ToUeItemId(*item);
		params.state = static_cast<uint8_t>(locked);
		return Dispatch(
			*context,
			NTE_OBFUSCATE_STRING("ServerEquipmentItemLocked").c_str(),
			&params);
	}
} // namespace nte::equipment
