//! Reading packages: plaintext info without a code, contents with one.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::code::TransferCode;
use crate::crypto::Key;
use crate::error::{Error, Result};
use crate::format::*;
use crate::log::read_log;
use crate::writer::{read_chunk, unlock};

/// What the dashboard can show without a transfer code.
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub dir: PathBuf,
    pub header: Header,
    pub status: Option<Status>,
}

pub fn read_info(dir: &Path) -> Result<PackageInfo> {
    Ok(PackageInfo { dir: dir.to_path_buf(), header: read_header(dir)?, status: read_status(dir)? })
}

/// All packages under a transfers directory, newest first, each with its
/// folder. Folders that are not packages are ignored; damaged and partly
/// wiped packages are returned as errors, listed first.
pub fn list_packages(transfers_dir: &Path) -> std::io::Result<Vec<(PathBuf, Result<PackageInfo>)>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(transfers_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        match read_info(&entry.path()) {
            Err(Error::NotAPackage) => {}
            other => out.push((entry.path(), other)),
        }
    }
    out.sort_by_key(|(_, r)| std::cmp::Reverse(r.as_ref().map(|i| i.header.created_unix).unwrap_or(i64::MAX)));
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub id: u64,
    pub path: String,
    pub meta: Meta,
    /// Bytes actually stored.
    pub size: u64,
    pub chunks: Vec<ChunkRef>,
    pub blake3: [u8; 32],
    pub changed_during_copy: bool,
}

#[derive(Debug, Clone)]
pub enum Entry {
    Dir {
        path: String,
        meta: Meta,
    },
    File(FileEntry),
    Link {
        path: String,
        target: String,
        meta: Meta,
    },
    Skipped {
        path: String,
        reason: SkipReason,
        detail: String,
    },
    /// Alternate data stream belonging to the file at `path`.
    AltStream {
        path: String,
        name: String,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub begin: BeginInfo,
    pub entries: Vec<Entry>,
    /// The export finished. If false the package was interrupted.
    pub complete: bool,
    /// Files that were being written when the export stopped.
    pub incomplete_files: Vec<String>,
}

pub struct PackageReader {
    dir: PathBuf,
    header: Header,
    key: Key,
}

impl PackageReader {
    pub fn open(dir: &Path, code: &TransferCode) -> Result<Self> {
        let header = read_header(dir)?;
        let key = unlock(&header, code)?;
        Ok(Self { dir: dir.to_path_buf(), header, key })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn manifest(&self) -> Result<Manifest> {
        let contents = read_log(&self.dir.join(LOG_FILE), &self.key, &self.header.package_id)?;
        let mut begin = None;
        let mut complete = false;
        let mut slots: Vec<Option<Entry>> = Vec::new();
        let mut open_files: HashMap<u64, usize> = HashMap::new();

        for rec in contents.records {
            match rec {
                Record::Begin(b) => begin = Some(b),
                Record::Dir { path, meta } => slots.push(Some(Entry::Dir { path, meta })),
                Record::Link { path, target, meta } => slots.push(Some(Entry::Link { path, target, meta })),
                Record::Skipped { path, reason, detail } => slots.push(Some(Entry::Skipped { path, reason, detail })),
                Record::AltStream { path, name, data } => slots.push(Some(Entry::AltStream { path, name, data })),
                Record::FileStart { id, path, meta, .. } => {
                    open_files.insert(id, slots.len());
                    slots.push(Some(Entry::File(FileEntry {
                        id,
                        path,
                        meta,
                        size: 0,
                        chunks: Vec::new(),
                        blake3: [0; 32],
                        changed_during_copy: false,
                    })));
                }
                Record::Chunks { id, chunks } => {
                    let slot = open_files.get(&id).copied().ok_or_else(|| orphan(id))?;
                    if let Some(Entry::File(f)) = &mut slots[slot] {
                        f.size += chunks.iter().map(|c| c.plain_len as u64).sum::<u64>();
                        f.chunks.extend(chunks);
                    }
                }
                Record::FileEnd { id, blake3, changed_during_copy } => {
                    let slot = open_files.remove(&id).ok_or_else(|| orphan(id))?;
                    if let Some(Entry::File(f)) = &mut slots[slot] {
                        f.blake3 = blake3;
                        f.changed_during_copy = changed_during_copy;
                    }
                }
                Record::FileAbort { id } => {
                    let slot = open_files.remove(&id).ok_or_else(|| orphan(id))?;
                    slots[slot] = None;
                }
                Record::End { .. } => complete = true,
            }
        }
        let mut incomplete_files = Vec::new();
        for (_, slot) in open_files {
            if let Some(Entry::File(f)) = slots[slot].take() {
                incomplete_files.push(f.path);
            }
        }
        incomplete_files.sort();
        Ok(Manifest {
            begin: begin.ok_or_else(|| Error::Corrupt("manifest has no beginning record".into()))?,
            entries: slots.into_iter().flatten().collect(),
            complete,
            incomplete_files,
        })
    }

    /// Decrypt a file into `out`, verifying every chunk and the whole-file
    /// hash. On error, `out` may hold partial data: write to a temporary
    /// location and only keep it on success.
    pub fn read_file(&self, f: &FileEntry, out: &mut dyn Write) -> Result<()> {
        let mut hasher = blake3::Hasher::new();
        let mut cache: Option<(u32, File)> = None;
        for (i, c) in f.chunks.iter().enumerate() {
            let plain = read_chunk(&self.dir, &self.header, &self.key, &mut cache, f.id, i as u64, c)?;
            hasher.update(&plain);
            out.write_all(&plain)?;
        }
        if hasher.finalize().as_bytes() != &f.blake3 {
            return Err(Error::Corrupt(format!("content hash mismatch for {:?}", f.path)));
        }
        Ok(())
    }
}

fn orphan(id: u64) -> Error {
    Error::Corrupt(format!("manifest refers to unknown file {id}"))
}
