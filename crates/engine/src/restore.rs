//! Restoring a package into a destination profile folder, and verifying
//! packages.
//!
//! Safety rules (proposal §3.2, §9):
//! - Existing files are never overwritten or deleted.
//! - Every path from the package is validated before anything is written.
//! - Links already on the destination are never followed.
//! - Files are written to a temporary name and only appear under their real
//!   name once complete and verified against the package hash.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read};
use std::path::{Path, PathBuf};

use profileport_package::format::split_rel_path;
use profileport_package::{Entry, Error as PkgError, FileEntry, KnownFolder, PackageReader, SkipReason};

use crate::folders::{is_cloud_synced, FolderMap};
use crate::names::NameRules;
use crate::platform::to_system_time;
use crate::{Progress, SkipItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// Leave the existing file alone and report it.
    #[default]
    Skip,
    /// Restore next to it as "name (restored).ext".
    KeepBoth,
}

#[derive(Debug, Clone)]
pub struct RestoreOptions {
    pub on_conflict: ConflictPolicy,
    /// Re-read each written file and compare it with the package hash.
    pub verify: bool,
    /// File-name rules of the target. Defaults to the running system's.
    pub name_rules: NameRules,
    /// Where the target profile keeps its standard folders. `None` asks the
    /// running system (see [`crate::system::known_folders`]).
    pub known_folders: Option<Vec<(KnownFolder, PathBuf)>>,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self { on_conflict: ConflictPolicy::Skip, verify: true, name_rules: NameRules::native(), known_folders: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictOutcome {
    LeftExisting,
    RestoredAs(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RenameReason {
    /// The name isn't allowed on this system (e.g. `:` or `CON` on Windows).
    NotAllowedHere,
    /// Another file in the backup has the same name apart from letter case,
    /// which this system treats as the same name.
    CaseCollision,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub files: u64,
    pub bytes: u64,
    pub dirs: u64,
    /// Files already at the destination with identical content (for
    /// example from an earlier, interrupted run of the same restore).
    pub already_present: u64,
    /// Files whose written copy was re-read and matched the package hash.
    pub verified: u64,
    pub conflicts: Vec<(String, ConflictOutcome)>,
    /// Files restored under a different name: (package path, path written
    /// relative to its destination folder, reason).
    pub renamed: Vec<(String, String, RenameReason)>,
    pub failed: Vec<(String, String)>,
    pub links_not_restored: Vec<(String, String)>,
    /// Items the backup itself could not include.
    pub skipped_at_backup: Vec<SkipItem>,
    pub changed_at_backup: Vec<String>,
    /// The backup was interrupted; it may be missing files.
    pub backup_complete: bool,
    pub incomplete_at_backup: Vec<String>,
    /// Standard folders that received files, and where they are here.
    pub folders_used: Vec<(KnownFolder, PathBuf)>,
    /// Of those, the ones that look cloud-synchronised (OneDrive, …):
    /// restoring there uploads the files.
    pub cloud_synced: Vec<(KnownFolder, PathBuf)>,
    /// Alternate data streams written (Windows) or not applicable here.
    pub streams_restored: u64,
    pub streams_not_applicable: u64,
    pub source_os: String,
}

impl RestoreReport {
    /// Everything in the backup arrived and was verified, and the backup
    /// itself was complete and clean.
    pub fn fully_successful(&self) -> bool {
        self.failed.is_empty()
            && self.backup_complete
            && self.skipped_at_backup.is_empty()
            && self.incomplete_at_backup.is_empty()
            && self.conflicts.iter().all(|(_, c)| matches!(c, ConflictOutcome::RestoredAs(_)))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error(transparent)]
    Package(#[from] PkgError),
    #[error("cannot use {path} as the restore destination: {source}")]
    Target { path: PathBuf, source: io::Error },
    #[error("the package contains an unsafe path ({0:?}); nothing was restored")]
    UnsafePackage(String),
}

/// Where one package path goes on this system.
struct Placement {
    dir: PathBuf,
    name: String,
    /// Path shown to the technician, relative to the destination folder.
    display: String,
    renamed: Option<RenameReason>,
}

struct Ctx<'a> {
    reader: &'a PackageReader,
    root: &'a Path,
    opts: &'a RestoreOptions,
    map: FolderMap,
    /// Destination paths used in this run (case-folded where the target is
    /// case-insensitive), to detect two package paths landing on one name.
    claimed: HashSet<String>,
    /// Package path of each file → where it was written (or found).
    written: HashMap<String, PathBuf>,
    used_folders: HashSet<KnownFolder>,
    report: RestoreReport,
}

pub fn restore(
    reader: &PackageReader,
    target: &Path,
    opts: &RestoreOptions,
    progress: &mut dyn FnMut(&Progress),
) -> Result<RestoreReport, RestoreError> {
    let tmeta = fs::metadata(target).map_err(|e| RestoreError::Target { path: target.into(), source: e })?;
    if !tmeta.is_dir() {
        return Err(RestoreError::Target {
            path: target.into(),
            source: io::Error::new(io::ErrorKind::NotADirectory, "not a folder"),
        });
    }
    let manifest = reader.manifest()?;

    // Validate everything before writing anything.
    for e in &manifest.entries {
        let path = match e {
            Entry::Dir { path, .. } | Entry::Link { path, .. } | Entry::AltStream { path, .. } => path,
            Entry::File(f) => &f.path,
            Entry::Skipped { .. } => continue,
        };
        split_rel_path(path).map_err(|_| RestoreError::UnsafePackage(path.clone()))?;
    }

    let target_folders: Vec<(KnownFolder, PathBuf)> = opts
        .known_folders
        .clone()
        .unwrap_or_else(|| crate::system::known_folders(target))
        .into_iter()
        // A standard folder inside the profile that is reached through a
        // link would let restore write outside the profile. Folders outside
        // the profile come from the system's own settings and are used as is.
        .filter(|(_, p)| !passes_through_link(target, p))
        .collect();
    let mut cx = Ctx {
        reader,
        root: target,
        opts,
        map: FolderMap::new(&manifest.begin, target_folders),
        claimed: HashSet::new(),
        written: HashMap::new(),
        used_folders: HashSet::new(),
        report: RestoreReport {
            backup_complete: manifest.complete,
            incomplete_at_backup: manifest.incomplete_files.clone(),
            source_os: manifest.begin.source_os.clone(),
            ..Default::default()
        },
    };
    let mut prog = Progress::default();
    for e in &manifest.entries {
        match e {
            Entry::Dir { path, .. } => match cx.place_dir(path) {
                Ok(()) => cx.report.dirs += 1,
                Err(msg) => cx.report.failed.push((path.clone(), msg)),
            },
            Entry::File(f) => {
                cx.restore_file(f);
                if f.changed_during_copy {
                    cx.report.changed_at_backup.push(f.path.clone());
                }
                prog.files += 1;
                prog.bytes += f.size;
                prog.current = f.path.clone();
                progress(&prog);
            }
            Entry::AltStream { path, name, data } => cx.restore_stream(path, name, data),
            Entry::Link { path, target: t, .. } => cx.report.links_not_restored.push((path.clone(), t.clone())),
            Entry::Skipped { path, reason, detail } if *reason != SkipReason::Excluded => cx
                .report
                .skipped_at_backup
                .push(SkipItem { path: path.clone(), reason: *reason, detail: detail.clone() }),
            Entry::Skipped { .. } => {}
        }
    }
    let mut used: Vec<(KnownFolder, PathBuf)> =
        cx.map.target_folders().iter().filter(|(k, _)| cx.used_folders.contains(k)).cloned().collect();
    used.sort();
    cx.report.cloud_synced = used.iter().filter(|(_, p)| is_cloud_synced(p)).cloned().collect();
    cx.report.folders_used = used;
    Ok(cx.report)
}

impl Ctx<'_> {
    fn norm(&self, p: &Path) -> String {
        let s = p.to_string_lossy().into_owned();
        if self.opts.name_rules.case_insensitive() || cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    }

    /// Resolve a package path to a directory (created safely) and a final
    /// component name adapted to the target's rules.
    fn place(&mut self, path: &str) -> Result<Placement, String> {
        let comps = split_rel_path(path).map_err(|e| e.to_string())?;
        let located = self.map.locate(&comps, self.root);
        if let Some(kf) = located.folder {
            self.used_folders.insert(kf);
        }
        let rules = self.opts.name_rules;
        let adapted: Vec<String> = located
            .rest
            .iter()
            .map(|c| {
                let name = rules.adapt(c);
                // Defence in depth: on Windows, `\` and `:` are structural
                // whatever rules the caller asked for, so Windows rules
                // always apply on top.
                if cfg!(windows) {
                    NameRules::Windows.adapt(&name).into_owned()
                } else {
                    name.into_owned()
                }
            })
            .collect();
        let renamed = (adapted.iter().zip(&located.rest).any(|(a, o)| a != o)).then_some(RenameReason::NotAllowedHere);
        let (name, parents) = match adapted.split_last() {
            Some((n, p)) => (n.clone(), p.to_vec()),
            None => (String::new(), Vec::new()),
        };
        let dir = safe_dir(&located.base, path, &parents)?;
        let display = adapted.join("/");
        Ok(Placement { dir, name, display, renamed })
    }

    fn place_dir(&mut self, path: &str) -> Result<(), String> {
        let p = self.place(path)?;
        if p.name.is_empty() {
            return Ok(()); // a known folder itself
        }
        safe_dir(&p.dir, path, &[p.name.as_str()]).map(|_| ())
    }

    fn restore_file(&mut self, f: &FileEntry) {
        let mut p = match self.place(&f.path) {
            Ok(p) if !p.name.is_empty() => p,
            Ok(_) => return self.report.failed.push((f.path.clone(), "a file can't replace a standard folder".into())),
            Err(msg) => return self.report.failed.push((f.path.clone(), msg)),
        };

        // Two backup files that become one name here ("a:b" and "a?b" on
        // Windows, or "Notes.txt" and "notes.txt"): keep both.
        if self.claimed.contains(&self.norm(&p.dir.join(&p.name))) {
            p.name = alternative_for(&p.dir, &p.name, f);
            p.renamed = Some(RenameReason::CaseCollision);
        }
        let dest = p.dir.join(&p.name);
        self.claimed.insert(self.norm(&dest));

        if is_identical(&dest, f) {
            self.report.already_present += 1;
            self.written.insert(f.path.clone(), dest);
            return;
        }
        let mut final_name = p.name.clone();
        if exists(&dest) {
            match self.opts.on_conflict {
                ConflictPolicy::Skip => {
                    return self.report.conflicts.push((f.path.clone(), ConflictOutcome::LeftExisting));
                }
                ConflictPolicy::KeepBoth => {
                    final_name = alternative_for(&p.dir, &p.name, f);
                    // Restored alongside on an earlier run of this restore.
                    if is_identical(&p.dir.join(&final_name), f) {
                        self.report.already_present += 1;
                        self.written.insert(f.path.clone(), p.dir.join(&final_name));
                        return;
                    }
                }
            }
        }
        let dest = p.dir.join(&final_name);
        self.claimed.insert(self.norm(&dest));
        let tmp = p.dir.join(format!(".profileport-{}-{}.tmp", self.reader.header().package_id_hex(), f.id));

        let written = (|| -> Result<(), String> {
            let file = File::create_new(&tmp).map_err(|e| format!("could not create file: {e}"))?;
            let mut w = BufWriter::new(file);
            self.reader.read_file(f, &mut w).map_err(|e| e.to_string())?;
            let file = w.into_inner().map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            Ok(())
        })();
        if let Err(msg) = written {
            let _ = fs::remove_file(&tmp);
            return self.report.failed.push((f.path.clone(), msg));
        }

        // Move into place without ever replacing an existing file: a hard
        // link fails if the name exists. Fall back to rename on filesystems
        // without hard links, after re-checking.
        let placed = match fs::hard_link(&tmp, &dest) {
            Ok(()) => {
                let _ = fs::remove_file(&tmp);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(None),
            Err(_) if exists(&dest) => Err(None),
            Err(_) => fs::rename(&tmp, &dest).map_err(|e| Some(e.to_string())),
        };
        match placed {
            Ok(()) => {}
            Err(None) => {
                let _ = fs::remove_file(&tmp);
                return self.report.conflicts.push((f.path.clone(), ConflictOutcome::LeftExisting));
            }
            Err(Some(msg)) => {
                let _ = fs::remove_file(&tmp);
                return self.report.failed.push((f.path.clone(), msg));
            }
        }
        let shown = match p.display.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{final_name}"),
            None => final_name.clone(),
        };
        if final_name != p.name {
            self.report.conflicts.push((f.path.clone(), ConflictOutcome::RestoredAs(shown.clone())));
        }
        if let Some(reason) = p.renamed {
            self.report.renamed.push((f.path.clone(), shown, reason));
        }

        apply_times(&dest, f);
        if self.opts.verify {
            match hash_file(&dest) {
                Ok(h) if h == f.blake3 => self.report.verified += 1,
                Ok(_) => self.report.failed.push((f.path.clone(), "verification failed: written data differs".into())),
                Err(e) => self.report.failed.push((f.path.clone(), format!("verification failed: {e}"))),
            }
        }
        apply_attributes(&dest, f);
        self.written.insert(f.path.clone(), dest);
        self.report.files += 1;
        self.report.bytes += f.size;
    }

    fn restore_stream(&mut self, path: &str, name: &str, data: &[u8]) {
        if self.opts.name_rules != NameRules::Windows || !cfg!(windows) {
            self.report.streams_not_applicable += 1;
            return;
        }
        let Some(dest) = self.written.get(path) else { return };
        if !profileport_package::format::valid_stream_name(name) {
            return self.report.failed.push((format!("{path}:{name}"), "invalid stream name".into()));
        }
        match write_stream(dest, name, data) {
            Ok(()) => self.report.streams_restored += 1,
            Err(e) => self.report.failed.push((format!("{path}:{name}"), e.to_string())),
        }
    }
}

/// Modification time, plus creation time on Windows. Done before setting
/// read-only: Windows can't change times on a read-only file.
fn apply_times(dest: &Path, f: &FileEntry) {
    let Ok(file) = fs::OpenOptions::new().write(true).open(dest) else { return };
    let mut times = fs::FileTimes::new();
    if let Some(t) = f.meta.modified.as_ref().and_then(to_system_time) {
        times = times.set_modified(t);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTimesExt;
        if let Some(t) = f.meta.created.as_ref().and_then(to_system_time) {
            times = times.set_created(t);
        }
    }
    let _ = file.set_times(times);
}

fn apply_attributes(dest: &Path, f: &FileEntry) {
    #[cfg(windows)]
    {
        let attrs = f.meta.win_attributes.unwrap_or(0);
        let hidden = attrs & crate::win::HIDDEN != 0;
        let system = attrs & crate::win::SYSTEM != 0;
        if hidden || system || f.meta.readonly {
            let _ = crate::win::set_attributes(dest, hidden, system, f.meta.readonly);
        }
    }
    #[cfg(not(windows))]
    if f.meta.readonly {
        if let Ok(m) = fs::metadata(dest) {
            let mut p = m.permissions();
            p.set_readonly(true);
            let _ = fs::set_permissions(dest, p);
        }
    }
}

#[cfg(windows)]
fn write_stream(dest: &Path, name: &str, data: &[u8]) -> io::Result<()> {
    // The stream is written after attributes; a read-only file refuses it,
    // so lift the flag around the write.
    let ro = fs::metadata(dest).map(|m| m.permissions().readonly()).unwrap_or(false);
    if ro {
        let mut p = fs::metadata(dest)?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        p.set_readonly(false);
        fs::set_permissions(dest, p)?;
    }
    let r = crate::win::write_stream(dest, name, data);
    if ro {
        let mut p = fs::metadata(dest)?.permissions();
        p.set_readonly(true);
        fs::set_permissions(dest, p)?;
    }
    r
}

#[cfg(not(windows))]
fn write_stream(_: &Path, _: &str, _: &[u8]) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "alternate data streams need Windows"))
}

