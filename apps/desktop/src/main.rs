#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod transfer;
slint::include_modules!();

use profileport_engine::{self as engine, package::TransferCode, ExportOptions, PreparedExport, Progress};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use transfer::{BackupItem, Badge, Choice, DriveChoice};

/// The package prepared for backup, waiting for the code to be confirmed.
/// Shared with the worker thread that prepares it, hence `Arc<Mutex>`.
type Prepared = Arc<Mutex<Option<(PreparedExport, TransferCode)>>>;

fn take_prepared(prepared: &Prepared) -> Option<(PreparedExport, TransferCode)> {
    prepared.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// What the cards and rows on screen stand for. UI thread only.
#[derive(Default)]
struct State {
    profiles: Vec<Choice>,
    drives: Vec<DriveChoice>,
    backups: Vec<BackupItem>,
    /// The backup made last, for "Check backup now".
    last_backup: Option<PathBuf>,
}

type Shared = Rc<RefCell<State>>;

/// The result of a background job, shown when it finishes.
struct Outcome {
    status: String,
    details: String,
    ok: bool,
    offer_wipe: bool,
    offer_check: bool,
    /// Select the backup in this folder afterwards.
    select: Option<PathBuf>,
}

impl Outcome {
    fn new((status, details): (String, String), ok: bool) -> Self {
        Self { status, details, ok, offer_wipe: false, offer_check: false, select: None }
    }
}

fn progress_callback(weak: slint::Weak<AppWindow>) -> impl FnMut(&Progress) {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |p| {
        if last.elapsed() < Duration::from_millis(150) {
            return;
        }
        last = Instant::now();
        let text = transfer::progress_text(p);
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_details(text.into());
            }
        });
    }
}

/// Run `task` off the UI thread. Its progress and outcome are shown on the
/// page it was started from; afterwards the drive and backup lists refresh.
fn run_job<F>(app: &AppWindow, title: &str, task: F)
where
    F: FnOnce(slint::Weak<AppWindow>) -> Result<Outcome, String> + Send + 'static,
{
    app.set_confirming_wipe(0);
    app.set_busy(true);
    app.set_offer_wipe(false);
    app.set_offer_check(false);
    app.set_status_ok(true);
    app.set_status(title.into());
    app.set_details("".into());
    app.set_result_page(app.get_page());
    let weak = app.as_weak();
    std::thread::spawn(move || {
        let result = task(weak.clone());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(app) = weak.upgrade() else { return };
            app.set_busy(false);
            let o = result.unwrap_or_else(|e| Outcome::new(("That didn't work".into(), e), false));
            app.set_status(o.status.into());
            app.set_details(o.details.into());
            app.set_status_ok(o.ok);
            app.set_offer_wipe(o.offer_wipe);
            app.set_offer_check(o.offer_check);
            let select = o.select.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            app.invoke_refresh(select.into());
        });
    });
}

/// Show a message in the result card without running a job.
fn show_message(app: &AppWindow, status: &str, details: String, ok: bool) {
    app.set_status(status.into());
    app.set_details(details.into());
    app.set_status_ok(ok);
    app.set_offer_wipe(false);
    app.set_offer_check(false);
    app.set_result_page(app.get_page());
}

// ---------------------------------------------------------------- models

fn set_profiles(app: &AppWindow, state: &State) {
    let items: Vec<ProfileItem> = state
        .profiles
        .iter()
        .map(|c| ProfileItem { title: c.title.clone().into(), subtitle: c.subtitle.clone().into(), current: c.current })
        .collect();
    app.set_profiles(ModelRc::new(VecModel::from(items)));
}

fn drive_item(d: &DriveChoice) -> DriveItem {
    DriveItem {
        title: d.title.clone().into(),
        subtitle: d.subtitle.clone().into(),
        removable: d.removable,
        free_fraction: d.free_fraction.unwrap_or(-1.0),
        free_text: d.free_text.clone().into(),
        backups: d.backups as i32,
    }
}

fn backup_row(b: &BackupItem) -> BackupRow {
    BackupRow {
        title: b.title.clone().into(),
        subtitle: b.subtitle.clone().into(),
        badge_text: b.badge_text.clone().into(),
        badge: match b.badge {
            Badge::Good => BadgeKind::Success,
            Badge::Attention => BadgeKind::Warning,
            Badge::Neutral => BadgeKind::Neutral,
        },
        old: b.old,
        interrupted: b.interrupted,
        partly_wiped: b.partly_wiped,
    }
}

