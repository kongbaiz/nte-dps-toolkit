#include "../src/shadow_vtable_hook.hpp"
#include "../src/viewport_generation_policy.hpp"
#include "../src/viewport_hook_policy.hpp"

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

int first_world;
int first_game_instance;
int first_local_player;
int first_viewport;
int next_local_player;
int first_original;
} // namespace

int main()
{
	using nte::hook::ViewportGenerationToken;
	using nte::hook::ViewportSessionIdentity;
	constexpr ViewportSessionIdentity first_session{
		&first_world,
		&first_game_instance,
		&first_local_player,
		&first_viewport,
		17,
	};
	constexpr ViewportSessionIdentity next_session{
		&first_world,
		&first_game_instance,
		&next_local_player,
		&first_viewport,
		17,
	};
	static_assert(nte::hook::CanPublishViewportSnapshot(
		7, 7, first_session, first_session));
	static_assert(!nte::hook::CanPublishViewportSnapshot(
		7, 8, first_session, first_session));
	static_assert(!nte::hook::CanPublishViewportSnapshot(
		7, 7, first_session, next_session));
	static_assert(nte::hook::NextViewportGeneration(
		ViewportGenerationToken{UINT64_MAX}).value == 1);
	static_assert(nte::hook::CanDispatchViewportGeneration(
		9, 9, 17, 17, first_session.viewport, first_session.viewport));
	static_assert(!nte::hook::CanDispatchViewportGeneration(
		8, 9, 17, 17, first_session.viewport, first_session.viewport));
	static_assert(!nte::hook::CanDispatchViewportGeneration(
		9, 9, 16, 17, first_session.viewport, first_session.viewport));

	constexpr nte::hook::ViewportOriginalBinding generation_one{
		first_session.viewport,
		&first_original,
		1,
		17,
	};
	static_assert(nte::hook::CanReuseViewportBinding(
		generation_one,
		first_session.viewport,
		&first_original,
		1,
		17));
	// An address-reused UObject belongs to a new generation even when its vtable
	// happens to contain the same original Tick address.
	static_assert(!nte::hook::CanReuseViewportBinding(
		generation_one,
		first_session.viewport,
		&first_original,
		2,
		17));

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

	if (!hook.Remove())
		return 5;
	if (viewport.vtable != original_vtable ||
		reinterpret_cast<Tick>(viewport.vtable[0])(&viewport, 3) != 10)
		return 6;

	// A dispatch may have captured the shadow vptr just before Remove. The
	// published allocation must remain readable for the process lifetime.
	if (reinterpret_cast<Tick>(published_shadow[0])(&viewport, 3) != 1010)
		return 7;
	for (size_t index = 1; index < 16; ++index)
	{
		if (!hook.Install(
				&viewport,
				0,
				reinterpret_cast<void*>(&HookedTick),
				original_vtable,
				reinterpret_cast<void*>(&OriginalTick)))
			return 8;
		if (!hook.Remove())
			return 9;
	}
	if (hook.Install(
			&viewport,
			0,
			reinterpret_cast<void*>(&HookedTick),
			original_vtable,
			reinterpret_cast<void*>(&OriginalTick)))
		return 10;

	std::puts("shadow_vtable_hook_tests: PASS");
	return 0;
}
