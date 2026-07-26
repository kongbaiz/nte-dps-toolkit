#pragma once

#include "host_api.hpp"

#include <cstdint>

namespace nte::mods
{
	enum class IpcPumpResult : int32_t
	{
		Error = -1,
		Idle = 0,
		Processed = 1,
	};

	IpcPumpResult PumpLiveIpc(
		const PluginContext* context,
		uint32_t capabilities);
	void CloseIpc();
} // namespace nte::mods
