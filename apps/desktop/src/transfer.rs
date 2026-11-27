//! The desktop app's logic, kept free of Slint so it can be tested: what the
//! screens list, what they check before enabling a button, and the text they
//! show. `main.rs` only moves values between these functions and the UI.

use profileport_engine::{
    self as engine,
    package::{self, PackageReader, PackageState, SkipReason, TransferCode},
    system::{DriveInfo, ProfileInfo},
    *,
};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

// ---------------------------------------------------------------- choices

/// A folder offered in a profile picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub path: PathBuf,
    /// The account name, or the folder's name.
    pub title: String,
    /// The folder.
    pub subtitle: String,
    /// The signed-in user's profile.
    pub current: bool,
}

/// The profiles on this PC, the signed-in user's first.
pub fn profile_choices(profiles: &[ProfileInfo]) -> Vec<Choice> {
    let mut sorted: Vec<&ProfileInfo> = profiles.iter().collect();
    sorted.sort_by_key(|p| (!p.current, p.name.to_lowercase()));
    sorted
        .into_iter()
        .map(|p| Choice {
            path: p.path.clone(),
            title: p.name.clone(),
            subtitle: p.path.display().to_string(),
            current: p.current,
        })
        .collect()
}

/// A folder picked with "Other folder…".
pub fn folder_choice(path: &Path) -> Choice {
    Choice {
        path: path.to_path_buf(),
        title: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
        subtitle: path.display().to_string(),
        current: false,
    }
}

/// A drive offered in the drive pickers.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveChoice {
    pub path: PathBuf,
    /// "KINGSTON (E:)", "E:", "KINGSTON".
    pub title: String,
    /// "USB · 28.1 GB free of 57.6 GB · 2 backups".
    pub subtitle: String,
    pub removable: bool,
    /// Free space as a fraction of the drive, when known.
    pub free_fraction: Option<f32>,
    pub free_text: String,
    pub backups: usize,
}

pub fn drive_choices(drives: &[DriveInfo]) -> Vec<DriveChoice> {
    drives
        .iter()
        .map(|d| {
            let backups = count_backups(&d.path);
            let mut parts = vec![if d.removable { "USB".to_string() } else { "Fixed disk".to_string() }];
            let (free_fraction, free_text) = match (d.free_bytes, d.total_bytes) {
                (Some(free), Some(total)) if total > 0 => {
                    parts.push(format!("{} free of {}", human_bytes(free), human_bytes(total)));
                    (Some(free as f32 / total as f32), human_bytes(free))
                }
                _ => (None, String::new()),
            };
            parts.push(plural(backups, "backup"));
            DriveChoice {
                path: d.path.clone(),
                title: drive_title(d),
                subtitle: parts.join(" · "),
                removable: d.removable,
                free_fraction,
                free_text,
                backups,
            }
        })
        .collect()
}

/// "KINGSTON (E:)" on Windows; the label, or else the folder, elsewhere.
fn drive_title(d: &DriveInfo) -> String {
    let path = d.path.to_string_lossy();
    let letter = path.strip_suffix('\\').filter(|p| p.len() == 2 && p.ends_with(':'));
    match (letter, d.label.is_empty()) {
        (Some(l), false) => format!("{} ({l})", d.label),
        (Some(l), true) => l.to_string(),
        (None, false) => d.label.clone(),
        (None, true) => path.into_owned(),
    }
}

fn plural(n: usize, what: &str) -> String {
    match n {
        0 => format!("no {what}s"),
        1 => format!("1 {what}"),
        n => format!("{n} {what}s"),
    }
}

fn count_backups(drive: &Path) -> usize {
    package::list_packages(&transfers_dir(drive))
        .map(|list| list.iter().filter(|(_, r)| r.is_ok() || matches!(r, Err(package::Error::PartlyWiped))).count())
        .unwrap_or(0)
}

/// How a backup's state is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Good,
    Attention,
    Neutral,
}

