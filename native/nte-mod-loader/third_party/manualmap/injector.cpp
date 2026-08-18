#include "injector.h"

#include <algorithm>
#include <cstring>
#include <limits>

#if defined(DISABLE_OUTPUT)
#define ILog(data, ...)
#else
#define ILog(text, ...) printf(text, __VA_ARGS__);
#endif

#ifdef _WIN64
#define CURRENT_ARCH IMAGE_FILE_MACHINE_AMD64
#else
#define CURRENT_ARCH IMAGE_FILE_MACHINE_I386
#endif

namespace {
	constexpr SIZE_T kMaxManualMapFileBytes = 64ULL * 1024ULL * 1024ULL;
	constexpr DWORD kMaxManualMapImageBytes = 512UL * 1024UL * 1024UL;
	constexpr WORD kMaxManualMapSections = 96;

	bool RangeFits(SIZE_T offset, SIZE_T length, SIZE_T total) {
		return offset <= total && length <= total - offset;
	}

	bool ZeroRemoteMemory(HANDLE process, BYTE* address, SIZE_T size) {
		const BYTE zeros[4096]{};
		while (size != 0) {
			const SIZE_T chunk = size < sizeof(zeros) ? size : sizeof(zeros);
			SIZE_T written = 0;
			if (!WriteProcessMemory(process, address, zeros, chunk, &written) ||
				written != chunk) {
				return false;
			}
			address += chunk;
			size -= chunk;
		}
		return true;
	}

	bool RvaFileSpan(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount, DWORD rva,
		const BYTE** span, SIZE_T* available) {
		if (rva < optional->SizeOfHeaders) {
			if (rva >= fileSize) return false;
			*span = data + rva;
			*available = (std::min)(
				static_cast<SIZE_T>(optional->SizeOfHeaders - rva), fileSize - rva);
			return true;
		}
		for (WORD index = 0; index < sectionCount; ++index) {
			const auto& section = sections[index];
			if (rva < section.VirtualAddress) continue;
			const SIZE_T delta = static_cast<SIZE_T>(rva - section.VirtualAddress);
			if (delta >= section.SizeOfRawData) continue;
			const SIZE_T offset = static_cast<SIZE_T>(section.PointerToRawData) + delta;
			if (offset >= fileSize) return false;
			*span = data + offset;
			*available = (std::min)(
				static_cast<SIZE_T>(section.SizeOfRawData) - delta, fileSize - offset);
			return true;
		}
		return false;
	}

