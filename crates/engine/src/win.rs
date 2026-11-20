//! Windows-only file metadata: attributes, alternate data streams.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{GetLastError, ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard, GetFileAttributesW, SetFileAttributesW,
    FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM, INVALID_FILE_ATTRIBUTES,
    WIN32_FIND_STREAM_DATA,
};

/// `\\?\` form, so paths longer than 260 characters work with raw Win32.
pub(crate) fn verbatim(path: &Path) -> PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if s.starts_with(r"\\?\") {
        path.to_path_buf()
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{unc}"))
    } else if path.is_absolute() {
        PathBuf::from(format!(r"\\?\{s}"))
    } else {
        path.to_path_buf()
    }
}

fn wide(path: &Path) -> Vec<u16> {
    verbatim(path).as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Set or clear Hidden, System and Read-only, leaving other attributes.
pub(crate) fn set_attributes(path: &Path, hidden: bool, system: bool, readonly: bool) -> io::Result<()> {
    let w = wide(path);
    // SAFETY: NUL-terminated path.
    let mut attrs = unsafe { GetFileAttributesW(w.as_ptr()) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    for (bit, on) in
        [(FILE_ATTRIBUTE_HIDDEN, hidden), (FILE_ATTRIBUTE_SYSTEM, system), (FILE_ATTRIBUTE_READONLY, readonly)]
    {
        if on {
            attrs |= bit
        } else {
            attrs &= !bit
        }
    }
    // SAFETY: NUL-terminated path.
    if unsafe { SetFileAttributesW(w.as_ptr(), attrs) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) const HIDDEN: u32 = FILE_ATTRIBUTE_HIDDEN;
pub(crate) const SYSTEM: u32 = FILE_ATTRIBUTE_SYSTEM;

/// Named alternate data streams of a file, with sizes. The unnamed main
/// stream (`::$DATA`) is not included.
pub(crate) fn alt_streams(path: &Path) -> io::Result<Vec<(String, u64)>> {
    let w = wide(path);
    let mut data = WIN32_FIND_STREAM_DATA::default();
    // SAFETY: `data` is a WIN32_FIND_STREAM_DATA for FindStreamInfoStandard.
    let handle = unsafe {
        FindFirstStreamW(w.as_ptr(), FindStreamInfoStandard, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(), 0)
    };
    if handle == INVALID_HANDLE_VALUE {
        // SAFETY: plain FFI call.
        return if unsafe { GetLastError() } == ERROR_HANDLE_EOF {
            Ok(Vec::new())
        } else {
            Err(io::Error::last_os_error())
        };
    }
    let mut out = Vec::new();
    loop {
        let len = data.cStreamName.iter().position(|&c| c == 0).unwrap_or(data.cStreamName.len());
        let raw = String::from_utf16_lossy(&data.cStreamName[..len]); // ":name:$DATA"
        if let Some(name) = raw.strip_prefix(':').and_then(|r| r.strip_suffix(":$DATA")) {
            if !name.is_empty() {
                out.push((name.to_string(), data.StreamSize.max(0) as u64));
            }
        }
        // SAFETY: valid find handle and buffer.
        if unsafe { FindNextStreamW(handle, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast()) } == 0 {
            break;
        }
    }
    // SAFETY: valid find handle.
    unsafe { FindClose(handle) };
    Ok(out)
}

fn stream_path(path: &Path, name: &str) -> PathBuf {
    let mut s: OsString = verbatim(path).into_os_string();
    s.push(":");
    s.push(name);
    PathBuf::from(s)
}

pub(crate) fn read_stream(path: &Path, name: &str) -> io::Result<Vec<u8>> {
    std::fs::read(stream_path(path, name))
}

pub(crate) fn write_stream(path: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    std::fs::write(stream_path(path, name), data)
}
