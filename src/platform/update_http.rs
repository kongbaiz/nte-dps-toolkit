use std::ffi::c_void;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
    WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPTION_SECURE_PROTOCOLS, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect,
    WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetOption, WinHttpSetTimeouts,
};

const USER_AGENT: &str = concat!("NTE-DPS-Tool-Updater/", env!("CARGO_PKG_VERSION"));
const RESOLVE_TIMEOUT_MS: i32 = 15_000;
const CONNECT_TIMEOUT_MS: i32 = 15_000;
const SEND_TIMEOUT_MS: i32 = 30_000;
const RECEIVE_TIMEOUT_MS: i32 = 30_000;
const READ_BUFFER_SIZE: usize = 64 * 1024;

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
    ResponseTooLarge,
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
            Self::ResponseTooLarge => formatter.write_str("update response exceeds the size limit"),
            Self::SizeMismatch { expected, actual } => write!(
                formatter,
                "download size mismatch: expected {expected} bytes, received {actual} bytes"
            ),
            Self::File(error) => write!(formatter, "update file operation failed: {error}"),
        }
    }
}

impl std::error::Error for HttpError {}

pub fn get_bytes(url: &str, maximum_size: usize) -> Result<Vec<u8>, HttpError> {
    let parsed = ParsedHttpsUrl::parse(url)?;
    let mut last_transport_error = None;
    for mode in ProxyMode::ALL {
        match get_bytes_once(&parsed, mode, maximum_size) {
            Ok(bytes) => return Ok(bytes),
            Err(error @ HttpError::Transport { .. }) => last_transport_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_transport_error.expect("proxy mode list is not empty"))
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
    let mut last_transport_error = None;
    for mode in ProxyMode::ALL {
        match download_file_once(&parsed, mode, destination, expected_size, &mut progress) {
            Ok(()) => return Ok(()),
            Err(error @ HttpError::Transport { .. }) => last_transport_error = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_transport_error.expect("proxy mode list is not empty"))
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
        let next_length = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or(HttpError::ResponseTooLarge)?;
        if next_length > maximum_size {
            return Err(HttpError::ResponseTooLarge);
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
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or(HttpError::ResponseTooLarge)?;
        if downloaded > expected_size {
            return Err(HttpError::ResponseTooLarge);
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
}
