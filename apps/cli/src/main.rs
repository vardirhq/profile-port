//! `profileport-cli` — command-line front end to the migration engine.
//!
//! Exit codes: 0 = complete, 1 = finished with issues to review, 2 = failed.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use profileport_engine::package::{self, PackageReader, PackageState, SkipReason, TransferCode};
use profileport_engine::*;

#[derive(Parser)]
#[command(name = "profileport-cli", version, about = "Move a Windows user's files between PCs via a USB drive")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the user profiles on this computer and their standard folders
    Profiles,
    /// List the drives a backup can go to (USB sticks and other non-system drives)
    Drives,
    /// List the backups on a ProfilePort drive (no code needed)
    List {
        /// Root of the USB drive, e.g. E:\
        #[arg(long)]
        drive: PathBuf,
    },
    /// Back up a profile folder to the drive
    Export {
        /// Profile folder, e.g. C:\Users\anna
        source: PathBuf,
        /// Root of the USB drive, e.g. E:\
        #[arg(long)]
        drive: PathBuf,
        /// Job ticket number (the only plaintext label on the drive)
        #[arg(long)]
        ticket: String,
        /// Source computer name (default: this computer)
        #[arg(long)]
        computer: Option<String>,
        /// Profile name (default: the source folder's name)
        #[arg(long)]
        profile: Option<String>,
        /// Don't ask for the code to be typed back (scripted use only)
        #[arg(long)]
        no_confirm: bool,
    },
    /// Continue an interrupted backup
    Resume {
        package: PathBuf,
        /// Transfer code; prompted for if omitted (avoids shell history)
        #[arg(long)]
        code: Option<String>,
    },
    /// Check that every file in a backup decrypts and matches its hash
    Verify {
        package: PathBuf,
        #[arg(long)]
        code: Option<String>,
    },
    /// Restore a backup into a profile folder on this PC
    Restore {
        package: PathBuf,
        /// Destination profile folder, e.g. C:\Users\anna.smith
        #[arg(long)]
        to: PathBuf,
        #[arg(long)]
        code: Option<String>,
        /// When a file already exists, restore ours as "name (restored).ext"
        /// instead of leaving it out
        #[arg(long)]
        keep_both: bool,
        /// Skip reading back restored files to verify them
        #[arg(long)]
        no_verify: bool,
        /// Allow restoring into folders synced to OneDrive or similar (this
        /// uploads the files). Asked interactively when omitted.
        #[arg(long)]
        allow_cloud_folders: bool,
    },
    /// Wipe backups from the drive once the job is done: destroy the key,
    /// then delete the files. Asks for confirmation; cannot be undone.
    Wipe {
        /// Backup folder(s) to wipe
        packages: Vec<PathBuf>,
        /// Wipe backups on this drive instead (with --all or --older-than)
        #[arg(long, conflicts_with = "packages")]
        drive: Option<PathBuf>,
        /// With --drive: every backup on the drive ("prepare for next customer")
        #[arg(long, requires = "drive", conflicts_with = "older_than")]
        all: bool,
        /// With --drive: backups created at least this many days ago
        #[arg(long, requires = "drive", value_name = "DAYS")]
        older_than: Option<u32>,
        /// Also overwrite all backup data before deleting it (slow: about as
        /// long as the backup took)
        #[arg(long)]
        overwrite: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Profiles => profiles(),
        Cmd::Drives => drives(),
        Cmd::List { drive } => list(&drive),
        Cmd::Export { source, drive, ticket, computer, profile, no_confirm } => {
            run_export(&source, &drive, &ticket, computer, profile, no_confirm)
        }
        Cmd::Resume { package, code } => run_resume(&package, code),
        Cmd::Verify { package, code } => run_verify(&package, code),
        Cmd::Restore { package, to, code, keep_both, no_verify, allow_cloud_folders } => {
            run_restore(&package, &to, code, keep_both, no_verify, allow_cloud_folders)
        }
        Cmd::Wipe { packages, drive, all, older_than, overwrite } => {
            run_wipe(packages, drive.as_deref(), all, older_than, overwrite)
        }
    };
    match result {
        Ok(Outcome::Clean) => ExitCode::from(0),
        Ok(Outcome::Issues) => ExitCode::from(1),
        Err(e) => {
            eprintln!("\nerror: {e}");
            ExitCode::from(2)
        }
    }
}

