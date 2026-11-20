use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use profileport_engine::package::{KdfParams, PackageReader, PackageState, SkipReason, TransferCode, WriterOptions};
use profileport_engine::*;

fn opts() -> ExportOptions {
    ExportOptions {
        writer: WriterOptions {
            chunk_size: 64 * 1024,
            pack_size: 256 * 1024,
            commit_bytes: 128 * 1024,
            commit_records: 16,
            kdf: KdfParams::insecure_fast_for_tests(),
        },
        ..Default::default()
    }
}

fn labels() -> SourceLabels {
    SourceLabels { computer: "OLD-PC".into(), profile: "anna".into() }
}

fn big_data(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A small but awkward profile.
fn make_profile(root: &Path) {
    let w = |p: &str, d: &[u8]| {
        let path = root.join(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, d).unwrap();
    };
    w("Documents/report.docx", b"quarterly numbers");
    w("Documents/Unicode ÆØÅ 日本 (1).txt", "hej verden".as_bytes());
    w("Documents/empty.txt", b"");
    w("Desktop/.hidden", b"dot file");
    w("Pictures/holiday.raw", &big_data(1_000_003, 1)); // ~16 chunks, several packs
    w("AppData/Roaming/App/settings.ini", b"[ui]\ntheme=dark\n");
    w("AppData/Local/Temp/junk.tmp", b"cache");
    let mut deep = String::from("Documents");
    for i in 0..30 {
        deep.push_str(&format!("/level{i}"));
    }
    w(&format!("{deep}/deep.txt"), b"bottom");
    fs::create_dir_all(root.join("Music")).unwrap(); // empty folder

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        // Legacy-style junction and a loop that would hang a naive copier.
        symlink("AppData/Roaming", root.join("Application Data")).unwrap();
        symlink("..", root.join("AppData/Local/loop")).unwrap();
    }
}

