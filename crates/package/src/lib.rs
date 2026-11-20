//! ProfilePort transfer package format (see `docs/decisions/0002-package-format.md`
//! and `docs/package-format.md`).
//!
//! A package is a directory holding an encrypted, chunked copy of a user
//! profile. It is unlocked with a [`TransferCode`]; without the code only
//! the ticket number, creation time and progress counters are readable.

pub mod code;
mod crypto;
mod error;
pub mod format;
mod log;
mod reader;
mod writer;

pub use code::{CodeError, TransferCode};
pub use crypto::KdfParams;
pub use error::{Error, Result};
pub use format::{
    BeginInfo, ChunkRef, Header, KnownFolder, KnownFolderEntry, Meta, PackageState, SkipReason, Status, Timestamp,
};
pub use reader::{list_packages, read_info, Entry, FileEntry, Manifest, PackageInfo, PackageReader};
pub use writer::{FileResult, PackageWriter, PartialFile, ResumeState, WriterOptions};
