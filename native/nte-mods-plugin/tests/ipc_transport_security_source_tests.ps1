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

Require-Pattern $ipc 'FILE_FLAG_FIRST_PIPE_INSTANCE' `
    "The fixed pipe name remains silently joinable after pre-creation."
Require-Pattern $ipc 'PIPE_REJECT_REMOTE_CLIENTS' `
    "The pipe no longer rejects remote clients."
Require-Pattern $ipc 'D:P\(A;;GA;;;SY\)\(A;;GA;;;BA\)\(A;;GA;;;IU\)[\s\S]+S:\(ML;;NW;;;ME\)' `
    "The local IPC descriptor no longer permits the medium-integrity interactive desktop client."
Reject-Pattern $ipc 'TokenIdentity|TokenSessionId|GetNamedPipeClientProcessId|OpenProcessToken|GetSecurityInfo|ValidateIpcClient|ValidatePresenceEventSecurity|AddAccessAllowedAceEx' `
    "The native transport still contains process-token or exact owner/DACL authentication."
Require-Pattern $ipc 'CreateEventW\([\s\S]{0,200}security\.Get\(\)' `
    "Runtime presence is not published with the shared local IPC descriptor."
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
Reject-Pattern $rustClient 'TokenIdentity|GetNamedPipeServerProcessId|ProcessIdToSessionId|GetSecurityInfo|validate_pipe_security|authenticate_pipe_server|query_current_token_identity|open_authenticated_pipe' `
    "The desktop client still contains process-token or exact owner/DACL authentication."
Require-Pattern $rustClient 'probe_runtime_presence[\s\S]+open_runtime_pipe' `
    "Runtime readiness no longer requires a successful local pipe connection."
Reject-Pattern $rustClient 'CreateFileW\([\s\S]{0,500}READ_CONTROL' `
    "The IPC client still requests security-descriptor inspection rights."
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
