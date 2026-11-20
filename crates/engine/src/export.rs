//! Exporting a profile folder into a package, and resuming an export.

use std::collections::HashSet;
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use profileport_package::format::{split_rel_path, validate_ticket};
use profileport_package::{
    BeginInfo, Entry, Error as PkgError, PackageReader, PackageWriter, PartialFile, SkipReason, Status, TransferCode,
    WriterOptions,
};

use crate::exclude::Exclusions;
use crate::platform::{classify, fingerprint, is_cloud_only, meta_of};
use crate::{native_path, Progress, SkipItem};

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub writer: WriterOptions,
    pub exclusions: Exclusions,
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
        let root = self.root.clone();
        let mut job = Job::new(self.writer, root.clone(), opts, progress, HashSet::new());
        job.walk_dir(&root, "")?;
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
    let begin = BeginInfo {
        source_root: root.to_string_lossy().into_owned(),
        computer: labels.computer.clone(),
        profile: labels.profile.clone(),
        started_unix: (stamp / 1_000_000_000) as i64,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let (writer, code) = PackageWriter::create(&dir, ticket, begin, opts.writer.clone())?;
    Ok((PreparedExport { writer, root }, code))
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
    let mut job = Job::new(writer, root.clone(), opts, progress, state.done);
    if let Some(partial) = state.partial {
        job.resume_partial(&partial)?;
    }
    job.walk_dir(&root, "")?;
    job.finish(code)
}

struct Job<'a> {
    w: PackageWriter,
    root: PathBuf,
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
        opts: &'a ExportOptions,
        progress: &'a mut dyn FnMut(&Progress),
        done: HashSet<String>,
    ) -> Self {
        let prog = Progress { files: w.status().files, bytes: w.status().bytes, current: String::new() };
        Self { w, root, opts, progress, done, prog, crash_budget: opts.crash_after_bytes }
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
            }
            Err(PkgError::SourceRead(e)) => self.skip(rel, classify(&e), &format!("read failed: {e}"))?,
            Err(e) => return Err(e.into()),
        }
        self.prog.current = rel.to_string();
        (self.progress)(&self.prog);
        Ok(())
    }

    fn resume_partial(&mut self, p: &PartialFile) -> Result<(), ExportError> {
        let path = native_path(&self.root, &p.path);
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
