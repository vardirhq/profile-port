# ProfilePort

A portable Windows utility for moving a user's files and settings from one PC
to another via a USB drive. Plug in the drive, launch ProfilePort, pick a
user, and transfer their data — with verification and an honest report of
anything that couldn't be copied.

> **Status:** early development.
> - **Engine + CLI:** export, resume, verify and restore work end to end,
>   with an encrypted package format. Not yet Windows-specific: no profile
>   discovery, VSS, Windows attribute/ADS restore or `SeBackupPrivilege` yet.
> - **Desktop UI:** dashboard prototype with sample data, not yet wired to
>   the engine.

![Dashboard](docs/screenshots/dashboard-light.png)

## Stack

Rust + [Slint](https://slint.dev), shipped as a single portable executable for
Windows 10 and 11. See [ADR 0001](docs/decisions/0001-gui-framework.md) for
why Slint rather than Tauri.

## Repository layout

```text
crates/package/        encrypted transfer package format (docs/package-format.md)
crates/engine/         export / resume / verify / restore engine
apps/cli/              `profileport` command-line tool
apps/desktop/          Slint desktop app (prototype)
  ui/theme.slint       design tokens (colours, type, spacing, motion)
  ui/components.slint  reusable components
  ui/app.slint         main window / dashboard
  ui/icons/            Lucide icons (ISC)
  ui/fonts/            Inter (SIL OFL 1.1)
docs/decisions/        architecture decision records
docs/design/ui.md      visual system and UI workflow
docs/package-format.md  on-disk format specification
docs/windows-gotchas.md platform behaviours the engine must handle
scripts/screenshot.sh   headless screenshots of the UI (Linux)
scripts/windows-smoke.ps1  end-to-end test on real Windows (run in CI)
```

## Using the CLI

```text
profileport export C:\Users\anna --drive E:\ --ticket 48231
    Shows the transfer code, asks for it to be typed back, then copies.
    Write the code on the device tag, not in the ticket system.

profileport list --drive E:\                  backups on the drive (no code needed)
profileport resume  <package-folder>         continue after an interruption
profileport verify  <package-folder>         decrypt and hash-check everything
profileport restore <package-folder> --to C:\Users\anna.smith [--keep-both]
```

The code is prompted for when `--code` is omitted, which keeps it out of
shell history. Exit codes: `0` complete, `1` finished with items to review,
`2` failed.

Restore never overwrites or deletes anything. Existing files are left alone
and reported (or, with `--keep-both`, restored as `name (restored).ext`).
Files already present with identical content count as done, so re-running an
interrupted restore is safe.

## Building and testing

```sh
cargo test --workspace --exclude profileport-desktop   # engine, format, CLI
cargo build --release -p profileport-cli
cargo run -p profileport-desktop          # run the prototype
PROFILEPORT_THEME=dark cargo run -p profileport-desktop
scripts/screenshot.sh                     # regenerate docs/screenshots (Linux + Xvfb)
```

For a Windows release build: `cargo build --release -p profileport-cli -p profileport-desktop`
on Windows (x64 or ARM64 native; see ADR 0005 on why not 32-bit). CI does
this on every push and attaches both `.exe` files to the run as an artifact.

### What is tested where

| Where | What |
|---|---|
| Unit and integration tests (Linux and Windows CI) | Transfer codes, encryption and tamper detection, plaintext-leak check, torn writes, crash-and-resume (including repeated crashes and source files changed meanwhile), restore safety (no overwrite, no following destination links, path validation), idempotent restore |
| `scripts/windows-smoke.ps1` (Windows CI) | Junctions and loops, paths over 260 characters, a file locked by another process, read-only, hidden, Mark of the Web, timestamps, restoring to a different username. Known gaps are listed, not hidden |
| Not yet tested | Real Windows 10/11 *client* PCs, real USB sticks being unplugged, OneDrive placeholders, BitLocker, ARM64, the GUI on Windows |

## Key decisions

| Decision | Status |
|---|---|
| [Slint, not Tauri](docs/decisions/0001-gui-framework.md) | Accepted |
| [Encrypted, chunked package format](docs/decisions/0002-package-format.md) | Accepted |
| [Native Rust copy engine, not Robocopy](docs/decisions/0003-copy-engine.md) | Accepted |
| [Always run elevated](docs/decisions/0004-privilege-model.md) | Accepted |
| [Locked files: avoid, then VSS; registry archived, not merged](docs/decisions/0005-locked-files-and-registry.md) | Accepted |

## Suggested build order

1. **Engine as a CLI first:** profile discovery, export of known folders into
   the package format with manifest and hashes, restore to a different
   username, verification.
2. **Then the GUI on top**, once the engine survives pulled-USB tests.
3. **Then** installed-app inventory, browser adapters and USB management.
