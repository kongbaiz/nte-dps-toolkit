#pragma once

#include "host_api.hpp"

#include <cstdint>

namespace nte::mods
{
	enum class IpcKernelService : uint8_t
	{
		EquipModule,
		EquipCore,
		UnequipModule,
		UnequipCore,
		UnequipAll,
		EquipOneKey,
		MoveModuleToCharacter,
		MoveCoreToCharacter,
		SetItemDiscarded,
		SetItemLocked,
		QueryCombatClockTransitions,
		QueryModEvents,
		QueryModLogs,
		QueryCharacterEffects,
	};

	enum class IpcPumpResult : int32_t
	{
		Error = -1,
		Idle = 0,
		Processed = 1,
	};

	enum class IpcCloseResult : uint8_t
	{
		Closed,
		DrainFailed,
	};

	enum class RuntimePresenceCloseResult : uint8_t
	{
		Closed,
		CloseFailed,
	};

	IpcPumpResult PumpLiveIpc(const PluginContext* context);
	bool OpenRuntimePresence();
	RuntimePresenceCloseResult CloseRuntimePresence();
	NteModsStatus InvokeIpcKernelService(
		IpcKernelService service,
		const PluginContext* context,
		const NteModsIpcRequest& request,
		NteModsIpcResponse& response);
	void SetIpcStopping(bool stopping);
	IpcCloseResult CloseIpc();
} // namespace nte::mods
