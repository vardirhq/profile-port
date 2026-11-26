//! Creating and resuming packages.
//!
//! Durability rule: a manifest record that refers to chunk data is only
//! written to the log *after* the pack file holding that data has been
//! synced. Records are therefore buffered and flushed in batches
//! ([`PackageWriter::commit`]): sync packs → append records → sync log.
//! After a crash or pulled drive, everything up to the last commit is
//! intact and [`PackageWriter::resume`] continues from there.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use crate::code::TransferCode;
use crate::crypto::{derive_kek, open, random_bytes, seal, KdfParams, Key, OVERHEAD};
use crate::error::{Error, Result};
use crate::format::*;
use crate::log::{read_log, LogWriter};

#[derive(Debug, Clone)]
pub struct WriterOptions {
    /// Plaintext bytes per chunk.
    pub chunk_size: u32,
    /// Start a new pack file after this many bytes.
    pub pack_size: u64,
    /// Commit (fsync) after this many new chunk bytes…
    pub commit_bytes: u64,
    /// …or this many buffered records, whichever comes first.
    pub commit_records: usize,
    pub kdf: KdfParams,
}

impl Default for WriterOptions {
    fn default() -> Self {
        Self {
            chunk_size: 4 << 20,
            pack_size: 64 << 20,
            commit_bytes: 32 << 20,
            commit_records: 512,
            kdf: KdfParams::recommended(),
        }
    }
}

/// A file that was being written when the previous session stopped.
#[derive(Debug, Clone)]
pub struct PartialFile {
    pub id: u64,
    pub path: String,
    pub meta: Meta,
    pub size: u64,
    pub(crate) chunks: Vec<ChunkRef>,
}

impl PartialFile {
    /// Source offset to continue reading from.
    pub fn bytes_stored(&self) -> u64 {
        self.chunks.iter().map(|c| c.plain_len as u64).sum()
    }
}

#[derive(Debug)]
pub struct ResumeState {
    pub begin: BeginInfo,
    /// Paths already fully handled (stored, linked or skipped).
    pub done: HashSet<String>,
    pub partial: Option<PartialFile>,
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub blake3: [u8; 32],
    pub bytes: u64,
    pub changed_during_copy: bool,
}

pub struct PackageWriter {
    dir: PathBuf,
    header: Header,
    key: Key,
    opts: WriterOptions,
    log: LogWriter,
    pack: Option<File>,
    pack_index: u32,
    pack_len: u64,
    pending: Vec<Record>,
    pending_bytes: u64,
    next_file_id: u64,
    status: Status,
    changed_files: u64,
}

pub(crate) fn unlock(header: &Header, code: &TransferCode) -> Result<Key> {
    let kek = derive_kek(code, &header.kdf)?;
    let raw = Zeroizing::new(open(&kek, &header.key_aad(), &header.wrapped_key).ok_or(Error::WrongCode)?);
    Key::from_bytes(&raw).ok_or_else(|| Error::Corrupt("wrapped key has the wrong length".into()))
}

pub(crate) fn read_chunk(
    dir: &Path,
    header: &Header,
    key: &Key,
    file: &mut Option<(u32, File)>,
    file_id: u64,
    index: u64,
    c: &ChunkRef,
) -> Result<Vec<u8>> {
    if c.stored_len as usize > header.chunk_size as usize + OVERHEAD {
        return Err(Error::Corrupt(format!("chunk {index} of file {file_id} is oversized")));
    }
    if file.as_ref().map(|(i, _)| *i) != Some(c.pack) {
        *file = Some((c.pack, File::open(dir.join(PACKS_DIR).join(pack_file_name(c.pack)))?));
    }
    let f = &mut file.as_mut().unwrap().1;
    f.seek(SeekFrom::Start(c.offset))?;
    let mut sealed = vec![0u8; c.stored_len as usize];
    f.read_exact(&mut sealed).map_err(|e| match e.kind() {
        ErrorKind::UnexpectedEof => Error::Corrupt(format!("pack {} is truncated", c.pack)),
        _ => e.into(),
    })?;
    let plain = open(key, &chunk_aad(&header.package_id, file_id, index), &sealed)
        .ok_or_else(|| Error::Corrupt(format!("chunk {index} of file {file_id} failed authentication")))?;
    if plain.len() != c.plain_len as usize {
        return Err(Error::Corrupt(format!("chunk {index} of file {file_id} has the wrong length")));
    }
    Ok(plain)
}

/// Fill `buf` as far as possible; returns bytes read (short only at EOF).
fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

