//! Linux / Unix: profiles from /etc/passwd, folders from XDG user-dirs.

use std::fs;
use std::path::{Path, PathBuf};

use profileport_package::KnownFolder;

use super::ProfileInfo;

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
