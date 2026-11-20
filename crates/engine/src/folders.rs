//! Mapping standard folders from the source system to the target system.
//!
//! The package records where the source kept each [`KnownFolder`] (for
//! example Windows `Documents`, or `OneDrive/Documents` when redirected, or
//! `@Documents` for a folder outside the profile). On restore, a file under
//! the source's Documents goes under the *target's* Documents, wherever that
//! is (e.g. `~/Dokumenter` on a Norwegian Linux desktop). Everything else
//! keeps its path relative to the profile.

use std::path::{Path, PathBuf};

use profileport_package::format::split_rel_path;
use profileport_package::{BeginInfo, KnownFolder};

pub(crate) struct FolderMap {
    source: Vec<(KnownFolder, Vec<String>)>,
    target: Vec<(KnownFolder, PathBuf)>,
    source_case_insensitive: bool,
}

pub(crate) struct Located<'a> {
    pub base: PathBuf,
    pub rest: Vec<&'a str>,
    pub folder: Option<KnownFolder>,
}

impl FolderMap {
    pub fn new(begin: &BeginInfo, target: Vec<(KnownFolder, PathBuf)>) -> Self {
        let source = begin
            .known_folders
            .iter()
            .filter_map(|e| Some((e.folder, split_rel_path(&e.path).ok()?.into_iter().map(String::from).collect())))
            .collect();
        Self { source, target, source_case_insensitive: matches!(begin.source_os.as_str(), "windows" | "macos") }
    }

    pub fn target_folders(&self) -> &[(KnownFolder, PathBuf)] {
        &self.target
    }

    /// Where a package path goes: a base directory on the target and the
    /// components below it (not yet adapted to the target's name rules).
    pub fn locate<'a>(&self, comps: &[&'a str], root: &Path) -> Located<'a> {
        let eq =
            |a: &str, b: &str| if self.source_case_insensitive { a.to_lowercase() == b.to_lowercase() } else { a == b };
        let best = self
            .source
            .iter()
            .filter(|(_, prefix)| prefix.len() <= comps.len() && prefix.iter().zip(comps).all(|(p, c)| eq(p, c)))
            .max_by_key(|(_, prefix)| prefix.len());
        if let Some((kf, prefix)) = best {
            if let Some((_, base)) = self.target.iter().find(|(k, _)| k == kf) {
                return Located { base: base.clone(), rest: comps[prefix.len()..].to_vec(), folder: Some(*kf) };
            }
        }
        // Not in a known folder, or the target lacks it. A folder that was
        // outside the source profile is stored as "@Name": restore it as
        // "Name" in the profile rather than with the marker.
        let mut rest = comps.to_vec();
        if let Some(first) = rest.first_mut() {
            if let Some(stripped) = first.strip_prefix('@') {
                if !stripped.is_empty() {
                    *first = stripped;
                }
            }
        }
        Located { base: root.to_path_buf(), rest, folder: None }
    }
}

/// Does this path look like it is synchronised to a cloud service? Restoring
/// there uploads the customer's files (docs/platform-gotchas.md).
pub fn is_cloud_synced(path: &Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy().to_lowercase();
        s.starts_with("onedrive")
            || s == "dropbox"
            || s == "google drive"
            || s == "icloud drive"
            // macOS iCloud Drive: ~/Library/Mobile Documents/com~apple~CloudDocs
            || s == "mobile documents"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use profileport_package::KnownFolderEntry;

    fn begin(os: &str, folders: &[(KnownFolder, &str)]) -> BeginInfo {
        BeginInfo {
            source_os: os.into(),
            known_folders: folders
                .iter()
                .map(|(f, p)| KnownFolderEntry { folder: *f, path: p.to_string(), source_path: String::new() })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn windows_folders_land_in_translated_linux_folders() {
        let b = begin(
            "windows",
            &[
                (KnownFolder::Documents, "OneDrive/Documents"),
                (KnownFolder::Pictures, "Pictures"),
                (KnownFolder::Music, "@Music"),
            ],
        );
        let home = Path::new("/home/anna");
        let map = FolderMap::new(
            &b,
            vec![
                (KnownFolder::Documents, home.join("Dokumenter")),
                (KnownFolder::Pictures, home.join("Bilder")),
                (KnownFolder::Music, PathBuf::from("/mnt/data/Musikk")),
            ],
        );
        let at = |p: &str| {
            let l = map.locate(&p.split('/').collect::<Vec<_>>(), home);
            (l.base, l.rest.join("/"))
        };
        assert_eq!(at("OneDrive/Documents/Taxes/2025.pdf"), (home.join("Dokumenter"), "Taxes/2025.pdf".into()));
        // Windows names are case-insensitive: "pictures" is Pictures.
        assert_eq!(at("pictures/cat.jpg"), (home.join("Bilder"), "cat.jpg".into()));
        assert_eq!(at("@Music/song.mp3"), (PathBuf::from("/mnt/data/Musikk"), "song.mp3".into()));
        // Not a known folder: stays relative to the profile.
        assert_eq!(at("Favorites/x.url"), (home.to_path_buf(), "Favorites/x.url".into()));
        // OneDrive itself, outside Documents, also stays.
        assert_eq!(at("OneDrive/Other/y.txt"), (home.to_path_buf(), "OneDrive/Other/y.txt".into()));
    }

    #[test]
    fn linux_names_are_case_sensitive() {
        let b = begin("linux", &[(KnownFolder::Documents, "Dokumenter")]);
        let root = Path::new("C:/Users/anna");
        let map = FolderMap::new(&b, vec![(KnownFolder::Documents, root.join("Documents"))]);
        let l = map.locate(&["Dokumenter", "a.txt"], root);
        assert_eq!((l.base, l.rest), (root.join("Documents"), vec!["a.txt"]));
        let l = map.locate(&["dokumenter", "a.txt"], root);
        assert_eq!(l.base, root.to_path_buf(), "different folder on Linux");
    }

    #[test]
    fn missing_target_folder_falls_back_without_marker() {
        let b = begin("windows", &[(KnownFolder::Videos, "@Videos")]);
        let root = Path::new("/home/anna");
        let map = FolderMap::new(&b, vec![]);
        let l = map.locate(&["@Videos", "clip.mp4"], root);
        assert_eq!((l.base, l.rest), (root.to_path_buf(), vec!["Videos", "clip.mp4"]));
    }

    #[test]
    fn cloud_detection() {
        assert!(is_cloud_synced(Path::new("C:/Users/anna/OneDrive - Contoso/Documents")));
        assert!(!is_cloud_synced(Path::new("C:/Users/anna/Documents")));
    }
}