/// Is `path` inside `root` and reached through a symlink or junction?
fn passes_through_link(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else { return false };
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        cur.push(c);
        if fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

/// Walk/create `comps` under `root`, refusing to pass through links or
/// files. Returns the directory path.
fn safe_dir<S: AsRef<str>>(root: &Path, rel: &str, comps: &[S]) -> Result<PathBuf, String> {
    let mut cur = root.to_path_buf();
    for c in comps {
        cur.push(c.as_ref());
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(format!("{rel}: a folder on this PC is a link ({}); not following it", cur.display()))
            }
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{rel}: a file is in the way at {}", cur.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => match fs::create_dir(&cur) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && cur.is_dir() => {}
                Err(e) => return Err(format!("{rel}: could not create folder: {e}")),
            },
            Err(e) => return Err(format!("{rel}: {e}")),
        }
    }
    Ok(cur)
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

/// "name.ext" → "name (restored).ext", "name (restored 2).ext", …: the
/// first that doesn't exist, or for which `reuse` says the existing file is
/// already the right one.
fn alternative_name_with(dir: &Path, name: &str, reuse: impl Fn(&Path) -> bool) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = if n == 1 { format!("{stem} (restored){ext}") } else { format!("{stem} (restored {n}){ext}") };
        let path = dir.join(&candidate);
        if !exists(&path) || reuse(&path) {
            return candidate;
        }
    }
    unreachable!()
}

