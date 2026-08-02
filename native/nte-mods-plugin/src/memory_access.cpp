#include "memory_access.hpp"

#include <Windows.h>

namespace nte::mods::memory
{
	namespace
	{
		void CopyBytes(void* destination, const void* source, size_t size)
		{
			auto* output = static_cast<uint8_t*>(destination);
			const auto* input = static_cast<const uint8_t*>(source);
			for (size_t index = 0; index < size; ++index)
				output[index] = input[index];
		}

		bool IsWritableRange(void* address, size_t size)
		{
			if (address == nullptr || size == 0)
				return false;

			MEMORY_BASIC_INFORMATION memory{};
			if (VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
				memory.State != MEM_COMMIT ||
				(memory.Protect & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
				return false;
			const DWORD protection = memory.Protect & 0xFF;
			if (protection != PAGE_READWRITE &&
				protection != PAGE_WRITECOPY &&
				protection != PAGE_EXECUTE_READWRITE &&
				protection != PAGE_EXECUTE_WRITECOPY)
				return false;

			const uintptr_t start = reinterpret_cast<uintptr_t>(address);
			const uintptr_t region_start =
				reinterpret_cast<uintptr_t>(memory.BaseAddress);
			if (memory.RegionSize > UINTPTR_MAX - region_start)
				return false;
			const uintptr_t region_end = region_start + memory.RegionSize;
			return start >= region_start && start <= region_end &&
				size <= region_end - start;
		}
	} // namespace

	uintptr_t ImageBase()
	{
		return reinterpret_cast<uintptr_t>(GetModuleHandleW(nullptr));
	}

	bool IsReadableRange(const void* address, size_t size)
	{
		if (address == nullptr || size == 0)
			return false;

		MEMORY_BASIC_INFORMATION memory{};
		if (VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
			memory.State != MEM_COMMIT ||
			(memory.Protect & (PAGE_GUARD | PAGE_NOACCESS)) != 0)
			return false;

		const uintptr_t start = reinterpret_cast<uintptr_t>(address);
		const uintptr_t region_start = reinterpret_cast<uintptr_t>(memory.BaseAddress);
		if (memory.RegionSize > UINTPTR_MAX - region_start)
			return false;

		const uintptr_t region_end = region_start + memory.RegionSize;
		return start >= region_start && start <= region_end &&
			size <= region_end - start;
	}

	bool IsExecutableAddress(const void* address)
	{
		MEMORY_BASIC_INFORMATION memory{};
		if (address == nullptr ||
			VirtualQuery(address, &memory, sizeof(memory)) != sizeof(memory) ||
			memory.State != MEM_COMMIT || (memory.Protect & PAGE_GUARD) != 0)
			return false;

		const DWORD protection = memory.Protect & 0xFF;
		return protection == PAGE_EXECUTE || protection == PAGE_EXECUTE_READ ||
			protection == PAGE_EXECUTE_READWRITE ||
			protection == PAGE_EXECUTE_WRITECOPY;
	}

	bool ReadBytes(
		const void* base,
		size_t offset,
		void* destination,
		size_t size)
	{
		if (base == nullptr || destination == nullptr)
			return false;

		const uintptr_t base_address = reinterpret_cast<uintptr_t>(base);
		if (offset > UINTPTR_MAX - base_address)
			return false;
		const auto* address = reinterpret_cast<const void*>(base_address + offset);
		if (!IsReadableRange(address, size))
			return false;

		CopyBytes(destination, address, size);
		return true;
	}

	bool WriteBytes(
		void* base,
		size_t offset,
		const void* source,
		size_t size)
	{
		if (base == nullptr || source == nullptr)
			return false;

		const uintptr_t base_address = reinterpret_cast<uintptr_t>(base);
		if (offset > UINTPTR_MAX - base_address)
			return false;
		auto* address = reinterpret_cast<void*>(base_address + offset);
		if (!IsWritableRange(address, size))
			return false;

		CopyBytes(address, source, size);
		return true;
	}
} // namespace nte::mods::memory
