$ErrorActionPreference = "Stop"

$pluginRoot = Split-Path -Parent $PSScriptRoot
$repositoryRoot = Split-Path -Parent (Split-Path -Parent $pluginRoot)
$sourceRoot = Join-Path $pluginRoot "src"
$ipc = Get-Content -LiteralPath (Join-Path $sourceRoot "ipc_transport.cpp") -Raw
$header = Get-Content -LiteralPath (Join-Path $sourceRoot "ipc_transport.hpp") -Raw
$runtime = Get-Content -LiteralPath (Join-Path $sourceRoot "plugin_runtime.cpp") -Raw
$protocol = Get-Content -LiteralPath (Join-Path $pluginRoot "include\nte_mods_ipc.h") -Raw
$rustClient = Get-Content -LiteralPath (Join-Path $repositoryRoot "src\platform\mods_plugin.rs") -Raw
$buildWorkflow = Get-Content -LiteralPath (Join-Path $repositoryRoot ".github\workflows\build.yml") -Raw

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

Require-Pattern $ipc 'IpcTransportState::AwaitingClientAck' `
    "The server disconnects before a bounded client acknowledgement/close-read."
Require-Pattern $protocol 'NTE_MODS_IPC_DELIVERY_ACK_MAGIC\s+0x4145544Eu' `
    "The delivery acknowledgement magic is missing or unstable."
Require-Pattern $protocol 'NTE_MODS_IPC_DELIVERY_ACK_SIZE\s+16u[\s\S]+NteModsIpcDeliveryAck[\s\S]+request_id' `
    "The bounded delivery acknowledgement contract is missing."
Require-Pattern $ipc 'BeginIpcClientAck[\s\S]+ReadFile\([\s\S]+ipc_delivery_ack' `
    "The response path does not start an overlapped client acknowledgement read."
Require-Pattern $ipc 'WriteFile\([\s\S]+ipc_response[\s\S]+BeginIpcClientAck' `
    "A completed response write does not transition to acknowledgement delivery."
Require-Pattern $ipc 'AwaitingClientAck[\s\S]+IPC_CLIENT_IO_TIMEOUT_MS[\s\S]+BeginIpcClose' `
    "The acknowledgement wait is not bounded by the transport timeout."
Require-Pattern $ipc 'AwaitingClientAck[\s\S]+ERROR_BROKEN_PIPE[\s\S]+ReconnectIpcPipe' `
    "Legacy clients that close after reading the response are not treated as delivered."
Reject-Pattern $ipc 'FlushFileBuffers\s*\(' `
    "The viewport transport must not block while flushing a named pipe."

Require-Pattern $ipc 'SE_GROUP_LOGON_ID' `
    "The pipe DACL is not bound to the current logon SID."
Require-Pattern $ipc 'TokenSessionId' `
    "The client token session is not validated."
Require-Pattern $ipc 'GetNamedPipeClientProcessId' `
    "The connected pipe client process is not identified."
Require-Pattern $ipc 'OpenProcessToken[\s\S]+EqualSid' `
    "The connected client token user/logon identity is not compared."
Require-Pattern $ipc 'FILE_FLAG_FIRST_PIPE_INSTANCE' `
    "The fixed pipe name remains silently joinable after pre-creation."
Reject-Pattern $ipc 'A;;GA;;;IU|GENERIC_ALL' `
    "IPC objects still grant Generic All to Interactive Users."
Reject-Pattern $ipc 'FILE_GENERIC_(?:READ|WRITE)|GENERIC_(?:READ|WRITE)|FILE_CREATE_PIPE_INSTANCE' `
    "The pipe client DACL still includes generic access or server-instance creation rights."
Require-Pattern $ipc 'IPC_PIPE_CLIENT_ACCESS\s*=\s*[\r\n\t ]*FILE_READ_DATA\s*\|\s*FILE_WRITE_DATA\s*\|\s*SYNCHRONIZE' `
    "The pipe client DACL is not limited to explicit read/write/synchronize rights."

Require-Pattern $ipc 'GetSecurityInfo' `
    "The runtime-presence event owner/DACL is not inspected."
Require-Pattern $ipc 'IPC_PRESENCE_OWNER_ACCESS\s*=\s*[\r\n\t ]*EVENT_MODIFY_STATE\s*\|\s*SYNCHRONIZE\s*\|\s*READ_CONTROL' `
    "The runtime-presence creator handle is missing its exact server rights."
Require-Pattern $ipc 'IPC_PRESENCE_CLIENT_ACCESS\s*=\s*[\r\n\t ]*SYNCHRONIZE\s*\|\s*READ_CONTROL' `
    "The runtime-presence logon client ACE cannot inspect the exact owner/DACL."
Require-Pattern $ipc 'AceCount\s*==\s*1[\s\S]+ace->Mask\s*==\s*IPC_PRESENCE_CLIENT_ACCESS[\s\S]+expected\.LogonSid' `
    "Presence security validation does not require one exact read-only logon ACE."
