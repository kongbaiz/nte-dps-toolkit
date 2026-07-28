#pragma once

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	constexpr size_t PROCESS_EVENT_PARAM_CAPACITY = 256;

	struct ProcessEventRecord
	{
		void* object;
		void* function;
		uint16_t params_size;
		std::array<uint8_t, PROCESS_EVENT_PARAM_CAPACITY> params;
	};

	void StartPluginRuntime(HMODULE module);
	void StopPluginRuntime();
	bool WatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function);
	bool UnwatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function);
	bool PopProcessEvent(
		uint32_t program_index,
		ProcessEventRecord& event);
	void ResetProcessEventWatches();
} // namespace nte::mods
