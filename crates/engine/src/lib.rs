//! ProfilePort migration engine: export a profile folder into a package,
//! resume interrupted exports, verify packages and restore them.
//!
//! The engine is GUI-independent; the CLI and the desktop app both use it.

mod exclude;
mod export;
mod platform;
mod restore;

use std::path::{Path, PathBuf};

pub use exclude::{Exclusions, DEFAULT_EXCLUSIONS};
pub use export::{
    export, prepare_export, resume_export, ExportError, ExportOptions, ExportReport, PreparedExport, SourceLabels,
};
pub use profileport_package as package;
pub use restore::{
    restore, verify, ConflictOutcome, ConflictPolicy, RestoreError, RestoreOptions, RestoreReport, VerifyReport,
};

use profileport_package::SkipReason;

#[derive(Debug, Clone, Default)]
pub struct Progress {
    pub files: u64,
    pub bytes: u64,
    pub current: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkipItem {
    pub path: String,
    pub reason: SkipReason,
    pub detail: String,
}

/// Where packages live on a ProfilePort drive.
pub fn transfers_dir(drive_root: &Path) -> PathBuf {
    drive_root.join("ProfilePort").join("transfers")
}

/// Convert a validated '/'-separated package path to a native path.
pub(crate) fn native_path(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for c in rel.split('/') {
        p.push(c);
    }
    p
}