	bool RvaRangeIsFileBacked(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount, DWORD rva,
		SIZE_T length) {
		const BYTE* span = nullptr;
		SIZE_T available = 0;
		return RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			rva, &span, &available) && length <= available;
	}

	bool ValidateCStringRva(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount, DWORD rva) {
		const BYTE* span = nullptr;
		SIZE_T available = 0;
		if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			rva, &span, &available)) {
			return false;
		}
		constexpr SIZE_T kMaxImportNameBytes = 32768;
		const SIZE_T limit = (std::min)(available, kMaxImportNameBytes);
		return memchr(span, 0, limit) != nullptr;
	}

	bool ValidateImportDirectory(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount,
		const IMAGE_DATA_DIRECTORY& directory) {
		if (directory.Size == 0) return true;
		if (directory.Size < sizeof(IMAGE_IMPORT_DESCRIPTOR) ||
			!RvaRangeIsFileBacked(data, fileSize, optional, sections, sectionCount,
				directory.VirtualAddress, directory.Size)) {
			return false;
		}
		const BYTE* descriptorBytes = nullptr;
		SIZE_T descriptorAvailable = 0;
		if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			directory.VirtualAddress, &descriptorBytes, &descriptorAvailable)) {
			return false;
		}
		const auto* descriptors =
			reinterpret_cast<const IMAGE_IMPORT_DESCRIPTOR*>(descriptorBytes);
		const SIZE_T descriptorCount =
			directory.Size / sizeof(IMAGE_IMPORT_DESCRIPTOR);
		bool terminated = false;
		for (SIZE_T index = 0; index < descriptorCount; ++index) {
			const auto& descriptor = descriptors[index];
			if (descriptor.Name == 0) {
				terminated = descriptor.OriginalFirstThunk == 0 &&
					descriptor.FirstThunk == 0 && descriptor.TimeDateStamp == 0 &&
					descriptor.ForwarderChain == 0;
				break;
			}
			if (descriptor.FirstThunk == 0 ||
				!ValidateCStringRva(data, fileSize, optional, sections, sectionCount,
					descriptor.Name)) {
				return false;
			}
			const DWORD sourceThunkRva = descriptor.OriginalFirstThunk != 0
				? descriptor.OriginalFirstThunk
				: descriptor.FirstThunk;
			const BYTE* thunkBytes = nullptr;
			SIZE_T thunkAvailable = 0;
			if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
				sourceThunkRva, &thunkBytes, &thunkAvailable)) {
				return false;
			}
			const auto* thunks = reinterpret_cast<const IMAGE_THUNK_DATA*>(thunkBytes);
			const SIZE_T thunkCapacity = thunkAvailable / sizeof(IMAGE_THUNK_DATA);
			SIZE_T thunkCount = 0;
			for (; thunkCount < thunkCapacity; ++thunkCount) {
				const ULONG_PTR value = thunks[thunkCount].u1.AddressOfData;
				if (value == 0) break;
				if (!IMAGE_SNAP_BY_ORDINAL(value)) {
					if (value > (std::numeric_limits<DWORD>::max)()) return false;
					const DWORD nameRva = static_cast<DWORD>(value);
					if (!RvaRangeIsFileBacked(data, fileSize, optional, sections,
						sectionCount, nameRva, sizeof(WORD)) ||
						nameRva > (std::numeric_limits<DWORD>::max)() - sizeof(WORD) ||
						!ValidateCStringRva(data, fileSize, optional, sections,
							sectionCount, nameRva + sizeof(WORD))) {
						return false;
					}
				}
			}
			if (thunkCount == thunkCapacity ||
				thunkCount > ((std::numeric_limits<SIZE_T>::max)() /
					sizeof(IMAGE_THUNK_DATA)) - 1) {
				return false;
			}
			const SIZE_T destinationBytes =
				(thunkCount + 1) * sizeof(IMAGE_THUNK_DATA);
			if (!RangeFits(descriptor.FirstThunk, destinationBytes,
				optional->SizeOfImage)) {
				return false;
			}
		}
		return terminated;
	}

	bool ValidateRelocationDirectory(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount,
		const IMAGE_DATA_DIRECTORY& directory) {
		if (directory.Size == 0) return true;
		const BYTE* bytes = nullptr;
		SIZE_T available = 0;
		if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			directory.VirtualAddress, &bytes, &available) || directory.Size > available) {
			return false;
		}
		SIZE_T cursor = 0;
		while (cursor < directory.Size) {
			if (!RangeFits(cursor, sizeof(IMAGE_BASE_RELOCATION), directory.Size)) {
				return false;
			}
			const auto* block = reinterpret_cast<const IMAGE_BASE_RELOCATION*>(bytes + cursor);
			if (block->SizeOfBlock < sizeof(IMAGE_BASE_RELOCATION) ||
				block->SizeOfBlock > directory.Size - cursor ||
				(block->SizeOfBlock - sizeof(IMAGE_BASE_RELOCATION)) % sizeof(WORD) != 0) {
				return false;
			}
			const SIZE_T entryCount =
				(block->SizeOfBlock - sizeof(IMAGE_BASE_RELOCATION)) / sizeof(WORD);
			const auto* entries = reinterpret_cast<const WORD*>(block + 1);
			for (SIZE_T index = 0; index < entryCount; ++index) {
				const WORD type = entries[index] >> 12;
				if (type == IMAGE_REL_BASED_ABSOLUTE) continue;
#ifdef _WIN64
				if (type != IMAGE_REL_BASED_DIR64) return false;
#else
				if (type != IMAGE_REL_BASED_HIGHLOW) return false;
#endif
				const SIZE_T patchRva = static_cast<SIZE_T>(block->VirtualAddress) +
					(entries[index] & 0x0FFF);
				if (!RangeFits(patchRva, sizeof(ULONG_PTR), optional->SizeOfImage)) {
					return false;
				}
			}
			cursor += block->SizeOfBlock;
		}
		return cursor == directory.Size;
	}

	bool ValidateTlsDirectory(const BYTE* data, SIZE_T fileSize,
		const IMAGE_OPTIONAL_HEADER* optional,
		const IMAGE_SECTION_HEADER* sections, WORD sectionCount,
		const IMAGE_DATA_DIRECTORY& directory) {
		if (directory.Size == 0) return true;
		if (directory.Size < sizeof(IMAGE_TLS_DIRECTORY) ||
			!RvaRangeIsFileBacked(data, fileSize, optional, sections, sectionCount,
				directory.VirtualAddress, sizeof(IMAGE_TLS_DIRECTORY))) {
			return false;
		}
		const BYTE* tlsBytes = nullptr;
		SIZE_T available = 0;
		if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			directory.VirtualAddress, &tlsBytes, &available)) {
			return false;
		}
		const auto* tls = reinterpret_cast<const IMAGE_TLS_DIRECTORY*>(tlsBytes);
		if (tls->AddressOfCallBacks == 0) return true;
		const ULONGLONG imageBase = static_cast<ULONGLONG>(optional->ImageBase);
		const ULONGLONG callbacks = static_cast<ULONGLONG>(tls->AddressOfCallBacks);
		if (callbacks < imageBase || callbacks - imageBase >
			(std::numeric_limits<DWORD>::max)()) {
			return false;
		}
		const DWORD callbacksRva = static_cast<DWORD>(callbacks - imageBase);
		const BYTE* callbackBytes = nullptr;
		SIZE_T callbackAvailable = 0;
		if (!RvaFileSpan(data, fileSize, optional, sections, sectionCount,
			callbacksRva, &callbackBytes, &callbackAvailable)) {
			return false;
		}
		const auto* callbackValues = reinterpret_cast<const ULONG_PTR*>(callbackBytes);
		const SIZE_T callbackCapacity = callbackAvailable / sizeof(ULONG_PTR);
		for (SIZE_T index = 0; index < callbackCapacity; ++index) {
			const ULONGLONG value = static_cast<ULONGLONG>(callbackValues[index]);
			if (value == 0) return true;
			if (value < imageBase || value - imageBase >= optional->SizeOfImage) {
				return false;
			}
		}
		return false;
	}

	bool SectionNameEquals(const IMAGE_SECTION_HEADER& section, const char* name) {
		const SIZE_T length = strlen(name);
		return length <= IMAGE_SIZEOF_SHORT_NAME &&
			memcmp(section.Name, name, length) == 0 &&
			(length == IMAGE_SIZEOF_SHORT_NAME || section.Name[length] == 0);
	}
}