/// A backup in the lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupItem {
    pub path: PathBuf,
    /// "Ticket 4711", or the folder name for a partly wiped backup.
    pub title: String,
    /// "27 Nov 2026 14:03 UTC · 2.9 GB · 16 days old".
    pub subtitle: String,
    pub badge_text: String,
    pub badge: Badge,
    pub ticket: Option<String>,
    /// 14 or more days old: should be wiped once the job is done.
    pub old: bool,
    /// The backup was interrupted and can be continued.
    pub interrupted: bool,
    /// An earlier wipe was interrupted: only wiping is possible.
    pub partly_wiped: bool,
}

/// The backups on `drive`, newest first, and a line summarising them.
pub fn list_backups(drive: &Path) -> (Vec<BackupItem>, String) {
    let dir = transfers_dir(drive);
    if !dir.is_dir() {
        return (Vec::new(), "There are no backups on this drive.".into());
    }
    let listed = match package::list_packages(&dir) {
        Ok(l) => l,
        Err(e) => return (Vec::new(), format!("The drive could not be read: {e}")),
    };
    let now = now_unix();
    let (mut items, mut damaged) = (Vec::new(), 0);
    for (path, item) in listed {
        match item {
            Ok(info) => {
                let state = info.status.as_ref().map(|s| s.state);
                let (badge_text, badge) = match state {
                    Some(PackageState::Complete) => ("Complete", Badge::Good),
                    Some(PackageState::CompleteWithIssues) => ("Review", Badge::Attention),
                    Some(PackageState::InProgress) => ("Interrupted", Badge::Attention),
                    None => ("Unknown", Badge::Neutral),
                };
                let days = (now - info.header.created_unix) / 86_400;
                let mut parts = vec![utc_date_time(info.header.created_unix)];
                if let Some(s) = &info.status {
                    parts.push(human_bytes(s.bytes));
                    if s.skipped > 0 {
                        parts.push(plural(s.skipped as usize, "file") + " skipped");
                    }
                }
                if days >= 14 {
                    parts.push(format!("{days} days old"));
                }
                items.push(BackupItem {
                    title: format!("Ticket {}", info.header.ticket),
                    subtitle: parts.join(" · "),
                    badge_text: badge_text.into(),
                    badge,
                    ticket: Some(info.header.ticket),
                    old: days >= 14,
                    interrupted: state == Some(PackageState::InProgress),
                    partly_wiped: false,
                    path,
                });
            }
            Err(package::Error::PartlyWiped) => items.push(BackupItem {
                title: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                subtitle: "Can't be opened any more. Wipe it again to finish.".into(),
                badge_text: "Partly wiped".into(),
                badge: Badge::Attention,
                ticket: None,
                old: false,
                interrupted: false,
                partly_wiped: true,
                path,
            }),
            Err(_) => damaged += 1,
        }
    }
    let mut summary = match items.len() {
        0 => "There are no backups on this drive.".to_string(),
        n => format!("{} on this drive.", plural(n, "backup")),
    };
    if damaged > 0 {
        summary.push_str(&format!(" {damaged} damaged backup(s) not shown; see `profileport-cli list`."));
    }
    (items, summary)
}

// ---------------------------------------------------------------- input checks

/// What to show under the ticket field: empty when it's fine (or untouched).
pub fn ticket_feedback(ticket: &str) -> String {
    if ticket.is_empty() || package::format::validate_ticket(ticket).is_ok() {
        String::new()
    } else {
        "Use up to 32 letters, digits, - or _ (no spaces). The ticket is the only readable label on the drive, \
         so don't use names."
            .into()
    }
}