Reject-Pattern $ipc 'WinHighLabelSid|found_owner_ace' `
    "Presence still relies on an elevated owner ACE instead of a least-rights published DACL."
Require-Pattern $ipc 'CreateEventExW[\s\S]+ERROR_ALREADY_EXISTS[\s\S]+ValidatePresenceEventSecurity[\s\S]+SetEvent' `
    "A pre-created runtime-presence event is not rejected before publication."
Require-Pattern $header 'enum class IpcCloseResult[\s\S]+Closed[\s\S]+DrainFailed' `
    "IPC close does not expose a typed drain result."
Require-Pattern $header 'enum class RuntimePresenceCloseResult[\s\S]+Closed[\s\S]+CloseFailed' `
    "Presence close does not expose a typed CloseHandle result."
Require-Pattern $header 'void\s+SetIpcStopping\(bool\s+stopping\)' `
    "The runtime owner cannot close the IPC admission gate before teardown."
Require-Pattern $ipc 'IpcCloseResult\s+CloseIpc\(\)[\s\S]+IpcCloseResult::DrainFailed' `
    "A failed OVERLAPPED drain can still be reported as a successful close."
Require-Pattern $ipc 'RuntimePresenceCloseResult\s+CloseRuntimePresence\(\)[\s\S]+CloseHandle[\s\S]+RuntimePresenceCloseResult::CloseFailed' `
    "A failed presence CloseHandle can still be reported as successful."

Require-Pattern $runtime 'if\s*\(\s*!OpenRuntimePresence\(\)\s*\)[\s\S]+PluginLifecycleState::Stopped[\s\S]+return\s+PluginStartResult::Failed[\s\S]+PluginLifecycleState::Running' `
    "Presence publication failure can still be exposed as a running plugin."

Require-Pattern $rustClient 'FILE_FLAG_OVERLAPPED' `
    "The desktop client still opens the IPC pipe for synchronous I/O."
Require-Pattern $rustClient 'GetOverlappedResultEx' `
    "The desktop client does not bound request/response/ACK completion."
Require-Pattern $rustClient 'CancelIoEx' `
    "The desktop client does not cancel a timed-out named-pipe operation."
Require-Pattern $rustClient 'CancelIoEx[\s\S]+GetOverlappedResultEx\([\s\S]+IPC_CANCEL_DRAIN_TIMEOUT_MS' `
    "The desktop client does not perform a bounded terminal drain after cancellation."
Require-Pattern $rustClient 'OverlappedWaitError::Undrained[\s\S]+operation\.quarantine\(\)' `
    "An undrained client operation can release kernel-referenced memory or retry indefinitely."
Require-Pattern $rustClient 'GetNamedPipeServerProcessId' `
    "The desktop client does not bind the pipe handle to its server PID."
Require-Pattern $rustClient 'QueryFullProcessImageNameW' `
    "The desktop client does not constrain the pipe server to HTGame.exe."
Require-Pattern $rustClient 'TokenSessionId[\s\S]+SE_GROUP_LOGON_ID' `
    "The desktop client does not compare user/logon/session token identity."
Require-Pattern $rustClient 'GetSecurityInfo[\s\S]+OWNER_SECURITY_INFORMATION\s*\|\s*DACL_SECURITY_INFORMATION' `
    "The desktop client does not authenticate the presence event owner/DACL."
Require-Pattern $rustClient 'probe_runtime_presence[\s\S]+open_authenticated_pipe' `
    "A fixed-name presence event can still be reported online without an authenticated game pipe."
Reject-Pattern $rustClient 'CreateFileW\([\s\S]{0,500}OPEN_EXISTING,\s*0,\s*ptr::null_mut\(\)' `
    "The IPC pipe can still be opened without FILE_FLAG_OVERLAPPED."
Reject-Pattern $rustClient '(?:ReadFile|WriteFile)\([\s\S]{0,350}&mut\s+immediate' `
    "An overlapped pipe operation still trusts the synchronous byte-count out parameter."
Require-Pattern $rustClient 'WriteFile\([\s\S]{0,350}ptr::null_mut\(\)[\s\S]{0,700}await_overlapped' `
    "Overlapped writes do not obtain their terminal byte count from GetOverlappedResultEx."
Require-Pattern $rustClient 'ReadFile\([\s\S]{0,350}ptr::null_mut\(\)[\s\S]{0,700}await_overlapped' `
    "Overlapped reads do not obtain their terminal byte count from GetOverlappedResultEx."

foreach ($requiredGate in @(
    'ipc_transport_security_source_tests\.ps1',
    'native_lifecycle_source_tests\.ps1',
    'test_native_signature_policy\.ps1',
    'test_mod_runtime_schema\.ps1'
)) {
    Require-Pattern $buildWorkflow $requiredGate `
        "The automated Windows build does not run native gate: $requiredGate"
}

Write-Output "ipc_transport_security_source_tests: PASS"
