use std::ffi::c_void;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR, ERROR_WINHTTP_AUTODETECTION_FAILED,
    ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT, ERROR_WINHTTP_CANNOT_CONNECT,
    ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED, ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED_PROXY,
    ERROR_WINHTTP_CLIENT_CERT_NO_ACCESS_PRIVATE_KEY, ERROR_WINHTTP_CLIENT_CERT_NO_PRIVATE_KEY,
    ERROR_WINHTTP_CONNECTION_ERROR, ERROR_WINHTTP_LOGIN_FAILURE, ERROR_WINHTTP_NAME_NOT_RESOLVED,
    ERROR_WINHTTP_SCRIPT_EXECUTION_ERROR, ERROR_WINHTTP_SECURE_CERT_CN_INVALID,
    ERROR_WINHTTP_SECURE_CERT_DATE_INVALID, ERROR_WINHTTP_SECURE_CERT_REV_FAILED,
    ERROR_WINHTTP_SECURE_CERT_REVOKED, ERROR_WINHTTP_SECURE_CERT_WRONG_USAGE,
    ERROR_WINHTTP_SECURE_CHANNEL_ERROR, ERROR_WINHTTP_SECURE_FAILURE,
    ERROR_WINHTTP_SECURE_FAILURE_PROXY, ERROR_WINHTTP_SECURE_INVALID_CA,
    ERROR_WINHTTP_SECURE_INVALID_CERT, ERROR_WINHTTP_TIMEOUT,
    ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT, ERROR_WINHTTP_UNHANDLED_SCRIPT_TYPE, URL_COMPONENTS,
    WINHTTP_ACCESS_TYPE, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_ADDREQ_FLAG_REPLACE,
    WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};

const USER_AGENT: &str = concat!("NTE-DPS-Tool-Updater/", env!("CARGO_PKG_VERSION"));
const RESOLVE_TIMEOUT_MS: i32 = 15_000;
const CONNECT_TIMEOUT_MS: i32 = 15_000;
const SEND_TIMEOUT_MS: i32 = 30_000;
const RECEIVE_TIMEOUT_MS: i32 = 30_000;
const READ_BUFFER_SIZE: usize = 64 * 1024;
// Retryable gateway failures get two additional attempts without changing route.
const SAME_ROUTE_RETRY_LIMIT: usize = 2;

#[derive(Clone, Copy, Debug)]
enum ProxyMode {
    Automatic,
    WinHttpDefault,
    Direct,
}

impl ProxyMode {
    const ALL: [Self; 3] = [Self::Automatic, Self::WinHttpDefault, Self::Direct];

    fn access_type(self) -> WINHTTP_ACCESS_TYPE {
        match self {
            Self::Automatic => WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            Self::WinHttpDefault => WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            Self::Direct => WINHTTP_ACCESS_TYPE_NO_PROXY,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Windows automatic proxy",
            Self::WinHttpDefault => "WinHTTP default proxy",
            Self::Direct => "direct connection",
        }
    }
}

#[derive(Debug)]
pub enum HttpError {
    InvalidUrl(String),
    Transport {
        mode: &'static str,
        source: io::Error,
    },
    Status(u32),
    ResponseTooLarge {
        maximum: u64,
        received_at_least: u64,
    },
    PackageLargerThanManifest {
        expected: u64,
        received_at_least: u64,
    },
    SizeMismatch {
        expected: u64,
        actual: u64,
    },
    File(io::Error),
}

impl fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(detail) => write!(formatter, "invalid update URL: {detail}"),
            Self::Transport { mode, source } => {
                write!(formatter, "{mode} request failed: {source}")
            }
            Self::Status(status) => write!(formatter, "update server returned HTTP {status}"),
            Self::ResponseTooLarge {
                maximum,
                received_at_least,
            } => write!(
                formatter,
                "update response exceeds the allowed size: maximum {maximum} bytes, received at least {received_at_least} bytes"
            ),
            Self::PackageLargerThanManifest {
                expected,
                received_at_least,
            } => write!(
                formatter,
                "update package is larger than the signed manifest: manifest declares {expected} bytes, received at least {received_at_least} bytes"
            ),
            Self::SizeMismatch { expected, actual } => write!(
                formatter,
                "update package size does not match the signed manifest: manifest declares {expected} bytes, received {actual} bytes"
            ),
            Self::File(error) => write!(formatter, "update file operation failed: {error}"),
        }
    }
}