enum Outcome {
    Clean,
    Issues,
}

type Res = Result<Outcome, Box<dyn std::error::Error>>;

// ---------------------------------------------------------------- profiles

fn profiles() -> Res {
    let list = system::list_profiles();
    if list.is_empty() {
        println!("No user profiles found.");
        return Ok(Outcome::Clean);
    }
    for p in list {
        println!("{}{}  {}", p.name, if p.current { "  (you)" } else { "" }, p.path.display());
        for (kf, path) in system::known_folders(&p.path) {
            let cloud = if is_cloud_synced(&path) { "  [cloud-synced]" } else { "" };
            println!("    {:<10} {}{}", format!("{kf:?}"), path.display(), cloud);
        }
    }
    Ok(Outcome::Clean)
}

// ---------------------------------------------------------------- drives

fn drives() -> Res {
    let list = system::list_drives();
    if list.is_empty() {
        println!("No drives found apart from the system drive. Plug in the USB drive and try again.");
        return Ok(Outcome::Clean);
    }
    for d in list {
        let space = match (d.free_bytes, d.total_bytes) {
            (Some(f), Some(t)) => format!("{} free of {}", human_bytes(f), human_bytes(t)),
            _ => "size unknown".into(),
        };
        let backups = package::list_packages(&transfers_dir(&d.path)).map(|l| l.len()).unwrap_or(0);
        println!(
            "{:<24} {:<16} {:<10} {:<26} {} backup(s)",
            d.path.display(),
            d.label,
            if d.removable { "USB" } else { "fixed disk" },
            space,
            backups
        );
    }
    Ok(Outcome::Clean)
}

// ---------------------------------------------------------------- list

fn list(drive: &Path) -> Res {
    let dir = transfers_dir(drive);
    if !dir.is_dir() {
        println!("No backups on this drive.");
        return Ok(Outcome::Clean);
    }
    let packages = package::list_packages(&dir)?;
    if packages.is_empty() {
        println!("No backups on this drive.");
        return Ok(Outcome::Clean);
    }
    let now = now_unix();
    println!("{:<14} {:<17} {:>10} {:>8}  {:<14} FOLDER", "TICKET", "CREATED (UTC)", "SIZE", "FILES", "STATE");
    for (dir, p) in packages {
        match p {
            Ok(info) => {
                let (bytes, files, state) = match &info.status {
                    Some(s) => (human_bytes(s.bytes), s.files.to_string(), state_label(s.state, s.skipped)),
                    None => ("?".into(), "?".into(), "unknown".into()),
                };
                let age_days = (now - info.header.created_unix) / 86_400;
                let age = if age_days >= 14 { format!("  ({age_days} days old)") } else { String::new() };
                println!(
                    "{:<14} {:<17} {:>10} {:>8}  {:<14} {}{}",
                    info.header.ticket,
                    utc_date_time(info.header.created_unix),
                    bytes,
                    files,
                    state,
                    info.dir.file_name().unwrap_or_default().to_string_lossy(),
                    age
                );
            }
            Err(e) => println!("{:<14} {}: {e}", "-", dir.file_name().unwrap_or_default().to_string_lossy()),
        }
    }
    Ok(Outcome::Clean)
}

fn state_label(s: PackageState, skipped: u64) -> String {
    match s {
        PackageState::InProgress => "interrupted".into(),
        PackageState::Complete => "complete".into(),
        PackageState::CompleteWithIssues if skipped > 0 => format!("partial ({skipped} skipped)"),
        PackageState::CompleteWithIssues => "complete*".into(),
    }
}

// ---------------------------------------------------------------- export

fn run_export(
    source: &Path,
    drive: &Path,
    ticket: &str,
    computer: Option<String>,
    profile: Option<String>,
    no_confirm: bool,
) -> Res {
    let labels = SourceLabels {
        computer: computer.unwrap_or_else(this_computer),
        profile: profile
            .unwrap_or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
    };
    let opts = ExportOptions::default();
    println!("Preparing backup of {} …", source.display());
    let (prepared, code) = prepare_export(source, &transfers_dir(drive), ticket, &labels, &opts)?;

    // The code is shown and confirmed *before* copying: if the export is
    // interrupted it is the only way to resume.
    show_code(ticket, &code);
    if !no_confirm {
        if let Err(e) = confirm_code(&code) {
            let dir = prepared.package_dir().to_path_buf();
            return Err(match prepared.discard() {
                Ok(()) => format!("{e}. Nothing was copied and the empty backup was removed.").into(),
                Err(d) => format!("{e}. The empty backup at {} could not be removed ({d}).", dir.display()).into(),
            });
        }
    }

    println!("\nBacking up …");
    let mut bar = ProgressLine::new();
    let report = prepared.run(&code, &opts, &mut |p| bar.update(p))?;
    bar.finish();
    print_export_report(&report);
    Ok(if report.has_issues() { Outcome::Issues } else { Outcome::Clean })
}

