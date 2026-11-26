use profileport_engine::{
    self as engine,
    package::{PackageReader, TransferCode},
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
    fn backup_drive_inside_source_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let drive = temp.path().join("drive");
        fs::create_dir(&drive).unwrap();
        assert!(prepare(temp.path(), &drive, "123").is_err());
    }
}
