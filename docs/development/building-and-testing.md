# Build, test, and contribute

## Prerequisites and workspace

Install a current stable Rust toolchain and Cargo. The workspace contains the
package library, engine, CLI and desktop app. The desktop uses Slint; native
windowing, font and graphics development libraries may be needed on Linux.
There is no documented minimum supported Rust version; CI follows stable.

```sh
cargo test --workspace --exclude profileport-desktop
cargo test -p profileport-desktop
cargo build --release -p profileport-cli -p profileport-desktop
cargo run -p profileport-desktop
```

On Windows the release binaries are `target/release/profileport-cli.exe` and
`target/release/ProfilePort.exe`; the distinct names avoid a collision on a
case-insensitive filesystem. The desktop targets Windows 10/11, but the current
GUI has not yet been visually or interactively verified on Windows clients.
`PROFILEPORT_THEME=dark` opts into dark mode; other values use the default light
mode. Automatic system-theme detection is not implemented.

Running the desktop app on Linux needs X11 or Wayland with the
`libxkbcommon-x11` library (package `libxkbcommon-x11-0` on Debian/Ubuntu). The
**Other folder…** dialog uses the XDG desktop portal there. To make a folder
show up in the drive list (for a test machine or a headless run), set
`PROFILEPORT_DRIVES` to it, separated like `PATH`.

The Windows-only code can be linted from Linux or macOS without a Windows
machine (check only, nothing is linked):

```sh
rustup target add x86_64-pc-windows-msvc
cargo clippy -p profileport-package -p profileport-engine -p profileport-cli -p profileport-desktop \
  --features blake3/pure --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

`blake3/pure` skips blake3's assembly, which would need Microsoft's assembler.

## Automated coverage

Run `cargo test --workspace` where desktop dependencies are available. The
engine and package suites cover encryption, corruption, crash/resume, path
validation, mapping and conflict behavior. Desktop unit tests exercise the
transfer bridge with temporary directories, including a backup/verify/restore
round trip, listing and wiping backups, drive and profile labels, ticket and
code feedback, and selected safety checks. They do not drive the Slint UI.

CI (`.github/workflows/ci.yml`) currently does the following:

| Runner | Checks |
|---|---|
| Linux | Clippy and tests excluding desktop, Clippy of all crates for the Windows target (check only), release CLI build, Unix CLI smoke test |
| macOS | Tests excluding desktop, release CLI build, Unix CLI smoke test |
| Windows | Tests excluding desktop, desktop transfer tests, release CLI and desktop builds, Windows CLI smoke test |

GitHub-hosted Windows is a Windows runner, not proof of a Windows 10/11 client
GUI workflow. The scripts exercise real CLI commands, not button clicks.
`scripts/windows-smoke.ps1` reports PASS/FAIL and known GAP results. It
creates and **removes** a `pp-smoke` directory under `RUNNER_TEMP` or the system
temp directory: inspect it and use a disposable environment. Run from the repo:

```powershell
cargo build --release -p profileport-cli
./scripts/windows-smoke.ps1 -Exe target/release/profileport-cli.exe
```

On Linux/macOS, after building the release CLI, run:

```sh
scripts/unix-smoke.sh target/release/profileport-cli
```

Inspect scripts before running them; they create and remove test data. Check
exit status and review GAP entries instead of equating a green run with feature
completeness.

## Visual and real-device checks

`scripts/screenshot.sh [scale]` builds the desktop and captures both themes on
Linux using Xvfb, ImageMagick and xdotool. It writes over the two screenshot
files in `docs/screenshots`; save or compare existing files before running it.
The checked-in screenshots depict the old dashboard, not the current transfer
screens. The helper is not an interactive test and relies on the current
machine's profile list. Use fictional data when capturing screenshots.

Follow the [VM test plan](../guides/windows-vm-testing.md) to assess the GUI,
Windows account behavior, restore conflicts, locked files and virtual-drive
interruption. Real USB, Windows client, cloud-placeholder, BitLocker and ARM64
coverage remain separate until actually tested and recorded.

## Documentation maintenance

Update the user guides, status/limitations, relevant ADR notes, and package
specification when behavior changes. ADRs record decisions; an accepted status
does not imply their whole design is implemented. Keep the desktop and CLI
instructions distinct, avoid exposing secrets in examples, and state what was
actually tested. Do not regenerate old screenshots without reviewing them.