fn show_code(ticket: &str, code: &TransferCode) {
    println!();
    println!("  ┌──────────────────────────────────────────────────────────────┐");
    println!("  │  Transfer code for ticket {ticket:<35}│");
    println!("  │                                                              │");
    println!("  │                     {code}                           │");
    println!("  │                                                              │");
    println!("  │  Store it in the ticket's SENSITIVE-DATA field,              │");
    println!("  │  not in ordinary notes. It is not stored on the drive.       │");
    println!("  │  Without it this backup can't be restored or resumed.        │");
    println!("  └──────────────────────────────────────────────────────────────┘");
}

fn confirm_code(code: &TransferCode) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let Some(line) = prompt("\nType the code back to confirm you've written it down: ")? else {
            return Err("no confirmation given".into());
        };
        match TransferCode::parse(&line) {
            Ok(typed) if typed == *code => {
                println!("Confirmed.");
                return Ok(());
            }
            Ok(_) => println!("That's a valid code, but not this one. The code is: {code}"),
            Err(e) => println!("{e}. The code is: {code}"),
        }
    }
}

fn print_export_report(r: &ExportReport) {
    println!();
    match r.status.state {
        PackageState::Complete => println!("✔ Backup complete — ticket {}", r.ticket),
        _ => println!("⚠ Backup finished with items to review — ticket {}", r.ticket),
    }
    println!("  {} files, {} folders, {}", r.files, r.dirs, human_bytes(r.bytes));
    println!("  Stored in {}", r.package_dir.display());
    if !r.excluded.is_empty() {
        println!("  {} cache/temp location(s) excluded by rule", r.excluded.len());
    }
    if !r.links.is_empty() {
        println!("  {} link(s) recorded, not followed", r.links.len());
    }
    if r.alt_streams > 0 {
        println!("  {} file stream(s) stored (e.g. \"downloaded from the internet\" markers)", r.alt_streams);
    }
    for e in &r.known_folders {
        println!("  {:<10} {}", format!("{:?}", e.folder), e.source_path);
    }
    print_skips("Not copied", &r.skipped);
    print_list("Changed while being copied (copied anyway; check these)", &r.changed);
}

// ---------------------------------------------------------------- resume / verify

fn run_resume(package_dir: &Path, code: Option<String>) -> Res {
    let info = package::read_info(package_dir)?;
    let code = get_code(code, &info.header.ticket)?;
    println!("Resuming backup for ticket {} …", info.header.ticket);
    let mut bar = ProgressLine::new();
    let report = resume_export(package_dir, &code, &ExportOptions::default(), &mut |p| bar.update(p))?;
    bar.finish();
    print_export_report(&report);
    Ok(if report.has_issues() { Outcome::Issues } else { Outcome::Clean })
}

fn run_verify(package_dir: &Path, code: Option<String>) -> Res {
    let info = package::read_info(package_dir)?;
    let code = get_code(code, &info.header.ticket)?;
    let reader = PackageReader::open(package_dir, &code)?;
    println!("Verifying ticket {} …", info.header.ticket);
    let mut bar = ProgressLine::new();
    let r = verify(&reader, &mut |p| bar.update(p))?;
    bar.finish();
    println!("\n  {} files ({}) decrypted and matched their hashes", r.files, human_bytes(r.bytes));
    if !r.complete {
        println!("⚠ This backup was interrupted and is incomplete. Run `profileport-cli resume`.");
    }
    print_list("Incomplete when the backup stopped", &r.incomplete_files);
    print_failures("DAMAGED", &r.failed);
    Ok(if r.failed.is_empty() && r.complete { Outcome::Clean } else { Outcome::Issues })
}

// ---------------------------------------------------------------- restore

