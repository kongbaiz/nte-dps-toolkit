$ErrorActionPreference = "Stop"

$pluginRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $pluginRoot "src"

function Require-Pattern {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if ($Text -notmatch $Pattern) {
        throw $Message
    }
}

function Reject-Pattern {
    param(
        [Parameter(Mandatory = $true)][string]$Text,
        [Parameter(Mandatory = $true)][string]$Pattern,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if ($Text -match $Pattern) {
        throw $Message
    }
}

$ipc = Get-Content -LiteralPath (Join-Path $sourceRoot "ipc_transport.cpp") -Raw
Require-Pattern $ipc 'SRWLOCK\s+ipc_transport_lock\s*=\s*SRWLOCK_INIT' `
    "IPC HANDLE/event/OVERLAPPED state is not protected by one transport lock."
Require-Pattern $ipc 'CancelIoEx[\s\S]+GetOverlappedResult\([\s\S]+TRUE\)' `
    "IPC cancellation does not wait for the specific OVERLAPPED completion."
Require-Pattern $ipc 'ipc_operation\.Complete\(ipc_generation\)[\s\S]+ipc_overlapped\s*=\s*\{\}' `
    "The OVERLAPPED structure can be cleared before its completion is consumed."
Require-Pattern $ipc 'IpcTransportState::Closing[\s\S]+PollIpcClose\(\)' `
    "The viewport timeout path does not defer cancellation completion polling."
Require-Pattern $ipc 'PumpLiveIpc[\s\S]+IpcTransportTryGuard[\s\S]+IpcPumpResult::Idle' `
    "The viewport pump can block behind a synchronous transport close."
Require-Pattern $ipc 'EnabledCapabilities\(\)[\s\S]+CAPABILITY_IPC' `
    "A stale tick can reopen IPC after the capability was disabled."

$shadow = Get-Content -LiteralPath (Join-Path $sourceRoot "shadow_vtable_hook.cpp") -Raw
$remove = [regex]::Match(
    $shadow,
    'void\s+ShadowVTableHook::Remove\(\)\s+noexcept[\s\S]+?\n\}',
    [System.Text.RegularExpressions.RegexOptions]::Singleline).Value
if (-not $remove) {
    throw "ShadowVTableHook::Remove was not found."
}
Reject-Pattern $remove 'VirtualFree\s*\(' `
    "Published shadow vtables are freed without a dispatch grace period."
Require-Pattern $shadow `
    'expected_vtable\s*==\s*nullptr[\s\S]+expected_original\s*==\s*nullptr[\s\S]+TryCompareExchangePointer\([\s\S]+original_vtable' `
    "Shadow hook publication does not require the exact captured vtable/original CAS lineage."

$runtime = Get-Content -LiteralPath (Join-Path $sourceRoot "plugin_runtime.cpp") -Raw
$resetWatches = [regex]::Match(
    $runtime,
    'void\s+ResetProcessEventWatches\(\)[\s\S]+?ReleaseSRWLockExclusive\(&process_event_lock\);\s*\}',
    [System.Text.RegularExpressions.RegexOptions]::Singleline).Value
if (-not $resetWatches) {
    throw "ResetProcessEventWatches was not found."
}
Reject-Pattern $resetWatches `
    'hook\.(object|original|original_vtable|vtable)\s*=\s*nullptr' `
    "Reset clears an original binding that a late ProcessEvent detour still needs."
Require-Pattern $runtime `
    'HookedProcessEventInstance[\s\S]+HookedProcessEventClass' `
    "Instance and class ProcessEvent hooks do not have independent detours."
Require-Pattern $runtime `
    'original_vtable\s*==\s*vtable[\s\S]+HookedProcessEventClass[\s\S]+HookedProcessEventInstance' `
    "ProcessEvent hook modes are not bound to an immutable vtable lineage."
Require-Pattern $runtime `
    'captured_params[\s\S]+memory::ReadBytes\([\s\S]+event\.params\s*=\s*captured_params' `
    "ProcessEvent params can be copied from game memory after only VirtualQuery validation."
Require-Pattern $runtime `
    'InstallViewportHook[\s\S]+ResolveViewport\(\)\s*!=\s*viewport[\s\S]+\.Install\(' `
    "The background viewport hook does not revalidate the full object chain before vptr CAS."
Require-Pattern $runtime `
    'hook\.object\s*==\s*object\s*&&\s*hook\.hook\.IsInstalled\(\)[\s\S]+hook\.hook\.CanInstall\(\)[\s\S]+candidate\.object\s*==\s*nullptr' `
    "ProcessEvent record lookup does not prefer the active binding before reusable or empty records."
Require-Pattern $runtime `
    'HookedProcessEventInstance\)[\s\S]+target_hook->original_vtable,[\s\S]+target_hook->original' `
    "ProcessEvent shadow publication is not tied to the immutable original binding."

$dllMain = Get-Content -LiteralPath (Join-Path $sourceRoot "nte_mods_plugin.cpp") -Raw
Reject-Pattern $dllMain `
    'LoadLibrary|FreeLibrary|CreateThread|WaitForSingleObject|InitializeDwmapiProxy|StopPluginRuntime|ShutdownDwmapiProxy|DisableThreadLibraryCalls' `
    "DllMain contains loader work, teardown work, or cross-thread waiting."
Require-Pattern $dllMain `
    'reason\s*==\s*DLL_PROCESS_ATTACH\s*&&[\s\S]+IsExplicitAttach\(reserved\)[\s\S]+StartPluginRuntime\(module\)' `
    "Manual-map runtime start is not gated by the explicit shared attach marker."

$memory = Get-Content -LiteralPath (Join-Path $sourceRoot "memory_access.cpp") -Raw
Require-Pattern $memory '__try[\s\S]+CopyBytes[\s\S]+__except' `
    "Game-memory copies are still based only on VirtualQuery check/use validation."

$hostApi = Get-Content -LiteralPath (Join-Path $sourceRoot "host_api.cpp") -Raw
Require-Pattern $hostApi `
    'ReadObjectSnapshot[\s\S]+InvokeNativeProcessEvent[\s\S]+__finally' `
    "UObject dispatch does not use checked snapshots with guaranteed flag restoration."
Require-Pattern $hostApi `
    'definition_snapshot[\s\S]+HashEffectName\(definition_snapshot\.name[\s\S]+definition_snapshot\.index' `
    "Party effect sampling still dereferences a definition after an advisory range check."

$modRuntime = Get-Content -LiteralPath (Join-Path $sourceRoot "mod_runtime.cpp") -Raw
Require-Pattern $modRuntime `
    'ReadPointerArrayFirst[\s\S]+memory::ReadPointer<void>\(array\.data, 0\)' `
    "The first game-owned array pointer is still read without the guarded memory helper."
Require-Pattern $modRuntime `
    'SamplePartyEffectsGuarded[\s\S]+__try[\s\S]+ResolveGameValue' `
    "Character-effect session resolution is outside the guarded game-thread boundary."

Write-Output "native_lifecycle_source_tests: PASS"
