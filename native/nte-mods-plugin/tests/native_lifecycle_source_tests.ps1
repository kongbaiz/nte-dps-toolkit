$ErrorActionPreference = "Stop"

$pluginRoot = Split-Path -Parent $PSScriptRoot
$sourceRoot = Join-Path $pluginRoot "src"
$project = Get-Content -LiteralPath (Join-Path $pluginRoot "nte-mods-plugin.vcxproj") -Raw

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

Require-Pattern $project '<BufferSecurityCheck>true</BufferSecurityCheck>' `
    "Release x64 does not enable the compiler stack security cookie (/GS)."
Require-Pattern $project '<SDLCheck>true</SDLCheck>' `
    "Release x64 does not enable additional SDL security diagnostics (/sdl)."
Require-Pattern $project '<ClInclude Include="src\\vtable_patch_policy.hpp" />' `
    "The typed vtable patch policy is not part of the native project."

$ipc = Get-Content -LiteralPath (Join-Path $sourceRoot "ipc_transport.cpp") -Raw
Require-Pattern $ipc 'SRWLOCK\s+ipc_transport_lock\s*=\s*SRWLOCK_INIT' `
    "IPC HANDLE/event/OVERLAPPED state is not protected by one transport lock."
Require-Pattern $ipc 'CancelIoEx[\s\S]+WaitForSingleObject\(\s*ipc_event,\s*IPC_CLIENT_IO_TIMEOUT_MS\)[\s\S]+GetOverlappedResult\([\s\S]+FALSE\)' `
    "IPC cancellation does not bound and consume the specific OVERLAPPED completion."
Reject-Pattern $ipc 'GetOverlappedResult\([\s\S]{0,180},\s*TRUE\)' `
    "IPC teardown can block indefinitely while draining an OVERLAPPED operation."
Require-Pattern $ipc 'CloseIpcPipe\(\)[\s\S]+ipc_transport_state\s*=\s*IpcTransportState::Closing[\s\S]+DrainIpcOperation\(\)' `
    "A bounded IPC drain failure can later resume as a dispatchable request."
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
    'bool\s+ShadowVTableHook::Remove\(\)\s+noexcept[\s\S]+?\n\}',
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
    'bool\s+ResetProcessEventWatches\(\)[\s\S]+?ReleaseSRWLockExclusive\(&process_event_lock\);[\s\S]+?return\s+all_removed;\s*\}',
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
Require-Pattern $runtime `
    'ReplaceProcessEventVTableEntry[\s\S]+hook::ReplaceProtectedPointer\([\s\S]+ApplyProcessEventClassPatch' `
    "Class-wide ProcessEvent hooks do not use the typed protection/rollback policy."
Require-Pattern $runtime `
    'FailProcessEventVTableIntegrity\(\)[\s\S]+process_event_vtable_healthy\s*=\s*false[\s\S]+result\.RequiresFailClosed\(\)[\s\S]+FailProcessEventVTableIntegrity\(\)' `
    "A protection or rollback integrity failure does not close the ProcessEvent hook gate."
Require-Pattern $runtime `
    'process_event_vtable_healthy\s*=\s*false[\s\S]+process_event_subscription_count\s*=\s*0[\s\S]+queue\.count\s*=\s*0' `
    "A ProcessEvent integrity failure can still project retained subscriptions or queued data."
Require-Pattern $runtime `
    'if\s*\(!process_event_vtable_healthy\)[\s\S]+ReleaseSRWLockExclusive\(&process_event_lock\);[\s\S]+return false' `
    "New ProcessEvent subscriptions remain available after a typed vtable integrity failure."
Require-Pattern $runtime `
    'AcquireSRWLockExclusive\(&process_event_lock\);[\s\S]+memory::ReadValue\(\s*object,\s*0,\s*vtable\)[\s\S]+process_event_original' `
    "ProcessEvent vtable/slot snapshot is not linearized under the hook lock."
Require-Pattern $runtime `
    'ApplyProcessEventClassPatch\([\s\S]+RequiresBindingRetention\(\)[\s\S]+process_event_class_hook_binding_count' `
    "Class bindings cannot distinguish unpublished failures from late-dispatch-safe publication."
Require-Pattern $runtime `
    'if\s*\(\s*replacement_is_installed\s*&&\s*result\.RequiresBindingRetention\(\)\s*\)\s*MarkRuntimeDetourPublished\(\)' `
    "A transiently published class detour can still be reported as unload-safe after rollback."
Require-Pattern $runtime `
    'bool\s+UnwatchProcessEvent\([\s\S]+ApplyProcessEventClassPatch\([\s\S]+if\s*\(!patch_result\.Applied\(\)\)[\s\S]+return false' `
    "Unwatch reports success after a class vtable teardown failure."
Require-Pattern $runtime `
    'enum class\s+PluginLifecycleState[\s\S]+SRWLOCK\s+runtime_lifecycle_lock\s*=\s*SRWLOCK_INIT' `
    "Plugin Start/Stop does not have one explicit lifecycle owner/state."
Require-Pattern $runtime `
    'runtime_stopping[\s\S]+InterlockedExchange\([\s\S]*&runtime_stopping,\s*1\)[\s\S]+SetIpcStopping\(true\)' `
    "Runtime shutdown does not close both detour and IPC dispatch gates first."
Require-Pattern $runtime `
    'StopPluginRuntime\(\)[\s\S]+Reset\(\)[\s\S]+RestoreViewportHook\(\)[\s\S]+CloseIpc\(\)[\s\S]+CloseRuntimePresence\(\)' `
    "Runtime shutdown does not attempt every independently owned teardown resource."
Require-Pattern $runtime `
    'reset_succeeded\s*=\s*runtime::Reset\(\)[\s\S]+viewport_succeeded\s*=\s*RestoreViewportHook\(\)[\s\S]+ipc_succeeded\s*=\s*CloseIpc\(\)' `
    "Runtime shutdown does not aggregate typed teardown results before reporting unloadability."
Require-Pattern $runtime `
    'ResetProcessEventWatches\(\)[\s\S]+all_removed\s*=\s*hook\.hook\.Remove\(\)\s*&&\s*all_removed' `
    "Instance ProcessEvent teardown failures are not propagated."
Require-Pattern $runtime `
    'TryCompareExchangeProcessEventSlot[\s\S]+__try[\s\S]+InterlockedCompareExchangePointer[\s\S]+__except' `
    "A stale ProcessEvent class slot can fault outside a narrow SEH boundary."
Require-Pattern $runtime `
    'IsImageRange\([\s\S]+slot[\s\S]+ReplaceProtectedPointer' `
    "Class-wide ProcessEvent patching accepts a non-image vtable slot."

$dllMain = Get-Content -LiteralPath (Join-Path $sourceRoot "nte_mods_plugin.cpp") -Raw
$dllMainBody = [regex]::Match(
    $dllMain,
    'BOOL\s+WINAPI\s+DllMain\([\s\S]+?\n\}',
    [System.Text.RegularExpressions.RegexOptions]::Singleline).Value
if (-not $dllMainBody) {
    throw "DllMain was not found."
}
Reject-Pattern $dllMainBody `
    'LoadLibrary|FreeLibrary|CreateThread|WaitForSingleObject|InitializeDwmapiProxy|StopPluginRuntime|ShutdownDwmapiProxy|DisableThreadLibraryCalls' `
    "DllMain contains loader work, teardown work, or cross-thread waiting."
Require-Pattern $dllMainBody `
    'reason\s*==\s*DLL_PROCESS_ATTACH\s*&&[\s\S]+IsExplicitAttach\(reserved\)[\s\S]+StartPluginRuntime\(module\)' `
    "Manual-map runtime start is not gated by the explicit shared attach marker."
Require-Pattern $dllMain `
    'NteModsPluginShutdown[\s\S]+PluginStopAllowsUnload\([\s\S]+StopPluginRuntime\(\)' `
    "Manual-map deployments have no explicit unload-safe shutdown export."

$memory = Get-Content -LiteralPath (Join-Path $sourceRoot "memory_access.cpp") -Raw
Require-Pattern $memory '__try[\s\S]+CopyBytes[\s\S]+__except' `
    "Game-memory copies are still based only on VirtualQuery check/use validation."
