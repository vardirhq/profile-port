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
        free: "129.4 GB".into(),
        backups_fraction: 0.479,
        app_fraction: 0.016,
        backups_size: "122.5 GB".into(),
        app_size: "4.1 GB".into(),
        stale_count: 1,
        stale_threshold: "14 days".into(),
    });

    // Only the ticket's own backup is unlocked: the technician entered its
    // transfer code. Every other package shows plaintext metadata only.
    let backup = |ticket: &str, created: &str, size: &str, status, detail: &str| BackupEntry {
        ticket: ticket.into(),
        unlocked: false,
        profile: "".into(),
        computer: "".into(),
        created: created.into(),
        size: size.into(),
        status,
        detail: detail.into(),
        age: "".into(),
        stale: false,
    };
    let backups = vec![
        BackupEntry {
            unlocked: true,
            profile: "j.miller".into(),
            computer: "DESKTOP-4KQ7F2".into(),
            ..backup("48231", "Today, 10:42", "58.3 GB", BackupStatus::Verified, "")
        },
        backup("48219", "Yesterday, 16:05", "31.7 GB", BackupStatus::Partial, "3 files skipped"),
        backup("48204", "18 Nov, 09:12", "20.1 GB", BackupStatus::Interrupted, "62% copied"),
        BackupEntry {
            age: "16 days old".into(),
            stale: true,
            ..backup("48117", "4 Nov, 14:30", "12.4 GB", BackupStatus::Verified, "")
        },
    ];
    app.set_backups(Rc::new(slint::VecModel::from(backups)).into());

    app.set_show_notice(true);
    app.set_notice_title("Backup interrupted".into());
    app.set_notice_message("Ticket 48204 stopped at 62%. Reconnect the source PC to continue.".into());

    app.window().set_size(slint::LogicalSize::new(1200.0, 760.0));
    app.run()
}