/// Whether the typed code is a well-formed transfer code, and what to show.
/// A well-formed code can still belong to another backup; that is only known
/// when it is used.
pub fn code_feedback(code: &str) -> (bool, String) {
    let symbols = code.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    if symbols == 0 {
        return (false, String::new());
    }
    match TransferCode::parse(code) {
        Ok(_) => (true, "Code format is correct.".into()),
        Err(_) if symbols < 12 => (false, format!("{symbols} of 12 characters.")),
        Err(e) => (false, format!("{e}.")),
    }
}

/// Cloud-synced folders at the destination, as lines to show. Restoring into
/// them uploads the files, so the app asks first.
pub fn cloud_folders(target: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if is_cloud_synced(target) {
        out.push(target.display().to_string());
    }
    for (kf, path) in system::known_folders(target) {
        if is_cloud_synced(&path) {
            out.push(format!("{kf:?}: {}", path.display()));
        }
    }
    out
}

// ---------------------------------------------------------------- actions

pub fn prepare(source: &Path, drive: &Path, ticket: &str) -> Result<(PreparedExport, TransferCode)> {
    let source = source.canonicalize()?;
    let drive = drive.canonicalize()?;
    if !drive.is_dir() || drive.starts_with(&source) {
        return Err("Choose a drive outside the profile being backed up.".into());
    }
    let labels = SourceLabels {
        computer: std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "unknown".into()),
        profile: source.file_name().unwrap_or_default().to_string_lossy().into_owned(),
    };
    Ok(engine::prepare_export(&source, &transfers_dir(&drive), ticket, &labels, &Default::default())?)
}

/// Remove a prepared package that the technician cancelled before copying.
/// Returns the text to show.
pub fn cancel(job: PreparedExport) -> String {
    let dir = job.package_dir().to_path_buf();
    match job.discard() {
        Ok(()) => "Nothing was copied, and the empty backup was removed from the drive.".into(),
        Err(e) => format!("Nothing was copied, but the empty backup at {} could not be removed: {e}", dir.display()),
    }
}

pub fn verify_package(path: &Path, code: &str, progress: &mut dyn FnMut(&Progress)) -> Result<VerifyReport> {
    let code = TransferCode::parse(code)?;
    let reader = PackageReader::open(path, &code)?;
    Ok(engine::verify(&reader, progress)?)
}

pub fn restore_package(
    path: &Path,
    target: &Path,
    code: &str,
    allow_cloud: bool,
    progress: &mut dyn FnMut(&Progress),
) -> Result<RestoreReport> {
    let code = TransferCode::parse(code)?;
    let reader = PackageReader::open(path, &code)?;
    if !allow_cloud && !cloud_folders(target).is_empty() {
        return Err("The destination has cloud-synced folders. Confirm the upload before restoring.".into());
    }
    let options = RestoreOptions { known_folders: Some(system::known_folders(target)), ..Default::default() };
    Ok(engine::restore(&reader, target, &options, progress)?)
}

/// The question to ask before wiping the backup at `path`.
pub fn wipe_question(path: &Path) -> Result<String> {
    match package::read_info(path) {
        Ok(info) => Ok(format!(
            "Wipe the backup for ticket {} from the drive? Its key is destroyed and its files are deleted. This \
             can't be undone, so only wipe it once the restore is verified and the customer has checked their files.",
            info.header.ticket
        )),
        Err(package::Error::PartlyWiped) => {
            Ok("This backup was partly wiped earlier and can't be opened any more. Finish wiping it?".into())
        }
        Err(e) => Err(e.into()),
    }
}

/// Wipe the backup at `path`. Returns the text to show.
pub fn wipe_package(path: &Path, overwrite: bool, progress: &mut dyn FnMut(&Progress)) -> Result<String> {
    wipe_question(path)?; // refuses what isn't a backup before touching anything
    let r = package::wipe(path, &package::WipeOptions { overwrite_data: overwrite }, &mut |done, total| {
        progress(&Progress {
            files: 0,
            bytes: done,
            current: format!("Overwriting backup data: {} of {}", human_bytes(done), human_bytes(total)),
        })
    })?;
    let mut text = format!("The key was destroyed and {} files were deleted from the drive.", r.files_removed);
    if r.bytes_overwritten > 0 {
        text.push_str(&format!(" {} of backup data was overwritten first.", human_bytes(r.bytes_overwritten)));
    }
    text.push_str(" The code in the ticket's sensitive-data field is cleared when the ticket is set to finished.");
    Ok(text)
}

