//! Linux / Unix: profiles from /etc/passwd (macOS: /Users), folders from
//! XDG user-dirs (macOS uses fixed English folder names; Finder translates
//! them for display only).

use std::fs;
use std::path::{Path, PathBuf};

use profileport_package::KnownFolder;

use super::{DriveInfo, ProfileInfo};

/// macOS keeps people in Directory Services, not /etc/passwd; their homes
/// are the folders in /Users (except the shared and guest ones).
#[cfg(target_os = "macos")]
pub(super) fn list_profiles() -> Vec<ProfileInfo> {
    let current = std::env::var_os("HOME").map(PathBuf::from);
    let Ok(entries) = fs::read_dir("/Users") else { return Vec::new() };
    let mut out: Vec<ProfileInfo> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if name.starts_with('.') || name == "Shared" || name == "Guest" {
                return None;
            }
            let path = e.path();
            Some(ProfileInfo { current: current.as_deref() == Some(path.as_path()), name, path, sid: None })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(not(target_os = "macos"))]
pub(super) fn list_profiles() -> Vec<ProfileInfo> {
    let current = std::env::var_os("HOME").map(PathBuf::from);
    let Ok(passwd) = fs::read_to_string("/etc/passwd") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in passwd.lines() {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 7 {
            continue;
        }
        let uid: u32 = f[2].parse().unwrap_or(0);
        let shell = f[6];
        // Regular accounts: uid ≥ 1000, not `nobody`, with a login shell.
        if uid < 1000 || uid == 65534 || shell.ends_with("nologin") || shell.ends_with("/false") {
            continue;
        }
        let home = PathBuf::from(f[5]);
        if !home.is_dir() {
            continue;
        }
        out.push(ProfileInfo {
            name: f[0].to_string(),
            current: current.as_deref() == Some(home.as_path()),
            path: home,
            sid: None,
        });
    }
    out
}

pub(super) fn known_folders(profile: &Path) -> Vec<(KnownFolder, PathBuf)> {
    match fs::read_to_string(profile.join(".config/user-dirs.dirs")) {
        Ok(content) => parse_user_dirs(&content, profile),
        Err(_) => Vec::new(),
    }
}

