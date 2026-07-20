#pragma once

#include <cstddef>
#include <cstdint>

namespace nte::equipment::memory
{
	uintptr_t ImageBase();
	bool IsReadableRange(const void* address, size_t size);
	bool IsExecutableAddress(const void* address);
	bool ReadBytes(
		const void* base,
		size_t offset,
		void* destination,
		size_t size);

	template <typename T>
	bool ReadValue(const void* base, size_t offset, T& value)
	{
		return ReadBytes(base, offset, &value, sizeof(T));
	}

	template <typename T>
	T* ReadPointer(const void* base, size_t offset)
	{
		T* value = nullptr;
		return ReadValue(base, offset, value) ? value : nullptr;
	}
} // namespace nte::equipment::memory
