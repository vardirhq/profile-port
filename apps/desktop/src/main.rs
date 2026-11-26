#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod transfer;
slint::include_modules!();

use profileport_engine::{
    self as engine, package::TransferCode, ExportOptions, ExportReport, PreparedExport, Progress,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// The package prepared for backup, waiting for the code to be confirmed.
/// Shared with the worker thread that prepares it, hence `Arc<Mutex>`.
type Prepared = Arc<Mutex<Option<(PreparedExport, TransferCode)>>>;

fn take_prepared(prepared: &Prepared) -> Option<(PreparedExport, TransferCode)> {
    prepared.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn progress_callback(weak: slint::Weak<AppWindow>) -> impl FnMut(&Progress) {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |p| {
        if last.elapsed() < Duration::from_millis(150) {
            return;
        }
        last = Instant::now();
        let text = format!("{} files · {} bytes\n{}", p.files, p.bytes, p.current);
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_details(text.into());
            }
        });
    }
}

fn run_job<F>(app: &AppWindow, title: &str, task: F)
where
    F: FnOnce(slint::Weak<AppWindow>) -> Result<(String, String), String> + Send + 'static,
{
    app.set_busy(true);
    app.set_status(title.into());
    app.set_details("Working…".into());
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let result = task(weak.clone());
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_busy(false);
                match result {
                    Ok((status, details)) => {
                        app.set_status(status.into());
                        app.set_details(details.into());
                    }
                    Err(error) => {
                        app.set_status("Operation failed".into());
                        app.set_details(error.into());
                    }
                }
            }
        });
    });
}

