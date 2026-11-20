// Prevent a console window from opening alongside the GUI in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

use std::rc::Rc;

fn main() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;

    // Theme: follow PROFILEPORT_THEME if set (used for screenshots); otherwise
    // the real app will read the Windows "app mode" setting.
    let dark = matches!(std::env::var("PROFILEPORT_THEME").as_deref(), Ok("dark"));
    app.global::<Theme>().set_dark(dark);

    // ---- Sample data (prototype only) --------------------------------------
    app.set_version(env!("CARGO_PKG_VERSION").into());
    app.set_drive(DriveInfo {
        label: "KINGSTON".into(),
        letter: "E:".into(),
        filesystem: "exFAT".into(),
        total: "256 GB".into(),
        free: "141.8 GB".into(),
        backups_fraction: 0.43,
        app_fraction: 0.016,
        backups_size: "110.1 GB".into(),
        app_size: "4.1 GB".into(),
    });

    let backups = vec![
        BackupEntry {
            profile: "j.miller".into(),
            computer: "DESKTOP-4KQ7F2".into(),
            created: "Today, 10:42".into(),
            size: "58.3 GB".into(),
            status: BackupStatus::Verified,
            detail: "".into(),
        },
        BackupEntry {
            profile: "anna".into(),
            computer: "LAPTOP-ANNA".into(),
            created: "Yesterday, 16:05".into(),
            size: "31.7 GB".into(),
            status: BackupStatus::Partial,
            detail: "3 files skipped".into(),
        },
        BackupEntry {
            profile: "reception".into(),
            computer: "FRONTDESK-01".into(),
            created: "18 Nov, 09:12".into(),
            size: "20.1 GB".into(),
            status: BackupStatus::Interrupted,
            detail: "62% copied".into(),
        },
    ];
    app.set_backups(Rc::new(slint::VecModel::from(backups)).into());

    app.set_show_notice(true);
    app.set_notice_title("Backup interrupted".into());
    app.set_notice_message("reception on FRONTDESK-01 stopped at 62%. Reconnect that PC to continue.".into());

    app.window().set_size(slint::LogicalSize::new(1200.0, 760.0));
    app.run()
}
