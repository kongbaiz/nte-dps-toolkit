#pragma once

#include "host_api.hpp"
#include "nte_mods_ipc.h"

#include <Windows.h>

#include <cstdint>

namespace nte::mods::runtime
{
	constexpr uint32_t CAPABILITY_VIEWPORT_TICK = 1u << 0;
	constexpr uint32_t CAPABILITY_MEMORY_READ = 1u << 1;
	constexpr uint32_t CAPABILITY_IPC = 1u << 2;
	constexpr uint32_t CAPABILITY_SDK_READ = 1u << 3;
	constexpr uint32_t CAPABILITY_EQUIPMENT = 1u << 4;
	constexpr uint32_t CAPABILITY_COMBAT_CLOCK = 1u << 5;
	constexpr uint32_t CAPABILITY_LOG = 1u << 6;
	constexpr uint32_t CAPABILITY_GAME_SESSION = 1u << 7;
	constexpr uint32_t CAPABILITY_MEMORY_WRITE = 1u << 8;
	constexpr uint32_t CAPABILITY_UNREAL_REFLECTION = 1u << 9;
	constexpr uint32_t CAPABILITY_PROCESS_EVENT = 1u << 10;

	enum class ReloadResult
	{
		Unchanged,
		Changed,
		Error,
	};

	ReloadResult ReloadEnabledPrograms(const wchar_t* workspace);
	bool HasViewportTickPrograms();
	uint32_t EnabledCapabilities();
	void ExecuteViewportTickPrograms(void* viewport);
	NteModsStatus DispatchIpcRequestPrograms(
		const PluginContext* context,
		const NteModsIpcRequest& request,
		NteModsIpcResponse& response);
	uint32_t CopyModEvents(NteModEvent* output, uint32_t capacity);
	void Reset();
} // namespace nte::mods::runtime