impl std::error::Error for HttpError {}

pub fn get_bytes(url: &str, maximum_size: usize) -> Result<Vec<u8>, HttpError> {
    let parsed = ParsedHttpsUrl::parse(url)?;
    execute_with_proxy_routes(|mode| get_bytes_once(&parsed, mode, maximum_size))
}

pub fn download_file(
    url: &str,
    destination: &Path,
    expected_size: u64,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), HttpError> {
    let parsed = ParsedHttpsUrl::parse(url)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(HttpError::File)?;
    }
    execute_with_proxy_routes(|mode| {
        download_file_once(&parsed, mode, destination, expected_size, &mut progress)
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryDecision {
    RetrySameRoute,
    SwitchRoute,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportFailureKind {
    ProxyDiscovery,
    NameResolution,
    Connection,
    Timeout,
    Security,
    Authentication,
    Other,
}

impl TransportFailureKind {
    fn allows_route_switch(self) -> bool {
        matches!(
            self,
            Self::ProxyDiscovery | Self::NameResolution | Self::Connection | Self::Timeout
        )
    }
}

fn classify_transport_failure(source: &io::Error) -> TransportFailureKind {
    match source.raw_os_error().map(|code| code as u32) {
        Some(
            ERROR_WINHTTP_AUTODETECTION_FAILED
            | ERROR_WINHTTP_AUTO_PROXY_SERVICE_ERROR
            | ERROR_WINHTTP_BAD_AUTO_PROXY_SCRIPT
            | ERROR_WINHTTP_SCRIPT_EXECUTION_ERROR
            | ERROR_WINHTTP_UNABLE_TO_DOWNLOAD_SCRIPT
            | ERROR_WINHTTP_UNHANDLED_SCRIPT_TYPE,
        ) => TransportFailureKind::ProxyDiscovery,
        Some(ERROR_WINHTTP_NAME_NOT_RESOLVED) => TransportFailureKind::NameResolution,
        Some(ERROR_WINHTTP_CANNOT_CONNECT | ERROR_WINHTTP_CONNECTION_ERROR) => {
            TransportFailureKind::Connection
        }
        Some(ERROR_WINHTTP_TIMEOUT) => TransportFailureKind::Timeout,
        Some(
            ERROR_WINHTTP_CLIENT_CERT_NO_ACCESS_PRIVATE_KEY
            | ERROR_WINHTTP_CLIENT_CERT_NO_PRIVATE_KEY
            | ERROR_WINHTTP_SECURE_CERT_CN_INVALID
            | ERROR_WINHTTP_SECURE_CERT_DATE_INVALID
            | ERROR_WINHTTP_SECURE_CERT_REV_FAILED
            | ERROR_WINHTTP_SECURE_CERT_REVOKED
            | ERROR_WINHTTP_SECURE_CERT_WRONG_USAGE
            | ERROR_WINHTTP_SECURE_CHANNEL_ERROR
            | ERROR_WINHTTP_SECURE_FAILURE
            | ERROR_WINHTTP_SECURE_FAILURE_PROXY
            | ERROR_WINHTTP_SECURE_INVALID_CA
            | ERROR_WINHTTP_SECURE_INVALID_CERT,
        ) => TransportFailureKind::Security,
        Some(
            ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED
            | ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED_PROXY
            | ERROR_WINHTTP_LOGIN_FAILURE,
        ) => TransportFailureKind::Authentication,
        _ if source.kind() == io::ErrorKind::TimedOut => TransportFailureKind::Timeout,
        _ => TransportFailureKind::Other,
    }
}

fn classify_retry(error: &HttpError) -> RetryDecision {
    match error {
        HttpError::Transport { source, .. }
            if classify_transport_failure(source).allows_route_switch() =>
        {
            RetryDecision::SwitchRoute
        }
        // Proxy authentication is terminal on the current route. Even WinHTTP's default-proxy
        // mode may resolve to DIRECT or a bypass entry, so a 407 must not advance the route list.
        HttpError::Status(407) => RetryDecision::Terminal,
        HttpError::Status(502..=504) => RetryDecision::RetrySameRoute,
        HttpError::InvalidUrl(_)
        | HttpError::Transport { .. }
        | HttpError::Status(_)
        | HttpError::ResponseTooLarge { .. }
        | HttpError::PackageLargerThanManifest { .. }
        | HttpError::SizeMismatch { .. }
        | HttpError::File(_) => RetryDecision::Terminal,
    }
}

fn execute_with_proxy_routes<T>(
    mut request: impl FnMut(ProxyMode) -> Result<T, HttpError>,
) -> Result<T, HttpError> {
    for mode in ProxyMode::ALL {
        let mut same_route_retries = 0;
        loop {
            match request(mode) {
                Ok(value) => return Ok(value),
                Err(error) => match classify_retry(&error) {
                    RetryDecision::RetrySameRoute
                        if same_route_retries < SAME_ROUTE_RETRY_LIMIT =>
                    {
                        same_route_retries += 1;
                    }
                    RetryDecision::SwitchRoute if !matches!(mode, ProxyMode::Direct) => break,
                    RetryDecision::RetrySameRoute
                    | RetryDecision::SwitchRoute
                    | RetryDecision::Terminal => return Err(error),
                },
            }
        }
    }

    Err(HttpError::Transport {
        mode: "proxy route selection",
        source: io::Error::other("no WinHTTP proxy route was attempted"),
    })
}

fn get_bytes_once(
    parsed: &ParsedHttpsUrl,
    mode: ProxyMode,
    maximum_size: usize,
) -> Result<Vec<u8>, HttpError> {
    let request = open_get_request(
        parsed,
        mode,
        "Accept: application/json\r\nCache-Control: no-cache\r\n",
    )?;
    let status = request.status_code(mode)?;
    if status != 200 {
        return Err(HttpError::Status(status));
    }
    let mut bytes = Vec::new();
    request.read_chunks(mode, |chunk| {
        let next_length =
            bytes
                .len()
                .checked_add(chunk.len())
                .ok_or(HttpError::ResponseTooLarge {
                    maximum: maximum_size as u64,
                    received_at_least: u64::MAX,
                })?;
        if next_length > maximum_size {
            return Err(HttpError::ResponseTooLarge {
                maximum: maximum_size as u64,
                received_at_least: next_length as u64,
            });
        }
        bytes.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(bytes)
}

fn download_file_once(
    parsed: &ParsedHttpsUrl,
    mode: ProxyMode,
    destination: &Path,
    expected_size: u64,
    progress: &mut impl FnMut(u64, u64),
) -> Result<(), HttpError> {
    let existing_size = destination.metadata().map(|meta| meta.len()).unwrap_or(0);
    if existing_size > expected_size {
        File::create(destination).map_err(HttpError::File)?;
    }
    let existing_size = destination.metadata().map(|meta| meta.len()).unwrap_or(0);
    if existing_size == expected_size {
        progress(expected_size, expected_size);
        return Ok(());
    }
    let headers = if existing_size == 0 {
        "Accept: application/octet-stream\r\n".to_owned()
    } else {
        format!("Accept: application/octet-stream\r\nRange: bytes={existing_size}-\r\n")
    };
    let request = open_get_request(parsed, mode, &headers)?;
    let status = request.status_code(mode)?;
    let append = match status {
        200 => false,
        206 if existing_size > 0 => true,
        _ => return Err(HttpError::Status(status)),
    };
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(destination)
        .map_err(HttpError::File)?;
    let mut downloaded = if append { existing_size } else { 0 };
    progress(downloaded, expected_size);
    request.read_chunks(mode, |chunk| {
        downloaded = downloaded.checked_add(chunk.len() as u64).ok_or(
            HttpError::PackageLargerThanManifest {
                expected: expected_size,
                received_at_least: u64::MAX,
            },
        )?;
        if downloaded > expected_size {
            return Err(HttpError::PackageLargerThanManifest {
                expected: expected_size,
                received_at_least: downloaded,
            });
        }
        file.write_all(chunk).map_err(HttpError::File)?;
        progress(downloaded, expected_size);
        Ok(())
    })?;
    file.flush().map_err(HttpError::File)?;
    file.sync_all().map_err(HttpError::File)?;
    if downloaded != expected_size {
        return Err(HttpError::SizeMismatch {
            expected: expected_size,
            actual: downloaded,
        });
    }
    Ok(())
}

fn open_get_request(
    parsed: &ParsedHttpsUrl,
    mode: ProxyMode,
    headers: &str,
) -> Result<HttpRequest, HttpError> {
    let agent = wide_null(USER_AGENT);
    // SAFETY: All pointers reference null-terminated UTF-16 buffers for the duration of each
    // WinHTTP call. Every returned HINTERNET is wrapped immediately and closed by Drop.
    let session = unsafe { WinHttpOpen(agent.as_ptr(), mode.access_type(), null(), null(), 0) };
    let session = HttpHandle::new(session, mode)?;
    let secure_protocols = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
    // SAFETY: `session` is live and the option buffer points to a correctly sized u32.
    if unsafe {
        WinHttpSetOption(
            session.0,
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            (&secure_protocols as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    } == 0
    {
        return Err(transport_error(mode));
    }
    // SAFETY: `session` is a valid handle and timeout values are finite milliseconds.
    if unsafe {
        WinHttpSetTimeouts(
            session.0,
            RESOLVE_TIMEOUT_MS,
            CONNECT_TIMEOUT_MS,
            SEND_TIMEOUT_MS,
            RECEIVE_TIMEOUT_MS,
        )
    } == 0
    {
        return Err(transport_error(mode));
    }
    // SAFETY: `session` is a live WinHTTP session and `parsed.host` is null-terminated.
    let connection = unsafe { WinHttpConnect(session.0, parsed.host.as_ptr(), parsed.port, 0) };
    let connection = HttpHandle::new(connection, mode)?;
    let verb = wide_null("GET");
    // SAFETY: The session/connection handles are live; all optional pointer arguments are null,
    // and the verb/object buffers stay alive until the call returns.
    let request = unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            parsed.object.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        )
    };
    let request = HttpHandle::new(request, mode)?;
    let headers = wide_null(headers);
    // SAFETY: `request` is live and `headers` is a null-terminated UTF-16 buffer.
    if unsafe {
        WinHttpAddRequestHeaders(
            request.0,
            headers.as_ptr(),
            u32::MAX,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        )
    } == 0
    {
        return Err(transport_error(mode));
    }
    // SAFETY: `request` is a live request with no optional body.
    if unsafe { WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) } == 0 {
        return Err(transport_error(mode));
    }
    // SAFETY: `request` has been sent and the reserved argument is null as required.
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(transport_error(mode));
    }
    Ok(HttpRequest {
        request,
        _connection: connection,
        _session: session,
    })
}

struct HttpHandle(*mut c_void);

impl HttpHandle {
    fn new(handle: *mut c_void, mode: ProxyMode) -> Result<Self, HttpError> {
        if handle.is_null() {
            Err(transport_error(mode))
        } else {
            Ok(Self(handle))
        }
    }
}

struct HttpRequest {
    request: HttpHandle,
    _connection: HttpHandle,
    _session: HttpHandle,
}

impl HttpRequest {
    fn status_code(&self, mode: ProxyMode) -> Result<u32, HttpError> {
        let mut status = 0_u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: `self` is a live request handle; output points to a correctly sized u32.
        if unsafe {
            WinHttpQueryHeaders(
                self.request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut size,
                null_mut(),
            )
        } == 0
        {
            return Err(transport_error(mode));
        }
        Ok(status)
    }

    fn read_chunks(
        &self,
        mode: ProxyMode,
        mut consume: impl FnMut(&[u8]) -> Result<(), HttpError>,
    ) -> Result<(), HttpError> {
        let mut buffer = vec![0_u8; READ_BUFFER_SIZE];
        loop {
            let mut available = 0_u32;
            // SAFETY: `self` is a live request and `available` is writable.
            if unsafe { WinHttpQueryDataAvailable(self.request.0, &mut available) } == 0 {
                return Err(transport_error(mode));
            }
            if available == 0 {
                return Ok(());
            }
            let to_read = available.min(buffer.len() as u32);
            let mut read = 0_u32;
            // SAFETY: The output buffer has at least `to_read` bytes and `read` is writable.
            if unsafe {
                WinHttpReadData(
                    self.request.0,
                    buffer.as_mut_ptr().cast(),
                    to_read,
                    &mut read,
                )
            } == 0
            {
                return Err(transport_error(mode));
            }
            if read == 0 {
                return Ok(());
            }
            consume(&buffer[..read as usize])?;
        }
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        // SAFETY: The wrapper owns this non-null HINTERNET and closes it exactly once.
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

struct ParsedHttpsUrl {
    host: Vec<u16>,
    object: Vec<u16>,
    port: u16,
}

impl ParsedHttpsUrl {
    fn parse(url: &str) -> Result<Self, HttpError> {
        let wide = wide_null(url);
        let mut components = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwSchemeLength: u32::MAX,
            dwHostNameLength: u32::MAX,
            dwUserNameLength: u32::MAX,
            dwPasswordLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..URL_COMPONENTS::default()
        };
        // SAFETY: `wide` is a null-terminated URL buffer and `components` is correctly sized.
        if unsafe { WinHttpCrackUrl(wide.as_ptr(), 0, 0, &mut components) } == 0 {
            return Err(HttpError::InvalidUrl(
                io::Error::last_os_error().to_string(),
            ));
        }
        if components.nScheme != WINHTTP_INTERNET_SCHEME_HTTPS {
            return Err(HttpError::InvalidUrl(
                "only HTTPS update URLs are accepted".to_owned(),
            ));
        }
        if components.dwUserNameLength != 0 || components.dwPasswordLength != 0 {
            return Err(HttpError::InvalidUrl(
                "embedded URL credentials are rejected".to_owned(),
            ));
        }
        let host = unsafe {
            wide_component(components.lpszHostName, components.dwHostNameLength, "host")?
        };
        let mut object =
            unsafe { wide_component(components.lpszUrlPath, components.dwUrlPathLength, "path")? };
        let extra = unsafe {
            wide_component(
                components.lpszExtraInfo,
                components.dwExtraInfoLength,
                "query",
            )?
        };
        if object.is_empty() {
            object.push('/' as u16);
        }
        object.extend(extra);
        let mut host = host;
        host.push(0);
        object.push(0);
        Ok(Self {
            host,
            object,
            port: components.nPort,
        })
    }
}

unsafe fn wide_component(
    pointer: *mut u16,
    length: u32,
    name: &str,
) -> Result<Vec<u16>, HttpError> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null() || length == u32::MAX {
        return Err(HttpError::InvalidUrl(format!("URL {name} is invalid")));
    }
    // SAFETY: WinHttpCrackUrl returned this pointer/length pair into the caller-owned URL buffer.
    Ok(unsafe { std::slice::from_raw_parts(pointer, length as usize) }.to_vec())
}

fn wide_null(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn transport_error(mode: ProxyMode) -> HttpError {
    HttpError::Transport {
        mode: mode.label(),
        source: io::Error::last_os_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Networking::WinHttp::{
        ERROR_WINHTTP_AUTODETECTION_FAILED, ERROR_WINHTTP_CANNOT_CONNECT,
        ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED, ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED_PROXY,
        ERROR_WINHTTP_LOGIN_FAILURE, ERROR_WINHTTP_NAME_NOT_RESOLVED,
        ERROR_WINHTTP_SECURE_CERT_CN_INVALID, ERROR_WINHTTP_SECURE_FAILURE_PROXY,
        ERROR_WINHTTP_TIMEOUT,
    };

    fn winhttp_transport_error(mode: ProxyMode, code: u32) -> HttpError {
        HttpError::Transport {
            mode: mode.label(),
            source: io::Error::from_raw_os_error(code as i32),
        }
    }

    #[test]
    fn parses_https_url_with_query() {
        let parsed = ParsedHttpsUrl::parse(
            "https://updates.example.test:8443/channel/latest.json?platform=windows",
        )
        .unwrap();

        assert_eq!(parsed.port, 8443);
        assert_eq!(
            String::from_utf16(&parsed.host[..parsed.host.len() - 1]).unwrap(),
            "updates.example.test"
        );
        assert_eq!(
            String::from_utf16(&parsed.object[..parsed.object.len() - 1]).unwrap(),
            "/channel/latest.json?platform=windows"
        );
    }

    #[test]
    fn rejects_non_https_url() {
        assert!(ParsedHttpsUrl::parse("http://updates.example.test/latest.json").is_err());
    }

    #[test]
    fn oversized_package_error_identifies_manifest_mismatch() {
        let error = HttpError::PackageLargerThanManifest {
            expected: 12_937_599,
            received_at_least: 12_976_128,
        };

        assert_eq!(
            error.to_string(),
            "update package is larger than the signed manifest: manifest declares 12937599 bytes, received at least 12976128 bytes"
        );
    }

    #[test]
    fn short_package_error_identifies_manifest_mismatch() {
        let error = HttpError::SizeMismatch {
            expected: 12_937_599,
            actual: 12_000_000,
        };

        assert_eq!(
            error.to_string(),
            "update package size does not match the signed manifest: manifest declares 12937599 bytes, received 12000000 bytes"
        );
    }

    #[test]
    fn transport_failure_can_switch_to_the_next_route() {
        let mut attempts = Vec::new();

        let result = execute_with_proxy_routes(|mode| {
            attempts.push(mode.label());
            if attempts.len() == 1 {
                Err(HttpError::Transport {
                    mode: mode.label(),
                    source: io::Error::new(io::ErrorKind::TimedOut, "timed out"),
                })
            } else {
                Ok("downloaded")
            }
        });

        assert_eq!(result.unwrap(), "downloaded");
        assert_eq!(
            attempts,
            vec!["Windows automatic proxy", "WinHTTP default proxy"]
        );
    }

    #[test]
    fn explicitly_route_recoverable_transport_failures_can_switch_routes() {
        for code in [
            ERROR_WINHTTP_AUTODETECTION_FAILED,
            ERROR_WINHTTP_NAME_NOT_RESOLVED,
            ERROR_WINHTTP_CANNOT_CONNECT,
            ERROR_WINHTTP_TIMEOUT,
        ] {
            let mut attempts = Vec::new();

            let result = execute_with_proxy_routes(|mode| {
                attempts.push(mode.label());
                if attempts.len() == 1 {
                    Err(winhttp_transport_error(mode, code))
                } else {
                    Ok("downloaded")
                }
            });

            assert_eq!(result.unwrap(), "downloaded");
            assert_eq!(
                attempts,
                vec!["Windows automatic proxy", "WinHTTP default proxy"]
            );
        }
    }

    #[test]
    fn enterprise_proxy_certificate_and_authentication_transport_failures_are_terminal() {
        for code in [
            ERROR_WINHTTP_SECURE_FAILURE_PROXY,
            ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED_PROXY,
            ERROR_WINHTTP_SECURE_CERT_CN_INVALID,
            ERROR_WINHTTP_CLIENT_AUTH_CERT_NEEDED,
            ERROR_WINHTTP_LOGIN_FAILURE,
        ] {
            let mut attempts = Vec::new();

            let error = execute_with_proxy_routes::<()>(|mode| {
                attempts.push(mode.label());
                Err(winhttp_transport_error(mode, code))
            })
            .unwrap_err();

            assert!(matches!(error, HttpError::Transport { .. }));
            assert_eq!(attempts, vec!["Windows automatic proxy"]);
        }
    }

    #[test]
    fn unclassified_transport_failure_is_terminal() {
        let mut attempts = Vec::new();

        let error = execute_with_proxy_routes::<()>(|mode| {
            attempts.push(mode.label());
            Err(winhttp_transport_error(mode, 12_345))
        })
        .unwrap_err();

        assert!(matches!(error, HttpError::Transport { .. }));
        assert_eq!(attempts, vec!["Windows automatic proxy"]);
    }

    #[test]
    fn application_4xx_statuses_are_terminal() {
        for status in [401, 403, 404] {
            let mut attempts = Vec::new();

            let error = execute_with_proxy_routes::<()>(|mode| {
                attempts.push(mode.label());
                Err(HttpError::Status(status))
            })
            .unwrap_err();

            assert!(matches!(error, HttpError::Status(actual) if actual == status));
            assert_eq!(attempts, vec!["Windows automatic proxy"]);
        }
    }

    #[test]
    fn proxy_authentication_status_is_terminal_on_the_first_route() {
        let mut attempts = Vec::new();

        let error = execute_with_proxy_routes::<()>(|mode| {
            attempts.push(mode.label());
            Err(HttpError::Status(407))
        })
        .unwrap_err();

        assert!(matches!(error, HttpError::Status(407)));
        assert_eq!(attempts, vec!["Windows automatic proxy"]);
    }

    #[test]
    fn retryable_gateway_statuses_retry_only_the_same_route() {
        for status in [502, 503, 504] {
            let mut attempts = Vec::new();

            let error = execute_with_proxy_routes::<()>(|mode| {
                attempts.push(mode.label());
                Err(HttpError::Status(status))
            })
            .unwrap_err();

            assert!(matches!(error, HttpError::Status(actual) if actual == status));
            assert_eq!(
                attempts,
                vec![
                    "Windows automatic proxy",
                    "Windows automatic proxy",
                    "Windows automatic proxy"
                ]
            );
        }
    }

    #[test]
    fn gateway_502_can_succeed_on_the_second_same_route_attempt() {
        let mut attempts = Vec::new();

        let result = execute_with_proxy_routes(|mode| {
            attempts.push(mode.label());
            if attempts.len() == 1 {
                Err(HttpError::Status(502))
            } else {
                Ok("downloaded")
            }
        });

        assert_eq!(result.unwrap(), "downloaded");
        assert_eq!(
            attempts,
            vec!["Windows automatic proxy", "Windows automatic proxy"]
        );
    }

    #[test]
    fn gateway_502_can_succeed_on_the_third_same_route_attempt() {
        let mut attempts = Vec::new();

        let result = execute_with_proxy_routes(|mode| {
            attempts.push(mode.label());
            if attempts.len() < 3 {
                Err(HttpError::Status(502))
            } else {
                Ok("downloaded")
            }
        });

        assert_eq!(result.unwrap(), "downloaded");
        assert_eq!(
            attempts,
            vec![
                "Windows automatic proxy",
                "Windows automatic proxy",
                "Windows automatic proxy"
            ]
        );
    }

    #[test]
    fn only_whitelisted_5xx_statuses_are_retried() {
        for status in [500, 501, 505] {
            let mut attempts = Vec::new();

            let error = execute_with_proxy_routes::<()>(|mode| {
                attempts.push(mode.label());
                Err(HttpError::Status(status))
            })
            .unwrap_err();

            assert!(matches!(error, HttpError::Status(actual) if actual == status));
            assert_eq!(attempts, vec!["Windows automatic proxy"]);
        }
    }
}
