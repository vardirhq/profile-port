# AGENTS.md

Guidance for coding agents working in this repository. Human-facing docs live
in `README.md` and `docs/`. This file is the short version of what you need to
change the code safely. When it disagrees with the code, trust the code and
fix this file.

## What this is

ProfilePort moves a user's files and settings between PCs over a USB drive.
It is a Rust workspace in early development. The main target is Windows
10/11, but the engine and CLI also run on Linux and macOS, and a backup made
on any of the three can be restored on any other. The tool handles customer
data, so correctness and honest reporting matter more than features.

| Crate | Path | Role |
|---|---|---|
| `profileport-package` | `crates/package` | Encrypted, chunked package format: transfer codes, crypto, manifest log, writer/reader, crash recovery. Knows nothing about filesystems beyond its own directory. |
| `profileport-engine` | `crates/engine` | Walks profiles and writes them into packages; verify; restore; folder mapping; name rules; platform code. Re-exports the package crate as `profileport_engine::package`. |
| `profileport-cli` | `apps/cli` | `profileport-cli` binary: clap subcommands, prompts, printed reports, exit codes. |
| `profileport-desktop` | `apps/desktop` | Slint UI (`ProfilePort` binary). Its `src/transfer.rs` is a thin bridge to the engine. |

Dependencies point one way: `package` ← `engine` ← `cli` / `desktop`. The
apps call the engine directly; there is no service layer.

## Commands

```sh
cargo fmt --all -- --check                                                     # rustfmt.toml: max_width 120
cargo clippy --workspace --exclude profileport-desktop --all-targets -- -D warnings
cargo test --workspace --exclude profileport-desktop                           # package, engine, CLI
cargo test -p profileport-desktop                                              # transfer.rs bridge tests
cargo clippy -p profileport-desktop --all-targets -- -D warnings               # not in CI; currently clean
cargo build --release -p profileport-cli
scripts/unix-smoke.sh target/release/profileport-cli                           # Linux/macOS end to end
```

Before committing, run fmt, clippy and the workspace tests, plus the desktop
commands if you touched `apps/desktop`. Desktop builds and tests also work on
Linux when Slint's native windowing/font libraries are installed.

CI (`.github/workflows/ci.yml`):

| Runner | What runs |
|---|---|
| Linux | Clippy `-D warnings` (excluding desktop), tests excluding desktop, release CLI build, `unix-smoke.sh` |
| macOS | Tests excluding desktop, release CLI build, `unix-smoke.sh` |
| Windows | Tests excluding desktop, `cargo test -p profileport-desktop`, release CLI + desktop builds, `windows-smoke.ps1` |

Clippy only runs on Linux. `#[cfg(windows)]` code (`win.rs`,
`system/windows.rs`, the `windows-sys` calls) is never linted, and is only
compiled on the Windows runner. If you change it without a Windows build, say
so in the PR.

The smoke scripts create and then delete test directories; read them before
running them locally. `scripts/screenshot.sh` overwrites
`docs/screenshots/*` with whatever profiles the current machine has. Don't run
it unless asked.

## Code map

### `crates/package`

| File | Contents |
|---|---|
| `format.rs` | On-disk layout constants, `Header`, `Status`, `PackageState`, `Meta`, `Record` (the manifest record enum), `BeginInfo`, `KnownFolder`, `KnownFolderEntry`, `SkipReason`; `split_rel_path` (path validation), `validate_ticket`, `valid_stream_name`; CBOR helpers |
| `code.rs` | `TransferCode`: 10 Crockford base32 data symbols plus 2 check symbols; forgiving parsing |
| `crypto.rs` | XChaCha20-Poly1305 seal/open, Argon2id key wrapping, `KdfParams` (`recommended()`, and `insecure_fast_for_tests()`, which tests must use) |
| `log.rs` | Append-only encrypted `manifest.log`. A torn final frame is discarded; damage anywhere earlier is `Error::Corrupt`. |
| `writer.rs` | `PackageWriter`: `create`, `resume` (replays the log), `add_*`, `commit` (sync packs → append records → sync log), `finish`. `WriterOptions` sets chunk and pack sizes, the commit batch size and the KDF. |
| `reader.rs` | `PackageReader::open(dir, code)`, `manifest()` (builds `Entry` values from `Record`s), `read_file`; `read_info` / `list_packages` need no code |
| `error.rs` | The crate's single `Error` enum (thiserror) |