fn files_under(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let ft = e.file_type().unwrap();
            let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if ft.is_symlink() {
                continue;
            } else if ft.is_dir() {
                walk(root, &e.path(), out);
            } else {
                out.push((rel, fs::read(e.path()).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

struct Setup {
    _tmp: tempfile::TempDir,
    src: PathBuf,
    drive: PathBuf,
    dst: PathBuf,
}

fn setup() -> Setup {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("Users/anna");
    let drive = tmp.path().join("E");
    let dst = tmp.path().join("Users/anna.smith");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&drive).unwrap();
    fs::create_dir_all(&dst).unwrap();
    make_profile(&src);
    Setup { _tmp: tmp, src, drive, dst }
}

fn expected_files(src: &Path) -> Vec<(String, Vec<u8>)> {
    files_under(src).into_iter().filter(|(p, _)| !p.starts_with("AppData/Local/Temp")).collect()
}

#[test]
fn export_then_restore_to_a_different_username() {
    let s = setup();
    let mut ticks = 0;
    let (report, code) =
        export(&s.src, &transfers_dir(&s.drive), "48231", &labels(), &opts(), &mut |_| ticks += 1).unwrap();

    assert_eq!(report.status.state, PackageState::Complete, "{report:#?}");
    assert!(!report.has_issues());
    assert_eq!(report.excluded, vec!["AppData/Local/Temp".to_string()]);
    assert!(ticks > 0);
    #[cfg(unix)]
    {
        let links: Vec<_> = report.links.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(links, ["AppData/Local/loop", "Application Data"]);
    }

    // The code survives being written down and typed back in.
    let typed = TransferCode::parse(&code.to_string().to_lowercase()).unwrap();
    let reader = PackageReader::open(&report.package_dir, &typed).unwrap();
    let vr = verify(&reader, &mut |_| {}).unwrap();
    assert!(vr.complete && vr.failed.is_empty());
    assert_eq!(vr.files, report.files);

    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(rr.fully_successful(), "{rr:#?}");
    assert_eq!(rr.verified, rr.files);
    assert_eq!(files_under(&s.dst), expected_files(&s.src));
    assert!(s.dst.join("Music").is_dir(), "empty folders are restored");

    // Modification times survive.
    let t_src = fs::metadata(s.src.join("Documents/report.docx")).unwrap().modified().unwrap();
    let t_dst = fs::metadata(s.dst.join("Documents/report.docx")).unwrap().modified().unwrap();
    assert_eq!(t_src, t_dst);
}

#[test]
fn restore_never_overwrites() {
    let s = setup();
    let (report, code) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    let reader = PackageReader::open(&report.package_dir, &code).unwrap();

    fs::create_dir_all(s.dst.join("Documents")).unwrap();
    fs::write(s.dst.join("Documents/report.docx"), b"the new PC's own file").unwrap();

    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert_eq!(fs::read(s.dst.join("Documents/report.docx")).unwrap(), b"the new PC's own file");
    assert_eq!(rr.conflicts, vec![("Documents/report.docx".to_string(), ConflictOutcome::LeftExisting)]);
    assert!(!rr.fully_successful());

    // Keep both: the backup copy is restored alongside.
    let dst2 = s.dst.parent().unwrap().join("second");
    fs::create_dir_all(dst2.join("Documents")).unwrap();
    fs::write(dst2.join("Documents/report.docx"), b"mine").unwrap();
    let rr =
        restore(&reader, &dst2, &RestoreOptions { on_conflict: ConflictPolicy::KeepBoth, verify: true }, &mut |_| {})
            .unwrap();
    assert_eq!(fs::read(dst2.join("Documents/report.docx")).unwrap(), b"mine");
    assert_eq!(fs::read(dst2.join("Documents/report (restored).docx")).unwrap(), b"quarterly numbers");
    assert!(rr.failed.is_empty());
    assert!(rr.fully_successful(), "keep-both restores everything: {rr:#?}");

    // No temporary files are left behind.
    assert!(files_under(&dst2).iter().all(|(p, _)| !p.contains(".profileport-")));
}

#[cfg(unix)]
#[test]
fn restore_does_not_follow_links_on_the_destination() {
    let s = setup();
    let (report, code) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    let reader = PackageReader::open(&report.package_dir, &code).unwrap();

    let elsewhere = s.dst.parent().unwrap().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, s.dst.join("Documents")).unwrap();

    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(fs::read_dir(&elsewhere).unwrap().next().is_none(), "nothing written through the link");
    assert!(rr.failed.iter().any(|(p, m)| p.starts_with("Documents") && m.contains("link")));
    assert!(s.dst.join("Pictures/holiday.raw").is_file(), "the rest is restored");
}

/// Prepare a package (the code is known up front, as the UI shows it before
/// copying), then crash part-way through copying.
fn crash_export(s: &Setup, crash_after: u64) -> (PathBuf, TransferCode) {
    let (prepared, code) = prepare_export(&s.src, &transfers_dir(&s.drive), "48204", &labels(), &opts()).unwrap();
    let pkg = prepared.package_dir().to_path_buf();
    let crashing = ExportOptions { crash_after_bytes: Some(crash_after), ..opts() };
    let crashed = catch_unwind(AssertUnwindSafe(|| prepared.run(&code, &crashing, &mut |_| {})));
    assert!(crashed.is_err(), "the hook should have crashed the export");
    (pkg, code)
}

#[test]
fn resume_after_interruption_produces_identical_restore() {
    let s = setup();
    let (pkg, code) = crash_export(&s, 700_000);

    let info = package::read_info(&pkg).unwrap();
    assert_eq!(info.status.unwrap().state, PackageState::InProgress, "dashboard shows it as unfinished");
    let partial = PackageReader::open(&pkg, &code).unwrap().manifest().unwrap();
    assert!(!partial.complete);

    let report = resume_export(&pkg, &code, &opts(), &mut |_| {}).unwrap();
    assert_eq!(report.status.state, PackageState::Complete, "{report:#?}");
    assert!(!report.has_issues());

    let reader = PackageReader::open(&pkg, &code).unwrap();
    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(rr.fully_successful(), "{rr:#?}");
    assert_eq!(files_under(&s.dst), expected_files(&s.src));
}

#[test]
fn resume_survives_repeated_interruptions() {
    let s = setup();
    let (pkg, code) = crash_export(&s, 150_000);
    for _ in 0..5 {
        let crashing = ExportOptions { crash_after_bytes: Some(150_000), ..opts() };
        let r = catch_unwind(AssertUnwindSafe(|| resume_export(&pkg, &code, &crashing, &mut |_| {})));
        if let Ok(done) = r {
            done.unwrap();
            break;
        }
    }
    let _ = resume_export(&pkg, &code, &opts(), &mut |_| {});
    let reader = PackageReader::open(&pkg, &code).unwrap();
    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(rr.fully_successful(), "{rr:#?}");
    assert_eq!(files_under(&s.dst), expected_files(&s.src));
}

#[test]
fn resume_restarts_a_file_that_changed_while_interrupted() {
    let s = setup();
    let (pkg, code) = crash_export(&s, 700_000);

    // The customer kept using the PC: the half-copied file changed.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let new_content = big_data(900_000, 99);
    fs::write(s.src.join("Pictures/holiday.raw"), &new_content).unwrap();

    resume_export(&pkg, &code, &opts(), &mut |_| {}).unwrap();
    let reader = PackageReader::open(&pkg, &code).unwrap();
    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(rr.fully_successful(), "{rr:#?}");
    assert_eq!(fs::read(s.dst.join("Pictures/holiday.raw")).unwrap(), new_content);
}

#[test]
fn wrong_code_cannot_restore() {
    let s = setup();
    let (report, _code) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    let err = PackageReader::open(&report.package_dir, &TransferCode::generate()).err().unwrap();
    assert!(matches!(err, package::Error::WrongCode));
}

#[test]
fn damaged_package_is_reported_not_restored() {
    let s = setup();
    let (report, code) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    let pack = report.package_dir.join("packs/000001.pack");
    let mut bytes = fs::read(&pack).unwrap();
    bytes[5000] ^= 0xFF;
    fs::write(&pack, bytes).unwrap();

    let reader = PackageReader::open(&report.package_dir, &code).unwrap();
    let vr = verify(&reader, &mut |_| {}).unwrap();
    assert_eq!(vr.failed.len(), 1, "{vr:#?}");
    let bad = &vr.failed[0].0;

    let rr = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(rr.failed.iter().any(|(p, _)| p == bad));
    assert!(!s.dst.join(bad).exists(), "damaged file must not appear");
    assert!(!rr.fully_successful());
}

#[test]
fn unreadable_files_are_reported() {
    let s = setup();
    let secret = s.src.join("Documents/locked.txt");
    fs::write(&secret, b"x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    }
    if fs::File::open(&secret).is_ok() {
        eprintln!("running with privileges that bypass permissions; skipping");
        return;
    }
    let (report, _) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    assert_eq!(report.status.state, PackageState::CompleteWithIssues);
    assert!(report
        .skipped
        .iter()
        .any(|i| i.path == "Documents/locked.txt" && i.reason == SkipReason::PermissionDenied));
}

#[test]
fn bad_ticket_is_rejected_before_anything_is_written() {
    let s = setup();
    let err = export(&s.src, &transfers_dir(&s.drive), "../../x", &labels(), &opts(), &mut |_| {}).unwrap_err();
    assert!(matches!(err, ExportError::Package(package::Error::InvalidTicket(_))));
    assert!(!transfers_dir(&s.drive).exists());
}

#[test]
fn restoring_twice_is_safe_and_idempotent() {
    let s = setup();
    let (report, code) = export(&s.src, &transfers_dir(&s.drive), "1", &labels(), &opts(), &mut |_| {}).unwrap();
    let reader = PackageReader::open(&report.package_dir, &code).unwrap();
    let first = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(first.fully_successful());

    // As after an interrupted restore that is simply run again.
    let second = restore(&reader, &s.dst, &RestoreOptions::default(), &mut |_| {}).unwrap();
    assert!(second.fully_successful(), "{second:#?}");
    assert_eq!(second.files, 0);
    assert_eq!(second.already_present, first.files);
    assert!(second.conflicts.is_empty());
    assert_eq!(files_under(&s.dst), expected_files(&s.src));
}
