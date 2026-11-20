//! Restoring onto a system laid out differently from the source: standard
//! folders in other places (translated names, redirected drives) and
//! different file-name rules. The Windows name rules are pure logic, so they
//! are exercised here on any OS via `RestoreOptions::name_rules`.

use std::fs;
use std::path::{Path, PathBuf};

use profileport_engine::package::{
    BeginInfo, KdfParams, KnownFolder, KnownFolderEntry, Meta, PackageReader, PackageWriter, WriterOptions,
};
use profileport_engine::*;

fn writer_opts() -> WriterOptions {
    WriterOptions { kdf: KdfParams::insecure_fast_for_tests(), ..Default::default() }
}

fn labels() -> SourceLabels {
    SourceLabels { computer: "OLD-PC".into(), profile: "anna".into() }
}

fn write(root: &Path, rel: &str, data: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, data).unwrap();
}

fn read(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p).unwrap()
}

#[test]
fn standard_folders_land_where_the_target_keeps_them() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src/anna");
    let elsewhere = tmp.path().join("D/Music collection"); // redirected outside the profile
    let dst = tmp.path().join("dst/anna");
    write(&src, "OneDrive/Documents/Taxes/2025.pdf", "tax");
    write(&src, "Pictures/cat.jpg", "cat");
    write(&src, "Favorites/bank.url", "url");
    write(&elsewhere, "Album/song.mp3", "song");
    fs::create_dir_all(&dst).unwrap();

    // Source: Documents redirected into OneDrive, Music on another drive.
    let opts = ExportOptions {
        writer: writer_opts(),
        known_folders: Some(vec![
            (KnownFolder::Documents, src.join("OneDrive/Documents")),
            (KnownFolder::Pictures, src.join("Pictures")),
            (KnownFolder::Music, elsewhere.clone()),
        ]),
        ..Default::default()
    };
    let (report, code) = export(&src, &tmp.path().join("E"), "1", &labels(), &opts, &mut |_| {}).unwrap();
    let paths: Vec<_> = report.known_folders.iter().map(|e| (e.folder, e.path.as_str())).collect();
    assert!(paths.contains(&(KnownFolder::Documents, "OneDrive/Documents")));
    assert!(paths.contains(&(KnownFolder::Music, "@Music")), "outside folder stored as @Music: {paths:?}");

    // Target: a Norwegian Linux desktop.
    let target = vec![
        (KnownFolder::Documents, dst.join("Dokumenter")),
        (KnownFolder::Pictures, dst.join("Bilder")),
        (KnownFolder::Music, dst.join("Musikk")),
    ];
    for (_, p) in &target {
        fs::create_dir_all(p).unwrap();
    }
    let reader = PackageReader::open(&report.package_dir, &code).unwrap();
    let ropts = RestoreOptions { known_folders: Some(target), ..Default::default() };
    let r = restore(&reader, &dst, &ropts, &mut |_| {}).unwrap();
    assert!(r.fully_successful(), "{r:#?}");

    assert_eq!(read(dst.join("Dokumenter/Taxes/2025.pdf")), "tax");
    assert_eq!(read(dst.join("Bilder/cat.jpg")), "cat");
    assert_eq!(read(dst.join("Musikk/Album/song.mp3")), "song");
    assert_eq!(read(dst.join("Favorites/bank.url")), "url", "non-standard folders keep their place");
    assert!(!dst.join("OneDrive/Documents").exists(), "Documents moved, not duplicated");
    let used: Vec<_> = r.folders_used.iter().map(|(k, _)| *k).collect();
    assert_eq!(used, vec![KnownFolder::Documents, KnownFolder::Music, KnownFolder::Pictures]);
}

/// A package as made on Linux, holding names Windows can't use.
fn linux_style_package(dir: &Path) -> profileport_engine::package::TransferCode {
    let begin = BeginInfo {
        source_os: "linux".into(),
        known_folders: vec![KnownFolderEntry {
            folder: KnownFolder::Documents,
            path: "Documents".into(),
            source_path: "/home/anna/Documents".into(),
        }],
        ..Default::default()
    };
    let (mut w, code) = PackageWriter::create(dir, "2", begin, writer_opts()).unwrap();
    for (path, data) in [
        ("Documents/notes: draft?.txt", "notes"),
        ("Documents/CON", "con"),
        ("Documents/Report.txt", "upper"),
        ("Documents/report.txt", "lower"),
        ("Documents/ends with dot.", "dot"),
        ("Documents/a\\b", "backslash"),
        ("Documents/plain.txt", "plain"),
    ] {
        w.add_file(path, Meta::default(), data.len() as u64, &mut data.as_bytes(), &mut || false).unwrap();
    }
    w.finish().unwrap();
    code
}