On disk a package is `<drive>/ProfilePort/transfers/<ticket>-<hex>/` holding
`header.bin` and `status.cbor` (both plaintext), `manifest.log` and
`packs/*.pack` (both encrypted). The full spec is `docs/package-format.md`.

### `crates/engine`

| File | Contents |
|---|---|
| `lib.rs` | Public re-exports, `Progress`, `SkipItem`, `transfers_dir(drive)` |
| `export.rs` | `prepare_export` → `PreparedExport::run`; `export` (both steps in one call); `resume_export`; `ExportOptions` (including the hidden `crash_after_bytes` test hook); `ExportReport`; the private `Job` that does the walk |
| `restore.rs` | `restore` and `verify`; `RestoreOptions`, `ConflictPolicy`, `RestoreReport::fully_successful`, `VerifyReport`; destination path checks (`safe_dir`, `passes_through_link`) |
| `folders.rs` | `FolderMap`: maps a source known folder (or an `@Name` extra root) to the target's location; `is_cloud_synced` (a heuristic based on path names) |
| `names.rs` | `NameRules` (`Posix`/`Mac`/`Windows`): adapts each path component for the target, and decides whether names that differ only in case collide |
| `exclude.rs` | `Exclusions`, `DEFAULT_EXCLUSIONS`: case-insensitive patterns relative to the profile, where `*` matches one path component |
| `platform.rs` | Metadata capture (`meta_of`), timestamps, IO-error → `SkipReason` classification, cloud-only detection, change fingerprint |
| `win.rs` | Windows only: file attributes and alternate data streams |
| `system/` | `list_profiles`, `known_folders`, `os_name`, `default_name`; `unix.rs` (`/etc/passwd`, XDG user-dirs, macOS `/Users`), `windows.rs` (ProfileList registry, Shell / User Shell Folders) |

How export works: `prepare_export` validates the ticket, canonicalises the
source, plans the known folders (folders outside the profile become `@Name`
extra roots), creates the package and returns the code. The caller shows
the code and has it confirmed, then calls `run`. `Job` walks the tree with
`symlink_metadata`. It records links without following them, applies the
exclusions, adds `Skipped` records for unreadable items, and flags files that
changed while being read. The report is rebuilt from the manifest, so it
covers every session, not just the last one.

How restore works: open the reader, then validate every manifest path with
`split_rel_path` before writing anything. Build a `FolderMap` from the
target's known folders, excluding any reached through a link. For each
entry: place it, adapt its name, detect collisions, write to a temporary file,
verify, then rename into place. Existing files that differ are left alone
(`Skip`) or kept alongside as `name (restored).ext` (`KeepBoth`). Identical
files count as `already_present`.

### Apps

- `apps/cli/src/main.rs` is one file. `Cmd` is the clap enum, and `main`
  maps `Outcome::{Clean, Issues}` / `Err` to exit codes `0` / `1` / `2`.
  Codes are read with `get_code`, which prompts when `--code` is not given.
- `apps/desktop` is described under "Desktop (Slint)" below.

### Tests

| Where | Use for |
|---|---|
| `crates/package/tests/package.rs` | Format-level behaviour: round trips, wrong code, tampering, torn logs, resume, plaintext-leak check |
| `crates/engine/tests/engine.rs` | Export/resume/restore on the native platform, crash simulation through `crash_after_bytes` + `catch_unwind` |
| `crates/engine/tests/cross_os.rs` | Cross-OS behaviour on any host: pass explicit `NameRules` and `known_folders` instead of relying on the running OS |
| `#[cfg(test)]` modules | Pure helpers (code parsing, name rules, exclusions, folder mapping, CLI formatting, desktop bridge) |
| `scripts/*-smoke.*` | The real CLI binary end to end; Windows-only filesystem features (junctions, long paths, locked files, ADS) |

