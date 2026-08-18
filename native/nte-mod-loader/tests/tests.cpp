#include "nte/loader/Config/LoaderConfig.h"
#include "nte/loader/Injection/ShimPayload.h"
#include "nte/loader/Platform/ProcessLocator.h"
#include "nte/loader/Injection/ShimInjectionStrategy.h"
#include "shim/ProcessTarget.h"

#include <Windows.h>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <vector>

namespace {

int g_failures = 0;

// 显式检查（Release 下 assert 会被 NDEBUG 编译掉, 不能作为测试依据）
#define CHECK(cond)                                                                        \
    do {                                                                                   \
        if (!(cond)) {                                                                     \
            ++g_failures;                                                                  \
            std::wcout << L"FAIL " << L#cond << L" @" << __LINE__ << L"\n";                \
        }                                                                                  \
    } while (0)

std::filesystem::path GetExeDir() {
    std::wstring path(32768, L'\0');
    const DWORD length = GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
    path.resize(length);
    return std::filesystem::path(path).parent_path();
}

std::vector<std::uint8_t> ReadFileBytes(const std::filesystem::path& path) {
    std::ifstream stream(path, std::ios::binary);
    return std::vector<std::uint8_t>(std::istreambuf_iterator<char>(stream),
                                     std::istreambuf_iterator<char>());
}

bool WriteFileBytes(const std::filesystem::path& path,
                    const std::vector<std::uint8_t>& bytes) {
    std::ofstream stream(path, std::ios::binary | std::ios::trunc);
    return stream &&
           static_cast<bool>(stream.write(reinterpret_cast<const char*>(bytes.data()),
                                          static_cast<std::streamsize>(bytes.size())));
}

bool RvaToRaw(const std::vector<std::uint8_t>& bytes, DWORD rva, std::size_t& offset) {
    if (bytes.size() < sizeof(IMAGE_DOS_HEADER)) return false;
    const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(bytes.data());
    const std::size_t ntOffset = dos->e_lfanew > 0
        ? static_cast<std::size_t>(dos->e_lfanew)
        : bytes.size();
    if (ntOffset > bytes.size() || sizeof(IMAGE_NT_HEADERS) > bytes.size() - ntOffset) {
        return false;
    }
    const auto* nt = reinterpret_cast<const IMAGE_NT_HEADERS*>(bytes.data() + ntOffset);
    if (rva < nt->OptionalHeader.SizeOfHeaders) {
        offset = rva;
        return offset < bytes.size();
    }
    const auto* section = IMAGE_FIRST_SECTION(nt);
    for (WORD index = 0; index < nt->FileHeader.NumberOfSections; ++index, ++section) {
        if (rva >= section->VirtualAddress &&
            rva - section->VirtualAddress < section->SizeOfRawData) {
            offset = static_cast<std::size_t>(section->PointerToRawData) +
                     (rva - section->VirtualAddress);
            return offset < bytes.size();
        }
    }
    return false;
}

// 集成测试 1: 加载重建的 nte_shim.dll, 等待 %TEMP% 握手标记文件,
// 并验证 CreateProcessW 已被 MinHook patch。
void TestShimReadyHandshake() {
    const auto shim = GetExeDir() / L"nte_shim.dll";
    if (!std::filesystem::exists(shim)) {
        std::wcout << L"SKIP shim ready handshake (nte_shim.dll not present)\n";
        return;
    }
    SetEnvironmentVariableW(L"NTE_MOD_LOADER_DLL_PATH", L"C:\\nonexistent\\nte-mod-loader.dll");

    wchar_t markerPath[MAX_PATH]{};
    const DWORD len = GetTempPathW(MAX_PATH, markerPath);
    CHECK(len > 0 && len < MAX_PATH);
    wchar_t markerName[64]{};
	swprintf_s(markerName, nte::loader::kShimReadyMarkerFormat, GetCurrentProcessId(), 0ULL);
    wcscat_s(markerPath, markerName);
    std::error_code ec;
    std::filesystem::remove(markerPath, ec); // 清除可能的历史残留

    HMODULE module = LoadLibraryW(shim.c_str());
    CHECK(module != nullptr);
    if (module != nullptr) {
        // 轮询等待标记文件（shim 完成 MH 初始化并写标记）
        bool ready = false;
        const ULONGLONG deadline = GetTickCount64() + nte::loader::kShimReadyTimeoutMs;
        while (GetTickCount64() < deadline && !ready) {
            ready = std::filesystem::exists(markerPath, ec);
            if (!ready) Sleep(50);
        }
        CHECK(ready);
        if (ready) {
            BYTE* createProcess = reinterpret_cast<BYTE*>(
                GetProcAddress(GetModuleHandleW(L"kernel32"), "CreateProcessW"));
            CHECK(createProcess[0] == 0xE9 || createProcess[0] == 0xFF); // hook 已生效
            std::filesystem::remove(markerPath, ec);
        }
        FreeLibrary(module);
    }
    std::wcout << L"done: shim ready handshake\n";
}

// 集成测试 2: nte-mod-loader.exe 内嵌资源 101 必须等于重建的 nte_shim.dll。
void TestResource101MatchesBuiltShim() {
    const auto dir = GetExeDir();
    const auto exe = dir / L"nte-mod-loader.exe";
    const auto shim = dir / L"nte_shim.dll";
    if (!std::filesystem::exists(exe) || !std::filesystem::exists(shim)) {
        std::wcout << L"SKIP resource 101 check (nte-mod-loader.exe/nte_shim.dll not present)\n";
        return;
    }
    const auto shimBytes = ReadFileBytes(shim);
    HMODULE module = LoadLibraryExW(exe.c_str(), nullptr,
                                    LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE);
    CHECK(module != nullptr);
    if (module != nullptr) {
        HRSRC resourceInfo = FindResourceW(module, MAKEINTRESOURCEW(101), RT_RCDATA);
        CHECK(resourceInfo != nullptr);
        if (resourceInfo != nullptr) {
            HGLOBAL resource = LoadResource(module, resourceInfo);
            const void* data = resource ? LockResource(resource) : nullptr;
            const DWORD size = SizeofResource(module, resourceInfo);
            CHECK(data != nullptr);
            CHECK(size == shimBytes.size());
            if (data != nullptr && size == shimBytes.size()) {
                CHECK(std::memcmp(data, shimBytes.data(), size) == 0);
            }
        }
        FreeLibrary(module);
    }
    std::wcout << L"done: resource 101 vs built shim\n";
}

// 单元测试 3: ReadFileBytes（manual map 输入读取）。
void TestReadFileBytes() {
    const auto real = GetExeDir() / L"nte_shim.dll";
    if (std::filesystem::exists(real)) {
        const auto bytes = nte::loader::ShimPayload::ReadFileBytes(real);
        CHECK(bytes.has_value());
        CHECK(bytes->size() > 0x1000); // shim 至少 4 KB
    } else {
        std::wcout << L"SKIP read nte_shim.dll (not present)\n";
    }
    CHECK(!nte::loader::ShimPayload::ReadFileBytes(GetExeDir() / L"nte_missing_never_exists.dll").has_value());
    std::wcout << L"done: ReadFileBytes\n";
}

// 单元测试 4: ValidateInternalDll。
void TestValidateInternalDll() {
	const auto real = GetExeDir() / L"plugins" / L"dwmapi.dll";
    if (std::filesystem::exists(real)) {
        CHECK(nte::loader::ShimPayload::ValidateInternalDll(real));
    } else {
		std::wcout << L"SKIP validate packaged payload DLL (not present)\n";
    }

    const auto tempDir = std::filesystem::temp_directory_path();
    const auto garbage = tempDir / L"nte_test_garbage.dll";
    {
        std::ofstream stream(garbage, std::ios::binary);
        stream << "not a PE file";
    }
    CHECK(!nte::loader::ShimPayload::ValidateInternalDll(garbage));
    std::error_code ec;
    std::filesystem::remove(garbage, ec);

    // 只有最小 PE 头、没有完整 section table/映像布局的文件必须拒绝。
    const auto fake = tempDir / L"nte_test_fake.dll";
    {
        std::ofstream stream(fake, std::ios::binary);
        IMAGE_DOS_HEADER dos{};
        dos.e_magic = IMAGE_DOS_SIGNATURE;
        dos.e_lfanew = 0x40;
        stream.write(reinterpret_cast<const char*>(&dos), sizeof(dos));
        stream.seekp(dos.e_lfanew);
        const DWORD signature = IMAGE_NT_SIGNATURE;
        stream.write(reinterpret_cast<const char*>(&signature), sizeof(signature));
        IMAGE_FILE_HEADER fileHeader{};
        fileHeader.Machine = IMAGE_FILE_MACHINE_AMD64;
        fileHeader.Characteristics = IMAGE_FILE_DLL;
        fileHeader.SizeOfOptionalHeader = sizeof(IMAGE_OPTIONAL_HEADER64);
        stream.write(reinterpret_cast<const char*>(&fileHeader), sizeof(fileHeader));
        IMAGE_OPTIONAL_HEADER64 optional{};
        optional.Magic = IMAGE_NT_OPTIONAL_HDR64_MAGIC;
        stream.write(reinterpret_cast<const char*>(&optional), sizeof(optional));
    }
    CHECK(!nte::loader::ShimPayload::ValidateInternalDll(fake));
    std::filesystem::remove(fake, ec);

    CHECK(!nte::loader::ShimPayload::ValidateInternalDll(tempDir / L"nte_test_missing.dll"));

    const auto tooLarge = tempDir / L"nte_test_too_large.dll";
    {
        std::ofstream stream(tooLarge, std::ios::binary | std::ios::trunc);
        stream.seekp(static_cast<std::streamoff>(nte::loader::kMaxPayloadDllBytes));
        stream.put('\0');
    }
    CHECK(!nte::loader::ShimPayload::ReadFileBytes(tooLarge).has_value());
    std::filesystem::remove(tooLarge, ec);

    // 目录自身落在映像内仍不够：manual-map shellcode 会遍历 import/reloc
    // 内部记录，因此必须在注入前拒绝没有终止符或越界块的畸形输入。
    const auto shim = GetExeDir() / L"nte_shim.dll";
    const auto shimBytes = ReadFileBytes(shim);
    if (!shimBytes.empty()) {
        const auto malformedImport = tempDir / L"nte_test_bad_import.dll";
        auto bytes = shimBytes;
        auto* dos = reinterpret_cast<IMAGE_DOS_HEADER*>(bytes.data());
        auto* nt = reinterpret_cast<IMAGE_NT_HEADERS*>(bytes.data() + dos->e_lfanew);
        const auto importDirectory =
            nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT];
        std::size_t importOffset = 0;
        CHECK(importDirectory.Size >= sizeof(IMAGE_IMPORT_DESCRIPTOR));
        CHECK(RvaToRaw(bytes, importDirectory.VirtualAddress, importOffset));
        if (importDirectory.Size >= sizeof(IMAGE_IMPORT_DESCRIPTOR) &&
            RvaToRaw(bytes, importDirectory.VirtualAddress, importOffset)) {
            auto* descriptors = reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(
                bytes.data() + importOffset);
            const std::size_t count =
                importDirectory.Size / sizeof(IMAGE_IMPORT_DESCRIPTOR);
            bool changed = false;
            for (std::size_t index = 0; index < count; ++index) {
                if (descriptors[index].Name == 0) {
                    descriptors[index].Name = nt->OptionalHeader.SizeOfImage - 1;
                    descriptors[index].FirstThunk = nt->OptionalHeader.SizeOfImage - 1;
                    changed = true;
                    break;
                }
            }
            CHECK(changed);
            CHECK(WriteFileBytes(malformedImport, bytes));
            CHECK(!nte::loader::ShimPayload::ValidateInternalDll(malformedImport));
            std::filesystem::remove(malformedImport, ec);
        }

        const auto malformedReloc = tempDir / L"nte_test_bad_reloc.dll";
        bytes = shimBytes;
        dos = reinterpret_cast<IMAGE_DOS_HEADER*>(bytes.data());
        nt = reinterpret_cast<IMAGE_NT_HEADERS*>(bytes.data() + dos->e_lfanew);
        const auto relocDirectory =
            nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC];
        std::size_t relocOffset = 0;
        CHECK(relocDirectory.Size >= sizeof(IMAGE_BASE_RELOCATION));
        CHECK(RvaToRaw(bytes, relocDirectory.VirtualAddress, relocOffset));
        if (relocDirectory.Size >= sizeof(IMAGE_BASE_RELOCATION) &&
            RvaToRaw(bytes, relocDirectory.VirtualAddress, relocOffset)) {
            auto* relocation = reinterpret_cast<IMAGE_BASE_RELOCATION*>(
                bytes.data() + relocOffset);
            relocation->SizeOfBlock = relocDirectory.Size + 1;
            CHECK(WriteFileBytes(malformedReloc, bytes));
            CHECK(!nte::loader::ShimPayload::ValidateInternalDll(malformedReloc));
            std::filesystem::remove(malformedReloc, ec);
        }
    }
    std::wcout << L"done: ValidateInternalDll\n";
}