/// Parse `~/.config/user-dirs.dirs` (as written by `xdg-user-dirs-update`),
/// which holds the user's — possibly translated — folder names:
///
/// ```text
/// XDG_DOCUMENTS_DIR="$HOME/Dokumenter"
/// ```
///
/// A folder set to `$HOME` itself means "disabled" and is left out.
pub fn parse_user_dirs(content: &str, home: &Path) -> Vec<(KnownFolder, PathBuf)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let kf = match key.trim() {
            "XDG_DESKTOP_DIR" => KnownFolder::Desktop,
            "XDG_DOCUMENTS_DIR" => KnownFolder::Documents,
            "XDG_DOWNLOAD_DIR" => KnownFolder::Downloads,
            "XDG_MUSIC_DIR" => KnownFolder::Music,
            "XDG_PICTURES_DIR" => KnownFolder::Pictures,
            "XDG_VIDEOS_DIR" => KnownFolder::Videos,
            "XDG_TEMPLATES_DIR" => KnownFolder::Templates,
            "XDG_PUBLICSHARE_DIR" => KnownFolder::Public,
            _ => continue,
        };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else { continue };
        let value = value.replace("\\\"", "\"");
        let path = if let Some(rest) = value.strip_prefix("$HOME") {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                continue; // disabled
            }
            home.join(rest)
        } else if value.starts_with('/') {
            PathBuf::from(value)
        } else {
            continue; // not valid per the spec
        };
        out.push((kf, path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_translated_and_absolute_and_disabled_dirs() {
        let content = r#"
# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR="$HOME/Skrivebord"
XDG_DOWNLOAD_DIR="$HOME/Nedlastinger"
XDG_TEMPLATES_DIR="$HOME/"
XDG_PUBLICSHARE_DIR="$HOME"
XDG_DOCUMENTS_DIR="$HOME/Dokumenter"
XDG_MUSIC_DIR="/mnt/data/Musikk"
XDG_PICTURES_DIR="$HOME/Bilder"
XDG_VIDEOS_DIR="$HOME/Videoer"
XDG_BOGUS_DIR="$HOME/x"
"#;
        let home = Path::new("/home/anna");
        let mut got = parse_user_dirs(content, home);
        got.sort();
        assert_eq!(
            got,
            vec![
                (KnownFolder::Desktop, home.join("Skrivebord")),
                (KnownFolder::Documents, home.join("Dokumenter")),
                (KnownFolder::Downloads, home.join("Nedlastinger")),
                (KnownFolder::Music, PathBuf::from("/mnt/data/Musikk")),
                (KnownFolder::Pictures, home.join("Bilder")),
                (KnownFolder::Videos, home.join("Videoer")),
            ]
        );
    }
}

/// Mounted volumes under the usual places for removable and extra disks:
/// `/media`, `/run/media` and `/mnt` on Linux, `/Volumes` on macOS (except
/// the start-up disk, which is a link to `/`).
pub(super) fn list_drives() -> Vec<DriveInfo> {
    let mounts: Vec<PathBuf> = if cfg!(target_os = "macos") {
        fs::read_dir("/Volumes")
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        fs::read_to_string("/proc/self/mounts").map(|m| parse_mounts(&m)).unwrap_or_default()
    };
    let mut out: Vec<DriveInfo> = mounts
        .into_iter()
        .map(|path| {
            let (free_bytes, total_bytes) = space(&path);
            let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // Desktop environments mount removable media under /media and
            // /run/media; macOS mounts every extra volume under /Volumes.
            let removable = !path.starts_with("/mnt");
            DriveInfo { path, label, removable, free_bytes, total_bytes }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Mount points where desktops and people put extra drives, from the contents
/// of `/proc/self/mounts`, with its octal escapes (`\040` for a space)
/// decoded: `/media/<label>`, `/media/<user>/<label>`,
/// `/run/media/<user>/<label>` and `/mnt/<label>`. Deeper mounts are
/// containers' and tools' internals, not drives.
pub fn parse_mounts(content: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = content
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(unescape_mount)
        .filter(|p| is_drive_mount(p))
        .map(PathBuf::from)
        .collect();
    out.dedup();
    out
}

fn is_drive_mount(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').skip(1).collect();
    match parts.as_slice() {
        ["media", _] | ["media", _, _] | ["mnt", _] | ["run", "media", _, _] => parts.iter().all(|p| !p.is_empty()),
        _ => false,
    }
}

fn unescape_mount(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b)) {
            out.push((bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// (free bytes available to this user, total bytes) of the volume holding
/// `path`.
pub(super) fn space(path: &Path) -> (Option<u64>, Option<u64>) {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return (None, None) };
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return (None, None);
    }
    let unit = st.f_frsize as u64;
    (Some(st.f_bavail as u64 * unit), Some(st.f_blocks as u64 * unit))
}

#[cfg(test)]
mod drive_tests {
    use super::*;

    #[test]
    fn mounts_under_media_and_mnt_only_with_escapes_decoded() {
        let mounts = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
proc /proc proc rw 0 0
/dev/sdb1 /media/anna/KINGSTON vfat rw,nosuid 0 0
/dev/sdc1 /run/media/anna/My\\040Backup exfat rw 0 0
/dev/sdd1 /mnt/data ext4 rw 0 0
/dev/sde1 /mediaserver ext4 rw 0 0
overlay /mnt/sandbox/tools/v1 overlay ro 0 0
/dev/sdf1 /media/anna/stick/inner ext4 rw 0 0
tmpfs /run/user/1000 tmpfs rw 0 0
";
        assert_eq!(
            parse_mounts(mounts),
            vec![
                PathBuf::from("/media/anna/KINGSTON"),
                PathBuf::from("/run/media/anna/My Backup"),
                PathBuf::from("/mnt/data"),
            ]
        );
        assert_eq!(unescape_mount(r"a\134b\011c\04"), "a\\b\tc\\04", "truncated escapes stay literal");
    }

    #[test]
    fn space_of_an_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (free, total) = space(tmp.path());
        assert!(total.unwrap() > 0 && free.unwrap() <= total.unwrap());
        assert_eq!(space(&tmp.path().join("missing")), (None, None));
    }
}