#[test]
fn linux_names_are_made_valid_for_windows_and_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let code = linux_style_package(&pkg);
    let dst = tmp.path().join("Users/anna");
    fs::create_dir_all(dst.join("Documents")).unwrap();

    let reader = PackageReader::open(&pkg, &code).unwrap();
    let opts = RestoreOptions {
        name_rules: NameRules::Windows,
        known_folders: Some(vec![(KnownFolder::Documents, dst.join("Documents"))]),
        ..Default::default()
    };
    let r = restore(&reader, &dst, &opts, &mut |_| {}).unwrap();
    assert!(r.failed.is_empty(), "{r:#?}");

    let d = dst.join("Documents");
    assert_eq!(read(d.join("notes_ draft_.txt")), "notes");
    assert_eq!(read(d.join("_CON")), "con");
    assert_eq!(read(d.join("Report.txt")), "upper");
    assert_eq!(read(d.join("report (restored).txt")), "lower", "case collision kept, not overwritten");
    assert_eq!(read(d.join("ends with dot._")), "dot");
    assert_eq!(read(d.join("a_b")), "backslash", "backslash can't act as a separator");
    assert_eq!(read(d.join("plain.txt")), "plain");

    let mut renamed: Vec<_> = r.renamed.iter().map(|(p, n, why)| (p.as_str(), n.as_str(), *why)).collect();
    renamed.sort();
    assert_eq!(
        renamed,
        vec![
            ("Documents/CON", "_CON", RenameReason::NotAllowedHere),
            ("Documents/a\\b", "a_b", RenameReason::NotAllowedHere),
            ("Documents/ends with dot.", "ends with dot._", RenameReason::NotAllowedHere),
            ("Documents/notes: draft?.txt", "notes_ draft_.txt", RenameReason::NotAllowedHere),
            ("Documents/report.txt", "report (restored).txt", RenameReason::CaseCollision),
        ]
    );
    assert!(r.fully_successful(), "renames are reported, not failures");

    // Running the same restore again finds everything in place.
    let again = restore(&reader, &dst, &opts, &mut |_| {}).unwrap();
    assert_eq!(again.files, 0, "{again:#?}");
    assert_eq!(again.already_present, 7);
    let count = fs::read_dir(&d).unwrap().count();
    assert_eq!(count, 7, "no duplicates on re-run");
}

#[test]
fn keep_both_rerun_creates_no_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    write(&src, "Documents/report.docx", "backup");
    let (report, code) = export(
        &src,
        &tmp.path().join("E"),
        "3",
        &labels(),
        &ExportOptions { writer: writer_opts(), known_folders: Some(vec![]), ..Default::default() },
        &mut |_| {},
    )
    .unwrap();
    let dst = tmp.path().join("dst");
    write(&dst, "Documents/report.docx", "the new PC's own");
    let reader = PackageReader::open(&report.package_dir, &code).unwrap();
    let opts =
        RestoreOptions { on_conflict: ConflictPolicy::KeepBoth, known_folders: Some(vec![]), ..Default::default() };
    restore(&reader, &dst, &opts, &mut |_| {}).unwrap();
    let again = restore(&reader, &dst, &opts, &mut |_| {}).unwrap();
    assert_eq!(again.already_present, 1, "{again:#?}");
    let mut names: Vec<_> = fs::read_dir(dst.join("Documents")).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert_eq!(names, ["report (restored).docx", "report.docx"]);
    assert_eq!(read(dst.join("Documents/report.docx")), "the new PC's own");
}

#[test]
fn windows_streams_are_reported_as_not_applicable_elsewhere() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(
        &pkg,
        "4",
        BeginInfo { source_os: "windows".into(), ..Default::default() },
        writer_opts(),
    )
    .unwrap();
    w.add_file("a.pdf", Meta::default(), 3, &mut &b"pdf"[..], &mut || false).unwrap();
    w.add_alt_stream("a.pdf", "Zone.Identifier", b"[ZoneTransfer]\r\nZoneId=3\r\n").unwrap();
    w.finish().unwrap();
    let dst = tmp.path().join("dst");
    fs::create_dir_all(&dst).unwrap();
    let reader = PackageReader::open(&pkg, &code).unwrap();
    let opts = RestoreOptions { name_rules: NameRules::Posix, known_folders: Some(vec![]), ..Default::default() };
    let r = restore(&reader, &dst, &opts, &mut |_| {}).unwrap();
    assert_eq!((r.files, r.streams_restored, r.streams_not_applicable), (1, 0, 1));
    assert!(r.fully_successful());
}

#[test]
fn hostile_names_never_escape_on_windows_rules() {
    // Structurally valid package paths whose components would be traversal
    // on Windows if used raw.
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let (mut w, code) = PackageWriter::create(&pkg, "5", BeginInfo::default(), writer_opts()).unwrap();
    for path in ["..\\..\\escape.txt", "C:\\Windows\\evil.dll", "x/..\\..\\y.txt"] {
        w.add_file(path, Meta::default(), 1, &mut &b"!"[..], &mut || false).unwrap();
    }
    w.finish().unwrap();
    let root = tmp.path().join("victim/profile");
    fs::create_dir_all(&root).unwrap();
    let reader = PackageReader::open(&pkg, &code).unwrap();
    let opts = RestoreOptions { name_rules: NameRules::Windows, known_folders: Some(vec![]), ..Default::default() };
    let r = restore(&reader, &root, &opts, &mut |_| {}).unwrap();
    assert_eq!(r.files, 3, "{r:#?}");
    // Everything stayed inside the profile, as single plain names.
    let mut found: Vec<PathBuf> = Vec::new();
    for e in walk(tmp.path().join("victim").as_path()) {
        found.push(e.strip_prefix(&root).expect("nothing outside the profile").to_path_buf());
    }
    found.sort();
    let names: Vec<String> = found.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
    assert_eq!(names, [".._.._escape.txt", "C__Windows_evil.dll", "x/.._.._y.txt"]);
}

fn walk(dir: &Path) -> Vec<PathBuf> {
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
