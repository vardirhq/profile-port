//! On-disk layout and record types (format version 1).
//!
//! ```text
//! <package>/
//! ├── header.bin    magic ‖ version ‖ CBOR(Header)          plaintext
//! ├── status.cbor   CBOR(Status)                             plaintext
//! ├── manifest.log  frames: u32 LE len ‖ seal(CBOR(Record))  encrypted
//! └── packs/NNNNNN.pack  concatenated seal(chunk)            encrypted
//! ```
//!
//! Plaintext is limited to what the dashboard needs without a code: ticket
//! number, creation time, progress counters and state.

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::crypto::KdfParams;
use crate::error::{Error, Result};

pub const MAGIC: &[u8; 8] = b"PPORTPKG";
pub const FORMAT_VERSION: u16 = 1;

pub const HEADER_FILE: &str = "header.bin";
pub const STATUS_FILE: &str = "status.cbor";
pub const LOG_FILE: &str = "manifest.log";
pub const PACKS_DIR: &str = "packs";

pub(crate) fn pack_file_name(index: u32) -> String {
    format!("{index:06}.pack")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub package_id: [u8; 16],
    pub ticket: String,
    pub created_unix: i64,
    pub kdf: KdfParams,
    /// Package key sealed under the key derived from the transfer code.
    pub(crate) wrapped_key: Vec<u8>,
    pub chunk_size: u32,
}

impl Header {
    /// Binds the plaintext label fields to the key: swapping a ticket number
    /// between packages makes the code fail instead of mislabelling data.
    pub(crate) fn key_aad(&self) -> Vec<u8> {
        let mut aad = b"profileport header v1".to_vec();
        aad.extend_from_slice(&self.package_id);
        aad.extend_from_slice(&self.created_unix.to_le_bytes());
        aad.extend_from_slice(&self.chunk_size.to_le_bytes());
        aad.extend_from_slice(self.ticket.as_bytes());
        aad
    }

    pub fn package_id_hex(&self) -> String {
        self.package_id.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageState {
    /// Being written, or interrupted (no way to tell from the drive alone).
    InProgress,
    Complete,
    /// Complete, but some files were skipped or changed while copying.
    CompleteWithIssues,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub state: PackageState,
    pub files: u64,
    pub bytes: u64,
    pub skipped: u64,
    pub updated_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub modified: Option<Timestamp>,
    pub created: Option<Timestamp>,
    pub readonly: bool,
    /// Raw Windows file attributes, when captured on Windows.
    pub win_attributes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub pack: u32,
    pub offset: u64,
    pub stored_len: u32,
    pub plain_len: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    /// Matched an exclusion rule (caches, temp files).
    Excluded,
    PermissionDenied,
    /// Locked by another process.
    Locked,
    /// Disappeared between listing and reading.
    Vanished,
    /// Name can't be represented (not valid Unicode).
    InvalidName,
    /// OneDrive / cloud placeholder that isn't stored locally.
    CloudOnly,
    ReadError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginInfo {
    /// Absolute path of the source root, used to resume.
    pub source_root: String,
    pub computer: String,
    pub profile: String,
    pub started_unix: i64,
    pub tool_version: String,
}

/// One manifest entry. Paths are relative, '/'-separated, and are validated
/// again on restore (packages are untrusted input).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Record {
    Begin(BeginInfo),
    Dir {
        path: String,
        meta: Meta,
    },
    FileStart {
        id: u64,
        path: String,
        meta: Meta,
        size: u64,
    },
    Chunks {
        id: u64,
        chunks: Vec<ChunkRef>,
    },
    FileEnd {
        id: u64,
        blake3: [u8; 32],
        changed_during_copy: bool,
    },
    /// Reading the source failed part-way; the file is not in the package.
    FileAbort {
        id: u64,
    },
    Link {
        path: String,
        target: String,
        meta: Meta,
    },
    Skipped {
        path: String,
        reason: SkipReason,
        detail: String,
    },
    End {
        finished_unix: i64,
    },
}

pub(crate) fn chunk_aad(package_id: &[u8; 16], file_id: u64, index: u64) -> Vec<u8> {
    let mut aad = b"profileport chunk v1".to_vec();
    aad.extend_from_slice(package_id);
    aad.extend_from_slice(&file_id.to_le_bytes());
    aad.extend_from_slice(&index.to_le_bytes());
    aad
}

pub(crate) fn to_cbor<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(value, &mut out).expect("CBOR encoding to memory cannot fail");
    out
}

pub(crate) fn from_cbor<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    ciborium::from_reader(bytes).map_err(|e| Error::Corrupt(format!("undecodable data: {e}")))
}

pub(crate) fn encode_header(header: &Header) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&to_cbor(header));
    out
}

pub fn read_header(dir: &Path) -> Result<Header> {
    let bytes = match fs::read(dir.join(HEADER_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Error::NotAPackage),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() < 10 || &bytes[..8] != MAGIC {
        return Err(Error::NotAPackage);
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != FORMAT_VERSION {
        return Err(Error::UnsupportedVersion(version));
    }
    from_cbor(&bytes[10..])
}

pub fn read_status(dir: &Path) -> Result<Option<Status>> {
    match fs::read(dir.join(STATUS_FILE)) {
        Ok(b) => from_cbor(&b).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Write via a temporary file and rename, so a pulled drive leaves either
/// the old or the new version, never a torn one.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Ticket numbers are plaintext labels: keep them short and boring.
pub fn validate_ticket(ticket: &str) -> Result<()> {
    let ok = !ticket.is_empty()
        && ticket.len() <= 32
        && ticket.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidTicket(ticket.to_string()))
    }
}

/// Validate a relative package path and split it into components.
///
/// Rejects anything that could escape the restore root or address something
/// other than a plain file name: absolute paths, `..`, `.`, empty
/// components, backslashes, drive letters / stream names (`:`), and control
/// characters.
pub fn split_rel_path(path: &str) -> Result<Vec<&str>> {
    let bad = || Error::UnsafePath(path.to_string());
    if path.is_empty() || path.starts_with('/') {
        return Err(bad());
    }
    let parts: Vec<&str> = path.split('/').collect();
    for p in &parts {
        if p.is_empty() || *p == "." || *p == ".." || p.chars().any(|c| c == '\\' || c == ':' || (c as u32) < 0x20) {
            return Err(bad());
        }
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_validation() {
        for ok in ["a", "Documents/report.docx", "Música/日本/x y.txt", ".hidden/file"] {
            assert!(split_rel_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a//b",
            "a/./b",
            "C:/x",
            "a\\..\\b",
            "file.txt:stream",
            "a/\u{0}b",
            "a/",
        ] {
            assert!(split_rel_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ticket_validation() {
        assert!(validate_ticket("48231").is_ok());
        assert!(validate_ticket("JOB-2024_17").is_ok());
        for bad in ["", "a b", "../x", "x".repeat(33).as_str(), "Ünïcode"] {
            assert!(validate_ticket(bad).is_err(), "{bad:?}");
        }
    }
}
