//! ProfilePort migration engine: export a profile folder into a package,
//! resume interrupted exports, verify packages and restore them.
//!
//! The engine is GUI-independent; the CLI and the desktop app both use it.

mod exclude;
mod export;
mod folders;
mod names;
mod platform;
mod restore;
pub mod system;
#[cfg(windows)]
mod win;

use std::path::{Path, PathBuf};

pub use exclude::{Exclusions, DEFAULT_EXCLUSIONS};
pub use export::{
    export, prepare_export, resume_export, ExportError, ExportOptions, ExportReport, PreparedExport, SourceLabels,
};
pub use folders::is_cloud_synced;
pub use names::NameRules;
pub use profileport_package as package;
pub use restore::{
    restore, verify, ConflictOutcome, ConflictPolicy, RenameReason, RestoreError, RestoreOptions, RestoreReport,
    VerifyReport,
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