fn chosen_profile(state: &State, index: i32) -> Option<PathBuf> {
    state.profiles.get(usize::try_from(index).ok()?).map(|c| c.path.clone())
}

fn selected_drive(app: &AppWindow, state: &State) -> Option<PathBuf> {
    state.drives.get(usize::try_from(app.get_drive_index()).ok()?).map(|d| d.path.clone())
}

fn selected_backup(app: &AppWindow, state: &State) -> Option<BackupItem> {
    state.backups.get(usize::try_from(app.get_backup_index()).ok()?).cloned()
}

/// Ask for a folder and add it to the profile cards. Returns its index.
fn pick_folder(app: &AppWindow, state: &mut State, title: &str) -> Option<i32> {
    let path = rfd::FileDialog::new().set_title(title).pick_folder()?;
    let index = match state.profiles.iter().position(|c| c.path == path) {
        Some(i) => i,
        None => {
            state.profiles.push(transfer::folder_choice(&path));
            state.profiles.len() - 1
        }
    };
    set_profiles(app, state);
    Some(index as i32)
}

/// Re-read the connected drives, keeping the selected one if it's still
/// there, else preferring a drive that holds backups.
fn refresh_drives(app: &AppWindow, state: &mut State, select: Option<PathBuf>) {
    let fresh = transfer::drive_choices(&engine::system::list_drives());
    if fresh != state.drives {
        let previous = selected_drive(app, state);
        let index = previous
            .and_then(|p| fresh.iter().position(|d| d.path == p))
            .or_else(|| fresh.iter().position(|d| d.backups > 0))
            .unwrap_or(0);
        app.set_drives(ModelRc::new(VecModel::from(fresh.iter().map(drive_item).collect::<Vec<_>>())));
        app.set_drive_index(index as i32);
        state.drives = fresh;
    }
    refresh_backups(app, state, select);
}

/// Re-read the backups on the selected drive. Selects `select` if given,
/// else keeps the selection, else selects the newest backup.
fn refresh_backups(app: &AppWindow, state: &mut State, select: Option<PathBuf>) {
    let (items, summary) = match selected_drive(app, state) {
        Some(drive) => transfer::list_backups(&drive),
        None => (Vec::new(), String::new()),
    };
    let previous = selected_backup(app, state).map(|b| b.path);
    let same = |a: &PathBuf, b: &PathBuf| a == b || a.canonicalize().ok() == b.canonicalize().ok();
    let wanted = select.or(previous.clone());
    let index = wanted.and_then(|p| items.iter().position(|b| same(&b.path, &p))).unwrap_or(0);
    app.set_backups_summary(summary.into());
    app.set_old_backups(items.iter().filter(|b| b.old).count() as i32);
    if items != state.backups || app.get_backup_index() != index as i32 {
        app.set_backups(ModelRc::new(VecModel::from(items.iter().map(backup_row).collect::<Vec<_>>())));
        app.set_backup_index(if items.is_empty() { -1 } else { index as i32 });
        if items.get(index).map(|b| &b.path) != previous.as_ref() {
            clear_code(app);
        }
        state.backups = items;
    }
}

fn clear_code(app: &AppWindow) {
    app.set_code_entry("".into());
    app.set_code_ok(false);
    app.set_code_hint("".into());
}

fn update_target(app: &AppWindow, state: &State) {
    let warning = chosen_profile(state, app.get_target_index())
        .map(|p| transfer::cloud_folders(&p).join("\n"))
        .unwrap_or_default();
    app.set_cloud_warning(warning.into());
    app.set_allow_cloud(false);
}