Require-Pattern $memory `
    'IsImageRange[\s\S]+memory\.Type\s*==\s*MEM_IMAGE[\s\S]+IsImageExecutableAddress' `
    "Original game dispatch pointers are not bound to executable image memory."

$hostApi = Get-Content -LiteralPath (Join-Path $sourceRoot "host_api.cpp") -Raw
Require-Pattern $hostApi `
    'ReadObjectSnapshot[\s\S]+InvokeNativeProcessEvent[\s\S]+__finally' `
    "UObject dispatch does not use checked snapshots with guaranteed flag restoration."
Require-Pattern $hostApi `
    'definition_snapshot[\s\S]+HashEffectName\(definition_snapshot\.name[\s\S]+definition_snapshot\.index' `
    "Party effect sampling still dereferences a definition after an advisory range check."

$modRuntime = Get-Content -LiteralPath (Join-Path $sourceRoot "mod_runtime.cpp") -Raw
Require-Pattern $modRuntime `
    'ReloadEnabledPrograms[\s\S]+if\s*\(!ResetProcessEventWatches\(\)\)[\s\S]+ReleaseSRWLockExclusive\(&program_lock\);[\s\S]+return\s+RecordReloadError' `
    "Program reload can replace runtime state after ProcessEvent teardown failed."
Require-Pattern $modRuntime `
    'bool\s+Reset\(\)[\s\S]+if\s*\(!ResetProcessEventWatches\(\)\)[\s\S]+return\s+false[\s\S]+return\s+true' `
    "Runtime reset does not propagate ProcessEvent teardown failure."
Require-Pattern $modRuntime `
    'DisableProgramsFailClosedLocked\(\)[\s\S]+ProcessEvent teardown failed; runtime disabled\.' `
    "A partial ProcessEvent teardown is still projected as the previous healthy program set."
Require-Pattern $modRuntime `
    'ReadPointerArrayFirst[\s\S]+memory::ReadPointer<void>\(array\.data, 0\)' `
    "The first game-owned array pointer is still read without the guarded memory helper."
Require-Pattern $modRuntime `
    'SamplePartyEffectsGuarded[\s\S]+__try[\s\S]+ResolveGameValue' `
    "Character-effect session resolution is outside the guarded game-thread boundary."

Write-Output "native_lifecycle_source_tests: PASS"
