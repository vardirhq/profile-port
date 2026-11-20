use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use profileport_package::format::{LOG_FILE, PACKS_DIR};
use profileport_package::*;

fn opts() -> WriterOptions {
    WriterOptions {
        chunk_size: 1000,
        pack_size: 4096,
        commit_bytes: 2500,
        commit_records: 8,
        kdf: KdfParams::insecure_fast_for_tests(),
    }
}

fn begin() -> BeginInfo {
    BeginInfo {
        source_root: "/src".into(),
        computer: "PC-1".into(),
        profile: "anna".into(),
        started_unix: 0,
        tool_version: "test".into(),
        ..Default::default()
    }
}

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

fn unchanged() -> impl FnMut() -> bool {
    || false
}

fn files_of(m: &Manifest) -> Vec<&FileEntry> {
    m.entries.iter().filter_map(|e| if let Entry::File(f) = e { Some(f) } else { None }).collect()
}

fn read_all(r: &PackageReader, f: &FileEntry) -> Vec<u8> {
    let mut out = Vec::new();
    r.read_file(f, &mut out).unwrap();
    out
}

#[test]
fn roundtrip_many_sizes_across_packs() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&dir, "48231", begin(), opts()).unwrap();

    let sizes = [0usize, 1, 999, 1000, 1001, 2000, 9_999];
    w.add_dir("Documents", Meta::default()).unwrap();
    for (i, &n) in sizes.iter().enumerate() {
        let d = data(n, i as u8);
        let r = w
            .add_file(&format!("Documents/f{i}.bin"), Meta::default(), n as u64, &mut Cursor::new(&d), &mut unchanged())
            .unwrap();
        assert_eq!(r.bytes, n as u64);
        assert_eq!(r.blake3, *blake3::hash(&d).as_bytes());
    }
    w.add_link("Application Data", "AppData/Roaming", Meta::default()).unwrap();
    w.add_skipped("AppData/Local/Temp", SkipReason::Excluded, "cache").unwrap();
    let status = w.finish().unwrap();
    assert_eq!(status.state, PackageState::Complete);
    assert_eq!(status.files, sizes.len() as u64);
    assert!(fs::read_dir(dir.join(PACKS_DIR)).unwrap().count() > 3, "should span several packs");

    let info = read_info(&dir).unwrap();
    assert_eq!(info.header.ticket, "48231");
    assert_eq!(info.status.unwrap().state, PackageState::Complete);

    let reader = PackageReader::open(&dir, &TransferCode::parse(&code.to_string()).unwrap()).unwrap();
    let m = reader.manifest().unwrap();
    assert!(m.complete);
    assert_eq!(m.begin.profile, "anna");
    let files = files_of(&m);
    assert_eq!(files.len(), sizes.len());
    for (i, f) in files.iter().enumerate() {
        assert_eq!(read_all(&reader, f), data(sizes[i], i as u8), "{}", f.path);
    }
    assert!(m.entries.iter().any(|e| matches!(e, Entry::Link { target, .. } if target == "AppData/Roaming")));
}

#[test]
fn wrong_code_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (w, _code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    w.finish().unwrap();
    assert!(matches!(PackageReader::open(&dir, &TransferCode::generate()), Err(Error::WrongCode)));
}

#[test]
fn plaintext_contains_no_names_or_content() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, _code) = PackageWriter::create(&dir, "777", begin(), opts()).unwrap();
    let secret = b"SECRET-CONTENT-SECRET-CONTENT".repeat(10);
    w.add_file(
        "Documents/TaxReturn-Anna.pdf",
        Meta::default(),
        secret.len() as u64,
        &mut Cursor::new(&secret),
        &mut unchanged(),
    )
    .unwrap();
    w.finish().unwrap();

    let mut everything = Vec::new();
    for entry in walk(&dir) {
        everything.extend(fs::read(entry).unwrap());
    }
    let hay = String::from_utf8_lossy(&everything);
    for needle in ["TaxReturn", "Documents", "anna", "PC-1", "SECRET-CONTENT"] {
        assert!(!hay.contains(needle), "plaintext leak: {needle}");
    }
    assert!(hay.contains("777"), "ticket is the intended plaintext label");
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p))
        } else {
            out.push(p)
        }
    }
    out
}