fn this_computer() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "this computer".into())
}

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    app.global::<Theme>().set_dark(matches!(std::env::var("PROFILEPORT_THEME").as_deref(), Ok("dark")));
    app.set_version(env!("CARGO_PKG_VERSION").into());
    app.set_computer(this_computer().into());

    let state: Shared = Rc::new(RefCell::new(State {
        profiles: transfer::profile_choices(&engine::system::list_profiles()),
        ..Default::default()
    }));
    {
        let s = state.borrow();
        set_profiles(&app, &s);
        update_target(&app, &s);
    }
    refresh_drives(&app, &mut state.borrow_mut(), None);

    // Drives come and go: look again every two seconds, except while a job or
    // a question is on screen.
    let timer = Timer::default();
    {
        let (weak, state) = (app.as_weak(), state.clone());
        timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(app) = weak.upgrade() {
                if !app.get_busy() && !app.get_awaiting_confirmation() && app.get_confirming_wipe() == 0 {
                    refresh_drives(&app, &mut state.borrow_mut(), None);
                }
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_refresh(move |select| {
            if let Some(app) = weak.upgrade() {
                let select = (!select.is_empty()).then(|| PathBuf::from(select.as_str()));
                let mut s = state.borrow_mut();
                if select.is_some() {
                    s.last_backup.clone_from(&select);
                }
                refresh_drives(&app, &mut s, select);
            }
        });
    }

    // ------------------------------------------------ picking
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_drive_clicked(move |i| {
            if let Some(app) = weak.upgrade() {
                app.set_drive_index(i);
                app.set_backup_index(-1);
                refresh_backups(&app, &mut state.borrow_mut(), None);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_backup_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                clear_code(&app);
                app.set_confirming_wipe(0);
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_choose_source_folder(move || {
            if let Some(app) = weak.upgrade() {
                if let Some(i) = pick_folder(&app, &mut state.borrow_mut(), "Choose the folder to back up") {
                    app.set_source_index(i);
                }
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_target_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                update_target(&app, &state.borrow());
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_choose_target_folder(move || {
            if let Some(app) = weak.upgrade() {
                let mut s = state.borrow_mut();
                if let Some(i) = pick_folder(&app, &mut s, "Choose the folder to restore into") {
                    app.set_target_index(i);
                    update_target(&app, &s);
                }
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_ticket_edited(move || {
            if let Some(app) = weak.upgrade() {
                app.set_ticket_hint(transfer::ticket_feedback(app.get_ticket().trim()).into());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_code_edited(move || {
            if let Some(app) = weak.upgrade() {
                let (ok, hint) = transfer::code_feedback(app.get_code_entry().as_str());
                app.set_code_ok(ok);
                app.set_code_hint(hint.into());
            }
        });
    }

    // ------------------------------------------------ back up
    let prepared: Prepared = Arc::new(Mutex::new(None));
    {
        let (weak, state, pending) = (app.as_weak(), state.clone(), prepared.clone());
        app.on_prepare_backup(move || {
            let Some(app) = weak.upgrade() else { return };
            let s = state.borrow();
            let (Some(source), Some(drive)) = (chosen_profile(&s, app.get_source_index()), selected_drive(&app, &s))
            else {
                show_message(&app, "Choose whose files to back up and a drive first", String::new(), false);
                return;
            };
            let ticket = app.get_ticket().trim().to_string();
            app.set_busy(true);
            let (weak, pending) = (weak.clone(), pending.clone());
            // Key derivation (Argon2id) takes a noticeable moment: keep it off
            // the UI thread so the window stays responsive.
            std::thread::spawn(move || {
                let result = transfer::prepare(&source, &drive, &ticket).map_err(|e| e.to_string());
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(app) = weak.upgrade() else { return };
                    app.set_busy(false);
                    match result {
                        Ok((job, code)) => {
                            app.set_displayed_code(code.to_string().into());
                            app.set_code_stored(false);
                            app.set_awaiting_confirmation(true);
                            *pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((job, code));
                        }
                        Err(e) => show_message(&app, "The backup could not be prepared", e, false),
                    }
                });
            });
        });
    }
    {
        let (weak, pending) = (app.as_weak(), prepared.clone());
        app.on_cancel_preparation(move || {
            if let Some(app) = weak.upgrade() {
                app.set_awaiting_confirmation(false);
                app.set_displayed_code("".into());
                let details = match take_prepared(&pending) {
                    Some((job, _)) => transfer::cancel(job),
                    None => "Nothing was copied.".into(),
                };
                show_message(&app, "Backup cancelled", details, true);
                app.invoke_refresh("".into());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_confirm_backup(move || {
            let Some(app) = weak.upgrade() else { return };
            if !app.get_code_stored() {
                return;
            }
            let Some((job, code)) = take_prepared(&prepared) else { return };
            app.set_awaiting_confirmation(false);
            app.set_displayed_code("".into());
            run_job(&app, "Backing up…", move |weak| {
                let report = job
                    .run(&code, &ExportOptions::default(), &mut progress_callback(weak))
                    .map_err(|e| e.to_string())?;
                Ok(Outcome {
                    offer_check: true,
                    select: Some(report.package_dir.clone()),
                    ..Outcome::new(transfer::export_summary(&report), !report.has_issues())
                })
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_check_new_backup(move || {
            if let Some(app) = weak.upgrade() {
                let last = state.borrow().last_backup.clone();
                app.set_result_page(-1);
                app.set_page(2);
                refresh_drives(&app, &mut state.borrow_mut(), last);
                clear_code(&app);
            }
        });
    }

    // ------------------------------------------------ restore, check, continue
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_resume_backup(move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(item) = selected_backup(&app, &state.borrow()) else { return };
            let code = app.get_code_entry().to_string();
            run_job(&app, "Continuing the backup…", move |weak| {
                let code = TransferCode::parse(&code).map_err(|e| e.to_string())?;
                let report =
                    engine::resume_export(&item.path, &code, &ExportOptions::default(), &mut progress_callback(weak))
                        .map_err(|e| e.to_string())?;
                Ok(Outcome {
                    select: Some(item.path),
                    ..Outcome::new(transfer::export_summary(&report), !report.has_issues())
                })
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_verify_backup(move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(item) = selected_backup(&app, &state.borrow()) else { return };
            let code = app.get_code_entry().to_string();
            run_job(&app, "Checking the backup…", move |weak| {
                let r = transfer::verify_package(&item.path, &code, &mut progress_callback(weak))
                    .map_err(|e| e.to_string())?;
                Ok(Outcome {
                    select: Some(item.path),
                    ..Outcome::new(transfer::verify_summary(&r), r.complete && r.failed.is_empty())
                })
            });
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_restore_backup(move || {
            let Some(app) = weak.upgrade() else { return };
            let s = state.borrow();
            let (Some(item), Some(target)) = (selected_backup(&app, &s), chosen_profile(&s, app.get_target_index()))
            else {
                show_message(&app, "Choose a backup and where to restore it first", String::new(), false);
                return;
            };
            let code = app.get_code_entry().to_string();
            let allow_cloud = app.get_allow_cloud();
            run_job(&app, "Restoring…", move |weak| {
                let r =
                    transfer::restore_package(&item.path, &target, &code, allow_cloud, &mut progress_callback(weak))
                        .map_err(|e| e.to_string())?;
                let ok = r.fully_successful();
                Ok(Outcome {
                    offer_wipe: ok,
                    select: Some(item.path),
                    ..Outcome::new(transfer::restore_summary(&r), ok)
                })
            });
        });
    }

    // ------------------------------------------------ wipe
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_begin_wipe(move || {
            let Some(app) = weak.upgrade() else { return };
            let Some(item) = selected_backup(&app, &state.borrow()) else { return };
            match transfer::wipe_question(&item.path) {
                Ok(question) => {
                    app.set_wipe_question(question.into());
                    app.set_wipe_overwrite(false);
                    app.set_confirming_wipe(1);
                }
                Err(e) => show_message(&app, "This backup can't be wiped", e.to_string(), false),
            }
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_begin_wipe_all(move || {
            let Some(app) = weak.upgrade() else { return };
            let s = state.borrow();
            if s.backups.is_empty() {
                return;
            }
            app.set_wipe_question(transfer::wipe_all_question(&s.backups).into());
            app.set_wipe_overwrite(false);
            app.set_confirming_wipe(2);
        });
    }
    {
        let (weak, state) = (app.as_weak(), state.clone());
        app.on_wipe_confirmed(move || {
            let Some(app) = weak.upgrade() else { return };
            let s = state.borrow();
            let overwrite = app.get_wipe_overwrite();
            let all = app.get_confirming_wipe() == 2;
            clear_code(&app);
            if all {
                let Some(drive) = selected_drive(&app, &s) else { return };
                run_job(&app, "Wiping all backups…", move |weak| {
                    let (ok, text) = transfer::wipe_all(&drive, overwrite, &mut progress_callback(weak));
                    let title =
                        if ok { "Drive ready for the next customer" } else { "Some backups could not be wiped" };
                    Ok(Outcome::new((title.into(), text), ok))
                });
            } else {
                let Some(item) = selected_backup(&app, &s) else { return };
                run_job(&app, "Wiping the backup…", move |weak| {
                    let text = transfer::wipe_package(&item.path, overwrite, &mut progress_callback(weak))
                        .map_err(|e| e.to_string())?;
                    Ok(Outcome::new(("Backup wiped from the drive".into(), text), true))
                });
            }
        });
    }

    app.run()
}