void TestExactProcessTargetMatching() {
    using nte::shim::InjectionMode;
    CHECK(nte::shim::DetectMode(L"C:\\Game\\HTGame.exe", nullptr) == InjectionMode::Internal);
    CHECK(nte::shim::DetectMode(nullptr, const_cast<wchar_t*>(L"\"C:\\Game\\htgame.EXE\" -arg")) ==
          InjectionMode::Internal);
    CHECK(nte::shim::DetectMode(L"C:\\Game\\NTEGame.exe", nullptr) == InjectionMode::Shim);
    CHECK(nte::shim::DetectMode(L"C:\\Tools\\helper.exe",
                                const_cast<wchar_t*>(L"helper.exe --note HTGame.exe")) ==
          InjectionMode::None);
    CHECK(nte::shim::DetectMode(nullptr,
                                const_cast<wchar_t*>(L"helper.exe --note HTGame.exe")) ==
          InjectionMode::None);
    std::wcout << L"done: exact process target matching\n";
}

void TestInjectedLauncherCleanupSelection() {
    const std::set<DWORD> injected{42, 84};
    CHECK(nte::loader::ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
        42, L"NTEGame.exe", injected));
    CHECK(nte::loader::ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
        84, L"nteglobalgame.EXE", injected));
    CHECK(!nte::loader::ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
        21, L"NTEGame.exe", injected));
    CHECK(!nte::loader::ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
        42, L"HTGame.exe", injected));
    CHECK(!nte::loader::ShimInjectionStrategy::ShouldTerminateInjectedLauncher(
        42, L"NTEGame.exe.backup", injected));
    std::wcout << L"done: injected launcher cleanup selection\n";
}

} // namespace

