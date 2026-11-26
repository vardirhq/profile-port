# Architecture and transfer lifecycle

ProfilePort separates its user interfaces from the transfer engine and package
format. This document describes the implementation on the default branch;
decision records describe design intent and can include future work.

| Location | Responsibility |
|---|---|
| `apps/desktop` | Slint UI, profile/package selection, code confirmation, worker threads, progress and result text |
| `apps/cli` | Command-line parsing, interactive prompts, reports and exit codes |
| `crates/engine` | Filesystem traversal, profiles and known folders, exclusions, export/resume, verification, restore and platform handling |
| `crates/package` | Transfer codes, encrypted package, manifest, pack files, reading and crash recovery |
| `scripts` | CLI smoke tests and a Linux screenshot helper |

The desktop and CLI call the engine directly. The desktop's `transfer.rs` adds
preflight checks and a path-based cloud warning; it is not a separate service.
The desktop runs long copy, resume, verify and restore operations on worker
threads and posts progress/results to the UI event loop. Preparing a package is
performed in the UI callback. The current UI has no cancellation of an active
operation, no persisted report and no overall-size-based progress estimate.

## Export

1. The UI resolves an existing source and drive directory. The desktop rejects
   a drive nested inside the source. The engine validates the ticket and
   creates a uniquely named package under `ProfilePort/transfers`.
2. `prepare_export` creates a package and code; the desktop requires the user
   to type the code back before `PreparedExport::run`. The package already
   exists when the code is shown. Canceling leaves it incomplete.
3. The engine walks the profile and discovered external known-folder roots,
   applies exclusions and records links, metadata, skipped files and changed
   files. `ExportOptions` supplies exclusions and writer options. Current UI
   and CLI use defaults.
4. Package data is encrypted in chunks and manifest records are committed
   after pack data is synced. The final report is assembled from the manifest.
   See [package format](../package-format.md) for the exact on-disk protocol.

An export is a live read, not a consistent snapshot. When the process or
storage disappears, resume reads the last committed state, removes
uncommitted pack tail data, and continues using the stored original source
paths and the supplied options. A partial file whose source fingerprint changed
is restarted. Source paths can be unavailable, and there is no volume-GUID
remapping interface. A completed package is not a rolling incremental backup.

## Verify and restore

`PackageReader::open` uses the code to unlock the package. Verification reads
the manifest and stored files, decrypts them and checks the hashes. Its report
contains verified files, failed files, completeness and incomplete files; it is
not an inventory of everything that should have been backed up. Status metadata
is informational. The package format's authenticated records and paths are
still subject to implementation and underlying-storage limitations.

Before writing, restore validates stored paths, resolves target known folders,
adapts names and detects collisions. Default conflict policy skips different
existing files; CLI callers can choose `KeepBoth`. Restore uses temporary files
and verifies restored content by default, and checks existing content to support
idempotent reruns. It reports source omissions, conflicts, failures, name
changes and links not restored. See [coverage](../guides/coverage.md).

The desktop checks the target and detected known-folder paths for cloud-like
names, requiring a user checkbox when detected. The CLI checks detected known
folders and prompts unless explicitly overridden. This is a heuristic, not a
cloud API or a complete prevention mechanism.

## Scope boundaries

The code does not currently provide VSS snapshots, an elevation manifest,
backup/restore privileges, offline registry hive loading, a signed-in-user
detector, an installed-program inventory, browser-specific migration adapters,
or registry merge. User registry files may be backed up only if actually read;
they are not guaranteed to be archived. CLI file-level smoke tests are not a
GUI interaction test. See [current coverage](../guides/coverage.md),
[security](security.md), and [testing](building-and-testing.md).

## Changing the system

- A new package record or field needs backward-compatibility and corruption
  tests plus an update to [the format spec](../package-format.md).
- Changes to paths, folder mapping, conflict handling, links or resume need
  tests against hostile inputs and interruptions, on the relevant platform.
- Do not label content verified merely because copying finished, or infer
  completeness from a clean content-hash check.
- Keep customer-identifying information and transfer codes out of plaintext
  labels, routine diagnostics and test fixtures.
