//! What the running system knows about user profiles: which profiles exist
//! and where each keeps its standard folders.

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
pub use unix::parse_user_dirs;

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