/// The question to ask before wiping every backup on a drive.
pub fn wipe_all_question(items: &[BackupItem]) -> String {
    let tickets: Vec<&str> = items.iter().filter_map(|b| b.ticket.as_deref()).collect();
    let mut q = format!(
        "Wipe all {} from this drive, ready for the next customer? Each backup's key is destroyed and its files \
         are deleted. This can't be undone.",
        plural(items.len(), "backup")
    );
    if !tickets.is_empty() {
        q.push_str(&format!(" Tickets: {}.", tickets.join(", ")));
    }
    q
}

/// Wipe every backup on `drive` (including partly wiped ones). Damaged
/// backups and anything else in the folder are left alone. Returns
/// (all wiped, text to show).
pub fn wipe_all(drive: &Path, overwrite: bool, progress: &mut dyn FnMut(&Progress)) -> (bool, String) {
    let (items, _) = list_backups(drive);
    let (mut wiped, mut failed) = (0, Vec::new());
    for item in &items {
        match wipe_package(&item.path, overwrite, progress) {
            Ok(_) => wiped += 1,
            Err(e) => failed.push(format!("{}: {e}", item.title)),
        }
    }
    let mut text = format!("{} wiped from the drive.", plural(wiped, "backup"));
    if !failed.is_empty() {
        text.push_str(&format!("\nCould not be wiped:\n{}", failed.join("\n")));
    }
    text.push_str(
        "\nThe codes in the tickets' sensitive-data fields are cleared when the tickets are set to finished.",
    );
    (failed.is_empty(), text)
}

// ---------------------------------------------------------------- report text

pub fn progress_text(p: &Progress) -> String {
    if p.files == 0 && p.bytes > 0 && p.current.starts_with("Overwriting") {
        return p.current.clone();
    }
    format!("{} files · {}\n{}", p.files, human_bytes(p.bytes), p.current)
}

/// (title, details) after a backup.
pub fn export_summary(r: &ExportReport) -> (String, String) {
    let title = if r.has_issues() { "Backup finished — some items need review" } else { "Backup complete" };
    let mut lines = vec![format!(
        "{} files and {} folders, {}. Not yet verified: while this PC is still here, choose Check backup now and enter the code from the \
         ticket. That proves the stored code is right.",
        r.files,
        r.dirs,
        human_bytes(r.bytes)
    )];
    lines.extend(r.skipped.iter().map(|s| format!("Not copied: {} — {}", s.path, skip_reason(s.reason))));
    lines.extend(r.changed.iter().map(|p| format!("Changed while being copied (check it): {p}")));
    if !r.excluded.is_empty() {
        lines.push(format!("{} cache or temporary location(s) left out by rule.", r.excluded.len()));
    }
    if !r.links.is_empty() {
        lines.push(format!("{} link(s) recorded, not followed.", r.links.len()));
    }
    (title.into(), lines.join("\n"))
}

/// (title, details) after checking a backup.
pub fn verify_summary(r: &VerifyReport) -> (String, String) {
    let ok = r.complete && r.failed.is_empty();
    let title = if ok { "Backup checked — all stored files are intact" } else { "Check found problems" };
    let mut lines = vec![format!(
        "{} files ({}) were read back and matched. This checks what is stored, not whether every file on the \
         old PC was copied: see the backup report for that.",
        r.files,
        human_bytes(r.bytes)
    )];
    if !r.complete {
        lines.push("The backup was interrupted. Continue it on the old PC before relying on it.".into());
    }
    lines.extend(r.incomplete_files.iter().map(|p| format!("Incomplete: {p}")));
    lines.extend(r.failed.iter().map(|(p, e)| format!("Damaged: {p} — {e}")));
    (title.into(), lines.join("\n"))
}

