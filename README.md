# ProfilePort

A portable Windows utility for moving a user's files and settings from one PC
to another via a USB drive. Plug in the drive, launch ProfilePort, pick a
user, and transfer their data — with verification and an honest report of
anything that couldn't be copied.

> **Status:** early development.
> - **Engine + CLI:** export, resume, verify and restore work end to end,
>   with an encrypted package format. Windows profile discovery, standard
>   (known) folders, hidden/system attributes, creation times and the
>   "downloaded from the internet" marker are handled. Backups move between
>   Windows, Linux and macOS in any direction (CLI, tested in CI on all
>   three). Not yet: `SeBackupPrivilege` for other users'
>   files, VSS, loading offline registry hives, cache-bypassing verification.
> - **Desktop UI:** early interface connected to the engine for backup,
>   resume, verification, and restore. It has not yet been interactively or
>   visually validated on Windows clients. Drive and folder paths are entered
>   manually; per-folder selection and durable reports are not implemented.

**Use disposable test data:** the current transfer screens have not yet been
visually or interactively reviewed, and physical Windows/USB testing remains
outstanding. The checked-in screenshots show the earlier dashboard prototype.
Keep an independent backup and the original data until recovery is confirmed.

## Documentation

- [Desktop transfer guide](docs/guides/desktop.md) and [CLI guide](docs/guides/cli.md) — backup, resume, verify, and restore.
- [What a transfer includes](docs/guides/coverage.md) — exclusions, links, metadata, and limits.
- [Troubleshooting and recovery](docs/guides/troubleshooting.md) — failure cases and CLI cautions.
- [Windows VM test plan](docs/guides/windows-vm-testing.md) — repeatable tests with disposable data.
- [Architecture and transfer lifecycle](docs/development/architecture.md) and [security and data handling](docs/development/security.md).
- [Build and test guide](docs/development/building-and-testing.md), [package format](docs/package-format.md), and [architecture decisions](docs/decisions/README.md).

## Stack

Rust + [Slint](https://slint.dev), shipped as a single portable executable for
Windows 10 and 11. See [ADR 0001](docs/decisions/0001-gui-framework.md) for
why Slint rather than Tauri.

## Repository layout

```text
crates/package/        encrypted transfer package format (docs/package-format.md)
crates/engine/         export / resume / verify / restore engine
apps/cli/              `profileport-cli` command-line tool
apps/desktop/          Slint desktop transfer app
  ui/theme.slint       design tokens (colours, type, spacing, motion)
  ui/components.slint  reusable components
  ui/app.slint         current transfer screens
  ui/icons/            Lucide icons (ISC)
  ui/fonts/            Inter (SIL OFL 1.1)
docs/decisions/        architecture decision records
docs/design/ui.md      visual system and UI workflow
docs/package-format.md  on-disk format specification
docs/platform-gotchas.md platform behaviours the engine must handle
scripts/screenshot.sh   headless screenshots of the UI (Linux)
scripts/windows-smoke.ps1  end-to-end test on real Windows (run in CI)
```

## Using the CLI

```text
profileport-cli export C:\Users\anna --drive E:\ --ticket 48231
    Shows the transfer code, asks for it to be typed back, then copies.
    Write the code on the device tag, not in the ticket system.

profileport-cli profiles                      user profiles on this PC and their standard folders
profileport-cli list --drive E:\              backups on the drive (no code needed)
profileport-cli resume  <package-folder>         continue after an interruption
profileport-cli verify  <package-folder>         decrypt and hash-check everything
profileport-cli restore <package-folder> --to C:\Users\anna.smith [--keep-both]
```

The desktop app is `ProfilePort.exe`; the CLI is deliberately named
`profileport-cli.exe`, because Windows file names are case-insensitive and
the two would otherwise overwrite each other in the build folder.

The code is prompted for when `--code` is omitted, which keeps it out of
shell history. Exit codes: `0` complete, `1` finished with items to review,
`2` failed.

Restore puts standard folders where the *target* keeps them (Documents
redirected to OneDrive on the old PC goes into Documents on the new one; a
Windows backup restored on Linux goes into the XDG folders, translated names
included), and adapts file names the target can't use, reporting each
rename. If a target folder is synced to OneDrive or similar, restore asks
first, because restoring there uploads the files. See
[ADR 0006](docs/decisions/0006-cross-platform-restore.md).

Restore never overwrites or deletes anything. Existing files are left alone
and reported (or, with `--keep-both`, restored as `name (restored).ext`).
Files already present with identical content count as done, so re-running an
interrupted restore is safe.

## Building and testing

```sh
cargo test --workspace --exclude profileport-desktop   # engine, format, CLI
cargo build --release -p profileport-cli
cargo test -p profileport-desktop           # desktop transfer integration tests
cargo run -p profileport-desktop          # run the desktop app
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
| `scripts/unix-smoke.sh` (Linux and macOS CI) | The real CLI end to end: export, list, verify, restore to another user, re-restore; names with `:`, Unicode, symlinks, caches, timestamps |
| `scripts/windows-smoke.ps1` (Windows CI) | Junctions and loops, paths over 260 characters, a file locked by another process, read-only, hidden, Mark of the Web, timestamps, restoring to a different username. Known gaps are listed, not hidden |
| Not yet tested | Real Windows 10/11 *client* PCs and Macs, real USB sticks being unplugged, OneDrive placeholders, iCloud, BitLocker, ARM64 Windows, the GUI on Windows |

## Key decisions

| Decision | Status |
|---|---|
| [Slint, not Tauri](docs/decisions/0001-gui-framework.md) | Accepted |
| [Encrypted, chunked package format](docs/decisions/0002-package-format.md) | Accepted |
| [Native Rust copy engine, not Robocopy](docs/decisions/0003-copy-engine.md) | Accepted |
| [Always run elevated](docs/decisions/0004-privilege-model.md) | Accepted |
| [Locked files: avoid, then VSS; registry archived, not merged](docs/decisions/0005-locked-files-and-registry.md) | Accepted |
| [Cross-platform restore: Windows, Linux, macOS](docs/decisions/0006-cross-platform-restore.md) | Accepted |

## Suggested build order

1. **Engine as a CLI first:** profile discovery, export of known folders into
   the package format with manifest and hashes, restore to a different
   username, verification.
2. **Desktop transfer flow:** continue validating the GUI and engine together,
   including pulled-USB tests on real Windows hardware.
3. **Then** installed-app inventory, browser adapters and USB management.