fn run_restore(
    package_dir: &Path,
    to: &Path,
    code: Option<String>,
    keep_both: bool,
    no_verify: bool,
    allow_cloud_folders: bool,
) -> Res {
    let info = package::read_info(package_dir)?;
    let code = get_code(code, &info.header.ticket)?;
    let reader = PackageReader::open(package_dir, &code)?;
    let m = reader.manifest()?;
    println!("Ticket {}: {} on {}  →  {}", info.header.ticket, m.begin.profile, m.begin.computer, to.display());
    if !m.complete {
        println!("⚠ This backup is incomplete (it was interrupted). Restoring what it contains.");
    }
    let target_folders = system::known_folders(to);
    let cloud: Vec<String> = target_folders
        .iter()
        .filter(|(_, p)| is_cloud_synced(p))
        .map(|(k, p)| format!("{k:?}: {}", p.display()))
        .collect();
    if !cloud.is_empty() && !allow_cloud_folders {
        print_list("These folders on this PC are synced to the cloud; files restored there will be uploaded", &cloud);
        let ok = io::stdin().is_terminal()
            && prompt("\nRestore into them anyway? [y/N] ")?.is_some_and(|a| a.trim().eq_ignore_ascii_case("y"));
        if !ok {
            return Err(
                "restore cancelled: target folders are cloud-synced (use --allow-cloud-folders to proceed)".into()
            );
        }
    }
    let opts = RestoreOptions {
        on_conflict: if keep_both { ConflictPolicy::KeepBoth } else { ConflictPolicy::Skip },
        verify: !no_verify,
        known_folders: Some(target_folders),
        ..Default::default()
    };
    let mut bar = ProgressLine::new();
    let r = restore(&reader, to, &opts, &mut |p| bar.update(p))?;
    bar.finish();

    println!();
    if r.fully_successful() {
        println!("✔ Restore complete");
    } else {
        println!("⚠ Restore finished with items to review");
    }
    if r.read_back_skipped {
        println!("  Restored files were NOT read back and checked (--no-verify). Check them before relying on them.");
    }
    println!("  {} files restored ({}), {} verified, {} folders", r.files, human_bytes(r.bytes), r.verified, r.dirs);
    if r.already_present > 0 {
        println!("  {} files were already there with identical content", r.already_present);
    }
    if !r.source_os.is_empty() && r.source_os != system::os_name() {
        println!("  Backup made on {}, restored on {}", r.source_os, system::os_name());
    }
    for (kf, path) in &r.folders_used {
        println!("  {:<10} → {}", format!("{kf:?}"), path.display());
    }
    if r.streams_restored > 0 {
        println!("  {} \"downloaded from the internet\" markers and other file streams restored", r.streams_restored);
    }
    if r.streams_not_applicable > 0 {
        println!("  {} Windows-only file streams not applicable on this system", r.streams_not_applicable);
    }
    let renamed: Vec<String> = r
        .renamed
        .iter()
        .map(|(p, n, why)| {
            let why = match why {
                RenameReason::NotAllowedHere => "name not allowed on this system",
                RenameReason::CaseCollision => "same name as another file apart from letter case",
            };
            format!("{p}  → {n}  ({why})")
        })
        .collect();
    print_list("Renamed", &renamed);
    let conflicts: Vec<String> = r
        .conflicts
        .iter()
        .map(|(p, c)| match c {
            ConflictOutcome::LeftExisting => format!("{p}  (already existed; left as is)"),
            ConflictOutcome::RestoredAs(n) => format!("{p}  → restored as {n}"),
        })
        .collect();
    print_list("Name conflicts", &conflicts);
    print_failures("FAILED", &r.failed);
    print_skips("Not in the backup", &r.skipped_at_backup);
    print_list("Changed while being backed up (check these)", &r.changed_at_backup);
    print_list("Incomplete in the backup", &r.incomplete_at_backup);
    let links: Vec<String> = r.links_not_restored.iter().map(|(p, t)| format!("{p} → {t}")).collect();
    print_list("Links not restored (Windows recreates the standard ones)", &links);

    if r.fully_successful() && io::stdin().is_terminal() {
        println!("\nOnce the customer has checked their files, the backup should be wiped from the drive.");
        if let Some(answer) = prompt("Restore verified. Wipe this backup from the drive now? [y/N] ")? {
            if answer.trim().eq_ignore_ascii_case("y") {
                drop(reader);
                let w = wipe_one(package_dir, false)?;
                print_wiped(&w);
            }
        }
    }
    Ok(if r.fully_successful() { Outcome::Clean } else { Outcome::Issues })
}