impl PackageWriter {
    /// Create a new package in `dir` (which must not exist yet).
    pub fn create(dir: &Path, ticket: &str, begin: BeginInfo, opts: WriterOptions) -> Result<(Self, TransferCode)> {
        validate_ticket(ticket)?;
        if opts.chunk_size == 0 {
            return Err(Error::Corrupt("chunk size must be positive".into()));
        }
        fs::create_dir(dir)?;
        fs::create_dir(dir.join(PACKS_DIR))?;

        let code = TransferCode::generate();
        let key = Key::random();
        let mut header = Header {
            package_id: random_bytes(),
            ticket: ticket.to_string(),
            created_unix: now_unix(),
            kdf: opts.kdf.clone(),
            wrapped_key: Vec::new(),
            chunk_size: opts.chunk_size,
        };
        let kek = derive_kek(&code, &header.kdf)?;
        header.wrapped_key = seal(&kek, &header.key_aad(), key.as_bytes());
        write_atomic(&dir.join(HEADER_FILE), &encode_header(&header))?;

        let log = LogWriter::create(&dir.join(LOG_FILE), header.package_id)?;
        let mut w = Self {
            dir: dir.to_path_buf(),
            header,
            key,
            opts,
            log,
            pack: None,
            pack_index: 0,
            pack_len: 0,
            pending: vec![Record::Begin(begin)],
            pending_bytes: 0,
            next_file_id: 0,
            status: Status {
                state: PackageState::InProgress,
                files: 0,
                bytes: 0,
                skipped: 0,
                updated_unix: now_unix(),
            },
            changed_files: 0,
        };
        w.commit()?;
        Ok((w, code))
    }

