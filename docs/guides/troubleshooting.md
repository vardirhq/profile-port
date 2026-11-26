# Troubleshooting and recovery

ProfilePort is early software. When a transfer is uncertain, preserve the
source and package, retain the code securely, and work from disposable copies
where possible. Do not post a transfer code, real ticket, or private file path
in a public issue.

| Symptom | What to check |
|---|---|
| No profiles found | Check that this is the running installation, the profile exists, and the account can access it. Offline Windows installations are not enumerated as installations. Type an existing folder path if appropriate. |
| Cannot prepare backup | Check source and drive paths exist, ticket uses only ASCII letters, digits, `_` or `-`, and the drive is not inside the source. Check permissions and available space. |
| Code does not match | Re-enter the saved code; parsing ignores spaces and hyphens, case, and common `O`/`0`, `I`/`1` and `L`/`1` confusions. A valid but wrong code cannot unlock the package. There is no reset or recovery mechanism. |
| Backup interrupted | Reconnect the transfer drive and source at their original paths, select the package and use **Resume backup** with its code. Resume is supported for unfinished exports; preserve the original package until recovery is understood. |
| Wrong drive after reconnect | Drive letters can change. Locate the package on the actual test drive and enter its current path. Resume also needs the source paths recorded during export; volume identity tracking is not implemented. |
| Skipped or changed files | Review the displayed report; the CLI truncates individual lists after 25 entries, and the desktop export shows only counts for exclusions and links. Close applications and check access or cloud availability. Verification of stored bytes does not recover missing files or make an inconsistent copy consistent. A new backup may be necessary. |
| Verification fails | Preserve the package and source; check that the drive is connected and the correct code and package are being used. Record the reported failing files. Do not treat other verified files as proof the whole backup is usable. |
| Destination conflict | Desktop restore leaves different existing content in place and reports the conflict. Inspect both files. CLI `--keep-both` is available, but the desktop UI does not expose it. |
| Cloud warning | Restoring into a synced folder can upload data. Desktop detection checks the target and known-folder paths using name heuristics, which are incomplete. Confirm the destination and authorization before enabling the checkbox. |
| App cannot access another user | Running as an administrator does not by itself implement backup privileges or guarantee access. `SeBackupPrivilege` and offline hive loading are not implemented. Use test data, check access, and record skipped items. |
| Incomplete or missing app settings | AppData is copied as files where accessible, but installed-program inventory, browser migration adapters, full registry migration and app reinstall are not implemented. Open and check applications separately. |

## CLI cautions

The CLI's `--code` option places the code in the command line, which can be
visible in shell history or process inspection; omit it for an interactive
prompt when possible. `--no-confirm` skips the code type-back and is intended
for scripted use. `--no-verify` disables read-back of restored files; the
restore then always finishes with items to review (exit code `1`) and no
deletion is offered. In interactive use the CLI may offer to delete a package
after a restore it considers fully successful (read back and nothing to
review); retain it until your own checks are complete. The GUI does not expose
this deletion prompt.

Status files and package lists are informational, not proof of content
integrity. A finished export, stored-file verification, and a verified restore
answer different questions; use all relevant reports and compare expected data.
