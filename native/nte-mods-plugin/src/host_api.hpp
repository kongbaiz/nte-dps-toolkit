#pragma once

#include "nte_mods_ipc.h"

#include <cstdint>

namespace nte::mods
{
	struct PluginContext
	{
		void* player_state;
		void* player_controller;
	};

	enum class SdkReadApi : uint8_t
	{
		PlayerCharacter,
		PlayerState,
		GamePaused,
		AttackTarget,
		CurrentWeapon,
		CharacterLevel,
		CharacterHpMilli,
		CharacterHpMaxMilli,
		CharacterIsAlive,
		CharacterIsDead,
		CharacterIsControlled,
		CharacterSlomoMilli,
	};

	bool IsEquipmentRpcCacheReady();
	bool IsEquipmentRpcCacheReadyFor(const PluginContext* context);
	void PrepareEquipmentRpcCache(const PluginContext* context);
	uint64_t SampleCombatClockState(void* player_controller);
	void ForwardCombatClockState(uint32_t pause_type_mask, uint32_t state_flags);
	bool InvokeSdkReadApi(
		void* object,
		SdkReadApi api,
		uint64_t argument,
		uint64_t& result);
	uint32_t CopyCombatClockTransitions(
		NteCombatClockTransition* output,
		uint32_t capacity);

	NteModsStatus EquipOneKey(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteEquipmentPlacement* placements,
		uint32_t placement_count,
		const NteItemNetId* core);
	NteModsStatus UnequipAll(
		const PluginContext* context,
		const NteItemNetId* character);
	NteModsStatus EquipModule(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column);
	NteModsStatus UnequipModule(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment);
	NteModsStatus EquipCore(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteModsStatus UnequipCore(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteModsStatus MoveModuleToCharacter(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* equipment,
		int32_t row,
		int32_t column);
	NteModsStatus MoveCoreToCharacter(
		const PluginContext* context,
		const NteItemNetId* character,
		const NteItemNetId* core);
	NteModsStatus SetItemDiscarded(
		const PluginContext* context,
		const NteItemNetId* item,
		uint32_t discarded);
	NteModsStatus SetItemLocked(
		const PluginContext* context,
		const NteItemNetId* item,
		uint32_t locked);
} // namespace nte::mods
