#pragma once

#include "equipment_rpc.hpp"

#include <cstdint>

namespace nte::equipment
{
	enum class IpcPumpResult : int32_t
	{
		Error = -1,
		Idle = 0,
		Processed = 1,
	};

	using PlayerStateResolver = void* (*)(void* user_data);
	using PlayerControllerResolver = void* (*)(void* user_data);

	IpcPumpResult PumpLiveIpc(
		void* user_data,
		PlayerStateResolver resolve_player_state,
		PlayerControllerResolver resolve_player_controller);
	void CloseIpc();
} // namespace nte::equipment
