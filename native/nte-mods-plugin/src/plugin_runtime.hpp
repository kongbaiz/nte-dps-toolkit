#pragma once

#include <Windows.h>

namespace nte::mods
{
	void StartPluginRuntime(HMODULE module);
	void StopPluginRuntime();
} // namespace nte::mods
