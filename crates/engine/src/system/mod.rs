//! What the running system knows about user profiles (which profiles exist
//! and where each keeps its standard folders) and about the drives a backup
//! could be written to.

use std::path::{Path, PathBuf};

use profileport_package::KnownFolder;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as imp;
#[cfg(windows)]
use windows as imp;

#[cfg(unix)]
pub use unix::{parse_mounts, parse_user_dirs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInfo {
    /// Account / folder name.
    pub name: String,
    pub path: PathBuf,
    /// Windows security identifier, when known.
    pub sid: Option<String>,
    /// The profile of the user running ProfilePort.
    pub current: bool,
}

/// User profiles on this system (not system or service accounts).
pub fn list_profiles() -> Vec<ProfileInfo> {
    imp::list_profiles()
}

/// Where the profile at `profile` keeps its standard folders. Folders that
/// can't be determined fall back to the platform's default names. Only
/// folders that exist are returned.
pub fn known_folders(profile: &Path) -> Vec<(KnownFolder, PathBuf)> {
    let mut out: Vec<(KnownFolder, PathBuf)> = imp::known_folders(profile);
    for kf in KnownFolder::ALL {
        if !out.iter().any(|(k, _)| *k == kf) {
            if let Some(name) = default_name(kf) {
                out.push((kf, profile.join(name)));
            }
        }
    }
    out.retain(|(_, p)| p.is_dir());
    out.sort();
    out
}

/// A drive (volume) that a backup could be written to or read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    /// Its root: `E:\`, `/media/anna/KINGSTON`, `/Volumes/KINGSTON`.
    pub path: PathBuf,
    /// The volume label, or the mount folder's name; may be empty.
    pub label: String,
    /// The system reports it as removable media (USB stick, card). External
    /// USB hard drives usually report as fixed.
    pub removable: bool,
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}

/// Drives other than the system drive: USB sticks, external and secondary
/// disks. Removable drives come first. Folders listed in the
/// `PROFILEPORT_DRIVES` environment variable (separated like `PATH`) are
/// added, for test machines whose transfer disk is not detected as a drive.
pub fn list_drives() -> Vec<DriveInfo> {
    let mut out = imp::list_drives();
    if let Some(extra) = std::env::var_os("PROFILEPORT_DRIVES") {
        for path in std::env::split_paths(&extra).filter(|p| p.is_dir()) {
            if !out.iter().any(|d| d.path == path) {
                let (free_bytes, total_bytes) = imp::space(&path);
                let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                out.push(DriveInfo { path, label, removable: false, free_bytes, total_bytes });
            }
        }
    }
    out.sort_by_key(|d| !d.removable);
    out
}

/// "windows", "linux", "macos", …
pub fn os_name() -> &'static str {
    std::env::consts::OS
}

/// Default location of a known folder relative to the profile, if the
/// platform has one.
pub fn default_name(kf: KnownFolder) -> Option<&'static str> {
    use KnownFolder::*;
    if cfg!(windows) {
        Some(match kf {
            Desktop => "Desktop",
            Documents => "Documents",
            Downloads => "Downloads",
            Music => "Music",
            Pictures => "Pictures",
            Videos => "Videos",
            Templates => "AppData/Roaming/Microsoft/Windows/Templates",
            Public => return None, // C:\Users\Public is shared, not per-user
        })
    } else if cfg!(target_os = "macos") {
        Some(match kf {
            Desktop => "Desktop",
            Documents => "Documents",
            Downloads => "Downloads",
            Music => "Music",
            Pictures => "Pictures",
            Videos => "Movies",
            Public => "Public",
            Templates => return None,
        })
    } else {
        Some(match kf {
            Desktop => "Desktop",
            Documents => "Documents",
            Downloads => "Downloads",
            Music => "Music",
            Pictures => "Pictures",
            Videos => "Videos",
            Templates => "Templates",
            Public => "Public",
        })
    }
}
