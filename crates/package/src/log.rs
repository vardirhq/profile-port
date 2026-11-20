//! The encrypted, append-only manifest log.
//!
//! Each frame is `u32 LE length ‖ seal(CBOR(record))`, authenticated with
//! the package id and the frame's index, so frames cannot be reordered,
//! dropped from the middle, or moved between packages without detection.
//!
//! A frame that is cut short or fails authentication **at the very end** of
//! the file is treated as a torn write from a pulled drive and discarded.
//! Anything wrong earlier in the file is corruption.

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::crypto::{open, seal, Key, OVERHEAD};
use crate::error::{Error, Result};
use crate::format::{from_cbor, to_cbor, Record};

const MAX_FRAME: u64 = 64 << 20;

fn frame_aad(package_id: &[u8; 16], index: u64) -> Vec<u8> {
    let mut aad = b"profileport log v1".to_vec();
    aad.extend_from_slice(package_id);
    aad.extend_from_slice(&index.to_le_bytes());
    aad
}

pub(crate) struct LogWriter {
    file: BufWriter<File>,
    package_id: [u8; 16],
    next_index: u64,
}

impl LogWriter {
    pub fn create(path: &Path, package_id: [u8; 16]) -> Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Self { file: BufWriter::new(file), package_id, next_index: 0 })
    }

    /// Reopen for appending after discarding anything past `valid_len`.
    pub fn reopen(path: &Path, package_id: [u8; 16], valid_len: u64, next_index: u64) -> Result<Self> {
        let mut file = OpenOptions::new().write(true).open(path)?;
        file.set_len(valid_len)?;
        file.sync_all()?;
        file.seek(SeekFrom::End(0))?;
        Ok(Self { file: BufWriter::new(file), package_id, next_index })
    }

    pub fn append(&mut self, key: &Key, record: &Record) -> Result<()> {
        let sealed = seal(key, &frame_aad(&self.package_id, self.next_index), &to_cbor(record));
        self.file.write_all(&(sealed.len() as u32).to_le_bytes())?;
        self.file.write_all(&sealed)?;
        self.next_index += 1;
        Ok(())
    }

    pub fn sync(&mut self) -> Result<()> {
        self.file.flush()?;
        self.file.get_ref().sync_data()?;
        Ok(())
    }
}

pub(crate) struct LogContents {
    pub records: Vec<Record>,
    /// Length of the intact prefix; anything after it is a torn tail.
    pub valid_len: u64,
}

pub(crate) fn read_log(path: &Path, key: &Key, package_id: &[u8; 16]) -> Result<LogContents> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut r = BufReader::new(file);
    let mut records = Vec::new();
    let mut pos = 0u64;
    loop {
        if len - pos < 4 {
            break; // clean end, or a torn length prefix
        }
        let mut len_bytes = [0u8; 4];
        r.read_exact(&mut len_bytes)?;
        let frame_len = u32::from_le_bytes(len_bytes) as u64;
        let frame_end = pos + 4 + frame_len;
        if frame_end > len {
            break; // torn frame at the end
        }
        if frame_len > MAX_FRAME || frame_len < OVERHEAD as u64 {
            return Err(Error::Corrupt(format!("manifest frame {} has an invalid length", records.len())));
        }
        let mut sealed = vec![0u8; frame_len as usize];
        r.read_exact(&mut sealed)?;
        match open(key, &frame_aad(package_id, records.len() as u64), &sealed) {
            Some(plain) => records.push(from_cbor(&plain)?),
            None if frame_end == len => break, // torn write of the last frame
            None => return Err(Error::Corrupt(format!("manifest frame {} failed authentication", records.len()))),
        }
        pos = frame_end;
    }
    Ok(LogContents { records, valid_len: pos })
}