/// (title, details) after a restore.
pub fn restore_summary(r: &RestoreReport) -> (String, String) {
    let title =
        if r.fully_successful() { "Restore complete and checked" } else { "Restore finished — review needed" };
    let mut d = vec![format!(
        "{} files restored ({}) and checked: {}. {} already there, identical.",
        r.files,
        human_bytes(r.bytes),
        r.verified,
        r.already_present
    )];
    if !r.backup_complete {
        d.push("The backup was incomplete, so files may be missing.".into());
    }
    d.extend(r.failed.iter().map(|(p, e)| format!("Failed: {p} — {e}")));
    d.extend(r.skipped_at_backup.iter().map(|s| format!("Not in the backup: {} — {}", s.path, skip_reason(s.reason))));
    d.extend(r.changed_at_backup.iter().map(|p| format!("Changed while being backed up (check it): {p}")));
    d.extend(r.incomplete_at_backup.iter().map(|p| format!("Incomplete in the backup: {p}")));
    d.extend(r.conflicts.iter().map(|(p, c)| match c {
        ConflictOutcome::LeftExisting => format!("Already existed with different content, left as is: {p}"),
        ConflictOutcome::RestoredAs(n) => format!("Already existed, restored as {n}: {p}"),
    }));
    d.extend(r.renamed.iter().map(|(p, n, _)| format!("Renamed for this system: {p} → {n}")));
    d.extend(r.links_not_restored.iter().map(|(p, t)| format!("Link not restored: {p} → {t}")));
    if r.fully_successful() {
        d.push("When the customer has checked their files, wipe the backup from the drive.".into());
    }
    (title.into(), d.join("\n"))
}

fn skip_reason(r: SkipReason) -> &'static str {
    match r {
        SkipReason::PermissionDenied => "access denied",
        SkipReason::Locked => "in use by another program",
        SkipReason::Vanished => "disappeared during the backup",
        SkipReason::InvalidName => "unsupported file name",
        SkipReason::CloudOnly => "only in the cloud, not on this PC",
        SkipReason::ReadError => "could not be read",
        SkipReason::Excluded => "excluded",
    }
}

// ---------------------------------------------------------------- formatting