bool ValidateManualMapImage(const BYTE* pSrcData, SIZE_T FileSize) {
	if (pSrcData == nullptr || FileSize < sizeof(IMAGE_DOS_HEADER) ||
		FileSize > kMaxManualMapFileBytes) {
		return false;
	}

	const auto* dos = reinterpret_cast<const IMAGE_DOS_HEADER*>(pSrcData);
	if (dos->e_magic != IMAGE_DOS_SIGNATURE || dos->e_lfanew <= 0) {
		return false;
	}
	const SIZE_T ntOffset = static_cast<SIZE_T>(dos->e_lfanew);
	if (!RangeFits(ntOffset, sizeof(DWORD) + sizeof(IMAGE_FILE_HEADER), FileSize)) {
		return false;
	}

	const auto* signature = reinterpret_cast<const DWORD*>(pSrcData + ntOffset);
	if (*signature != IMAGE_NT_SIGNATURE) {
		return false;
	}
	const auto* fileHeader = reinterpret_cast<const IMAGE_FILE_HEADER*>(signature + 1);
	if (fileHeader->Machine != CURRENT_ARCH ||
		(fileHeader->Characteristics & IMAGE_FILE_DLL) == 0 ||
		fileHeader->NumberOfSections == 0 ||
		fileHeader->NumberOfSections > kMaxManualMapSections ||
		fileHeader->SizeOfOptionalHeader < sizeof(IMAGE_OPTIONAL_HEADER)) {
		return false;
	}

	const SIZE_T optionalOffset = ntOffset + sizeof(DWORD) + sizeof(IMAGE_FILE_HEADER);
	if (!RangeFits(optionalOffset, fileHeader->SizeOfOptionalHeader, FileSize)) {
		return false;
	}
	const auto* optional = reinterpret_cast<const IMAGE_OPTIONAL_HEADER*>(pSrcData + optionalOffset);
#ifdef _WIN64
	if (optional->Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC) return false;
#else
	if (optional->Magic != IMAGE_NT_OPTIONAL_HDR32_MAGIC) return false;
#endif
	if (optional->SizeOfImage < 0x1000 || optional->SizeOfImage > kMaxManualMapImageBytes ||
		optional->SizeOfHeaders == 0 || optional->SizeOfHeaders > FileSize ||
		optional->SizeOfHeaders > optional->SizeOfImage ||
		optional->AddressOfEntryPoint == 0 ||
		optional->AddressOfEntryPoint >= optional->SizeOfImage) {
		return false;
	}

	const SIZE_T sectionOffset = optionalOffset + fileHeader->SizeOfOptionalHeader;
	const SIZE_T sectionBytes =
		static_cast<SIZE_T>(fileHeader->NumberOfSections) * sizeof(IMAGE_SECTION_HEADER);
	if (!RangeFits(sectionOffset, sectionBytes, FileSize)) {
		return false;
	}
	const auto* sections = reinterpret_cast<const IMAGE_SECTION_HEADER*>(pSrcData + sectionOffset);
	bool entryPointIsExecutableAndFileBacked = false;
	for (WORD index = 0; index < fileHeader->NumberOfSections; ++index) {
		const auto& section = sections[index];
		if (section.SizeOfRawData != 0 &&
			!RangeFits(section.PointerToRawData, section.SizeOfRawData, FileSize)) {
			return false;
		}
		const SIZE_T virtualBytes =
			(section.Misc.VirtualSize > section.SizeOfRawData)
				? section.Misc.VirtualSize
				: section.SizeOfRawData;
		if (virtualBytes != 0 &&
			(section.VirtualAddress < optional->SizeOfHeaders ||
			!RangeFits(section.VirtualAddress, virtualBytes, optional->SizeOfImage))) {
			return false;
		}
		for (WORD previous = 0; previous < index && virtualBytes != 0; ++previous) {
			const auto& other = sections[previous];
			const SIZE_T otherBytes = (other.Misc.VirtualSize > other.SizeOfRawData)
				? other.Misc.VirtualSize
				: other.SizeOfRawData;
			if (otherBytes != 0 && section.VirtualAddress <
				static_cast<SIZE_T>(other.VirtualAddress) + otherBytes &&
				other.VirtualAddress < static_cast<SIZE_T>(section.VirtualAddress) + virtualBytes) {
				return false;
			}
		}
		if (optional->AddressOfEntryPoint >= section.VirtualAddress &&
			optional->AddressOfEntryPoint - section.VirtualAddress < section.SizeOfRawData &&
			(section.Characteristics & IMAGE_SCN_MEM_EXECUTE) != 0) {
			entryPointIsExecutableAndFileBacked = true;
		}
	}
	if (!entryPointIsExecutableAndFileBacked) return false;

	const DWORD directoryCount =
		(optional->NumberOfRvaAndSizes < IMAGE_NUMBEROF_DIRECTORY_ENTRIES)
			? optional->NumberOfRvaAndSizes
			: IMAGE_NUMBEROF_DIRECTORY_ENTRIES;
	for (DWORD index = 0; index < directoryCount; ++index) {
		const auto& directory = optional->DataDirectory[index];
		if (directory.Size == 0) continue;
		if (index == IMAGE_DIRECTORY_ENTRY_SECURITY) {
			if (!RangeFits(directory.VirtualAddress, directory.Size, FileSize)) return false;
		} else if (!RangeFits(directory.VirtualAddress, directory.Size,
			optional->SizeOfImage) ||
			!RvaRangeIsFileBacked(pSrcData, FileSize, optional, sections,
				fileHeader->NumberOfSections, directory.VirtualAddress, directory.Size)) {
			return false;
		}
	}
	const IMAGE_DATA_DIRECTORY emptyDirectory{};
	const auto& importDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_IMPORT
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT]
		: emptyDirectory;
	const auto& relocationDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_BASERELOC
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC]
		: emptyDirectory;
	const auto& tlsDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_TLS
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_TLS]
		: emptyDirectory;
	const auto& delayImportDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT]
		: emptyDirectory;
	const auto& managedDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_COM_DESCRIPTOR
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_COM_DESCRIPTOR]
		: emptyDirectory;
	// The shellcode resolves the normal import table only and is native-only.
	if (delayImportDirectory.Size != 0 || managedDirectory.Size != 0) return false;
	if (!ValidateImportDirectory(pSrcData, FileSize, optional, sections,
		fileHeader->NumberOfSections,
		importDirectory) ||
		!ValidateRelocationDirectory(pSrcData, FileSize, optional, sections,
			fileHeader->NumberOfSections,
			relocationDirectory) ||
		!ValidateTlsDirectory(pSrcData, FileSize, optional, sections,
			fileHeader->NumberOfSections,
			tlsDirectory)) {
		return false;
	}
