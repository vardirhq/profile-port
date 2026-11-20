//! Exporting a profile folder into a package, and resuming an export.

use std::collections::HashSet;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use profileport_package::format::{split_rel_path, validate_ticket};
use profileport_package::{
    BeginInfo, Entry, Error as PkgError, KnownFolder, KnownFolderEntry, PackageReader, PackageWriter, PartialFile,
    SkipReason, Status, TransferCode, WriterOptions,
};

use crate::exclude::Exclusions;
use crate::platform::{classify, fingerprint, is_cloud_only, meta_of};
use crate::{Progress, SkipItem};

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub writer: WriterOptions,
    pub exclusions: Exclusions,
    /// Where the source profile keeps its standard folders. `None` asks the
    /// running system (see [`crate::system::known_folders`]).
    pub known_folders: Option<Vec<(KnownFolder, PathBuf)>>,
    /// Test hook: panic after this many source bytes, imitating a crash or
    /// pulled drive (nothing after the last commit is written).
    #[doc(hidden)]
    pub crash_after_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct SourceLabels {
    pub computer: String,
    pub profile: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Package(#[from] PkgError),
    #[error("cannot read the source folder {path}: {source}")]
    Source { path: PathBuf, source: io::Error },
    #[error("the source folder {0} is not available; reconnect the source PC or drive and try again")]
    SourceMissing(PathBuf),
}

/// Summary of a whole package (not just the last session), built from its
/// manifest after the export finishes.
#[derive(Debug, Clone)]
pub struct ExportReport {
    pub package_dir: PathBuf,
    pub ticket: String,
    pub status: Status,
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    pub links: Vec<(String, String)>,
    /// Not copied for a reason the technician should see.
    pub skipped: Vec<SkipItem>,
    /// Matched an exclusion rule (caches, temp).
    pub excluded: Vec<String>,
    /// Copied, but the file changed while it was being read.
    pub changed: Vec<String>,
    /// Alternate data streams stored (Windows).
    pub alt_streams: u64,
    /// Standard folders found, as stored in the package.
    pub known_folders: Vec<KnownFolderEntry>,
}

impl ExportReport {
    pub fn has_issues(&self) -> bool {
        !self.skipped.is_empty() || !self.changed.is_empty()
    }
}

/// A created package that has not been filled yet. Show the transfer code
/// to the technician (and have them confirm it) *before* calling
/// [`PreparedExport::run`]: if the export is interrupted, the code is the
/// only way to resume it.
pub struct PreparedExport {
    writer: PackageWriter,
    root: PathBuf,
    /// Standard folders outside the profile: (package name "@X", path).
    extra_roots: Vec<(String, PathBuf)>,
}

impl PreparedExport {
    pub fn package_dir(&self) -> &Path {
        self.writer.dir()
    }

    /// Copy the profile into the package.
    pub fn run(
        self,
        code: &TransferCode,
        opts: &ExportOptions,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<ExportReport, ExportError> {
        let mut job = Job::new(self.writer, self.root, self.extra_roots, opts, progress, HashSet::new());
        job.walk_all()?;
        job.finish(code)
    }
}

/// Create a new, empty package for `source` (a profile folder) under
/// `transfers_dir`. Returns it with its transfer code, which is shown once
/// and never stored on the drive.
pub fn prepare_export(
    source: &Path,
    transfers_dir: &Path,
    ticket: &str,
    labels: &SourceLabels,
    opts: &ExportOptions,
) -> Result<(PreparedExport, TransferCode), ExportError> {
    validate_ticket(ticket)?;
    let root = fs::canonicalize(source).map_err(|e| ExportError::Source { path: source.into(), source: e })?;
    if !root.is_dir() {
        return Err(ExportError::Source {
            path: source.into(),
            source: io::Error::new(io::ErrorKind::NotADirectory, "not a folder"),
        });
    }
    fs::create_dir_all(transfers_dir).map_err(PkgError::Io)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = transfers_dir.join(format!("{ticket}-{stamp:x}"));
    let folders = opts.known_folders.clone().unwrap_or_else(|| crate::system::known_folders(&root));
    let (known_folders, extra_roots) = plan_known_folders(&root, &folders);
    let begin = BeginInfo {
        source_root: root.to_string_lossy().into_owned(),
        computer: labels.computer.clone(),
        profile: labels.profile.clone(),
        started_unix: (stamp / 1_000_000_000) as i64,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        source_os: crate::system::os_name().to_string(),
        known_folders,
    };
    let (writer, code) = PackageWriter::create(&dir, ticket, begin, opts.writer.clone())?;
    Ok((PreparedExport { writer, root, extra_roots }, code))
}

/// Express each standard folder as a package path. Folders inside the
/// profile keep their relative path; folders elsewhere (redirected to
/// another drive, say) become extra roots stored as `@Name`, unless they
/// sit inside another such folder.
fn plan_known_folders(
    root: &Path,
    folders: &[(KnownFolder, PathBuf)],
) -> (Vec<KnownFolderEntry>, Vec<(String, PathBuf)>) {
    let mut resolved: Vec<(KnownFolder, PathBuf)> = folders
        .iter()
        .filter_map(|(kf, p)| Some((*kf, fs::canonicalize(p).ok()?)))
        .filter(|(_, p)| p != root)
        .collect();
    // Shortest first, so an outside folder containing another is chosen as
    // the root.
    resolved.sort_by_key(|(_, p)| p.as_os_str().len());
    let mut entries = Vec::new();
    let mut extra: Vec<(String, PathBuf)> = Vec::new();
    for (kf, abs) in resolved {
        let path = if let Some(rel) = rel_under(root, &abs) {
            rel
        } else if let Some((name, rel)) = extra.iter().find_map(|(n, base)| {
            rel_under(base, &abs).map(|r| (n.clone(), r)).or_else(|| (base == &abs).then(|| (n.clone(), String::new())))
        }) {
            if rel.is_empty() {
                name
            } else {
                format!("{name}/{rel}")
            }
        } else {
            let name = format!("@{kf:?}");
            extra.push((name.clone(), abs.clone()));
            name
        };
        if split_rel_path(&path).is_ok() {
            entries.push(KnownFolderEntry { folder: kf, path, source_path: abs.to_string_lossy().into_owned() });
        }
    }
    (entries, extra)
}

/// `abs` relative to `base` as a '/'-separated package path, if it is
/// strictly inside it.
fn rel_under(base: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(base).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let parts = parts?;
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// [`prepare_export`] followed by [`PreparedExport::run`], for callers that
/// handle the code afterwards (tests, scripted use).
pub fn export(
    source: &Path,
    transfers_dir: &Path,
    ticket: &str,
    labels: &SourceLabels,
    opts: &ExportOptions,
    progress: &mut dyn FnMut(&Progress),
) -> Result<(ExportReport, TransferCode), ExportError> {
    let (prepared, code) = prepare_export(source, transfers_dir, ticket, labels, opts)?;
    let report = prepared.run(&code, opts, progress)?;
    Ok((report, code))
}

/// Continue an interrupted export from its last committed point.
pub fn resume_export(
    package_dir: &Path,
    code: &TransferCode,
    opts: &ExportOptions,
    progress: &mut dyn FnMut(&Progress),
) -> Result<ExportReport, ExportError> {
    let (writer, state) = PackageWriter::resume(package_dir, code, opts.writer.clone())?;
    let root = PathBuf::from(&state.begin.source_root);
    if !root.is_dir() {
        return Err(ExportError::SourceMissing(root));
    }
    let extra_roots: Vec<(String, PathBuf)> = state
        .begin
        .known_folders
        .iter()
        .filter(|e| e.path.starts_with('@') && !e.path.contains('/'))
        .map(|e| (e.path.clone(), PathBuf::from(&e.source_path)))
        .collect();
    if let Some((_, missing)) = extra_roots.iter().find(|(_, p)| !p.is_dir()) {
        return Err(ExportError::SourceMissing(missing.clone()));
    }
    let mut job = Job::new(writer, root, extra_roots, opts, progress, state.done);
    if let Some(partial) = state.partial {
        job.resume_partial(&partial)?;
    }
    job.walk_all()?;
    job.finish(code)
}

struct Job<'a> {
    w: PackageWriter,
    root: PathBuf,
    extra_roots: Vec<(String, PathBuf)>,
    opts: &'a ExportOptions,
    progress: &'a mut dyn FnMut(&Progress),
    done: HashSet<String>,
    prog: Progress,
    crash_budget: Option<u64>,
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

impl<'a> Job<'a> {
    fn new(
        w: PackageWriter,
        root: PathBuf,
        extra_roots: Vec<(String, PathBuf)>,
        opts: &'a ExportOptions,
        progress: &'a mut dyn FnMut(&Progress),
        done: HashSet<String>,
    ) -> Self {
        let prog = Progress { files: w.status().files, bytes: w.status().bytes, current: String::new() };
        Self { w, root, extra_roots, opts, progress, done, prog, crash_budget: opts.crash_after_bytes }
    }

    /// The profile, then any standard folders that live outside it.
    fn walk_all(&mut self) -> Result<(), ExportError> {
        let root = self.root.clone();
        self.walk_dir(&root, "")?;
        for (name, abs) in self.extra_roots.clone() {
            if !self.done.contains(&name) {
                let meta =
                    fs::symlink_metadata(&abs).map_err(|e| ExportError::Source { path: abs.clone(), source: e })?;
                self.w.add_dir(&name, meta_of(&meta))?;
            }
            self.walk_dir(&abs, &name)?;
        }
        Ok(())
    }

    /// Source location of a package path written by this export.
    fn source_path(&self, rel: &str) -> Option<PathBuf> {
        let mut comps = rel.split('/');
        let first = comps.next()?;
        let (mut p, rest): (PathBuf, Vec<&str>) = match self.extra_roots.iter().find(|(n, _)| n == first) {
            Some((_, abs)) => (abs.clone(), comps.collect()),
            None => (self.root.clone(), rel.split('/').collect()),
        };
        for c in rest {
            // Package data is untrusted: never let a component act as a
            // separator or stream name on Windows.
            if cfg!(windows) && c.contains(['\\', ':']) {
                return None;
            }
            p.push(c);
        }
        Some(p)
    }

    fn skip(&mut self, rel: &str, reason: SkipReason, detail: &str) -> Result<(), ExportError> {
        if self.done.contains(rel) {
            return Ok(());
        }
        self.w.add_skipped(rel, reason, detail)?;
        Ok(())
    }

    fn walk_dir(&mut self, abs: &Path, rel: &str) -> Result<(), ExportError> {
        let entries = match fs::read_dir(abs) {
            Ok(rd) => rd,
            Err(e) if rel.is_empty() => return Err(ExportError::Source { path: abs.into(), source: e }),
            Err(e) => return self.skip(rel, classify(&e), &format!("folder could not be listed: {e}")),
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let os_name = entry.file_name();
            let Some(name) = os_name.to_str() else {
                let lossy = join_rel(rel, &os_name.to_string_lossy());
                self.skip(&lossy, SkipReason::InvalidName, "name is not valid Unicode")?;
                continue;
            };
            let child = join_rel(rel, name);
            if split_rel_path(&child).is_err() {
                self.skip(&child, SkipReason::InvalidName, "name contains characters that can't be stored safely")?;
                continue;
            }
            if self.opts.exclusions.matches(&child) {
                self.skip(&child, SkipReason::Excluded, "cache or temporary data")?;
                continue;
            }
            let path = entry.path();
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    self.skip(&child, classify(&e), &e.to_string())?;
                    continue;
                }
            };
            let ft = meta.file_type();
            if ft.is_symlink() {
                // Junctions and symlinks are recorded, never followed:
                // profiles contain legacy junctions that loop.
                if !self.done.contains(&child) {
                    let target = fs::read_link(&path).map(|t| t.to_string_lossy().into_owned()).unwrap_or_default();
                    self.w.add_link(&child, &target, meta_of(&meta))?;
                }
            } else if ft.is_dir() {
                if !self.done.contains(&child) {
                    self.w.add_dir(&child, meta_of(&meta))?;
                }
                self.prog.current = child.clone();
                (self.progress)(&self.prog);
                self.walk_dir(&path, &child)?;
            } else if ft.is_file() {
                if self.done.contains(&child) {
                    continue;
                }
                if is_cloud_only(&meta) {
                    self.skip(&child, SkipReason::CloudOnly, "stored in OneDrive, not on this PC")?;
                    continue;
                }
                self.copy_file(&path, &child, &meta)?;
            } else {
                self.skip(&child, SkipReason::ReadError, "not a regular file")?;
            }
        }
        Ok(())
    }

    fn copy_file(&mut self, path: &Path, rel: &str, meta: &Metadata) -> Result<(), ExportError> {
        let before = fingerprint(meta);
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) => return self.skip(rel, classify(&e), &e.to_string()),
        };
        let mut reader = CrashReader { inner: &mut file, budget: &mut self.crash_budget };
        let mut changed = || match fs::symlink_metadata(path) {
            Ok(m) => fingerprint(&m) != before,
            Err(_) => true,
        };
        match self.w.add_file(rel, meta_of(meta), meta.len(), &mut reader, &mut changed) {
            Ok(r) => {
                self.prog.files += 1;
                self.prog.bytes += r.bytes;
                self.copy_streams(path, rel)?;
            }
            Err(PkgError::SourceRead(e)) => self.skip(rel, classify(&e), &format!("read failed: {e}"))?,
            Err(e) => return Err(e.into()),
        }
        self.prog.current = rel.to_string();
        (self.progress)(&self.prog);
        Ok(())
    }

    /// Small alternate data streams (Windows), e.g. the "downloaded from
    /// the internet" marker. File systems without streams have none.
    #[cfg(windows)]
    fn copy_streams(&mut self, path: &Path, rel: &str) -> Result<(), ExportError> {
        let Ok(streams) = crate::win::alt_streams(path) else { return Ok(()) };
        for (name, size) in streams {
            if !profileport_package::format::valid_stream_name(&name) {
                continue;
            }
            if size as usize > PackageWriter::MAX_ALT_STREAM {
                self.w.add_skipped(
                    &format!("{rel}:{name}"),
                    SkipReason::ReadError,
                    &format!("alternate data stream of {size} bytes is too large to store"),
                )?;
                continue;
            }
            match crate::win::read_stream(path, &name) {
                Ok(data) => self.w.add_alt_stream(rel, &name, &data)?,
                Err(e) => self.w.add_skipped(&format!("{rel}:{name}"), classify(&e), &e.to_string())?,
            }
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn copy_streams(&mut self, _: &Path, _: &str) -> Result<(), ExportError> {
        Ok(())
    }

    fn resume_partial(&mut self, p: &PartialFile) -> Result<(), ExportError> {
        let Some(path) = self.source_path(&p.path) else {
            self.w.abandon_partial(p)?;
            return Ok(());
        };
        let unchanged = matches!(
            fs::symlink_metadata(&path),
            Ok(m) if m.is_file() && m.len() == p.size && meta_of(&m).modified == p.meta.modified
        );
        if !unchanged {
            // Start the file over; the walk will add it again.
            self.w.abandon_partial(p)?;
            return Ok(());
        }
        let meta = fs::symlink_metadata(&path).map_err(|e| ExportError::Source { path: path.clone(), source: e })?;
        let before = fingerprint(&meta);
        let opened = File::open(&path).and_then(|mut f| f.seek(SeekFrom::Start(p.bytes_stored())).map(|_| f));
        let mut file = match opened {
            Ok(f) => f,
            Err(_) => {
                self.w.abandon_partial(p)?;
                return Ok(());
            }
        };
        let mut reader = CrashReader { inner: &mut file, budget: &mut self.crash_budget };
        let mut changed = || match fs::symlink_metadata(&path) {
            Ok(m) => fingerprint(&m) != before,
            Err(_) => true,
        };
        match self.w.resume_file(p, &mut reader, &mut changed) {
            Ok(r) => {
                self.prog.files += 1;
                self.prog.bytes += r.bytes;
                self.copy_streams(&path, &p.path)?;
            }
            Err(PkgError::SourceRead(e)) => self.w.add_skipped(&p.path, classify(&e), &format!("read failed: {e}"))?,
            Err(e) => return Err(e.into()),
        }
        self.done.insert(p.path.clone());
        Ok(())
    }

    fn finish(self, code: &TransferCode) -> Result<ExportReport, ExportError> {
        let dir = self.w.dir().to_path_buf();
        let status = self.w.finish()?;
        summarize(&dir, code, status)
    }
}

fn summarize(dir: &Path, code: &TransferCode, status: Status) -> Result<ExportReport, ExportError> {
    let reader = PackageReader::open(dir, code)?;
    let m = reader.manifest()?;
    let mut r = ExportReport {
        package_dir: dir.to_path_buf(),
        ticket: reader.header().ticket.clone(),
        status,
        dirs: 0,
        files: 0,
        bytes: 0,
        links: Vec::new(),
        skipped: Vec::new(),
        excluded: Vec::new(),
        changed: Vec::new(),
        alt_streams: 0,
        known_folders: m.begin.known_folders.clone(),
    };
    for e in m.entries {
        match e {
            Entry::Dir { .. } => r.dirs += 1,
            Entry::File(f) => {
                r.files += 1;
                r.bytes += f.size;
                if f.changed_during_copy {
                    r.changed.push(f.path);
                }
            }
            Entry::Link { path, target, .. } => r.links.push((path, target)),
            Entry::Skipped { path, reason: SkipReason::Excluded, .. } => r.excluded.push(path),
            Entry::Skipped { path, reason, detail } => r.skipped.push(SkipItem { path, reason, detail }),
            Entry::AltStream { .. } => r.alt_streams += 1,
        }
    }
    Ok(r)
}

/// Pass-through reader with an optional simulated crash.
struct CrashReader<'a> {
    inner: &'a mut File,
    budget: &'a mut Option<u64>,
}

impl Read for CrashReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.budget {
            None => self.inner.read(buf),
            Some(0) => panic!("simulated crash (crash_after_bytes)"),
            Some(left) => {
                let max = (*left).min(buf.len() as u64) as usize;
                let n = self.inner.read(&mut buf[..max])?;
                *left -= n as u64;
                Ok(n)
            }
        }
    }
}