    /// Reopen an unfinished package. Discards anything written after the
    /// last commit and reports what is already done.
    pub fn resume(dir: &Path, code: &TransferCode, opts: WriterOptions) -> Result<(Self, ResumeState)> {
        let header = read_header(dir)?;
        let key = unlock(&header, code)?;
        let log_path = dir.join(LOG_FILE);
        let contents = read_log(&log_path, &key, &header.package_id)?;

        let mut begin = None;
        let mut done = HashSet::new();
        let mut starts: HashMap<u64, (String, Meta, u64)> = HashMap::new();
        let mut chunks: HashMap<u64, Vec<ChunkRef>> = HashMap::new();
        let mut ended = HashSet::new();
        let mut next_file_id = 0u64;
        let mut pack_end: (u32, u64) = (0, 0);
        let mut status =
            Status { state: PackageState::InProgress, files: 0, bytes: 0, skipped: 0, updated_unix: now_unix() };
        let mut changed_files = 0;

        for rec in &contents.records {
            match rec {
                Record::Begin(b) => begin = Some(b.clone()),
                Record::Dir { path, .. } | Record::Link { path, .. } => {
                    done.insert(path.clone());
                }
                Record::FileStart { id, path, meta, size } => {
                    starts.insert(*id, (path.clone(), meta.clone(), *size));
                    next_file_id = next_file_id.max(id + 1);
                }
                Record::Chunks { id, chunks: c } => {
                    for ch in c {
                        pack_end = pack_end.max((ch.pack, ch.offset + ch.stored_len as u64));
                    }
                    chunks.entry(*id).or_default().extend_from_slice(c);
                }
                Record::FileEnd { id, changed_during_copy, .. } => {
                    ended.insert(*id);
                    if let Some((path, _, _)) = starts.get(id) {
                        done.insert(path.clone());
                        status.files += 1;
                        status.bytes += chunks.get(id).map_or(0, |c| c.iter().map(|c| c.plain_len as u64).sum());
                    }
                    if *changed_during_copy {
                        changed_files += 1;
                    }
                }
                Record::FileAbort { id } => {
                    ended.insert(*id);
                }
                Record::Skipped { path, reason, .. } => {
                    done.insert(path.clone());
                    if *reason != SkipReason::Excluded {
                        status.skipped += 1;
                    }
                }
                Record::AltStream { .. } => {}
                Record::End { .. } => return Err(Error::AlreadyComplete),
            }
        }
        let begin = begin.ok_or_else(|| Error::Corrupt("manifest has no beginning record".into()))?;
        let partial = starts.iter().filter(|(id, _)| !ended.contains(*id)).max_by_key(|(id, _)| **id).map(
            |(id, (path, meta, size))| PartialFile {
                id: *id,
                path: path.clone(),
                meta: meta.clone(),
                size: *size,
                chunks: chunks.remove(id).unwrap_or_default(),
            },
        );

        // Drop pack data that no committed record refers to.
        let packs = dir.join(PACKS_DIR);
        for entry in fs::read_dir(&packs)? {
            let entry = entry?;
            let idx =
                entry.file_name().to_str().and_then(|n| n.strip_suffix(".pack")).and_then(|n| n.parse::<u32>().ok());
            if let Some(idx) = idx {
                if idx > pack_end.0 {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        let mut pack =
            OpenOptions::new().create(true).write(true).truncate(false).open(packs.join(pack_file_name(pack_end.0)))?;
        pack.set_len(pack_end.1)?;
        pack.sync_all()?;
        pack.seek(SeekFrom::End(0))?;

        let log = LogWriter::reopen(&log_path, header.package_id, contents.valid_len, contents.records.len() as u64)?;
        let w = Self {
            dir: dir.to_path_buf(),
            header,
            key,
            opts, // chunk size comes from the header, not opts
            log,
            pack: Some(pack),
            pack_index: pack_end.0,
            pack_len: pack_end.1,
            pending: Vec::new(),
            pending_bytes: 0,
            next_file_id,
            status,
            changed_files,
        };
        Ok((w, ResumeState { begin, done, partial }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    fn chunk_size(&self) -> usize {
        self.header.chunk_size as usize
    }

    pub fn add_dir(&mut self, path: &str, meta: Meta) -> Result<()> {
        split_rel_path(path)?;
        self.push(Record::Dir { path: path.to_string(), meta })
    }

    pub fn add_link(&mut self, path: &str, target: &str, meta: Meta) -> Result<()> {
        split_rel_path(path)?;
        self.push(Record::Link { path: path.to_string(), target: target.to_string(), meta })
    }

    /// Record something that was deliberately or unavoidably not copied.
    /// The path is informational only (it may be a lossy rendering of an
    /// unrepresentable name) and is never restored.
    pub fn add_skipped(&mut self, path: &str, reason: SkipReason, detail: &str) -> Result<()> {
        if reason != SkipReason::Excluded {
            self.status.skipped += 1;
        }
        self.push(Record::Skipped { path: path.to_string(), reason, detail: detail.to_string() })
    }

    /// Largest alternate data stream stored inline in the manifest.
    pub const MAX_ALT_STREAM: usize = 64 * 1024;

    /// Attach a small alternate data stream to a file already added.
    pub fn add_alt_stream(&mut self, path: &str, name: &str, data: &[u8]) -> Result<()> {
        split_rel_path(path)?;
        if !valid_stream_name(name) || data.len() > Self::MAX_ALT_STREAM {
            return Err(Error::UnsafePath(format!("{path}:{name}")));
        }
        self.push(Record::AltStream { path: path.to_string(), name: name.to_string(), data: data.to_vec() })
    }

    /// Store a file read from `reader`. `changed` is called after the last
    /// byte is read and should report whether the source changed meanwhile.
    ///
    /// A read error from `reader` yields [`Error::SourceRead`]; the file is
    /// recorded as aborted and the package remains consistent.
    pub fn add_file(
        &mut self,
        path: &str,
        meta: Meta,
        size: u64,
        reader: &mut dyn Read,
        changed: &mut dyn FnMut() -> bool,
    ) -> Result<FileResult> {
        split_rel_path(path)?;
        let id = self.next_file_id;
        self.next_file_id += 1;
        self.pending.push(Record::FileStart { id, path: path.to_string(), meta, size });
        self.write_body(id, reader, blake3::Hasher::new(), Vec::new(), changed)
    }

    /// Continue a [`PartialFile`]. `reader` must be positioned at
    /// [`PartialFile::bytes_stored`] in the (unchanged) source file.
    pub fn resume_file(
        &mut self,
        partial: &PartialFile,
        reader: &mut dyn Read,
        changed: &mut dyn FnMut() -> bool,
    ) -> Result<FileResult> {
        if let Some(p) = &self.pack {
            p.sync_data()?;
        }
        let mut hasher = blake3::Hasher::new();
        let mut cache = None;
        for (i, c) in partial.chunks.iter().enumerate() {
            let plain = read_chunk(&self.dir, &self.header, &self.key, &mut cache, partial.id, i as u64, c)?;
            hasher.update(&plain);
        }
        self.write_body(partial.id, reader, hasher, partial.chunks.clone(), changed)
    }

    /// Give up on a partial file (for example because the source changed);
    /// the caller will typically add it again from the start.
    pub fn abandon_partial(&mut self, partial: &PartialFile) -> Result<()> {
        self.push(Record::FileAbort { id: partial.id })
    }

    fn write_body(
        &mut self,
        id: u64,
        reader: &mut dyn Read,
        mut hasher: blake3::Hasher,
        existing: Vec<ChunkRef>,
        changed: &mut dyn FnMut() -> bool,
    ) -> Result<FileResult> {
        let cs = self.chunk_size();
        let mut buf = Zeroizing::new(vec![0u8; cs]);
        let mut index = existing.len() as u64;
        let mut total: u64 = existing.iter().map(|c| c.plain_len as u64).sum();
        let mut batch = Vec::new();
        loop {
            let n = match read_full(reader, &mut buf) {
                Ok(n) => n,
                Err(e) => {
                    // Chunks already in the pack become unreferenced garbage.
                    self.push(Record::FileAbort { id })?;
                    return Err(Error::SourceRead(e));
                }
            };
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            let sealed = seal(&self.key, &chunk_aad(&self.header.package_id, id, index), &buf[..n]);
            let (pack, offset) = self.write_to_pack(&sealed)?;
            batch.push(ChunkRef { pack, offset, stored_len: sealed.len() as u32, plain_len: n as u32 });
            index += 1;
            total += n as u64;
            self.pending_bytes += n as u64;
            if self.pending_bytes >= self.opts.commit_bytes {
                self.pending.push(Record::Chunks { id, chunks: std::mem::take(&mut batch) });
                self.commit()?;
            }
            if n < cs {
                break;
            }
        }
        if !batch.is_empty() {
            self.pending.push(Record::Chunks { id, chunks: batch });
        }
        let changed_during_copy = changed();
        let blake3 = *hasher.finalize().as_bytes();
        self.status.files += 1;
        self.status.bytes += total;
        if changed_during_copy {
            self.changed_files += 1;
        }
        self.push(Record::FileEnd { id, blake3, changed_during_copy })?;
        Ok(FileResult { blake3, bytes: total, changed_during_copy })
    }

    fn write_to_pack(&mut self, sealed: &[u8]) -> Result<(u32, u64)> {
        if self.pack.is_some() && self.pack_len > 0 && self.pack_len + sealed.len() as u64 > self.opts.pack_size {
            // Pending records may refer to this pack: make it durable first.
            self.pack.take().unwrap().sync_data()?;
            self.pack_index += 1;
            self.pack_len = 0;
        }
        if self.pack.is_none() {
            let path = self.dir.join(PACKS_DIR).join(pack_file_name(self.pack_index));
            self.pack = Some(OpenOptions::new().create(true).write(true).truncate(true).open(path)?);
        }
        let offset = self.pack_len;
        self.pack.as_mut().unwrap().write_all(sealed)?;
        self.pack_len += sealed.len() as u64;
        Ok((self.pack_index, offset))
    }

    fn push(&mut self, rec: Record) -> Result<()> {
        self.pending.push(rec);
        if self.pending.len() >= self.opts.commit_records || self.pending_bytes >= self.opts.commit_bytes {
            self.commit()?;
        }
        Ok(())
    }

    /// Make everything written so far durable.
    pub fn commit(&mut self) -> Result<()> {
        if let Some(p) = &self.pack {
            p.sync_data()?;
        }
        let pending = std::mem::take(&mut self.pending);
        for rec in &pending {
            self.log.append(&self.key, rec)?;
        }
        self.log.sync()?;
        self.pending_bytes = 0;
        self.status.updated_unix = now_unix();
        write_atomic(&self.dir.join(STATUS_FILE), &to_cbor(&self.status))
    }

    /// Delete a package that no file data has been written to, for example
    /// when the technician cancels before the copy starts.
    ///
    /// Only the files [`PackageWriter::create`] makes are removed, one by
    /// one, and directories are removed only when empty. If the package
    /// holds pack data or anything unexpected, nothing is deleted and an
    /// error is returned.
    pub fn discard(self) -> Result<()> {
        if self.pack.is_some() || self.pack_index != 0 || self.next_file_id != 0 {
            return Err(Error::NotDiscardable);
        }
        let dir = self.dir.clone();
        drop(self); // close the log before deleting it (required on Windows)
        let packs = dir.join(PACKS_DIR);
        let expected = [HEADER_FILE, STATUS_FILE, LOG_FILE, PACKS_DIR];
        for entry in fs::read_dir(&dir)? {
            if !expected.iter().any(|n| entry.as_ref().is_ok_and(|e| e.file_name() == *n)) {
                return Err(Error::NotDiscardable);
            }
        }
        if fs::read_dir(&packs)?.next().is_some() {
            return Err(Error::NotDiscardable);
        }
        for name in [HEADER_FILE, STATUS_FILE, LOG_FILE] {
            match fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        fs::remove_dir(&packs)?;
        fs::remove_dir(&dir)?;
        Ok(())
    }

    /// Mark the package complete.
    pub fn finish(mut self) -> Result<Status> {
        self.pending.push(Record::End { finished_unix: now_unix() });
        self.status.state = if self.status.skipped > 0 || self.changed_files > 0 {
            PackageState::CompleteWithIssues
        } else {
            PackageState::Complete
        };
        self.commit()?;
        Ok(self.status.clone())
    }
}
