use std::mem::size_of;
use std::net::Ipv4Addr;
use std::ptr;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows_sys::Win32::Networking::WinSock::AF_INET;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use crate::storage::i18n::tf;

const GAME_PROCESS: &str = "HTGame.exe";
const GAME_TCP_PORT: u16 = 30031;
const MIB_TCP_STATE_ESTABLISHED: u32 = 5;
const MAX_TCP_TABLE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameNetwork {
    pub pid: u32,
    pub local_ip: Ipv4Addr,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkProbeErrorCode {
    ProcessSnapshotFailed,
    ProcessEnumerationFailed,
    TcpTableSizeQueryFailed,
    TcpTableQueryFailed,
    TcpTableInvalidSize,
    TcpTableMalformed,
}

impl NetworkProbeErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProcessSnapshotFailed => "PROCESS_SNAPSHOT_FAILED",
            Self::ProcessEnumerationFailed => "PROCESS_ENUMERATION_FAILED",
            Self::TcpTableSizeQueryFailed => "TCP_TABLE_SIZE_QUERY_FAILED",
            Self::TcpTableQueryFailed => "TCP_TABLE_QUERY_FAILED",
            Self::TcpTableInvalidSize => "TCP_TABLE_INVALID_SIZE",
            Self::TcpTableMalformed => "TCP_TABLE_MALFORMED",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkProbeFailure {
    pub code: NetworkProbeErrorCode,
    pub detail: String,
}

impl NetworkProbeFailure {
    pub fn new(code: NetworkProbeErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameNetworkUnavailable {
    ProcessNotFound,
    NoUsableConnection { pid: u32 },
}

impl GameNetworkUnavailable {
    pub fn detail(self) -> String {
        match self {
            Self::ProcessNotFound => tf("Game process {} not detected", &[GAME_PROCESS]),
            Self::NoUsableConnection { pid } => tf(
                "Detected {} (PID {}) but no usable IPv4 TCP connection for NIC lookup yet",
                &[GAME_PROCESS, &pid.to_string()],
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GameNetworkProbe {
    Connected(GameNetwork),
    NotConnected(GameNetworkUnavailable),
    ProbeFailed(NetworkProbeFailure),
}

pub fn game_process_is_running() -> Result<bool, String> {
    find_process_id(GAME_PROCESS)
        .map(|pid| pid.is_some())
        .map_err(|failure| failure.detail)
}

/// Locate the game's active IPv4 TCP connection (PID + local/remote endpoints) without requiring a
/// matching Npcap device. Manual capture mode uses this to recover `local_ip` for direction
/// inference even when auto device matching would fail (e.g. the game routes over a VPN adapter).
pub fn detect_game_network() -> GameNetworkProbe {
    classify_game_network_probe(find_process_id(GAME_PROCESS), tcp_connections_for_pid)
}

fn classify_game_network_probe(
    process: Result<Option<u32>, NetworkProbeFailure>,
    connections_for_pid: impl FnOnce(u32) -> Result<Vec<GameNetwork>, NetworkProbeFailure>,
) -> GameNetworkProbe {
    let pid = match process {
        Ok(Some(pid)) => pid,
        Ok(None) => {
            return GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound);
        }
        Err(failure) => return GameNetworkProbe::ProbeFailed(failure),
    };
    let connections = match connections_for_pid(pid) {
        Ok(connections) => connections,
        Err(failure) => return GameNetworkProbe::ProbeFailed(failure),
    };
    connections
        .iter()
        .find(|row| row.remote_port == GAME_TCP_PORT)
        .or_else(|| connections.first())
        .cloned()
        .map_or_else(
            || GameNetworkProbe::NotConnected(GameNetworkUnavailable::NoUsableConnection { pid }),
            GameNetworkProbe::Connected,
        )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessEnumerationOperation {
    First,
    Next,
}

impl ProcessEnumerationOperation {
    const fn api_name(self) -> &'static str {
        match self {
            Self::First => "Process32FirstW",
            Self::Next => "Process32NextW",
        }
    }
}

/// Classifies a failed ToolHelp enumeration call. `ERROR_NO_MORE_FILES` is the
/// normal exhausted-list result; every other Win32 error remains a typed probe
/// failure rather than being folded into process absence.
fn classify_process_enumeration_error(
    operation: ProcessEnumerationOperation,
    error: u32,
) -> Result<(), NetworkProbeFailure> {
    if error == ERROR_NO_MORE_FILES {
        return Ok(());
    }
    Err(NetworkProbeFailure::new(
        NetworkProbeErrorCode::ProcessEnumerationFailed,
        format!("{} failed with Win32 error {error}", operation.api_name()),
    ))
}

fn find_process_id(executable: &str) -> Result<Option<u32>, NetworkProbeFailure> {
    // SAFETY: Toolhelp snapshot functions are called with initialized structures and closed below.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            let error = GetLastError();
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::ProcessSnapshotFailed,
                format!("CreateToolhelp32Snapshot failed with Win32 error {error}"),
            ));
        }
        let result = (|| {
            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            if Process32FirstW(snapshot, &mut entry) == 0 {
                let error = GetLastError();
                return classify_process_enumeration_error(
                    ProcessEnumerationOperation::First,
                    error,
                )
                .map(|()| None);
            }
            loop {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|value| *value == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
                if name.eq_ignore_ascii_case(executable) {
                    return Ok(Some(entry.th32ProcessID));
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    let error = GetLastError();
                    return classify_process_enumeration_error(
                        ProcessEnumerationOperation::Next,
                        error,
                    )
                    .map(|()| None);
                }
            }
        })();
        CloseHandle(snapshot);
        result
    }
}

fn tcp_connections_for_pid(pid: u32) -> Result<Vec<GameNetwork>, NetworkProbeFailure> {
    // SAFETY: The buffer size is obtained from GetExtendedTcpTable and rows are read unaligned
    // because the table starts immediately after its 32-bit count.
    unsafe {
        let mut size = 0_u32;
        let first = GetExtendedTcpTable(
            ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
        if first == 0 && size == 0 {
            return Ok(Vec::new());
        }
        if first != ERROR_INSUFFICIENT_BUFFER {
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::TcpTableSizeQueryFailed,
                format!("GetExtendedTcpTable size query failed with Win32 error {first}"),
            ));
        }
        let requested_size = size as usize;
        if requested_size < size_of::<u32>() || requested_size > MAX_TCP_TABLE_BYTES {
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::TcpTableInvalidSize,
                format!("GetExtendedTcpTable requested invalid buffer size {requested_size}"),
            ));
        }
        let mut buffer = vec![0_u8; requested_size];
        let result = GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );
        if result != 0 {
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::TcpTableQueryFailed,
                format!("GetExtendedTcpTable failed with Win32 error {result}"),
            ));
        }
        let returned_size = size as usize;
        if returned_size < size_of::<u32>() || returned_size > buffer.len() {
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::TcpTableInvalidSize,
                format!("GetExtendedTcpTable returned invalid buffer size {returned_size}"),
            ));
        }
        let count = ptr::read_unaligned(buffer.as_ptr().cast::<u32>()) as usize;
        let rows_start = buffer.as_ptr().add(size_of::<u32>());
        let available_rows =
            returned_size.saturating_sub(size_of::<u32>()) / size_of::<MIB_TCPROW_OWNER_PID>();
        if count > available_rows {
            return Err(NetworkProbeFailure::new(
                NetworkProbeErrorCode::TcpTableMalformed,
                format!(
                    "GetExtendedTcpTable declared {count} rows but only {available_rows} are available"
                ),
            ));
        }
        let mut connections = Vec::new();
        for index in 0..count {
            let row = ptr::read_unaligned(
                rows_start
                    .add(index * size_of::<MIB_TCPROW_OWNER_PID>())
                    .cast::<MIB_TCPROW_OWNER_PID>(),
            );
            if row.dwOwningPid != pid || row.dwState != MIB_TCP_STATE_ESTABLISHED {
                continue;
            }
            let local_ip = Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
            let remote_ip = Ipv4Addr::from(row.dwRemoteAddr.to_ne_bytes());
            if local_ip.is_loopback() || local_ip.is_unspecified() || remote_ip.is_unspecified() {
                continue;
            }
            connections.push(GameNetwork {
                pid,
                local_ip,
                remote_ip,
                remote_port: decode_port(row.dwRemotePort),
            });
        }
        connections.sort_by_key(|row| (row.remote_port != GAME_TCP_PORT, row.remote_port));
        Ok(connections)
    }
}

