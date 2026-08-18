#include "nte/loader/App/LoaderApp.h"
#include "nte/loader/Config/LoaderConfig.h"

#include <Windows.h>

#include <filesystem>
#include <iostream>
#include <string>
#include <vector>

int wmain(int argc, wchar_t* argv[]) {
    std::vector<std::wstring> arguments;
    for (int i = 1; i < argc; ++i) {
        arguments.emplace_back(argv[i]);
    }

	std::wstring modulePath(32768, L'\0');
	const DWORD length = GetModuleFileNameW(nullptr, modulePath.data(), static_cast<DWORD>(modulePath.size()));
	if (length == 0 || length >= modulePath.size()) {
		std::wcerr << L"[ERROR] executable path lookup failed\n";
		return 2;
	}
    modulePath.resize(length);
    const auto directory = std::filesystem::path(modulePath).parent_path();
    const auto config = nte::loader::LoaderConfig::Parse(arguments, directory);

    return nte::loader::LoaderApp{}.Run(config);
}