Tests use `tempfile`, small `WriterOptions` (chunk and pack sizes in KiB) and
`KdfParams::insecure_fast_for_tests()`. Profile names in fixtures are
synthetic, such as `"anna"` and `"OLD-PC"`.

## Rules that matter here

**Data safety**
- Restore never overwrites or deletes user files. Don't weaken the conflict
  rules, and keep reruns idempotent.
- Never follow symlinks, junctions or reparse points on export, and never
  follow links already on the destination during restore. Record them
  instead.
- Every write path must survive the drive disappearing mid-write. Pack data
  is synced before the manifest record that refers to it; resume continues
  from the last commit and discards anything uncommitted.
- Treat package contents as hostile input. Validate every path with
  `split_rel_path` and adapt names with `NameRules` before joining them onto
  a real path.

**Honest reporting**
- Copying finishing is not verification, and a clean hash check doesn't
  prove the source was captured completely. Keep the wording in the apps
  distinct, e.g. "Backup complete — not yet verified".
- Skipped, excluded, changed-during-backup, renamed, conflicting and
  unrestored-link items must all reach the report. If you add a new outcome,
  decide whether it affects `ExportReport::has_issues` /
  `RestoreReport::fully_successful`, and make both apps show it.

**Secrets and privacy**
- Transfer codes, customer names and identifying paths must stay out of
  plaintext package files, logs, diagnostics, fixtures, examples and commit
  messages. Plaintext is limited to `header.bin` (ticket, time, KDF
  parameters), `status.cbor` (state and counters) and the folder name
  (ticket plus a suffix). `package.rs::plaintext_contains_no_names_or_content`
  guards this.
- The code is never written to the drive. Prefer prompting for it over
  passing it as an argument.

**Platform behaviour**
- Read `docs/platform-gotchas.md` before touching traversal, paths, names or
  metadata. Many entries there are design intent rather than implemented
  behaviour.
- Never assume where a known folder lives. Use `system::known_folders` or
  the paths recorded in the package.

## Recipes

**Add a CLI subcommand or flag.** Add a variant or field to `Cmd` in
`apps/cli/src/main.rs` and handle it in `main`'s match by returning
`Res` (`Outcome::Clean`/`Issues`, or `Err` for exit 2). Put the logic in the
engine if the desktop could use it too. Prompt for secrets instead of taking
them on the command line. Update `docs/guides/cli.md`, update the README if
the command appears there, and extend `scripts/unix-smoke.sh` and
`scripts/windows-smoke.ps1` when the command reaches the filesystem.

**Add a field to an existing record or `BeginInfo`.** Mark it
`#[serde(default)]` so existing v1 packages still decode (see `source_os`,
`known_folders`). Add a test that reads a package without the field. Document
the field in `docs/package-format.md`.

**Add a new `Record` variant or change record semantics.** CBOR enums are
externally tagged, and `from_cbor` maps an unknown variant to
`Error::Corrupt`, so older builds will reject any package containing the new
record. Decide whether that is acceptable or whether `FORMAT_VERSION` must be
bumped; `read_header` currently accepts version 1 only. Then update:
`writer.rs` (the `add_*` method, and the `resume` replay), `reader.rs`
(`manifest()` → `Entry`), `export.rs` (report assembly), `restore.rs`
(validation loop and handling), both apps' report output,
`docs/package-format.md`, and ADR 0002 if the design changes. Add round-trip,
resume and corruption tests in `crates/package/tests/package.rs`.

**Change exclusions.** Edit `DEFAULT_EXCLUSIONS` in `exclude.rs`. Patterns
use `/`, are relative to the profile, and are case-insensitive. Excluded
folders are reported, never silently dropped. Update
`docs/guides/coverage.md`.