// 说明: --once 的一轮退出行为与注册表“失效条目继续枚举”依赖真实进程/注册表,
// 无法在无副作用单元测试中构造; 前者由 launcher 二进制集成验证, 后者依赖
// HKCU 卸载键（有意不写入测试注册表）。InjectResult::TimedOut 的“不释放远程
// 内存”需要持久的挂起目标, 同样只做代码路径审查。
int wmain(int, wchar_t*[]) {
    const auto shim = nte::loader::LoaderConfig::Parse({}, L"C:\\sample");
    CHECK(shim.payloadDll == std::filesystem::path(L"C:\\sample\\plugins\\dwmapi.dll"));
    CHECK(!shim.dryRun); // 默认即执行模式（--execute 已取消）

    const auto dryRun = nte::loader::LoaderConfig::Parse({L"--dry-run", L"--once"}, L"C:\\sample");
    CHECK(dryRun.dryRun);
    CHECK(dryRun.oneShot);
    CHECK(nte::loader::ShimInjectionStrategy::TemporaryFilePattern() == L"nte_shim_*.dll");

    SetEnvironmentVariableW(L"NTE_MOD_LOADER_MONITOR_TIMEOUT", L"7");
    const auto timed = nte::loader::LoaderConfig::Parse({}, L"C:\\sample");
    CHECK(timed.monitorTimeoutSeconds == 7);
    SetEnvironmentVariableW(L"NTE_MOD_LOADER_MONITOR_TIMEOUT", nullptr);

	const auto managed = nte::loader::LoaderConfig::Parse(
		{L"--monitor-timeout", L"0", L"--stop-event",
		 L"Local\\NTE-DPS-TOOL-ModLoader-0123456789abcdef", L"--owner-pid", L"42"},
		L"C:\\sample");
	CHECK(managed.controlArgumentsValid);
	CHECK(managed.monitorTimeoutSeconds == 0);
	CHECK(managed.stopEventName ==
		L"Local\\NTE-DPS-TOOL-ModLoader-0123456789abcdef");
	CHECK(managed.ownerProcessId == 42);
	const auto invalidControl = nte::loader::LoaderConfig::Parse(
		{L"--stop-event", L"Global\\untrusted", L"--owner-pid", L"0"}, L"C:\\sample");
	CHECK(!invalidControl.controlArgumentsValid);

    // --dll <path> 自定义注入 DLL
    const auto customDll = nte::loader::LoaderConfig::Parse(
        {L"--dll", L"D:\\tools\\my_inject.dll"}, L"C:\\sample");
    CHECK(customDll.payloadDll.wstring() == L"D:\\tools\\my_inject.dll");
    CHECK(customDll.payloadDll.filename() == L"my_inject.dll");

    const auto processes = nte::loader::ProcessLocator{}.Enumerate();
    CHECK(!processes.empty());

    TestReadFileBytes();
    TestValidateInternalDll();
    TestExactProcessTargetMatching();
    TestInjectedLauncherCleanupSelection();
    TestShimReadyHandshake();
    TestResource101MatchesBuiltShim();

    if (g_failures == 0) {
        std::wcout << L"ALL_TESTS_PASSED\n";
        return 0;
    }
    std::wcout << L"FAILURES=" << g_failures << L"\n";
    return 1;
}