/// An alternative name for `name` in `dir` that is free or already holds
/// this exact file (so running the same restore again creates no
/// duplicates).
fn alternative_for(dir: &Path, name: &str, f: &FileEntry) -> String {
    alternative_name_with(dir, name, |p| is_identical(p, f))
}

/// A regular file (not a link) with exactly the backed-up content.
fn is_identical(path: &Path, f: &FileEntry) -> bool {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_file() && m.len() == f.size => hash_file(path).is_ok_and(|h| h == f.blake3),
        _ => false,
    }
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    // TODO(windows): open with FILE_FLAG_NO_BUFFERING so this reads the
    // device rather than the cache (ADR 0003).
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(*h.finalize().as_bytes())
}

#[derive(Debug, Clone, Default)]
pub struct VerifyReport {
    pub files: u64,
    pub bytes: u64,
    pub failed: Vec<(String, String)>,
    pub complete: bool,
    pub incomplete_files: Vec<String>,
}

/// Decrypt and hash every file in the package without writing anything.
pub fn verify(reader: &PackageReader, progress: &mut dyn FnMut(&Progress)) -> Result<VerifyReport, PkgError> {
    let m = reader.manifest()?;
    let mut r = VerifyReport { complete: m.complete, incomplete_files: m.incomplete_files, ..Default::default() };
    let mut prog = Progress::default();
    for e in &m.entries {
        if let Entry::File(f) = e {
            match reader.read_file(f, &mut io::sink()) {
                Ok(()) => {
                    r.files += 1;
                    r.bytes += f.size;
                }
                Err(e) => r.failed.push((f.path.clone(), e.to_string())),
            }
            prog.files += 1;
            prog.bytes += f.size;
            prog.current = f.path.clone();
            progress(&prog);
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternative_names() {
        let tmp = tempfile::tempdir().unwrap();
        let alternative_name = |d: &Path, n: &str| alternative_name_with(d, n, |_| false);
        assert_eq!(alternative_name(tmp.path(), "a.txt"), "a (restored).txt");
        fs::write(tmp.path().join("a (restored).txt"), "").unwrap();
        assert_eq!(alternative_name(tmp.path(), "a.txt"), "a (restored 2).txt");
        assert_eq!(alternative_name(tmp.path(), ".bashrc"), ".bashrc (restored)");
        assert_eq!(alternative_name(tmp.path(), "archive.tar.gz"), "archive.tar (restored).gz");
    }
}
