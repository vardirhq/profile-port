# AGENTS.md

Guidance for coding agents working in this repository. Human-facing docs live
in `README.md` and `docs/`; this file is the short version of what you need to
change code safely.

## What this is

ProfilePort moves a user's files and settings between PCs via a USB drive.
Rust workspace, early development. Primary target is Windows 10/11; the engine
and CLI also run on Linux and macOS, and backups restore across all three.
The tool handles customer data, so correctness and honest reporting matter
more than features.

| Crate | Path | Role |
|---|---|---|
| `profileport-package` | `crates/package` | Encrypted, chunked package format: transfer codes, crypto, manifest, writer/reader, crash recovery |
| `profileport-engine` | `crates/engine` | Profile and known-folder discovery, exclusions, export/resume, verify, restore, name rules, platform code (`system/{unix,windows}.rs`, `win.rs`) |
| `profileport-cli` | `apps/cli` | `profileport-cli` binary: argument parsing, prompts, reports, exit codes |
| `profileport-desktop` | `apps/desktop` | Slint UI (`ProfilePort` binary); `src/transfer.rs` bridges to the engine, `ui/*.slint` holds screens and theme |

Dependency direction: `package` ← `engine` ← `cli` / `desktop`. The apps call
the engine directly; there is no service layer. See
`docs/development/architecture.md`.

## Commands

```sh
cargo fmt --all -- --check                                                     # max_width = 120 (rustfmt.toml)
cargo clippy --workspace --exclude profileport-desktop --all-targets -- -D warnings
cargo test --workspace --exclude profileport-desktop                           # package, engine, CLI
cargo test -p profileport-desktop                                              # needs Slint's native deps on Linux
cargo build --release -p profileport-cli
scripts/unix-smoke.sh target/release/profileport-cli                           # Linux/macOS end-to-end
```

Run fmt, clippy and the workspace tests before committing; CI
(`.github/workflows/ci.yml`) enforces clippy with `-D warnings` on Linux and
runs tests plus smoke tests on Linux, macOS and Windows.
`scripts/windows-smoke.ps1` runs only on Windows. Windows-only code
(`#[cfg(windows)]`, `windows-sys`) is not compiled on Linux, so it is only
checked by Windows CI — keep it small and say so when you could not build it.

The smoke scripts create and delete test directories; read them before running
locally. `scripts/screenshot.sh` overwrites `docs/screenshots/*`; don't run it
unless asked.

## Rules that matter here

**Data safety**
- Restore never overwrites or deletes user files. Existing different files are
  skipped (or `--keep-both`); identical ones count as done so reruns are safe.
  Don't weaken this.
- Never follow reparse points / symlinks / junctions on export or destination
  links on restore; record them instead.
- Every write path must tolerate the drive vanishing mid-write. Resume works
  from the last committed record; pack data is synced before the manifest
  record that references it.
- Validate stored paths on restore as hostile input.

**Honest reporting**
- Don't call content "verified" just because a copy finished, and don't treat
  a clean hash check as proof the source was captured completely.
- Skipped, changed-during-backup, renamed and unrestored-link items must be
  reported, not dropped. CLI exit codes: `0` complete, `1` finished with items
  to review, `2` failed.

**Secrets and privacy**
- Transfer codes, customer names and identifying paths must not appear in
  plaintext package labels, logs, diagnostics, test fixtures, examples or
  commit messages. `header.bin`, `status.cbor` and package folder names are
  plaintext — keep anything identifying out of them.
- Use synthetic data in tests and screenshots.

**Package format**
- Any new record or field needs backward-compatibility and corruption tests
  and an update to `docs/package-format.md`. ADR 0002 covers the design.

**Platform behaviour**
- Check `docs/platform-gotchas.md` before touching traversal, paths, names or
  metadata (long paths, ADS/Mark of the Web, reserved names, case collisions,
  known-folder redirection, OneDrive).
- Changes to path handling, folder mapping, conflicts, links or resume need
  tests covering hostile inputs and interruption on the relevant platform.

## Documentation

Docs are written to state what is actually implemented and tested, and to
list gaps explicitly. Keep it that way:
- When behaviour changes, update the matching guide in `docs/guides/`, the
  status/limits in `README.md`, `docs/development/*` and ADR notes as relevant.
- ADRs in `docs/decisions/` record intent; "Accepted" doesn't mean
  implemented (e.g. VSS, `SeBackupPrivilege`, elevation and offline hive
  loading are not). Add a new ADR for a significant design decision.
- Don't claim GUI, Windows-client or real-USB coverage you didn't test.

## Style

- `rustfmt.toml`: `max_width = 120`, `use_small_heuristics = "Max"`.
- Errors via `thiserror`; library crates return typed errors, not panics.
- Prefer small, focused changes with tests next to the behaviour
  (`#[cfg(test)]` modules or `crates/*/tests/`).
- Keep the binary names distinct: `profileport-cli` vs `ProfilePort` collide
  on case-insensitive filesystems otherwise.