fn decode_port(value: u32) -> u16 {
    u16::from_be(value as u16)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

    fn failure(code: NetworkProbeErrorCode) -> NetworkProbeFailure {
        NetworkProbeFailure::new(code, "fixture failure")
    }

    fn connection(pid: u32, remote_port: u16) -> GameNetwork {
        GameNetwork {
            pid,
            local_ip: "192.168.1.2".parse().expect("fixture local IP"),
            remote_ip: "203.0.113.9".parse().expect("fixture remote IP"),
            remote_port,
        }
    }

    #[test]
    fn typed_probe_distinguishes_process_absence_without_querying_tcp() {
        let queried = Cell::new(false);

        let outcome = classify_game_network_probe(Ok(None), |_| {
            queried.set(true);
            Ok(Vec::new())
        });

        assert_eq!(
            outcome,
            GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound)
        );
        assert!(!queried.get());
    }

    #[test]
    fn typed_probe_distinguishes_running_process_without_connection() {
        let outcome = classify_game_network_probe(Ok(Some(42)), |_| Ok(Vec::new()));

        assert_eq!(
            outcome,
            GameNetworkProbe::NotConnected(GameNetworkUnavailable::NoUsableConnection { pid: 42 })
        );
    }

    #[test]
    fn typed_probe_preserves_process_and_tcp_failures_with_stable_codes() {
        let process_failure = failure(NetworkProbeErrorCode::ProcessSnapshotFailed);
        assert_eq!(
            classify_game_network_probe(Err(process_failure.clone()), |_| Ok(Vec::new())),
            GameNetworkProbe::ProbeFailed(process_failure)
        );

        let tcp_failure = failure(NetworkProbeErrorCode::TcpTableQueryFailed);
        assert_eq!(
            classify_game_network_probe(Ok(Some(7)), |_| Err(tcp_failure.clone())),
            GameNetworkProbe::ProbeFailed(tcp_failure)
        );
        assert_eq!(
            NetworkProbeErrorCode::TcpTableQueryFailed.as_str(),
            "TCP_TABLE_QUERY_FAILED"
        );
    }

    #[test]
    fn enumeration_access_denied_is_probe_failure_while_exhaustion_is_not_connected() {
        let access_denied = classify_process_enumeration_error(
            ProcessEnumerationOperation::First,
            ERROR_ACCESS_DENIED,
        )
        .map(|()| None);
        let access_denied_probe = classify_game_network_probe(access_denied, |_| Ok(Vec::new()));
        let GameNetworkProbe::ProbeFailed(failure) = access_denied_probe else {
            panic!("access denied must remain an OS probe failure");
        };
        assert_eq!(
            failure.code,
            NetworkProbeErrorCode::ProcessEnumerationFailed
        );
        assert!(failure.detail.contains("Win32 error 5"));

        let exhausted = classify_process_enumeration_error(
            ProcessEnumerationOperation::Next,
            ERROR_NO_MORE_FILES,
        )
        .map(|()| None);
        assert_eq!(
            classify_game_network_probe(exhausted, |_| Ok(Vec::new())),
            GameNetworkProbe::NotConnected(GameNetworkUnavailable::ProcessNotFound)
        );
    }

    #[test]
    fn typed_probe_prefers_the_known_game_service_port() {
        let outcome = classify_game_network_probe(Ok(Some(9)), |pid| {
            Ok(vec![connection(pid, 443), connection(pid, GAME_TCP_PORT)])
        });

        let GameNetworkProbe::Connected(network) = outcome else {
            panic!("expected a connected game network");
        };
        assert_eq!(network.remote_port, GAME_TCP_PORT);
    }
}
