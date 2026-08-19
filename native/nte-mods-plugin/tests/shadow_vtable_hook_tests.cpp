#include "../src/shadow_vtable_hook.hpp"

#include <Windows.h>

#include <cstdint>
#include <cstdio>

namespace
{
using Tick = int(__fastcall*)(void*, int);

struct FakeViewport
{
	void** vtable;
	int marker;
};

int __fastcall OriginalTick(void* object, int value)
{
	return static_cast<FakeViewport*>(object)->marker + value;
}

int __fastcall HookedTick(void* object, int value)
{
	return static_cast<FakeViewport*>(object)->marker + value + 1000;
}

int __fastcall AlternateTick(void* object, int value)
{
	return static_cast<FakeViewport*>(object)->marker + value + 2000;
}
} // namespace

int main()
{
	void* original_vtable[]{
		reinterpret_cast<void*>(&OriginalTick),
		nullptr,
	};
	void* alternate_vtable[]{
		reinterpret_cast<void*>(&AlternateTick),
		nullptr,
	};
	FakeViewport viewport{original_vtable, 7};

	nte::hook::ShadowVTableHook hook;
	// The binding and CAS expectation are one publication transaction. A wrong
	// original or a vptr change between capture and Install must publish nothing.
	if (hook.Install(
			&viewport,
			0,
			reinterpret_cast<void*>(&HookedTick),
			original_vtable,
			reinterpret_cast<void*>(&AlternateTick)) ||
		viewport.vtable != original_vtable)
		return 1;
	viewport.vtable = alternate_vtable;
	if (hook.Install(
			&viewport,
			0,
			reinterpret_cast<void*>(&HookedTick),
			original_vtable,
			reinterpret_cast<void*>(&OriginalTick)) ||
		viewport.vtable != alternate_vtable)
		return 2;
	viewport.vtable = original_vtable;
	if (!hook.Install(
			&viewport,
			0,
			reinterpret_cast<void*>(&HookedTick),
			original_vtable,
			reinterpret_cast<void*>(&OriginalTick)))
		return 3;
	void** published_shadow = viewport.vtable;
	if (published_shadow == original_vtable ||
		reinterpret_cast<Tick>(published_shadow[0])(&viewport, 3) != 1010)
		return 4;

	hook.Remove();
	if (viewport.vtable != original_vtable ||
		reinterpret_cast<Tick>(viewport.vtable[0])(&viewport, 3) != 10)
		return 5;

	// A dispatch may have captured the shadow vptr just before Remove. The
	// published allocation must remain readable for the process lifetime.
	if (reinterpret_cast<Tick>(published_shadow[0])(&viewport, 3) != 1010)
		return 6;
	for (size_t index = 1; index < 16; ++index)
	{
		if (!hook.Install(
				&viewport,
				0,
				reinterpret_cast<void*>(&HookedTick),
				original_vtable,
				reinterpret_cast<void*>(&OriginalTick)))
			return 7;
		hook.Remove();
	}
	if (hook.Install(
			&viewport,
			0,
			reinterpret_cast<void*>(&HookedTick),
			original_vtable,
			reinterpret_cast<void*>(&OriginalTick)))
		return 8;

	std::puts("shadow_vtable_hook_tests: PASS");
	return 0;
}
