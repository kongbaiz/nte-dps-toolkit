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
	};

	enum class IpcPumpResult : int32_t
	{
		Error = -1,
		Idle = 0,
		Processed = 1,
	};

	IpcPumpResult PumpLiveIpc(const PluginContext* context);
	bool OpenRuntimePresence();
	void CloseRuntimePresence();
	NteModsStatus InvokeIpcKernelService(
		IpcKernelService service,
		const PluginContext* context,
		const NteModsIpcRequest& request,
		NteModsIpcResponse& response);
	void CloseIpc();
} // namespace nte::mods
