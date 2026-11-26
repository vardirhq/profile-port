# Command-line guide

`profileport-cli` is the command-line interface to the same engine used by the
[desktop app](desktop.md). It supports Windows, Linux and macOS in CI; the
project is Windows-first and real client hardware and cloud integrations still
need testing. Use synthetic data while evaluating it and keep an independent
backup. Run `profileport-cli --help` or `profileport-cli <command> --help` for
options in the version you are running.

All paths below are examples. Substitute existing, disposable paths. On
Windows, quote paths containing spaces. The transfer location must be separate
from the source; the CLI does not provide all of the desktop preflight checks.

## Find profiles and folders

```powershell
profileport-cli profiles
```

The command lists profiles found on the current OS and available known-folder
paths. A folder's presence in this list is not proof that its files are
readable or fully local. Windows profile discovery uses the running system's
registry; it does not load an offline Windows installation or an offline user
hive. See [coverage](coverage.md).

## Export

```powershell
profileport-cli export 'C:\Users\test-source' --drive 'E:\' --ticket VM-001
```

The command creates a package under `E:\ProfilePort\transfers\`, displays a
code, and asks the operator to type it back before copying. Record the code
securely. The code is not stored on the drive, cannot be recovered by
ProfilePort, and is required to resume, verify or restore the package. The
plaintext ticket must be 1–32 ASCII letters, digits, `_`, or `-`; do not use
identifying information. The source profile and detected external known folders
are copied subject to default exclusions and access.

`--computer` and `--profile` override encrypted source labels. `--no-confirm`
skips the type-back for scripted use; it does not make recovery possible if
the code was not retained. CLI output includes the code on export. Avoid
capturing or sharing it in logs. If the type-back prompt ends without a
confirmation (end of input), the empty package is removed; if it cannot be
removed, the error names its folder. Killing the process at the prompt (for
example with Ctrl+C) can still leave an empty package. An interrupted copy may
be resumable.

Inspect the report for skipped files, files changed while copying, exclusions,
link and exclusion counts, the package path, and known folders. The CLI limits
individual entries in printed lists to 25 and reports how many more remain; it
does not provide a way to export the full list. Export completion does not
implicitly verify stored content. Terminal output is not a persistent report
file.

## List and resume

```powershell
profileport-cli list --drive 'E:\'
profileport-cli resume 'E:\ProfilePort\transfers\VM-001-example'
```

The example package folder name is a placeholder: use the actual path shown by
export or list. Listing does not require a code and displays plaintext ticket,
status, size, file count and folder metadata where available. Status is
informational. Resume asks for the code and the original source paths must be
accessible. It is for an unfinished export; a completed package is not an
incremental backup. Verify after resuming and inspect the report.

## Verify and restore

```powershell
profileport-cli verify 'E:\ProfilePort\transfers\VM-001-example'
profileport-cli restore 'E:\ProfilePort\transfers\VM-001-example' --to 'C:\Users\test-destination'
```

`verify` decrypts stored files and compares hashes, reporting failed or
incomplete items. It cannot show that every desired source file was included.
`restore` requires an existing target folder, maps detected standard folders,
and defaults to leaving different existing files untouched and reading back
restored files for verification. The command reports conflicts, name changes,
failures and issues present in the backup. Read the report and independently
compare expected content. Source links are recorded but not recreated.

`--keep-both` restores a conflicting package file under a different name;
review the reported names. `--allow-cloud-folders` bypasses the CLI's prompt
for detected cloud-synced known folders; detection is heuristic and incomplete.
Confirm the destination and upload implications before using it.

`--no-verify` disables restore read-back. Such a restore is never reported as
complete: it finishes with items to review (exit code `1`), says the files were
not checked, and does not offer to delete the package. Files already present
with identical content are still compared.

The optional `--code` argument to resume, verify and restore can expose the
code in shell history or process listings. Omit it to use the interactive
prompt when possible. Do not include actual codes in documentation, shared
scripts, screenshots or issue reports.

## Exit codes and diagnostics

| Exit | Meaning |
|---|---|
| `0` | Command reports a clean result; check command-specific meaning and options. |
| `1` | Command finished with items to review. |
| `2` | Command failed. |

For a CLI troubleshooting run, `PROFILEPORT_TRACE=1` causes per-folder and
per-file progress to be written to standard error. Paths can be sensitive:
use synthetic data and remove identifying information before sharing output.
The smoke test scripts capture command output and include safeguards for
their fixture; inspect them before use. See [troubleshooting](troubleshooting.md)
and [building and testing](../development/building-and-testing.md).
