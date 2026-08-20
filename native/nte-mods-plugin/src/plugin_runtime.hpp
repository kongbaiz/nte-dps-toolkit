#pragma once

#include <Windows.h>

#include <array>
#include <cstddef>
#include <cstdint>

namespace nte::mods
{
	// Reflected damage callbacks reach offset 0x15C.
	constexpr size_t PROCESS_EVENT_PARAM_CAPACITY = 512;

	struct ProcessEventRecord
	{
		void* object;
		void* function;
		uint16_t params_size;
		std::array<uint8_t, PROCESS_EVENT_PARAM_CAPACITY> params;
		uint64_t captured_u64;
	};

	enum class PluginStartResult : uint8_t
	{
		Started,
		AlreadyRunning,
		NotGameHost,
		InProgress,
		Failed,
	};

	enum class PluginStopResult : uint8_t
	{
		UnloadSafe,
		Resident,
		InProgress,
		TeardownIncomplete,
	};

	PluginStartResult StartPluginRuntime(HMODULE module);
	void RecordPluginModule(HMODULE module) noexcept;
	// Proxy deployments call this from DllMain only to create one finite worker.
	// Windows serializes DLL initialization, so the worker body cannot execute
	// until the current loader-lock callback has returned.
	bool ScheduleRecordedPluginRuntimeInitialization() noexcept;
	PluginStartResult InitializeRecordedPluginRuntime();
	PluginStopResult StopPluginRuntime();
	constexpr bool PluginStopAllowsUnload(PluginStopResult result) noexcept
	{
		return result == PluginStopResult::UnloadSafe;
	}
	bool WatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function);
	bool WatchProcessEventArrayU64(
		uint32_t program_index,
		void* object,
		void* function,
		uint64_t element_size,
		uint64_t value_offset);
	bool WatchProcessEventClassArrayU64(
		uint32_t program_index,
		void* object,
		void* function,
		uint64_t element_size,
		uint64_t value_offset);
	bool UnwatchProcessEvent(
		uint32_t program_index,
		void* object,
		void* function);
	bool PopProcessEvent(
		uint32_t program_index,
		ProcessEventRecord& event);
	bool ResetProcessEventWatches();
} // namespace nte::mods