**Change restore placement, names or conflicts.** Work in `restore.rs`,
`folders.rs` and `names.rs`. Add hostile-input cases to
`cross_os.rs::hostile_names_never_escape_on_windows_rules` or a sibling test
using explicit `NameRules::Windows`, so Linux CI exercises the Windows rules.
Cover reruns, both with the default policy and with `KeepBoth`. Update ADR
0006 and `docs/guides/coverage.md` if the behaviour changes.

**Add a new `SkipReason`.** Add it in `format.rs` (this is a record change,
see above). Map it in `platform.rs::classify` or wherever it arises, and give
it a label in the CLI's `print_skips`.

**Wire a desktop action to the engine.** See below.

## Desktop (Slint)

- `build.rs` compiles `ui/app.slint`, and `main.rs` pulls it in with
  `slint::include_modules!()`. `AppWindow` declares `in`/`in-out` properties
  and `callback`s. Rust reads them with `get_*`, writes them with `set_*`,
  and handles callbacks with `on_*`; Slint's kebab-case names become
  snake_case in Rust.
- Threading: callbacks run on the UI thread. Long operations go through
  `run_job`, which sets `busy`, runs the closure on a `std::thread`, and
  posts the `(status, details)` result back with
  `slint::invoke_from_event_loop`. Progress uses `progress_callback`, which
  is throttled to 150 ms. Don't touch `AppWindow` from the worker thread;
  capture `app.as_weak()` and upgrade it inside the event-loop closure.
  `prepare_backup` runs on the UI thread, and there is no cancellation of a
  running job.
- `transfer.rs` holds the logic that can be tested without a UI: preflight
  checks (drive outside source), the cloud-target check, and default
  options. Put new behaviour there with a `#[cfg(test)]` test, not in the
  `main.rs` closures.
- `ui/theme.slint` holds every colour, size and font token (`Theme.dark` is
  set from `PROFILEPORT_THEME=dark`). Use tokens, not literal colours.
  `components.slint` is mostly left over from the earlier prototype. The
  current screens use `Label` and Slint std-widgets.
- Bundled assets: Inter fonts (OFL) and Lucide icons (ISC), with their
  licence files in `ui/fonts/` and `ui/icons/`. Keep the licences when adding
  assets.
- Nothing drives the UI in tests. A successful build or a Linux screenshot
  is not evidence that the Windows GUI works. Say what you actually checked.
- In release builds `main.rs` sets `windows_subsystem = "windows"`, so there
  is no console and `println!` output is lost.

## Documentation

The docs state what is actually implemented and tested, and list gaps
explicitly. Keep it that way.
- When behaviour changes, update the matching guide in `docs/guides/`, the
  status and limitations in `README.md`, `docs/development/*`, and ADR notes
  where relevant.
- ADRs in `docs/decisions/` record intent. "Accepted" doesn't mean
  implemented: VSS, `SeBackupPrivilege`, elevation and offline hive loading
  are not implemented. Add a new ADR for a significant design decision and
  list it in `docs/decisions/README.md`.
- Don't claim GUI, Windows-client or real-USB coverage you didn't test.

## Style

- `rustfmt.toml`: `max_width = 120`, `use_small_heuristics = "Max"`.
- Library crates return typed `thiserror` errors (`package::Error`,
  `ExportError`, `RestoreError`, `CodeError`). `expect` is reserved for real
  invariants: OS RNG unavailable, in-memory CBOR/AEAD encryption,
  writer-internal state. Per-file problems become report entries, not
  errors. Only failures that stop the whole operation are `Err`.
- Error and report text is shown to technicians. Say what happened and what
  to do next (see `ExportError::SourceMissing`).
- Keep changes small, with tests next to the behaviour they cover.
- Keep the binary names distinct. `profileport-cli` and `ProfilePort` would
  overwrite each other on a case-insensitive filesystem if both were named
  `profileport`.
