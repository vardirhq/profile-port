//! Wiping a package from the drive once the job is done.
//!
//! A wipe is in three steps, each synced to the drive before the next:
//!
//! 1. **Destroy the key.** `header.bin`, which holds the wrapped package key,
//!    is overwritten in place with a "wiped" marker followed by random bytes.
//!    From then on the package cannot be opened even with its transfer code,
//!    and `read_header` reports it as [`Error::PartlyWiped`].
//! 2. **Optionally overwrite the data.** `manifest.log` and every pack are
//!    overwritten in place with random bytes.
//! 3. **Delete** the packs, the log, the status file and the header, then the
//!    folder itself.
//!
//! A wipe interrupted at any point (drive pulled) can simply be run again.
//!
//! Only files with the exact names a package consists of are ever touched.
//! If the folder holds anything else, nothing at all is changed and
//! [`Error::NotWipeable`] names the first unexpected entry, so pointing a wipe
//! at the wrong folder cannot destroy anything that isn't a package.
//!
//! What this does *not* guarantee: flash drives remap writes internally
//! (wear levelling), so an overwritten block can survive in a spare area that
//! only the drive's controller can reach. Everything in a package is
//! encrypted, so a surviving block is ciphertext; a surviving copy of the old
//! header can only be opened with the transfer code. Discarding the device
//! tag that holds the code is what finally makes such remnants unreadable.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::format::{read_header, HEADER_FILE, LOG_FILE, MAGIC, PACKS_DIR, STATUS_FILE, WIPED_MAGIC};

/// The temporary name `write_atomic` uses for `status.cbor`.
const STATUS_TMP: &str = "status.tmp";

const BLOCK: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct WipeOptions {
    /// Overwrite the manifest and every pack with random bytes before
    /// deleting them. Takes about as long as writing the backup did. Without
    /// it, only the header (and with it the key) is overwritten.
    pub overwrite_data: bool,
}

#[derive(Debug, Clone, Default)]
pub struct WipeReport {
    pub dir: PathBuf,
    /// The ticket, if the header was still readable when the wipe started.
    pub ticket: Option<String>,
    /// The header had already been destroyed by an earlier, interrupted wipe.
    pub was_partly_wiped: bool,
    pub files_removed: u64,
    /// Bytes of manifest and pack data overwritten (0 unless
    /// [`WipeOptions::overwrite_data`]).
    pub bytes_overwritten: u64,
}

/// Progress of the overwrite step: (bytes done, bytes total).
pub type WipeProgress<'a> = &'a mut dyn FnMut(u64, u64);

/// Destroy the package in `dir`. See the module documentation.
pub fn wipe(dir: &Path, opts: &WipeOptions, progress: WipeProgress) -> Result<WipeReport> {
    let inv = inventory(dir)?;
    let mut report = WipeReport { dir: dir.to_path_buf(), ..Default::default() };

    // 1. Destroy the key.
    if let Some(header) = &inv.header {
        match read_header(dir) {
            Ok(h) => report.ticket = Some(h.ticket),
            Err(Error::PartlyWiped) => report.was_partly_wiped = true,
            Err(_) => {} // damaged or newer format: still ours by its magic, still wiped
        }
        destroy_header(header)?;
    }

    // 2. Overwrite the data.
    if opts.overwrite_data {
        let total: u64 = inv.data.iter().map(|(_, len)| len).sum();
        let mut done = 0;
        let mut block = vec![0u8; BLOCK];
        getrandom::getrandom(&mut block).expect("operating system RNG unavailable");
        progress(0, total);
        for (path, len) in &inv.data {
            overwrite(path, *len, &block, &mut |n| {
                done += n;
                progress(done, total);
            })?;
        }
        report.bytes_overwritten = done;
    }

    // 3. Delete, header last so an interrupted wipe stays recognisable.
    for (path, _) in &inv.data {
        remove_file(path, &mut report)?;
    }
    if inv.has_packs_dir {
        remove_dir(&dir.join(PACKS_DIR))?;
    }
    for name in [STATUS_FILE, STATUS_TMP] {
        remove_file(&dir.join(name), &mut report)?;
    }
    if let Some(header) = &inv.header {
        remove_file(header, &mut report)?;
    }
    remove_dir(dir)?;
    Ok(report)
}