// ---------------------------------------------------------------- wipe

fn run_wipe(packages: Vec<PathBuf>, drive: Option<&Path>, all: bool, older_than: Option<u32>, overwrite: bool) -> Res {
    let targets: Vec<(PathBuf, String)> = match drive {
        Some(drive) => {
            if !all && older_than.is_none() {
                return Err("with --drive, say which backups to wipe: --all, or --older-than DAYS".into());
            }
            wipe_candidates(drive, older_than)?
        }
        None if packages.is_empty() => {
            return Err("name the backup folder(s) to wipe, or use --drive with --all or --older-than".into())
        }
        None => packages
            .into_iter()
            .map(|p| {
                let label = describe_for_wipe(&p);
                (p, label)
            })
            .collect(),
    };
    if targets.is_empty() {
        println!("No backups to wipe.");
        return Ok(Outcome::Clean);
    }

    println!("About to wipe {} backup(s). This cannot be undone:", targets.len());
    for (_, label) in &targets {
        println!("    {label}");
    }
    println!("Only wipe a backup once its restore is verified and the customer has checked their files.");
    if overwrite {
        println!("All backup data will be overwritten before it is deleted; this takes a while.");
    }
    // One named backup: type its ticket. Several, or a whole drive: type WIPE.
    let expected = match targets.as_slice() {
        [(dir, _)] if drive.is_none() => match package::read_info(dir) {
            Ok(info) => info.header.ticket,
            Err(_) => "WIPE".into(),
        },
        _ => "WIPE".into(),
    };
    let typed = prompt(&format!("\nType {expected} to confirm: "))?;
    if typed.as_deref().map(str::trim) != Some(expected.as_str()) {
        return Err("wipe cancelled: confirmation did not match; nothing was changed".into());
    }

    let mut failed = 0;
    for (dir, _) in &targets {
        match wipe_one(dir, overwrite) {
            Ok(w) => print_wiped(&w),
            Err(e) => {
                failed += 1;
                println!("✘ {}: {e}", dir.display());
            }
        }
    }
    if failed > 0 {
        return Err(format!("{failed} backup(s) could not be wiped; see above").into());
    }
    Ok(Outcome::Clean)
}

/// The packages on `drive` to wipe, oldest first, with a one-line label each.
/// Partly wiped packages are always included; damaged ones are listed so the
/// technician knows they're there, but left alone.
fn wipe_candidates(
    drive: &Path,
    older_than: Option<u32>,
) -> Result<Vec<(PathBuf, String)>, Box<dyn std::error::Error>> {
    let dir = transfers_dir(drive);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let min_age = older_than.map(|d| i64::from(d) * 86_400);
    let now = now_unix();
    let mut out = Vec::new();
    for (path, item) in package::list_packages(&dir)? {
        match item {
            Ok(info) => {
                if min_age.is_none_or(|age| now - info.header.created_unix >= age) {
                    out.push((info.header.created_unix, path.clone(), describe_for_wipe(&path)));
                }
            }
            Err(package::Error::PartlyWiped) => out.push((i64::MIN, path.clone(), describe_for_wipe(&path))),
            Err(e) => println!("Leaving {} alone: {e}", path.display()),
        }
    }
    out.sort();
    Ok(out.into_iter().map(|(_, p, d)| (p, d)).collect())
}

fn describe_for_wipe(dir: &Path) -> String {
    let folder = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
    match package::read_info(dir) {
        Ok(info) => {
            let state = match &info.status {
                Some(s) => state_label(s.state, s.skipped),
                None => "unknown".into(),
            };
            format!(
                "ticket {} · created {} UTC · {} · {}",
                info.header.ticket,
                utc_date_time(info.header.created_unix),
                state,
                folder
            )
        }
        Err(package::Error::PartlyWiped) => format!("{folder} (partly wiped earlier)"),
        Err(e) => format!("{} ({e})", dir.display()),
    }
}

