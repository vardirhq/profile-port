//! Windows: profiles from the registry ProfileList; known folders from the
//! Shell (current user, which follows redirection and OneDrive) or from the
//! user's "User Shell Folders" registry key (other signed-in users).

use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use profileport_package::KnownFolder;
use windows_sys::core::{GUID, PWSTR};
use windows_sys::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ,
    RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Templates,
    FOLDERID_Videos, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

use super::ProfileInfo;

const PROFILE_LIST: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
const USER_SHELL_FOLDERS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// A REG_SZ / REG_EXPAND_SZ value, *not* expanded.
fn reg_string(root: HKEY, subkey: &str, value: &str) -> Option<String> {
    let (sk, v) = (wide(subkey), wide(value));
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut len: u32 = 0;
    // SAFETY: valid NUL-terminated strings; null data pointer queries size.
    let rc = unsafe { RegGetValueW(root, sk.as_ptr(), v.as_ptr(), flags, null_mut(), null_mut(), &mut len) };
    if rc != ERROR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize / 2 + 1];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: buffer is `len` bytes long.
    let rc =
        unsafe { RegGetValueW(root, sk.as_ptr(), v.as_ptr(), flags, null_mut(), buf.as_mut_ptr().cast(), &mut len) };
    (rc == ERROR_SUCCESS).then(|| from_wide(&buf))
}

fn subkeys(root: HKEY, path: &str) -> Vec<String> {
    let mut key: HKEY = null_mut();
    // SAFETY: valid string; `key` receives the handle and is closed below.
    if unsafe { RegOpenKeyExW(root, wide(path).as_ptr(), 0, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return Vec::new();
    }
    let mut out = Vec::new();
    for index in 0.. {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        // SAFETY: `name` holds `len` UTF-16 units.
        let rc = unsafe {
            RegEnumKeyExW(key, index, name.as_mut_ptr(), &mut len, null_mut(), null_mut(), null_mut(), null_mut())
        };
        if rc == ERROR_NO_MORE_ITEMS || rc != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    // SAFETY: `key` was opened above.
    unsafe { RegCloseKey(key) };
    out
}

fn expand_env(s: &str) -> String {
    let src = wide(s);
    // SAFETY: null destination with size 0 returns the required size.
    let n = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), null_mut(), 0) };
    if n == 0 {
        return s.to_string();
    }
    let mut buf = vec![0u16; n as usize];
    // SAFETY: `buf` holds `n` units.
    let m = unsafe { ExpandEnvironmentStringsW(src.as_ptr(), buf.as_mut_ptr(), n) };
    if m == 0 || m > n {
        return s.to_string();
    }
    from_wide(&buf)
}

fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    norm(a) == norm(b)
}

pub(super) fn list_profiles() -> Vec<ProfileInfo> {
    let current = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let mut out = Vec::new();
    for sid in subkeys(HKEY_LOCAL_MACHINE, PROFILE_LIST) {
        // Local / domain (S-1-5-21-…) and Entra ID (S-1-12-1-…) users only;
        // skips SYSTEM, LOCAL SERVICE, NETWORK SERVICE and `.bak` leftovers.
        if !(sid.starts_with("S-1-5-21-") || sid.starts_with("S-1-12-1-")) || sid.ends_with(".bak") {
            continue;
        }
        let Some(raw) = reg_string(HKEY_LOCAL_MACHINE, &format!(r"{PROFILE_LIST}\{sid}"), "ProfileImagePath") else {
            continue;
        };
        let path = PathBuf::from(expand_env(&raw));
        if !path.is_dir() {
            continue;
        }
        out.push(ProfileInfo {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            current: current.as_deref().is_some_and(|c| same_path(c, &path)),
            sid: Some(sid),
            path,
        });
    }
    out
}

fn shell_folder(id: &GUID) -> Option<PathBuf> {
    let mut p: PWSTR = null_mut();
    // SAFETY: null token = current user; `p` is freed with CoTaskMemFree.
    let hr = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, null_mut(), &mut p) };
    let out = if hr == 0 && !p.is_null() {
        // SAFETY: on success `p` is a NUL-terminated string.
        let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
        Some(PathBuf::from(String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })))
    } else {
        None
    };
    // SAFETY: CoTaskMemFree accepts null.
    unsafe { CoTaskMemFree(p as *const _) };
    out
}

/// Registry value names under "User Shell Folders".
fn shell_folder_value(kf: KnownFolder) -> Option<&'static str> {
    Some(match kf {
        KnownFolder::Desktop => "Desktop",
        KnownFolder::Documents => "Personal",
        KnownFolder::Downloads => "{374DE290-123F-4565-9164-39C4925E467B}",
        KnownFolder::Music => "My Music",
        KnownFolder::Pictures => "My Pictures",
        KnownFolder::Videos => "My Video",
        KnownFolder::Templates => "Templates",
        KnownFolder::Public => return None,
    })
}

fn folder_id(kf: KnownFolder) -> Option<GUID> {
    Some(match kf {
        KnownFolder::Desktop => FOLDERID_Desktop,
        KnownFolder::Documents => FOLDERID_Documents,
        KnownFolder::Downloads => FOLDERID_Downloads,
        KnownFolder::Music => FOLDERID_Music,
        KnownFolder::Pictures => FOLDERID_Pictures,
        KnownFolder::Videos => FOLDERID_Videos,
        KnownFolder::Templates => FOLDERID_Templates,
        KnownFolder::Public => return None,
    })
}

/// Expand a "User Shell Folders" value for *that* user: %USERPROFILE% means
/// their profile, not ours.
fn expand_for_profile(value: &str, profile: &Path) -> PathBuf {
    let lower = value.to_ascii_lowercase();
    let replaced = match lower.find("%userprofile%") {
        Some(i) => format!("{}{}{}", &value[..i], profile.display(), &value[i + "%userprofile%".len()..]),
        None => value.to_string(),
    };
    PathBuf::from(expand_env(&replaced))
}

pub(super) fn known_folders(profile: &Path) -> Vec<(KnownFolder, PathBuf)> {
    let is_current = std::env::var_os("USERPROFILE").is_some_and(|c| same_path(Path::new(&c), profile));
    if is_current {
        return KnownFolder::ALL.iter().filter_map(|&kf| Some((kf, shell_folder(&folder_id(kf)?)?))).collect();
    }
    // Another user: their hive is under HKEY_USERS\<SID> while they are
    // signed in. (Loading an offline hive needs SeRestorePrivilege; TODO.)
    let Some(sid) = list_profiles().into_iter().find(|p| same_path(&p.path, profile)).and_then(|p| p.sid) else {
        return Vec::new();
    };
    let key = format!(r"{sid}\{USER_SHELL_FOLDERS}");
    KnownFolder::ALL
        .iter()
        .filter_map(|&kf| {
            let value = reg_string(HKEY_USERS, &key, shell_folder_value(kf)?)?;
            Some((kf, expand_for_profile(&value, profile)))
        })
        .collect()
}
