//! `profileport` — command-line front end to the migration engine.
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
#[command(name = "profileport", version, about = "Move a Windows user's files between PCs via a USB drive")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
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
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::List { drive } => list(&drive),
        Cmd::Export { source, drive, ticket, computer, profile, no_confirm } => {
            run_export(&source, &drive, &ticket, computer, profile, no_confirm)
        }
        Cmd::Resume { package, code } => run_resume(&package, code),
        Cmd::Verify { package, code } => run_verify(&package, code),
        Cmd::Restore { package, to, code, keep_both, no_verify } => {
            run_restore(&package, &to, code, keep_both, no_verify)
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
    for p in packages {
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
            Err(e) => println!("(damaged package: {e})"),
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
        confirm_code(&code, prepared.package_dir())?;
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
    println!("  │  Write this on the DEVICE TAG.                               │");
    println!("  │  Don't enter it in the ticket system.                        │");
    println!("  │  Without it this backup can't be restored or resumed.        │");
    println!("  └──────────────────────────────────────────────────────────────┘");
}

fn confirm_code(code: &TransferCode, package_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        let Some(line) = prompt("\nType the code back to confirm you've written it down: ")? else {
            return Err(format!(
                "no confirmation given. The empty backup is at {}; delete it or start again.",
                package_dir.display()
            )
            .into());
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
        println!("⚠ This backup was interrupted and is incomplete. Run `profileport resume`.");
    }
    print_list("Incomplete when the backup stopped", &r.incomplete_files);
    print_failures("DAMAGED", &r.failed);
    Ok(if r.failed.is_empty() && r.complete { Outcome::Clean } else { Outcome::Issues })
}

// ---------------------------------------------------------------- restore

fn run_restore(package_dir: &Path, to: &Path, code: Option<String>, keep_both: bool, no_verify: bool) -> Res {
    let info = package::read_info(package_dir)?;
    let code = get_code(code, &info.header.ticket)?;
    let reader = PackageReader::open(package_dir, &code)?;
    let m = reader.manifest()?;
    println!("Ticket {}: {} on {}  →  {}", info.header.ticket, m.begin.profile, m.begin.computer, to.display());
    if !m.complete {
        println!("⚠ This backup is incomplete (it was interrupted). Restoring what it contains.");
    }
    let opts = RestoreOptions {
        on_conflict: if keep_both { ConflictPolicy::KeepBoth } else { ConflictPolicy::Skip },
        verify: !no_verify,
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
    println!("  {} files restored ({}), {} verified, {} folders", r.files, human_bytes(r.bytes), r.verified, r.dirs);
    if r.already_present > 0 {
        println!("  {} files were already there with identical content", r.already_present);
    }
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
    print_list("Incomplete in the backup", &r.incomplete_at_backup);
    let links: Vec<String> = r.links_not_restored.iter().map(|(p, t)| format!("{p} → {t}")).collect();
    print_list("Links not restored (Windows recreates the standard ones)", &links);

    if r.fully_successful() && io::stdin().is_terminal() {
        if let Some(answer) = prompt("\nRestore verified. Delete this backup from the drive now? [y/N] ")? {
            if answer.trim().eq_ignore_ascii_case("y") {
                std::fs::remove_dir_all(package_dir)?;
                println!("Backup deleted. Discard the device tag when the job is closed.");
            }
        }
    }
    Ok(if r.fully_successful() { Outcome::Clean } else { Outcome::Issues })
}

// ---------------------------------------------------------------- helpers

fn get_code(code: Option<String>, ticket: &str) -> Result<TransferCode, Box<dyn std::error::Error>> {
    if let Some(c) = code {
        return Ok(TransferCode::parse(&c)?);
    }
    loop {
        let Some(line) = prompt(&format!("Transfer code for ticket {ticket} (from the device tag): "))? else {
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