fn export_result(report: ExportReport) -> (String, String) {
    let title = if report.has_issues() {
        "Backup finished — review required"
    } else {
        "Backup complete — not yet verified"
    };
    let mut lines = vec![format!(
        "{} files · {} bytes\nBackup folder: {}",
        report.files,
        report.bytes,
        report.package_dir.display()
    )];
    lines.extend(report.skipped.iter().map(|s| format!("Not copied: {} — {}", s.path, s.detail)));
    lines.extend(report.changed.iter().map(|p| format!("Changed while copying: {p}")));
    lines.push(format!(
        "{} excluded cache or temporary locations; {} links recorded but not followed.",
        report.excluded.len(),
        report.links.len()
    ));
    (title.into(), lines.join("\n"))
}

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    app.global::<Theme>().set_dark(matches!(std::env::var("PROFILEPORT_THEME").as_deref(), Ok("dark")));
    app.set_version(env!("CARGO_PKG_VERSION").into());
    let profiles = Rc::new(engine::system::list_profiles());
    let mut labels: Vec<SharedString> =
        profiles.iter().map(|p| format!("{} — {}", p.name, p.path.display()).into()).collect();
    if labels.is_empty() {
        labels.push("No profiles found — enter a source folder".into());
    }
    app.set_profiles(ModelRc::new(VecModel::from(labels)));
    if let Some(p) = profiles.first() {
        app.set_source(p.path.to_string_lossy().as_ref().into());
    }
    let weak = app.as_weak();
    let profile_sources = profiles.clone();
    app.on_profile_selected(move |i| {
        if let (Some(app), Some(profile)) = (weak.upgrade(), profile_sources.get(i as usize)) {
            app.set_source(profile.path.to_string_lossy().as_ref().into());
        }
    });

    if let Some(p) = profiles.first() {
        app.set_target_path(p.path.to_string_lossy().as_ref().into());
    }
    let weak = app.as_weak();
    app.on_target_selected(move |i| {
        if let (Some(app), Some(profile)) = (weak.upgrade(), profiles.get(i as usize)) {
            app.set_target_path(profile.path.to_string_lossy().as_ref().into());
        }
    });
    let packages: Rc<RefCell<Vec<PathBuf>>> = Rc::new(RefCell::new(Vec::new()));
    let weak = app.as_weak();
    let package_paths = packages.clone();
    app.on_scan_drive(move || {
        if let Some(app) = weak.upgrade() {
            let root = PathBuf::from(app.get_drive().as_str());
            let dir = engine::transfers_dir(&root);
            if !root.is_dir() {
                app.set_status("Choose an existing drive first".into());
                return;
            }
            if !dir.is_dir() {
                app.set_packages(ModelRc::new(VecModel::default()));
                package_paths.borrow_mut().clear();
                app.set_status("No backups found on this drive".into());
                return;
            }
            match engine::package::list_packages(&dir) {
                Ok(items) => {
                    let mut paths = Vec::new();
                    let mut labels: Vec<SharedString> = Vec::new();
                    let mut damaged = 0;
                    for item in items {
                        match item {
                            Ok(info) => {
                                let state = info
                                    .status
                                    .as_ref()
                                    .map(|s| format!("{:?}", s.state))
                                    .unwrap_or_else(|| "unknown".into());
                                labels.push(
                                    format!(
                                        "Ticket {} · {} · {}",
                                        info.header.ticket,
                                        state,
                                        info.dir.file_name().unwrap_or_default().to_string_lossy()
                                    )
                                    .into(),
                                );
                                paths.push(info.dir);
                            }
                            Err(_) => damaged += 1,
                        }
                    }
                    app.set_status(format!("{} backups found; {} damaged entries", paths.len(), damaged).into());
                    app.set_packages(ModelRc::new(VecModel::from(labels)));
                    if let Some(first) = paths.first() {
                        app.set_package_path(first.to_string_lossy().as_ref().into());
                    }
                    *package_paths.borrow_mut() = paths;
                }
                Err(e) => {
                    app.set_status("Could not list backups".into());
                    app.set_details(e.to_string().into());
                }
            }
        }
    });
    let weak = app.as_weak();
    app.on_package_selected(move |i| {
        if let (Some(app), Some(path)) = (weak.upgrade(), packages.borrow().get(i as usize)) {
            app.set_package_path(path.to_string_lossy().as_ref().into());
        }
    });
    let prepared: Prepared = Arc::new(Mutex::new(None));
    let weak = app.as_weak();
    let pending = prepared.clone();
    app.on_cancel_preparation(move || {
        if let Some(app) = weak.upgrade() {
            app.set_awaiting_confirmation(false);
            app.set_displayed_code("".into());
            app.set_code_entry("".into());
            app.set_status("Backup cancelled before copying".into());
            let details = match take_prepared(&pending) {
                Some((job, _)) => transfer::cancel(job),
                None => "Nothing was copied.".into(),
            };
            app.set_details(details.into());
        }
    });
    let weak = app.as_weak();
    let pending = prepared.clone();
    app.on_prepare_backup(move || {
        let Some(app) = weak.upgrade() else { return };
        let source = PathBuf::from(app.get_source().as_str());
        let drive = PathBuf::from(app.get_drive().as_str());
        let ticket = app.get_ticket().to_string();
        app.set_busy(true);
        app.set_status("Preparing backup…".into());
        app.set_details("Creating the package and its transfer code.".into());
        let weak = weak.clone();
        let pending = pending.clone();
        // Key derivation (Argon2id) takes a noticeable moment: keep it off
        // the UI thread so the window stays responsive.
        std::thread::spawn(move || {
            let result = transfer::prepare(&source, &drive, &ticket).map_err(|e| e.to_string());
            let _ = slint::invoke_from_event_loop(move || {
                let Some(app) = weak.upgrade() else { return };
                app.set_busy(false);
                match result {
                    Ok((job, code)) => {
                        app.set_package_path(job.package_dir().to_string_lossy().as_ref().into());
                        app.set_displayed_code(code.to_string().into());
                        app.set_code_entry("".into());
                        app.set_awaiting_confirmation(true);
                        app.set_status("Write down the transfer code".into());
                        app.set_details(
                            "The backup has not started. Type the code to confirm it before copying.".into(),
                        );
                        *pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((job, code));
                    }
                    Err(e) => {
                        app.set_status("Could not prepare backup".into());
                        app.set_details(e.into());
                    }
                }
            });
        });
    });
    let weak = app.as_weak();
    app.on_confirm_backup(move || {
        if let Some(app) = weak.upgrade() {
            let correct = prepared.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(_, code)| {
                TransferCode::parse(app.get_code_entry().as_str()).is_ok_and(|typed| typed == *code)
            });
            if !correct {
                app.set_status("Code does not match — try again".into());
                return;
            }
            let Some((job, code)) = take_prepared(&prepared) else {
                return;
            };
            app.set_awaiting_confirmation(false);
            app.set_displayed_code("".into());
            app.set_code_entry("".into());
            run_job(&app, "Backing up…", move |weak| {
                let report = job
                    .run(&code, &ExportOptions::default(), &mut progress_callback(weak))
                    .map_err(|e| e.to_string())?;
                Ok(export_result(report))
            });
        }
    });

    let weak = app.as_weak();
    app.on_resume_backup(move || {
        if let Some(app) = weak.upgrade() {
            let path = PathBuf::from(app.get_package_path().as_str());
            let code = app.get_code_entry().to_string();
            run_job(&app, "Resuming backup…", move |weak| {
                let code = TransferCode::parse(&code).map_err(|e| e.to_string())?;
                let report =
                    engine::resume_export(&path, &code, &ExportOptions::default(), &mut progress_callback(weak))
                        .map_err(|e| e.to_string())?;
                Ok(export_result(report))
            });
        }
    });
    let weak = app.as_weak();
    app.on_verify_backup(move || if let Some(app) = weak.upgrade() {
        let path = PathBuf::from(app.get_package_path().as_str());
        let code = app.get_code_entry().to_string();
        run_job(&app, "Verifying backup…", move |weak| {
            let report = transfer::verify_package(&path, &code, &mut progress_callback(weak)).map_err(|e| e.to_string())?;
            let title = if report.complete && report.failed.is_empty() { "Stored files verified" } else { "Verification needs review" };
            let mut details = vec![format!("{} files decrypted and matched their hashes. Verification does not establish whether all source files were copied.", report.files)];
            if !report.complete { details.push("The backup is incomplete; resume it before relying on it.".into()); }
            details.extend(report.incomplete_files.iter().map(|p| format!("Incomplete: {p}")));
            details.extend(report.failed.iter().map(|(p,e)| format!("Failed: {p} — {e}")));
            Ok((title.into(), details.join("\n")))
        });
    });
    let weak = app.as_weak();
    app.on_restore_backup(move || {
        if let Some(app) = weak.upgrade() {
            let path = PathBuf::from(app.get_package_path().as_str());
            let target = PathBuf::from(app.get_target_path().as_str());
            let code = app.get_code_entry().to_string();
            let allow_cloud = app.get_allow_cloud();
            run_job(&app, "Restoring…", move |weak| {
                let r = transfer::restore_package(&path, &target, &code, allow_cloud, &mut progress_callback(weak))
                    .map_err(|e| e.to_string())?;
                let title =
                    if r.fully_successful() { "Restore complete" } else { "Restore finished — review required" };
                let mut d = vec![format!(
                    "{} files restored; {} verified; {} already present.",
                    r.files, r.verified, r.already_present
                )];
                if !r.backup_complete {
                    d.push("The backup was incomplete.".into());
                }
                d.extend(r.failed.iter().map(|(p, e)| format!("Failed: {p} — {e}")));
                d.extend(r.skipped_at_backup.iter().map(|s| format!("Not backed up: {} — {}", s.path, s.detail)));
                d.extend(r.changed_at_backup.iter().map(|p| format!("Changed during backup: {p}")));
                d.extend(r.incomplete_at_backup.iter().map(|p| format!("Incomplete: {p}")));
                d.extend(r.conflicts.iter().map(|(p, c)| format!("Conflict: {p} — {c:?}")));
                d.extend(r.renamed.iter().map(|(p, n, _)| format!("Renamed: {p} → {n}")));
                d.extend(r.links_not_restored.iter().map(|(p, t)| format!("Link not restored: {p} → {t}")));
                Ok((title.into(), d.join("\n")))
            });
        }
    });
    app.run()
}