struct Inventory {
    header: Option<PathBuf>,
    has_packs_dir: bool,
    /// The manifest log and the packs, with their lengths.
    data: Vec<(PathBuf, u64)>,
}

/// Check that `dir` holds a package and nothing else, without changing it.
fn inventory(dir: &Path) -> Result<Inventory> {
    let meta = fs::symlink_metadata(dir).map_err(|e| match e.kind() {
        ErrorKind::NotFound => Error::NotAPackage,
        _ => e.into(),
    })?;
    if !meta.is_dir() {
        return Err(Error::NotAPackage);
    }
    let mut inv = Inventory { header: None, has_packs_dir: false, data: Vec::new() };
    let mut status_present = false;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        match name.to_str() {
            Some(HEADER_FILE) if kind.is_file() => inv.header = Some(entry.path()),
            Some(STATUS_FILE | STATUS_TMP) if kind.is_file() => status_present = true,
            Some(LOG_FILE) if kind.is_file() => inv.data.push((entry.path(), entry.metadata()?.len())),
            Some(PACKS_DIR) if kind.is_dir() => inv.has_packs_dir = true,
            _ => return Err(Error::NotWipeable(name.to_string_lossy().into_owned())),
        }
    }
    if inv.has_packs_dir {
        let mut packs = Vec::new();
        for entry in fs::read_dir(dir.join(PACKS_DIR))? {
            let entry = entry?;
            let name = entry.file_name();
            if !(entry.file_type()?.is_file() && name.to_str().is_some_and(is_pack_name)) {
                return Err(Error::NotWipeable(format!("{PACKS_DIR}/{}", name.to_string_lossy())));
            }
            packs.push((entry.path(), entry.metadata()?.len()));
        }
        packs.sort();
        inv.data.extend(packs);
    }
    match &inv.header {
        // A header must start with one of our two magics.
        Some(path) => {
            let mut start = [0u8; 8];
            let n = read_prefix(path, &mut start)?;
            if n < 8 || (&start != MAGIC && &start != WIPED_MAGIC) {
                return Err(Error::NotAPackage);
            }
        }
        // No header: an earlier wipe got as far as deleting it, or this is
        // not a package at all. Only an empty folder is safe to remove.
        None if inv.has_packs_dir || status_present || !inv.data.is_empty() => return Err(Error::NotAPackage),
        None => {}
    }
    Ok(inv)
}

fn is_pack_name(name: &str) -> bool {
    name.len() == 11 && name.ends_with(".pack") && name[..6].bytes().all(|b| b.is_ascii_digit())
}

fn read_prefix(path: &Path, buf: &mut [u8]) -> Result<usize> {
    use std::io::Read;
    let mut f = fs::File::open(path)?;
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

/// Overwrite the header in place, keeping its length: the wiped marker, then
/// random bytes over everything else, including the wrapped key.
fn destroy_header(path: &Path) -> Result<()> {
    let len = fs::metadata(path)?.len().max(WIPED_MAGIC.len() as u64) as usize;
    let mut bytes = vec![0u8; len];
    bytes[..WIPED_MAGIC.len()].copy_from_slice(WIPED_MAGIC);
    getrandom::getrandom(&mut bytes[WIPED_MAGIC.len()..]).expect("operating system RNG unavailable");
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    Ok(())
}

fn overwrite(path: &Path, len: u64, block: &[u8], done: &mut dyn FnMut(u64)) -> Result<()> {
    let mut f = OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(0))?;
    let mut left = len;
    while left > 0 {
        let n = left.min(block.len() as u64) as usize;
        f.write_all(&block[..n])?;
        left -= n as u64;
        done(n as u64);
    }
    f.sync_all()?;
    Ok(())
}

fn remove_file(path: &Path, report: &mut WipeReport) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            report.files_removed += 1;
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn remove_dir(path: &Path) -> Result<()> {
    match fs::remove_dir(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_names() {
        assert!(is_pack_name("000000.pack"));
        assert!(is_pack_name("123456.pack"));
        assert!(!is_pack_name("00000.pack"));
        assert!(!is_pack_name("abcdef.pack"));
        assert!(!is_pack_name("000000.pack.bak"));
        assert!(!is_pack_name("notes.txt"));
    }
}
