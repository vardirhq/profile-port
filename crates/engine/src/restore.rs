//! Restoring a package into a destination profile folder, and verifying
//! packages.
//!
//! Safety rules (proposal §3.2, §9):
//! - Existing files are never overwritten or deleted.
//! - Every path from the package is validated before anything is written.
//! - Links already on the destination are never followed.
//! - Files are written to a temporary name and only appear under their real
//!   name once complete and verified against the package hash.

use std::fs::{self, File};
use std::io::{self, BufWriter, Read};
use std::path::{Path, PathBuf};

use profileport_package::format::split_rel_path;
use profileport_package::{Entry, Error as PkgError, FileEntry, PackageReader, SkipReason};

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
}

impl Default for RestoreOptions {
    fn default() -> Self {
        Self { on_conflict: ConflictPolicy::Skip, verify: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictOutcome {
    LeftExisting,
    RestoredAs(String),
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
    pub failed: Vec<(String, String)>,
    pub links_not_restored: Vec<(String, String)>,
    /// Items the backup itself could not include.
    pub skipped_at_backup: Vec<SkipItem>,
    pub changed_at_backup: Vec<String>,
    /// The backup was interrupted; it may be missing files.
    pub backup_complete: bool,
    pub incomplete_at_backup: Vec<String>,
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
            Entry::Dir { path, .. } | Entry::Link { path, .. } => path,
            Entry::File(f) => &f.path,
            Entry::Skipped { .. } => continue,
        };
        split_rel_path(path).map_err(|_| RestoreError::UnsafePackage(path.clone()))?;
    }

    let mut report = RestoreReport {
        backup_complete: manifest.complete,
        incomplete_at_backup: manifest.incomplete_files.clone(),
        ..Default::default()
    };
    let mut prog = Progress::default();
    for e in &manifest.entries {
        match e {
            Entry::Dir { path, .. } => match safe_dir(target, path, &split_rel_path(path).unwrap()) {
                Ok(_) => report.dirs += 1,
                Err(msg) => report.failed.push((path.clone(), msg)),
            },
            Entry::File(f) => {
                restore_file(reader, target, f, opts, &mut report);
                prog.files += 1;
                prog.bytes += f.size;
                prog.current = f.path.clone();
                progress(&prog);
            }
            Entry::Link { path, target: t, .. } => report.links_not_restored.push((path.clone(), t.clone())),
            Entry::Skipped { path, reason, detail } if *reason != SkipReason::Excluded => {
                report.skipped_at_backup.push(SkipItem { path: path.clone(), reason: *reason, detail: detail.clone() })
            }
            Entry::Skipped { .. } => {}
        }
        if let Entry::File(f) = e {
            if f.changed_during_copy {
                report.changed_at_backup.push(f.path.clone());
            }
        }
    }
    Ok(report)
}

/// Walk/create `comps` under `root`, refusing to pass through links or
/// files. Returns the directory path.
fn safe_dir(root: &Path, rel: &str, comps: &[&str]) -> Result<PathBuf, String> {
    let mut cur = root.to_path_buf();
    for c in comps {
        cur.push(c);
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

/// "name.ext" → "name (restored).ext", "name (restored 2).ext", …
fn alternative_name(dir: &Path, name: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1.. {
        let candidate = if n == 1 { format!("{stem} (restored){ext}") } else { format!("{stem} (restored {n}){ext}") };
        if !exists(&dir.join(&candidate)) {
            return candidate;
        }
    }
    unreachable!()
}

fn restore_file(reader: &PackageReader, root: &Path, f: &FileEntry, opts: &RestoreOptions, report: &mut RestoreReport) {
    let comps = split_rel_path(&f.path).expect("validated above");
    let (name, parents) = comps.split_last().unwrap();
    let parent_rel = parents.join("/");
    let dir = match safe_dir(root, &f.path, parents) {
        Ok(d) => d,
        Err(msg) => return report.failed.push((f.path.clone(), msg)),
    };

    let mut final_name = name.to_string();
    if is_identical(&dir.join(name), f) {
        report.already_present += 1;
        return;
    }
    if exists(&dir.join(name)) {
        match opts.on_conflict {
            ConflictPolicy::Skip => {
                return report.conflicts.push((f.path.clone(), ConflictOutcome::LeftExisting));
            }
            ConflictPolicy::KeepBoth => final_name = alternative_name(&dir, name),
        }
    }
    let dest = dir.join(&final_name);
    let tmp = dir.join(format!(".profileport-{}-{}.tmp", reader.header().package_id_hex(), f.id));

    let written = (|| -> Result<(), String> {
        let file = File::create_new(&tmp).map_err(|e| format!("could not create file: {e}"))?;
        let mut w = BufWriter::new(file);
        reader.read_file(f, &mut w).map_err(|e| e.to_string())?;
        let file = w.into_inner().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(msg) = written {
        let _ = fs::remove_file(&tmp);
        return report.failed.push((f.path.clone(), msg));
    }

    // Move into place without ever replacing an existing file: a hard link
    // fails if the name exists. Fall back to rename on filesystems without
    // hard links, after re-checking.
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
            // Something appeared at that name meanwhile.
            let _ = fs::remove_file(&tmp);
            return report.conflicts.push((f.path.clone(), ConflictOutcome::LeftExisting));
        }
        Err(Some(msg)) => {
            let _ = fs::remove_file(&tmp);
            return report.failed.push((f.path.clone(), msg));
        }
    }
    if final_name != *name {
        let rel = if parent_rel.is_empty() { final_name.clone() } else { format!("{parent_rel}/{final_name}") };
        report.conflicts.push((f.path.clone(), ConflictOutcome::RestoredAs(rel)));
    }

    // Timestamps before read-only: Windows can't set times on a read-only
    // file.
    if let Some(t) = f.meta.modified.as_ref().and_then(to_system_time) {
        if let Ok(file) = fs::OpenOptions::new().write(true).open(&dest) {
            let _ = file.set_times(fs::FileTimes::new().set_modified(t));
        }
    }
    if opts.verify {
        match hash_file(&dest) {
            Ok(h) if h == f.blake3 => report.verified += 1,
            Ok(_) => report.failed.push((f.path.clone(), "verification failed: written data differs".into())),
            Err(e) => report.failed.push((f.path.clone(), format!("verification failed: {e}"))),
        }
    }
    if f.meta.readonly {
        if let Ok(m) = fs::metadata(&dest) {
            let mut p = m.permissions();
            p.set_readonly(true);
            let _ = fs::set_permissions(&dest, p);
        }
    }
    report.files += 1;
    report.bytes += f.size;
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
        assert_eq!(alternative_name(tmp.path(), "a.txt"), "a (restored).txt");
        fs::write(tmp.path().join("a (restored).txt"), "").unwrap();
        assert_eq!(alternative_name(tmp.path(), "a.txt"), "a (restored 2).txt");
        assert_eq!(alternative_name(tmp.path(), ".bashrc"), ".bashrc (restored)");
        assert_eq!(alternative_name(tmp.path(), "archive.tar.gz"), "archive.tar (restored).gz");
    }
}