#[test]
fn tampered_chunk_is_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    let d = data(3000, 7);
    w.add_file("a.bin", Meta::default(), 3000, &mut Cursor::new(&d), &mut unchanged()).unwrap();
    w.finish().unwrap();

    let pack = dir.join(PACKS_DIR).join("000000.pack");
    let mut f = OpenOptions::new().read(true).write(true).open(&pack).unwrap();
    f.seek(SeekFrom::Start(100)).unwrap();
    let mut b = [0u8; 1];
    f.read_exact(&mut b).unwrap();
    f.seek(SeekFrom::Start(100)).unwrap();
    f.write_all(&[b[0] ^ 0x40]).unwrap();
    drop(f);

    let reader = PackageReader::open(&dir, &code).unwrap();
    let m = reader.manifest().unwrap();
    let err = reader.read_file(files_of(&m)[0], &mut Vec::new()).unwrap_err();
    assert!(matches!(err, Error::Corrupt(_)), "{err}");
}

#[test]
fn relabelled_ticket_breaks_the_code() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (w, code) = PackageWriter::create(&dir, "11111", begin(), opts()).unwrap();
    w.finish().unwrap();
    let header = dir.join(format::HEADER_FILE);
    let bytes = fs::read(&header).unwrap();
    let pos = bytes.windows(5).position(|w| w == b"11111").unwrap();
    let mut changed = bytes.clone();
    changed[pos..pos + 5].copy_from_slice(b"22222");
    fs::write(&header, changed).unwrap();
    assert!(matches!(PackageReader::open(&dir, &code), Err(Error::WrongCode)));
}

#[test]
fn torn_log_tail_is_ignored_but_middle_damage_is_not() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    for i in 0..20 {
        w.add_dir(&format!("d{i}"), Meta::default()).unwrap();
    }
    w.commit().unwrap();
    drop(w); // simulated crash: no End record

    let log = dir.join(LOG_FILE);
    let good = fs::read(&log).unwrap();
    // Torn tail: a length prefix promising more than is there.
    let mut torn = good.clone();
    torn.extend_from_slice(&500u32.to_le_bytes());
    torn.extend_from_slice(&[0xAB; 40]);
    fs::write(&log, &torn).unwrap();
    let m = PackageReader::open(&dir, &code).unwrap().manifest().unwrap();
    assert!(!m.complete);
    assert_eq!(m.entries.len(), 20);

    // Damage in the middle of the log is corruption, not a torn write.
    let mut damaged = good.clone();
    damaged[60] ^= 1;
    fs::write(&log, &damaged).unwrap();
    assert!(matches!(PackageReader::open(&dir, &code).unwrap().manifest(), Err(Error::Corrupt(_))));
}

#[test]
fn resume_continues_a_partial_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let big = data(10_500, 3);
    let small = data(10, 4);
    let (mut w, code) = PackageWriter::create(&dir, "9", begin(), opts()).unwrap();
    w.add_file("small.bin", Meta::default(), 10, &mut Cursor::new(&small), &mut unchanged()).unwrap();

    // Crash part-way through the big file: the reader errors after 6 000
    // bytes, but we drop the writer *before* it can record the abort, the
    // way a pulled drive or killed process would.
    struct Crash<'a>(Cursor<&'a [u8]>, usize);
    impl Read for Crash<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.position() as usize >= self.1 {
                panic!("simulated crash");
            }
            let max = (self.1 - self.0.position() as usize).min(buf.len());
            self.0.read(&mut buf[..max])
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.add_file(
            "big.bin",
            Meta::default(),
            big.len() as u64,
            &mut Crash(Cursor::new(&big), 6000),
            &mut unchanged(),
        );
    }));
    assert!(result.is_err());
    // Garbage past the last commit, as an interrupted write would leave.
    let pack_dir = dir.join(PACKS_DIR);
    let last = fs::read_dir(&pack_dir).unwrap().map(|e| e.unwrap().path()).max().unwrap();
    OpenOptions::new().append(true).open(&last).unwrap().write_all(&[0xEE; 333]).unwrap();

    let (mut w, state) = PackageWriter::resume(&dir, &code, opts()).unwrap();
    assert!(state.done.contains("small.bin"));
    let partial = state.partial.expect("big.bin should be partial");
    assert_eq!(partial.path, "big.bin");
    let offset = partial.bytes_stored();
    assert!(offset > 0 && offset <= 6000, "offset {offset}");
    let mut rest = Cursor::new(&big[offset as usize..]);
    w.resume_file(&partial, &mut rest, &mut unchanged()).unwrap();
    w.finish().unwrap();

    let reader = PackageReader::open(&dir, &code).unwrap();
    let m = reader.manifest().unwrap();
    assert!(m.complete);
    assert!(m.incomplete_files.is_empty());
    let files = files_of(&m);
    assert_eq!(files.len(), 2);
    assert_eq!(read_all(&reader, files[0]), small);
    assert_eq!(read_all(&reader, files[1]), big);
}