#ifdef _WIN64
	const auto& exceptionDirectory = optional->NumberOfRvaAndSizes >
		IMAGE_DIRECTORY_ENTRY_EXCEPTION
		? optional->DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION]
		: emptyDirectory;
	if (exceptionDirectory.Size != 0 &&
		exceptionDirectory.Size % sizeof(IMAGE_RUNTIME_FUNCTION_ENTRY) != 0) {
		return false;
	}
#endif
	return true;
}

ManualMapResult ManualMapDll(HANDLE hProc, BYTE* pSrcData, SIZE_T FileSize, bool ClearHeader, bool ClearNonNeededSections, bool AdjustProtections, bool SEHExceptionSupport, DWORD fdwReason, LPVOID lpReserved) {
	IMAGE_NT_HEADERS* pOldNtHeader = nullptr;
	IMAGE_OPTIONAL_HEADER* pOldOptHeader = nullptr;
	IMAGE_FILE_HEADER* pOldFileHeader = nullptr;
	BYTE* pTargetBase = nullptr;

	if (hProc == nullptr || !ValidateManualMapImage(pSrcData, FileSize)) {
		ILog("Invalid file\n");
		return ManualMapResult::Failed;
	}

	pOldNtHeader = reinterpret_cast<IMAGE_NT_HEADERS*>(pSrcData + reinterpret_cast<IMAGE_DOS_HEADER*>(pSrcData)->e_lfanew);
	pOldOptHeader = &pOldNtHeader->OptionalHeader;
	pOldFileHeader = &pOldNtHeader->FileHeader;

	if (pOldFileHeader->Machine != CURRENT_ARCH) {
		ILog("Invalid platform\n");
		return ManualMapResult::Failed;
	}

	ILog("File ok\n");

	pTargetBase = reinterpret_cast<BYTE*>(VirtualAllocEx(hProc,
		reinterpret_cast<void*>(static_cast<ULONG_PTR>(pOldOptHeader->ImageBase)),
		pOldOptHeader->SizeOfImage, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
	if (!pTargetBase &&
		pOldOptHeader->DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC].Size != 0) {
		pTargetBase = reinterpret_cast<BYTE*>(VirtualAllocEx(hProc, nullptr,
			pOldOptHeader->SizeOfImage, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
	}
	if (!pTargetBase) {
		ILog("Target process memory allocation failed (ex) 0x%X\n", GetLastError());
		return ManualMapResult::Failed;
	}

	DWORD oldp = 0;
	if (!VirtualProtectEx(hProc, pTargetBase, pOldOptHeader->SizeOfImage,
		PAGE_EXECUTE_READWRITE, &oldp)) {
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	MANUAL_MAPPING_DATA data{ 0 };
	data.pLoadLibraryA = LoadLibraryA;
	data.pGetProcAddress = GetProcAddress;
#ifdef _WIN64
	data.pRtlAddFunctionTable = (f_RtlAddFunctionTable)RtlAddFunctionTable;
#else
	SEHExceptionSupport = false;
#endif
	data.pbase = pTargetBase;
	data.fdwReasonParam = fdwReason;
	data.reservedParam = lpReserved;
	data.SEHSupport = SEHExceptionSupport;

#ifdef _WIN64
	// Build the _CxxThrowException replacement stub. x64 calling convention:
	// rcx = exception object, rdx = ThrowInfo*. We assemble a 4-slot params
	// array on the stack and call RaiseException with our correct ImageBase
	// in slot 3 — that's the bit the original _CxxThrowException can't fill
	// in for a manually-mapped DLL, because RtlPcToFileHeader can't find us.
	data.pCxxThrowStub = nullptr;
	if (SEHExceptionSupport) {
		HMODULE hK32 = GetModuleHandleW(L"kernel32.dll");
		FARPROC pRaiseEx = hK32 ? GetProcAddress(hK32, "RaiseException") : nullptr;
		void* stubMem = pRaiseEx
			? VirtualAllocEx(hProc, nullptr, 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
			: nullptr;

		if (!stubMem) {
			ILog("WARNING: couldn't allocate CxxThrow stub; typed catches may fail\n");
		} else {
			// One page in the target holds three things:
			//   offset 0x000: stub code         (79 bytes)
			//   offset 0x080: UNWIND_INFO       (8 bytes)
			//   offset 0x0A0: RUNTIME_FUNCTION  (12 bytes)
			// The shellcode then RtlAddFunctionTable's the RUNTIME_FUNCTION so
			// the OS unwinder can walk past the stub's frame when looking for
			// a C++ EH handler in the caller.
			BYTE blob[0xB0] = {};
			BYTE stub[] = {
				0x48, 0x83, 0xEC, 0x48,                                  // sub  rsp, 0x48
				0xC7, 0x44, 0x24, 0x20, 0x20, 0x05, 0x93, 0x19,          // mov  [rsp+0x20], 0x19930520 (EH_MAGIC_NUMBER1)
				0xC7, 0x44, 0x24, 0x24, 0x00, 0x00, 0x00, 0x00,          // mov  [rsp+0x24], 0
				0x48, 0x89, 0x4C, 0x24, 0x28,                            // mov  [rsp+0x28], rcx ; obj
				0x48, 0x89, 0x54, 0x24, 0x30,                            // mov  [rsp+0x30], rdx ; ThrowInfo
				0x48, 0xB8, 0,0,0,0,0,0,0,0,                             // movabs rax, IMAGE_BASE  (patched at offset 32)
				0x48, 0x89, 0x44, 0x24, 0x38,                            // mov  [rsp+0x38], rax  ; param[3]
				0xB9, 0x63, 0x73, 0x6D, 0xE0,                            // mov  ecx, 0xE06D7363
				0xBA, 0x01, 0x00, 0x00, 0x00,                            // mov  edx, 1 (NONCONTINUABLE)
				0x41, 0xB8, 0x04, 0x00, 0x00, 0x00,                      // mov  r8d, 4
				0x4C, 0x8D, 0x4C, 0x24, 0x20,                            // lea  r9, [rsp+0x20]
				0x48, 0xB8, 0,0,0,0,0,0,0,0,                             // movabs rax, RaiseException (patched at offset 68)
				0xFF, 0xD0,                                              // call rax
				0xCC,                                                    // int3 (never reached)
			};
			ULONG_PTR imageBase = (ULONG_PTR)pTargetBase;
			ULONG_PTR raiseExceptionAddr = (ULONG_PTR)pRaiseEx;
			memcpy(stub + 32, &imageBase, 8);
			memcpy(stub + 68, &raiseExceptionAddr, 8);
			memcpy(blob, stub, sizeof(stub));

			// UNWIND_INFO at offset 0x80:
			//   Version=1 Flags=0 | SizeOfProlog=4 | CountOfCodes=1 | FrameRegister=0
			//   UnwindCode: { CodeOffset=4, UnwindOp=UWOP_ALLOC_SMALL(2), OpInfo=(0x48/8)-1=8 }
			blob[0x80] = 0x01;        // Version 1, flags 0
			blob[0x81] = 0x04;        // SizeOfProlog (sub rsp, 0x48 is 4 bytes)
			blob[0x82] = 0x01;        // CountOfCodes
			blob[0x83] = 0x00;        // FrameRegister/FrameOffset
			blob[0x84] = 0x04;        // CodeOffset
			blob[0x85] = 0x82;        // (OpInfo=8 << 4) | UWOP_ALLOC_SMALL(2)

			// RUNTIME_FUNCTION at offset 0xA0: { BeginAddr, EndAddr, UnwindData } as RVAs from stubMem.
			DWORD beginAddr = 0;
			DWORD endAddr   = (DWORD)sizeof(stub);
			DWORD unwindRva = 0x80;
			memcpy(blob + 0xA0, &beginAddr, 4);
			memcpy(blob + 0xA4, &endAddr,   4);
			memcpy(blob + 0xA8, &unwindRva, 4);

			if (WriteProcessMemory(hProc, stubMem, blob, sizeof(blob), nullptr)) {
				data.pCxxThrowStub = stubMem;
			} else {
				ILog("WARNING: couldn't write CxxThrow stub\n");
				VirtualFreeEx(hProc, stubMem, 0, MEM_RELEASE);
			}
		}
	}
#endif


	const auto freeThrowStub = [&]() {
#ifdef _WIN64
		if (data.pCxxThrowStub != nullptr) {
			VirtualFreeEx(hProc, data.pCxxThrowStub, 0, MEM_RELEASE);
			data.pCxxThrowStub = nullptr;
		}
#endif
	};

	// File header. The validator guarantees SizeOfHeaders is file-backed.
	const SIZE_T headerBytes = pOldOptHeader->SizeOfHeaders < 0x1000
		? pOldOptHeader->SizeOfHeaders
		: 0x1000;
	if (!WriteProcessMemory(hProc, pTargetBase, pSrcData, headerBytes, nullptr)) {
		ILog("Can't write file header 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	IMAGE_SECTION_HEADER* pSectionHeader = IMAGE_FIRST_SECTION(pOldNtHeader);
	for (UINT i = 0; i != pOldFileHeader->NumberOfSections; ++i, ++pSectionHeader) {
		if (pSectionHeader->SizeOfRawData) {
			if (!WriteProcessMemory(hProc, pTargetBase + pSectionHeader->VirtualAddress, pSrcData + pSectionHeader->PointerToRawData, pSectionHeader->SizeOfRawData, nullptr)) {
				ILog("Can't map sections: 0x%x\n", GetLastError());
				freeThrowStub();
				VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
				return ManualMapResult::Failed;
			}
		}
	}

	//Mapping params
	BYTE* MappingDataAlloc = reinterpret_cast<BYTE*>(VirtualAllocEx(hProc, nullptr, sizeof(MANUAL_MAPPING_DATA), MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
	if (!MappingDataAlloc) {
		ILog("Target process mapping allocation failed (ex) 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	if (!WriteProcessMemory(hProc, MappingDataAlloc, &data, sizeof(MANUAL_MAPPING_DATA), nullptr)) {
		ILog("Can't write mapping 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	//Shell code
	void* pShellcode = VirtualAllocEx(hProc, nullptr, 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
	if (!pShellcode) {
		ILog("Memory shellcode allocation failed (ex) 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	if (!WriteProcessMemory(hProc, pShellcode, Shellcode, 0x1000, nullptr)) {
		ILog("Can't write shellcode 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, pShellcode, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	ILog("Mapped DLL at %p\n", pTargetBase);
	ILog("Mapping info at %p\n", MappingDataAlloc);
	ILog("Shell code at %p\n", pShellcode);

	ILog("Data allocated\n");

#ifdef _DEBUG
	ILog("My shellcode pointer %p\n", Shellcode);
	ILog("Target point %p\n", pShellcode);
	system("pause");
#endif

	HANDLE hThread = CreateRemoteThread(hProc, nullptr, 0, reinterpret_cast<LPTHREAD_START_ROUTINE>(pShellcode), MappingDataAlloc, 0, nullptr);
	if (!hThread) {
		ILog("Thread creation failed 0x%X\n", GetLastError());
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, pShellcode, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	ILog("Thread created at: %p, waiting for return...\n", pShellcode);

	const DWORD kInjectTimeoutMs = 30000;
	const DWORD waitResult = WaitForSingleObject(hThread, kInjectTimeoutMs);
	CloseHandle(hThread);
	if (waitResult == WAIT_TIMEOUT) {
		// The remote thread may still access the mapping data and lpReserved.
		// Deliberately retain all remote allocations; callers also retain
		// lpReserved when this status is returned.
		ILog("Injection timed out after %u ms (DllMain may be hung)\n", kInjectTimeoutMs);
		return ManualMapResult::TimedOut;
	}
	MANUAL_MAPPING_DATA data_checked{};
	SIZE_T bytesRead = 0;
	if (waitResult != WAIT_OBJECT_0) {
		// WAIT_FAILED is also indeterminate: the thread may still execute.
		return ManualMapResult::TimedOut;
	}
	if (!ReadProcessMemory(hProc, MappingDataAlloc, &data_checked,
		sizeof(data_checked), &bytesRead) || bytesRead != sizeof(data_checked)) {
		// DllMain may already have spawned workers. Keep the mapped image and
		// throw stub alive, but the completed thread no longer uses these two blocks.
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, pShellcode, 0, MEM_RELEASE);
		return ManualMapResult::TimedOut;
	}
	const HINSTANCE hCheck = data_checked.hMod;
	if (hCheck == nullptr || hCheck == reinterpret_cast<HINSTANCE>(0x404040) ||
		hCheck == reinterpret_cast<HINSTANCE>(0x505050) ||
		hCheck == reinterpret_cast<HINSTANCE>(0x606060)) {
		freeThrowStub();
		VirtualFreeEx(hProc, pTargetBase, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE);
		VirtualFreeEx(hProc, pShellcode, 0, MEM_RELEASE);
		return ManualMapResult::Failed;
	}

	//CLEAR PE HEAD
	if (ClearHeader) {
		if (!ZeroRemoteMemory(hProc, pTargetBase, 0x1000)) {
			ILog("WARNING!: Can't clear HEADER\n");
		}
	}
	//END CLEAR PE HEAD


	if (ClearNonNeededSections) {
		pSectionHeader = IMAGE_FIRST_SECTION(pOldNtHeader);
		for (UINT i = 0; i != pOldFileHeader->NumberOfSections; ++i, ++pSectionHeader) {
			if (pSectionHeader->Misc.VirtualSize) {
				if ((SEHExceptionSupport ? 0 : SectionNameEquals(*pSectionHeader, ".pdata")) ||
					SectionNameEquals(*pSectionHeader, ".rsrc") ||
					SectionNameEquals(*pSectionHeader, ".reloc")) {
					ILog("Processing %s removal\n", pSectionHeader->Name);
					if (!ZeroRemoteMemory(hProc, pTargetBase + pSectionHeader->VirtualAddress, pSectionHeader->Misc.VirtualSize)) {
						ILog("Can't clear section %s: 0x%x\n", pSectionHeader->Name, GetLastError());
					}
				}
			}
		}
	}

	if (AdjustProtections) {
		pSectionHeader = IMAGE_FIRST_SECTION(pOldNtHeader);
		for (UINT i = 0; i != pOldFileHeader->NumberOfSections; ++i, ++pSectionHeader) {
			if (pSectionHeader->Misc.VirtualSize) {
				DWORD old = 0;
				DWORD newP = PAGE_READONLY;

				if ((pSectionHeader->Characteristics & IMAGE_SCN_MEM_WRITE) > 0) {
					newP = PAGE_READWRITE;
				}
				else if ((pSectionHeader->Characteristics & IMAGE_SCN_MEM_EXECUTE) > 0) {
					newP = PAGE_EXECUTE_READ;
				}
				if (VirtualProtectEx(hProc, pTargetBase + pSectionHeader->VirtualAddress, pSectionHeader->Misc.VirtualSize, newP, &old)) {
					ILog("section %s set as %lX\n", (char*)pSectionHeader->Name, newP);
				}
				else {
					ILog("FAIL: section %s not set as %lX\n", (char*)pSectionHeader->Name, newP);
				}
			}
		}
		DWORD old = 0;
		VirtualProtectEx(hProc, pTargetBase, IMAGE_FIRST_SECTION(pOldNtHeader)->VirtualAddress, PAGE_READONLY, &old);
	}

	if (!ZeroRemoteMemory(hProc, static_cast<BYTE*>(pShellcode), 0x1000)) {
		ILog("WARNING: Can't clear shellcode\n");
	}
	if (!VirtualFreeEx(hProc, pShellcode, 0, MEM_RELEASE)) {
		ILog("WARNING: can't release shell code memory\n");
	}
	if (!VirtualFreeEx(hProc, MappingDataAlloc, 0, MEM_RELEASE)) {
		ILog("WARNING: can't release mapping data memory\n");
	}

	return ManualMapResult::Success;
}

#define RELOC_FLAG32(RelInfo) ((RelInfo >> 0x0C) == IMAGE_REL_BASED_HIGHLOW)
#define RELOC_FLAG64(RelInfo) ((RelInfo >> 0x0C) == IMAGE_REL_BASED_DIR64)

#ifdef _WIN64
#define RELOC_FLAG RELOC_FLAG64
#else
#define RELOC_FLAG RELOC_FLAG32
#endif

#pragma runtime_checks( "", off )
#pragma optimize( "", off )
void __stdcall Shellcode(MANUAL_MAPPING_DATA* pData) {
	if (!pData) {
		return;
	}

	BYTE* pBase = pData->pbase;
	auto* pOpt = &reinterpret_cast<IMAGE_NT_HEADERS*>(pBase + reinterpret_cast<IMAGE_DOS_HEADER*>((uintptr_t)pBase)->e_lfanew)->OptionalHeader;

	auto _LoadLibraryA = pData->pLoadLibraryA;
	auto _GetProcAddress = pData->pGetProcAddress;
#ifdef _WIN64
	auto _RtlAddFunctionTable = pData->pRtlAddFunctionTable;
#endif
	auto _DllMain = reinterpret_cast<f_DLL_ENTRY_POINT>(pBase + pOpt->AddressOfEntryPoint);

	BYTE* LocationDelta = pBase - pOpt->ImageBase;
	if (LocationDelta) {
		if (pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC].Size) {
			auto* pRelocData = reinterpret_cast<IMAGE_BASE_RELOCATION*>(pBase + pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC].VirtualAddress);
			const auto* pRelocEnd = reinterpret_cast<IMAGE_BASE_RELOCATION*>(reinterpret_cast<uintptr_t>(pRelocData) + pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_BASERELOC].Size);
			while (pRelocData < pRelocEnd && pRelocData->SizeOfBlock) {
				UINT AmountOfEntries = (pRelocData->SizeOfBlock - sizeof(IMAGE_BASE_RELOCATION)) / sizeof(WORD);
				WORD* pRelativeInfo = reinterpret_cast<WORD*>(pRelocData + 1);

				for (UINT i = 0; i != AmountOfEntries; ++i, ++pRelativeInfo) {
					if (RELOC_FLAG(*pRelativeInfo)) {
						UINT_PTR* pPatch = reinterpret_cast<UINT_PTR*>(pBase + pRelocData->VirtualAddress + ((*pRelativeInfo) & 0xFFF));
						*pPatch += reinterpret_cast<UINT_PTR>(LocationDelta);
					}
				}
				pRelocData = reinterpret_cast<IMAGE_BASE_RELOCATION*>(reinterpret_cast<BYTE*>(pRelocData) + pRelocData->SizeOfBlock);
			}
		}
	}

	if (pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].Size) {
		auto* pImportDescr = reinterpret_cast<IMAGE_IMPORT_DESCRIPTOR*>(pBase + pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT].VirtualAddress);
		while (pImportDescr->Name) {
			char* szMod = reinterpret_cast<char*>(pBase + pImportDescr->Name);
			HINSTANCE hDll = _LoadLibraryA(szMod);
			if (!hDll) {
				pData->hMod = reinterpret_cast<HINSTANCE>(0x606060);
				return;
			}

			ULONG_PTR* pThunkRef = reinterpret_cast<ULONG_PTR*>(pBase + pImportDescr->OriginalFirstThunk);
			ULONG_PTR* pFuncRef = reinterpret_cast<ULONG_PTR*>(pBase + pImportDescr->FirstThunk);

			if (!pImportDescr->OriginalFirstThunk)
				pThunkRef = pFuncRef;

			for (; *pThunkRef; ++pThunkRef, ++pFuncRef) {
				if (IMAGE_SNAP_BY_ORDINAL(*pThunkRef)) {
					*pFuncRef = (ULONG_PTR)_GetProcAddress(hDll, reinterpret_cast<char*>(*pThunkRef & 0xFFFF));
				}
				else {
					auto* pImport = reinterpret_cast<IMAGE_IMPORT_BY_NAME*>(pBase + (*pThunkRef));
#ifdef _WIN64
					// Detect "_CxxThrowException" by name (char-by-char to avoid
					// referencing string literals from the injector's .rdata).
					const char* n = pImport->Name;
					bool isCxxThrow =
						pData->pCxxThrowStub &&
						n[0] == '_' && n[1] == 'C' && n[2] == 'x' && n[3] == 'x' &&
						n[4] == 'T' && n[5] == 'h' && n[6] == 'r' && n[7] == 'o' &&
						n[8] == 'w' && n[9] == 'E' && n[10] == 'x' && n[11] == 'c' &&
						n[12] == 'e' && n[13] == 'p' && n[14] == 't' && n[15] == 'i' &&
						n[16] == 'o' && n[17] == 'n' && n[18] == '\0';
					if (isCxxThrow) {
						*pFuncRef = (ULONG_PTR)pData->pCxxThrowStub;
					} else
#endif
					{
						*pFuncRef = (ULONG_PTR)_GetProcAddress(hDll, pImport->Name);
					}
					if (*pFuncRef == 0) {
						pData->hMod = reinterpret_cast<HINSTANCE>(0x606060);
						return;
					}
				}
			}
			++pImportDescr;
		}
	}

	if (pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_TLS].Size) {
		auto* pTLS = reinterpret_cast<IMAGE_TLS_DIRECTORY*>(pBase + pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_TLS].VirtualAddress);
		auto* pCallback = reinterpret_cast<PIMAGE_TLS_CALLBACK*>(pTLS->AddressOfCallBacks);
		for (; pCallback && *pCallback; ++pCallback)
			(*pCallback)(pBase, DLL_PROCESS_ATTACH, nullptr);
	}

	bool ExceptionSupportFailed = false;

#ifdef _WIN64

	if (pData->SEHSupport) {
		auto excep = pOpt->DataDirectory[IMAGE_DIRECTORY_ENTRY_EXCEPTION];
		if (excep.Size) {
			if (!_RtlAddFunctionTable(
				reinterpret_cast<IMAGE_RUNTIME_FUNCTION_ENTRY*>(pBase + excep.VirtualAddress),
				excep.Size / sizeof(IMAGE_RUNTIME_FUNCTION_ENTRY), (DWORD64)pBase)) {
				ExceptionSupportFailed = true;
			}
		}

		// Register the CxxThrow stub's own RUNTIME_FUNCTION so the OS unwinder
		// can walk through it on its way back to the throw site's frame.
		if (pData->pCxxThrowStub) {
			BYTE* stubBase = static_cast<BYTE*>(pData->pCxxThrowStub);
			if (!_RtlAddFunctionTable(
				reinterpret_cast<IMAGE_RUNTIME_FUNCTION_ENTRY*>(stubBase + 0xA0),
				1, (DWORD64)stubBase)) {
				ExceptionSupportFailed = true;
			}
		}
	}

#endif
	if (ExceptionSupportFailed) {
		pData->hMod = reinterpret_cast<HINSTANCE>(0x505050);
		return;
	}

	if (!_DllMain(pBase, pData->fdwReasonParam, pData->reservedParam)) {
		pData->hMod = reinterpret_cast<HINSTANCE>(0x606060);
		return;
	}

	pData->hMod = reinterpret_cast<HINSTANCE>(pBase);
}