pub fn human_bytes(b: u64) -> String {
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

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "27 Nov 2026 14:03 UTC", without a date library (same algorithm as the CLI).
fn utc_date_time(unix: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{d} {} {y} {:02}:{:02} UTC", MONTHS[(m - 1) as usize], secs / 3600, secs % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture {
        _temp: tempfile::TempDir,
        source: PathBuf,
        drive: PathBuf,
        target: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let drive = temp.path().join("drive");
        let target = temp.path().join("target");
        for d in [&source, &drive, &target] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(source.join("note.txt"), b"important").unwrap();
        Fixture { _temp: temp, source, drive, target }
    }

    fn backup(f: &Fixture, ticket: &str) -> (PathBuf, TransferCode) {
        let (prepared, code) = prepare(&f.source, &f.drive, ticket).unwrap();
        (prepared.run(&code, &Default::default(), &mut |_| {}).unwrap().package_dir, code)
    }

    #[test]
    fn desktop_transfer_roundtrip_and_verification() {
        let f = fixture();
        let (dir, code) = backup(&f, "123");
        let verified = verify_package(&dir, &code.to_string(), &mut |_| {}).unwrap();
        assert!(verified.complete && verified.failed.is_empty());
        assert!(verify_summary(&verified).0.contains("intact"));
        let restored = restore_package(&dir, &f.target, &code.to_string(), false, &mut |_| {}).unwrap();
        assert!(restored.fully_successful());
        let (title, details) = restore_summary(&restored);
        assert!(title.contains("complete") && details.contains("wipe the backup"), "{title}\n{details}");
        assert_eq!(fs::read(f.target.join("note.txt")).unwrap(), b"important");
    }

    #[test]
    fn cloud_destination_requires_confirmation_even_without_known_folders() {
        let f = fixture();
        let target = f.target.join("OneDrive");
        fs::create_dir(&target).unwrap();
        assert!(!cloud_folders(&target).is_empty());
        assert!(cloud_folders(&f.target).is_empty());
        let (dir, code) = backup(&f, "123");
        assert!(restore_package(&dir, &target, &code.to_string(), false, &mut |_| {}).is_err());
        assert!(!target.join("note.txt").exists());
    }

    #[test]
    fn cancelling_before_copying_removes_the_empty_package() {
        let f = fixture();
        let (prepared, _code) = prepare(&f.source, &f.drive, "123").unwrap();
        let dir = prepared.package_dir().to_path_buf();
        assert!(dir.is_dir());
        let message = cancel(prepared);
        assert!(message.contains("removed"), "{message}");
        assert!(!dir.exists());
        assert!(transfers_dir(&f.drive).is_dir(), "only the package itself is removed");
        assert!(f.source.join("note.txt").exists());
    }

    #[test]
    fn backup_drive_inside_source_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let drive = temp.path().join("drive");
        fs::create_dir(&drive).unwrap();
        assert!(prepare(temp.path(), &drive, "123").is_err());
    }

    #[test]
    fn backups_are_listed_by_ticket_and_wiped_without_typing() {
        let f = fixture();
        let (dir, _) = backup(&f, "4711");
        let (items, summary) = list_backups(&f.drive);
        assert_eq!(summary, "1 backup on this drive.");
        assert_eq!(items.len(), 1);
        // `prepare` canonicalises the drive (on Windows a `\\?\` long path);
        // the listing keeps the drive path as given, e.g. an 8.3 short name.
        assert_eq!(items[0].path.canonicalize().unwrap(), dir.canonicalize().unwrap());
        assert_eq!(items[0].ticket.as_deref(), Some("4711"));
        assert_eq!(items[0].title, "Ticket 4711");
        assert!(items[0].subtitle.ends_with("UTC · 9 B"), "{}", items[0].subtitle);
        assert_eq!((items[0].badge_text.as_str(), items[0].badge), ("Complete", Badge::Good));
        assert!(!items[0].interrupted && !items[0].partly_wiped && !items[0].old);
        assert!(wipe_question(&items[0].path).unwrap().contains("ticket 4711"));

        let message = wipe_package(&items[0].path, true, &mut |_| {}).unwrap();
        assert!(message.contains("key was destroyed") && message.contains("overwritten"), "{message}");
        assert!(!dir.exists());
        assert!(transfers_dir(&f.drive).is_dir());
        assert!(f.source.join("note.txt").exists());
        assert!(list_backups(&f.drive).0.is_empty());
    }

    #[test]
    fn an_unstarted_backup_is_listed_as_interrupted() {
        let f = fixture();
        let (prepared, _code) = prepare(&f.source, &f.drive, "77").unwrap();
        drop(prepared);
        let (items, _) = list_backups(&f.drive);
        assert!(items[0].interrupted);
        assert_eq!((items[0].badge_text.as_str(), items[0].badge), ("Interrupted", Badge::Attention));
    }

    #[test]
    fn preparing_for_the_next_customer_wipes_every_backup() {
        let f = fixture();
        backup(&f, "1");
        backup(&f, "2");
        let (items, _) = list_backups(&f.drive);
        let q = wipe_all_question(&items);
        assert!(q.contains("all 2 backups") && q.contains("1") && q.contains("2"), "{q}");
        fs::write(f.drive.join("notes.txt"), b"not a backup").unwrap();
        let (ok, text) = wipe_all(&f.drive, false, &mut |_| {});
        assert!(ok, "{text}");
        assert!(text.starts_with("2 backups wiped"), "{text}");
        assert!(list_backups(&f.drive).0.is_empty());
        assert!(f.drive.join("notes.txt").exists(), "only backups are touched");
    }

    #[test]
    fn wiping_refuses_a_folder_that_is_not_a_backup() {
        let f = fixture();
        assert!(wipe_question(&f.source).is_err());
        assert!(wipe_package(&f.source, false, &mut |_| {}).is_err());
        assert!(f.source.join("note.txt").exists());
    }

    #[test]
    fn drive_titles_space_and_backup_count() {
        let f = fixture();
        backup(&f, "1");
        backup(&f, "2");
        let gib = 1024 * 1024 * 1024;
        let drives = [
            DriveInfo {
                path: f.drive.clone(),
                label: "KINGSTON".into(),
                removable: true,
                free_bytes: Some(2 * gib),
                total_bytes: Some(8 * gib),
            },
            DriveInfo {
                path: f.target.clone(),
                label: String::new(),
                removable: false,
                free_bytes: None,
                total_bytes: None,
            },
        ];
        let choices = drive_choices(&drives);
        assert_eq!(choices[0].title, "KINGSTON");
        assert_eq!(choices[0].subtitle, "USB · 2.0 GB free of 8.0 GB · 2 backups");
        assert_eq!(
            (choices[0].free_fraction, choices[0].free_text.as_str(), choices[0].backups),
            (Some(0.25), "2.0 GB", 2)
        );
        assert_eq!(choices[1].title, f.target.display().to_string());
        assert_eq!(choices[1].subtitle, "Fixed disk · no backups");
        assert_eq!(choices[1].free_fraction, None);

        let letter = |label: &str| DriveInfo {
            path: PathBuf::from("E:\\"),
            label: label.into(),
            removable: true,
            free_bytes: None,
            total_bytes: None,
        };
        assert_eq!(drive_title(&letter("KINGSTON")), "KINGSTON (E:)");
        assert_eq!(drive_title(&letter("")), "E:");
    }

    #[test]
    fn signed_in_profile_comes_first() {
        let p = |name: &str, current| ProfileInfo { name: name.into(), path: PathBuf::from(name), sid: None, current };
        let choices = profile_choices(&[p("zoe", false), p("Bob", false), p("anna", true)]);
        let names: Vec<&str> = choices.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(names, ["anna", "Bob", "zoe"]);
        assert!(choices[0].current && !choices[1].current);
        assert_eq!(folder_choice(Path::new("/srv/old-disk/Users/anna")).title, "anna");
    }

    #[test]
    fn input_feedback() {
        assert_eq!(ticket_feedback(""), "");
        assert_eq!(ticket_feedback("INC-4711"), "");
        assert!(!ticket_feedback("anna smith").is_empty());

        assert_eq!(code_feedback(""), (false, String::new()));
        assert_eq!(code_feedback("7KQ2-M9"), (false, "6 of 12 characters.".into()));
        let code = TransferCode::generate().to_string();
        assert!(code_feedback(&code).0);
        assert!(code_feedback(&code.to_lowercase().replace('-', " ")).0, "entry is forgiving");
        // The check symbols are a function of the data symbols, so changing
        // the last one always gives an invalid code. (Changing a data symbol
        // is caught only almost always.)
        let mut typo: Vec<char> = code.chars().collect();
        let last = typo.len() - 1;
        typo[last] = if typo[last] == 'A' { 'B' } else { 'A' };
        let (ok, message) = code_feedback(&typo.into_iter().collect::<String>());
        assert!(!ok && !message.is_empty());
    }

    #[test]
    fn formatting() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(utc_date_time(1_795_000_000), "18 Nov 2026 11:06 UTC");
        assert_eq!(utc_date_time(951_782_400), "29 Feb 2000 00:00 UTC");
    }
}