fn wipe_one(dir: &Path, overwrite: bool) -> Result<package::WipeReport, Box<dyn std::error::Error>> {
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut shown = false;
    let report = package::wipe(dir, &package::WipeOptions { overwrite_data: overwrite }, &mut |done, total| {
        if io::stderr().is_terminal() && last.elapsed() >= Duration::from_millis(150) {
            last = Instant::now();
            shown = true;
            eprint!("\r\x1b[2K  overwriting {} of {}", human_bytes(done), human_bytes(total));
        }
    });
    if shown {
        eprint!("\r\x1b[2K");
    }
    Ok(report?)
}

fn print_wiped(w: &package::WipeReport) {
    let what = match &w.ticket {
        Some(t) => format!("ticket {t}"),
        None => w.dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
    };
    let overwritten = if w.bytes_overwritten > 0 {
        format!(", {} overwritten", human_bytes(w.bytes_overwritten))
    } else {
        String::new()
    };
    println!("✔ Wiped {what}: key destroyed, {} file(s) deleted{overwritten}.", w.files_removed);
    println!("  Remove the transfer code from the ticket when the job is closed.");
}

// ---------------------------------------------------------------- helpers

fn get_code(code: Option<String>, ticket: &str) -> Result<TransferCode, Box<dyn std::error::Error>> {
    if let Some(c) = code {
        return Ok(TransferCode::parse(&c)?);
    }
    loop {
        let Some(line) = prompt(&format!("Transfer code for ticket {ticket} (from the ticket): "))? else {
            return Err("a transfer code is required".into());
        };
        match TransferCode::parse(&line) {
            Ok(c) => return Ok(c),
            Err(e) => println!("{e}"),
        }
    }
}

/// Read one line; `None` on end of input.
fn prompt(text: &str) -> io::Result<Option<String>> {
    print!("{text}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim().to_string()))
}

fn print_list(title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    println!("\n  {title} ({}):", items.len());
    for i in items.iter().take(25) {
        println!("    {i}");
    }
    if items.len() > 25 {
        println!("    … and {} more", items.len() - 25);
    }
}

fn print_failures(title: &str, items: &[(String, String)]) {
    let lines: Vec<String> = items.iter().map(|(p, e)| format!("{p}: {e}")).collect();
    print_list(title, &lines);
}

fn print_skips(title: &str, items: &[SkipItem]) {
    let lines: Vec<String> = items
        .iter()
        .map(|i| {
            let why = match i.reason {
                SkipReason::PermissionDenied => "access denied",
                SkipReason::Locked => "in use by another program",
                SkipReason::Vanished => "disappeared during backup",
                SkipReason::InvalidName => "unsupported file name",
                SkipReason::CloudOnly => "only in OneDrive",
                SkipReason::ReadError => "could not be read",
                SkipReason::Excluded => "excluded",
            };
            format!("{}  ({why}; {})", i.path, i.detail)
        })
        .collect();
    print_list(title, &lines);
}

struct ProgressLine {
    last: Instant,
    shown: bool,
}

impl ProgressLine {
    fn new() -> Self {
        Self { last: Instant::now() - Duration::from_secs(1), shown: false }
    }

    fn update(&mut self, p: &Progress) {
        // Diagnostic mode: every folder and file the engine visits, one per
        // line, so a stall shows exactly where it happened.
        if std::env::var_os("PROFILEPORT_TRACE").is_some() {
            eprintln!("[trace] {} files · {} · {}", p.files, human_bytes(p.bytes), p.current);
            return;
        }
        if !io::stderr().is_terminal() || self.last.elapsed() < Duration::from_millis(150) {
            return;
        }
        self.last = Instant::now();
        self.shown = true;
        let mut current = p.current.clone();
        if current.chars().count() > 50 {
            current = format!(
                "…{}",
                current.chars().rev().take(49).collect::<Vec<_>>().into_iter().rev().collect::<String>()
            );
        }
        eprint!("\r\x1b[2K  {} files · {} · {}", p.files, human_bytes(p.bytes), current);
    }

    fn finish(&self) {
        if self.shown {
            eprint!("\r\x1b[2K");
        }
    }
}

fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn this_computer() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "unknown".into())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "2026-11-20 14:03" (UTC), without a date library.
fn utc_date_time(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's civil-from-days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(utc_date_time(0), "1970-01-01 00:00");
        assert_eq!(utc_date_time(1_795_000_000), "2026-11-18 11:06");
        assert_eq!(utc_date_time(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn bytes() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(58 * 1024 * 1024 * 1024), "58.0 GB");
    }
}
