use profileport_engine::{
    self as engine,
    package::{self, PackageReader, TransferCode},
    *,
};
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn prepare(source: &Path, drive: &Path, ticket: &str) -> Result<(PreparedExport, TransferCode)> {
    let source = source.canonicalize()?;
    let drive = drive.canonicalize()?;
    if !drive.is_dir() || drive.starts_with(&source) {
        return Err("Choose an existing drive outside the source profile.".into());
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
        Ok(()) => "Nothing was copied. The empty package was removed from the drive.".into(),
        Err(e) => format!("Nothing was copied, but the empty package at {} could not be removed: {e}", dir.display()),
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
    let folders = system::known_folders(target);
    if !allow_cloud && (is_cloud_synced(target) || folders.iter().any(|(_, path)| is_cloud_synced(path))) {
        return Err("Destination contains cloud-synced folders. Confirm cloud upload before restoring.".into());
    }
    let options = RestoreOptions { known_folders: Some(folders), ..Default::default() };
    Ok(engine::restore(&reader, target, &options, progress)?)
}

/// What the technician must type to confirm wiping the backup at `path`:
/// its ticket number, or `WIPE` if an earlier wipe already destroyed the
/// header. Also returns the instruction to show.
pub fn wipe_confirmation(path: &Path) -> Result<(String, String)> {
    match package::read_info(path) {
        Ok(info) => Ok((
            info.header.ticket.clone(),
            format!(
                "Type the ticket number {} to wipe this backup from the drive. This cannot be undone: \
                 only wipe it once the restore is verified and the customer has checked their files.",
                info.header.ticket
            ),
        )),
        Err(package::Error::PartlyWiped) => Ok((
            "WIPE".into(),
            "This backup was partly wiped earlier and can no longer be opened. Type WIPE to finish removing it.".into(),
        )),
        Err(e) => Err(e.into()),
    }
}

/// Wipe the backup at `path` if `typed` matches [`wipe_confirmation`].
/// Returns the text to show.
pub fn wipe_package(path: &Path, typed: &str, overwrite: bool, progress: &mut dyn FnMut(&Progress)) -> Result<String> {
    let (expected, _) = wipe_confirmation(path)?;
    if typed.trim() != expected {
        return Err(format!("Type {expected} exactly to confirm. Nothing was wiped.").into());
    }
    let r = package::wipe(path, &package::WipeOptions { overwrite_data: overwrite }, &mut |done, total| {
        progress(&Progress {
            files: 0,
            bytes: done,
            current: format!("Overwriting backup data: {done} of {total} bytes"),
        })
    })?;
    let mut text = format!("The key was destroyed and {} files were deleted from the drive.", r.files_removed);
    if r.bytes_overwritten > 0 {
        text.push_str(&format!(" {} bytes of backup data were overwritten first.", r.bytes_overwritten));
    }
    text.push_str(" Discard the device tag with the transfer code when the job is closed.");
    Ok(text)
}

/// One line per backup found on the drive, with the folder it refers to.
/// Damaged backups are listed but can only be inspected; partly wiped ones
/// can be selected so the wipe can be finished.
pub fn list_backups(drive: &Path) -> Result<(Vec<std::path::PathBuf>, Vec<String>, String)> {
    if !drive.is_dir() {
        return Err("Choose an existing drive first".into());
    }
    let dir = transfers_dir(drive);
    if !dir.is_dir() {
        return Ok((Vec::new(), Vec::new(), "No backups found on this drive".into()));
    }
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (mut paths, mut labels, mut damaged) = (Vec::new(), Vec::new(), 0);
    for (path, item) in package::list_packages(&dir)? {
        let folder = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match item {
            Ok(info) => {
                let state = match info.status.as_ref().map(|s| s.state) {
                    Some(package::PackageState::Complete) => "complete",
                    Some(package::PackageState::CompleteWithIssues) => "complete with items to review",
                    Some(package::PackageState::InProgress) => "interrupted",
                    None => "unknown state",
                };
                let days = (now - info.header.created_unix) / 86_400;
                let age = if days >= 14 { format!(" · {days} days old, wipe when done") } else { String::new() };
                labels.push(format!("Ticket {} · {state}{age} · {folder}", info.header.ticket));
                paths.push(path);
            }
            Err(package::Error::PartlyWiped) => {
                labels.push(format!("Partly wiped — wipe again to finish · {folder}"));
                paths.push(path);
            }
            Err(_) => damaged += 1,
        }
    }
    let mut status = format!("{} backups found", paths.len());
    if damaged > 0 {
        status.push_str(&format!("; {damaged} damaged entries not shown (see `profileport-cli list`)"));
    }
    Ok((paths, labels, status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn desktop_transfer_roundtrip_and_verification() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let drive = temp.path().join("drive");
        let target = temp.path().join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&drive).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("note.txt"), b"important").unwrap();
        let (prepared, code) = prepare(&source, &drive, "123").unwrap();
        let report = prepared.run(&code, &Default::default(), &mut |_| {}).unwrap();
        let verified = verify_package(&report.package_dir, &code.to_string(), &mut |_| {}).unwrap();
        assert!(verified.complete && verified.failed.is_empty());
        let restored = restore_package(&report.package_dir, &target, &code.to_string(), false, &mut |_| {}).unwrap();
        assert!(restored.fully_successful());
        assert_eq!(fs::read(target.join("note.txt")).unwrap(), b"important");
    }

    #[test]
    fn cloud_destination_requires_confirmation_even_without_known_folders() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let drive = temp.path().join("drive");
        let target = temp.path().join("OneDrive");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&drive).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("note.txt"), b"important").unwrap();
        let (prepared, code) = prepare(&source, &drive, "123").unwrap();
        let report = prepared.run(&code, &Default::default(), &mut |_| {}).unwrap();
        let result = restore_package(&report.package_dir, &target, &code.to_string(), false, &mut |_| {});
        assert!(result.is_err());
        assert!(!target.join("note.txt").exists());
    }

    #[test]
    fn cancelling_before_copying_removes_the_empty_package() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let drive = temp.path().join("drive");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&drive).unwrap();
        fs::write(source.join("note.txt"), b"important").unwrap();
        let (prepared, _code) = prepare(&source, &drive, "123").unwrap();
        let dir = prepared.package_dir().to_path_buf();
        assert!(dir.is_dir());
        let message = cancel(prepared);
        assert!(message.contains("removed"), "{message}");
        assert!(!dir.exists());
        assert!(transfers_dir(&drive).is_dir(), "only the package itself is removed");
        assert!(source.join("note.txt").exists());
    }

    #[test]
    fn backup_drive_inside_source_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let drive = temp.path().join("drive");
        fs::create_dir(&drive).unwrap();
        assert!(prepare(temp.path(), &drive, "123").is_err());
    }

    #[test]
    fn wipe_needs_the_ticket_typed_and_removes_only_the_backup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let drive = temp.path().join("drive");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&drive).unwrap();
        fs::write(source.join("note.txt"), b"important").unwrap();
        let (prepared, code) = prepare(&source, &drive, "4711").unwrap();
        let dir = prepared.run(&code, &Default::default(), &mut |_| {}).unwrap().package_dir;

        let (paths, labels, _) = list_backups(&drive).unwrap();
        // `prepare` canonicalises the drive (on Windows a `\\?\` long path);
        // the listing keeps the drive path as given, e.g. an 8.3 short name.
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0].canonicalize().unwrap(), dir.canonicalize().unwrap());
        assert!(labels[0].contains("4711"), "{labels:?}");

        let (expected, prompt) = wipe_confirmation(&dir).unwrap();
        assert_eq!(expected, "4711");
        assert!(prompt.contains("4711"));
        assert!(wipe_package(&dir, "", false, &mut |_| {}).is_err());
        assert!(wipe_package(&dir, "4712", false, &mut |_| {}).is_err());
        assert!(dir.is_dir(), "a wrong confirmation wipes nothing");

        let message = wipe_package(&dir, " 4711 ", true, &mut |_| {}).unwrap();
        assert!(message.contains("key was destroyed"), "{message}");
        assert!(message.contains("overwritten"), "{message}");
        assert!(!dir.exists());
        assert!(transfers_dir(&drive).is_dir());
        assert!(source.join("note.txt").exists());
        assert!(list_backups(&drive).unwrap().0.is_empty());
    }
}
