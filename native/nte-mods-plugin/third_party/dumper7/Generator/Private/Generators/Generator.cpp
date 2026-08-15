
#include "Generators/Generator.h"
#include "Managers/StructManager.h"
#include "Managers/EnumManager.h"
#include "Managers/MemberManager.h"
#include "Managers/PackageManager.h"

#include "HashStringTable.h"
#include "Utils.h"

#include "Unreal/NameArray.h"

#include "Platform.h"

#include <format>
#include <fstream>

namespace
{
	void InitNteResolvedOffsets()
	{
		const FChunkedFixedUObjectArrayLayout NteGObjectsLayout
		{
			.ObjectsOffset = 0x00,
			.MaxElementsOffset = 0x10,
			.NumElementsOffset = 0x14,
			.MaxChunksOffset = 0x18,
			.NumChunksOffset = 0x1C,
		};

		ObjectArray::Init(
			Settings::NteRuntimeOffsets::OffsetGObjects,
			Settings::NteRuntimeOffsets::GObjectsChunkSize,
			NteGObjectsLayout);

		FName::Init(
			Settings::NteRuntimeOffsets::OffsetAppendString,
			FName::EOffsetOverrideType::AppendString);

		if (!NameArray::TryInit(Settings::NteRuntimeOffsets::OffsetGNames, true))
		{
			std::cerr << "\nNTE preset: GNames override failed, continuing with AppendString for name conversion.\n\n";
		}

		std::cerr << std::format(
			"NTE preset: GWorld offset 0x{:X}, ProcessEvent offset 0x{:X}, ProcessEvent index 0x{:X}\n\n",
			Settings::NteRuntimeOffsets::OffsetGWorld,
			Settings::NteRuntimeOffsets::OffsetProcessEvent,
			Settings::NteRuntimeOffsets::IndexProcessEvent);
	}
}

inline void InitSettings()
{
	Settings::InitWeakObjectPtrSettings();
	Settings::InitLargeWorldCoordinateSettings();

	Settings::InitObjectPtrPropertySettings();
	Settings::InitArrayDimSizeSettings();
}


void Generator::InitEngineCore()
{
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
	/* manual override */
	//ObjectArray::Init(/*GObjects*/, /*Layout = Default*/); // FFixedUObjectArray (UEVersion < UE4.21)
	//ObjectArray::Init(/*GObjects*/, /*ChunkSize*/, /*Layout = Default*/); // FChunkedFixedUObjectArray (UEVersion >= UE4.21)

	//FName::Init(/*bForceGNames = false*/);
	//FName::Init(/*AppendString, FName::EOffsetOverrideType::AppendString*/);
	//FName::Init(/*ToString, FName::EOffsetOverrideType::ToString*/);
	//FName::Init(/*GNames, FName::EOffsetOverrideType::GNames, true/false*/);
 
	//Off::InSDK::ProcessEvent::InitPE(/*PEIndex*/);

	/* Back4Blood (requires manual GNames override) */
	//InitObjectArrayDecryption([](void* ObjPtr) -> uint8* { return reinterpret_cast<uint8*>(uint64(ObjPtr) ^ 0x8375); });

	/* Multiversus [Unsupported, weird GObjects-struct] */
	//InitObjectArrayDecryption([](void* ObjPtr) -> uint8* { return reinterpret_cast<uint8*>(uint64(ObjPtr) ^ 0x1B5DEAFD6B4068C); });

	InitNteResolvedOffsets();

	Off::Init();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
	PropertySizes::Init();

	{
		Off::InSDK::ProcessEvent::PEIndex = Settings::NteRuntimeOffsets::IndexProcessEvent;
		Off::InSDK::ProcessEvent::PEOffset = Settings::NteRuntimeOffsets::OffsetProcessEvent;
		Off::InSDK::World::GWorld = Settings::NteRuntimeOffsets::OffsetGWorld;

		std::cerr << std::format("PE-Offset: 0x{:X}\n", Off::InSDK::ProcessEvent::PEOffset);
		std::cerr << std::format("PE-Index: 0x{:X}\n\n", Off::InSDK::ProcessEvent::PEIndex);
		std::cerr << std::format("GWorld-Offset: 0x{:X}\n\n", Off::InSDK::World::GWorld);
	}

	Off::InSDK::Text::InitTextOffsets(); // Must be at this position, relies on offsets initialized in Off::InitPE()
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();

	InitSettings();
}

void Generator::InitInternal()
{
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
	// Initialize PackageManager with all packages, their names, structs, classes enums, functions and dependencies
	PackageManager::Init();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();

	// Initialize StructManager with all structs and their names
	StructManager::Init();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
	
	// Initialize EnumManager with all enums and their names
	EnumManager::Init();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
	
	// Initialized all Member-Name collisions
	MemberManager::Init();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();

	// Post-Initialize PackageManager after StructManager has been initialized. 'PostInit()' handles Cyclic-Dependencies detection
	PackageManager::PostInit();
	Settings::NteRuntimeOffsets::ThrowIfCancellationRequested();
}

bool Generator::SetupDumperFolder()
{
	try
	{
		DumperFolder = Settings::Generator::SDKGenerationPath;
		if (DumperFolder.empty())
			return false;
		fs::create_directories(DumperFolder);
	}
	catch (const std::filesystem::filesystem_error& fe)
	{
		std::cerr << "Could not create required folders! Info: \n";
		std::cerr << fe.what() << std::endl;
		return false;
	}

	return true;
}

bool Generator::SetupFolders(std::string& FolderName, fs::path& OutFolder)
{
	fs::path Dummy;
	std::string EmptyName = "";
	return SetupFolders(FolderName, OutFolder, EmptyName, Dummy);
}

bool Generator::SetupFolders(std::string& FolderName, fs::path& OutFolder, std::string& SubfolderName, fs::path& OutSubFolder)
{
	FileNameHelper::MakeValidFileName(FolderName);
	FileNameHelper::MakeValidFileName(SubfolderName);

	try
	{
		OutFolder = DumperFolder / FolderName;
		OutSubFolder = OutFolder / SubfolderName;
				
		if (fs::exists(OutFolder))
		{
			fs::path Old = OutFolder.generic_string() + "_OLD";

			fs::remove_all(Old);

			fs::rename(OutFolder, Old);
		}

		fs::create_directories(OutFolder);

		if (!SubfolderName.empty())
			fs::create_directories(OutSubFolder);
	}
	catch (const std::filesystem::filesystem_error& fe)
	{
		std::cerr << "Could not create required folders! Info: \n";
		std::cerr << fe.what() << std::endl;
		return false;
	}

	return true;
}