#[test]
fn source_read_error_aborts_only_that_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    struct Failing(usize);
    impl Read for Failing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0 == 0 {
                return Err(std::io::Error::other("bad sector"));
            }
            let n = self.0.min(buf.len());
            buf[..n].fill(1);
            self.0 -= n;
            Ok(n)
        }
    }
    let err = w.add_file("bad.bin", Meta::default(), 5000, &mut Failing(2500), &mut unchanged()).unwrap_err();
    assert!(matches!(err, Error::SourceRead(_)));
    w.add_skipped("bad.bin", SkipReason::ReadError, "bad sector").unwrap();
    w.add_file("ok.bin", Meta::default(), 3, &mut Cursor::new(b"abc"), &mut unchanged()).unwrap();
    assert_eq!(w.finish().unwrap().state, PackageState::CompleteWithIssues);

    let m = PackageReader::open(&dir, &code).unwrap().manifest().unwrap();
    let files = files_of(&m);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "ok.bin");
    assert!(m.entries.iter().any(|e| matches!(e, Entry::Skipped { reason: SkipReason::ReadError, .. })));
}

#[test]
fn completed_package_cannot_be_resumed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (w, code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    w.finish().unwrap();
    assert!(matches!(PackageWriter::resume(&dir, &code, opts()), Err(Error::AlreadyComplete)));
}

#[test]
fn listing_finds_packages_and_ignores_other_folders() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("not-a-package")).unwrap();
    for t in ["100", "200"] {
        let (w, _) = PackageWriter::create(&tmp.path().join(t), t, begin(), opts()).unwrap();
        w.finish().unwrap();
    }
    let list = list_packages(tmp.path()).unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().all(|r| r.is_ok()));
}

/// A crash right after a chunk was written but before it was committed
/// leaves bytes in the pack that no record refers to. Resume must discard
/// them before appending, or every later chunk offset would be wrong.
#[test]
fn resume_discards_uncommitted_pack_data() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    // One big pack, so the resumed writes go into the same file.
    let o = || WriterOptions { pack_size: 1 << 20, ..opts() };
    let big = data(9_000, 5);
    let (mut w, code) = PackageWriter::create(&dir, "9", begin(), o()).unwrap();

    // Budget 4 000 = exactly 4 chunks: chunks 1–3 are committed (≥ 2 500
    // bytes), chunk 4 is written to the pack but not yet committed, and
    // the next read crashes.
    struct Crash<'a>(&'a [u8], usize);
    impl Read for Crash<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.1 == 4000 {
                panic!("simulated crash");
            }
            let n = buf.len().min(4000 - self.1).min(self.0.len() - self.1);
            buf[..n].copy_from_slice(&self.0[self.1..self.1 + n]);
            self.1 += n;
            Ok(n)
        }
    }
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.add_file("big.bin", Meta::default(), big.len() as u64, &mut Crash(&big, 0), &mut unchanged());
    }));
    assert!(crashed.is_err());
    let pack = dir.join(PACKS_DIR).join("000000.pack");
    let len_before = fs::metadata(&pack).unwrap().len();

    let (mut w, state) = PackageWriter::resume(&dir, &code, o()).unwrap();
    let partial = state.partial.unwrap();
    assert_eq!(partial.bytes_stored(), 3000, "only committed chunks count");
    assert!(fs::metadata(&pack).unwrap().len() < len_before, "uncommitted chunk discarded");
    w.resume_file(&partial, &mut Cursor::new(&big[3000..]), &mut unchanged()).unwrap();
    w.finish().unwrap();

    let reader = PackageReader::open(&dir, &code).unwrap();
    let m = reader.manifest().unwrap();
    assert_eq!(read_all(&reader, files_of(&m)[0]), big);
}

#[test]
fn alt_streams_roundtrip_and_are_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&dir, "1", begin(), opts()).unwrap();
    w.add_file("a.pdf", Meta::default(), 3, &mut Cursor::new(b"pdf"), &mut unchanged()).unwrap();
    let motw = b"[ZoneTransfer]\r\nZoneId=3\r\n";
    w.add_alt_stream("a.pdf", "Zone.Identifier", motw).unwrap();
    assert!(w.add_alt_stream("a.pdf", "..\\evil", b"x").is_err());
    assert!(w.add_alt_stream("a.pdf", "big", &vec![0; PackageWriter::MAX_ALT_STREAM + 1]).is_err());
    w.finish().unwrap();
    let m = PackageReader::open(&dir, &code).unwrap().manifest().unwrap();
    let streams: Vec<_> = m
        .entries
        .iter()
        .filter_map(|e| if let Entry::AltStream { path, name, data } = e { Some((path, name, data)) } else { None })
        .collect();
    assert_eq!(streams.len(), 1);
    assert_eq!((streams[0].0.as_str(), streams[0].1.as_str()), ("a.pdf", "Zone.Identifier"));
    assert_eq!(streams[0].2.as_slice(), motw);
}
