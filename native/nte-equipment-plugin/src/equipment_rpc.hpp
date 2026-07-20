#pragma once

#include "nte_equipment_ipc.h"

#include <cstdint>

namespace nte::equipment
{
	struct EquipmentContext
	{
		void* player_state;
	};

	NteEquipmentStatus EquipOneKey(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteEquipmentPlacement* placements,
		uint32_t placement_count,
		const NteItemNetId* core);
	NteEquipmentStatus UnequipAll(
		const EquipmentContext* context,
		const NteItemNetId* character);
	NteEquipmentStatus EquipModule(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column);
	NteEquipmentStatus UnequipModule(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment);
	NteEquipmentStatus EquipCore(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteEquipmentStatus UnequipCore(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteEquipmentStatus MoveModuleToCharacter(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column);
	NteEquipmentStatus MoveCoreToCharacter(
		const EquipmentContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteEquipmentStatus SetItemDiscarded(
		const EquipmentContext* context,
		const NteItemNetId* item,
		uint32_t discarded);
	NteEquipmentStatus SetItemLocked(
		const EquipmentContext* context,
		const NteItemNetId* item,
		uint32_t locked);
} // namespace nte::equipment
