# What a transfer includes

This is a description of the current engine, not a promise that every item on
a particular computer can be read. Inspect the report for each transfer and
keep the source data until recovery has been checked independently.

## Source data

The default export walks the selected profile directory and also attempts to
include resolved standard folders outside it. Standard folders include Desktop,
Documents, Downloads, Music, Pictures, Videos, Templates and, where resolved,
Public. The actual location depends on the operating system and available
profile information. Other profile content, including accessible AppData files,
is included unless excluded or skipped. Programs themselves are not installed
on the destination, and copying their settings files does not guarantee that
those settings will work there.

On Windows, the current user uses the shell's known-folder API. Another user's
loaded registry hive can provide folder locations; otherwise the engine falls
back to existing default paths. It does not load an offline user hive. A
redirected folder may therefore be missed or mapped differently; inspect the
source and destination explicitly. The CLI prints discovered folder mappings;
the desktop restore screen currently does not show a mapping preview.

## Default exclusions

The defaults match these paths relative to the profile, case-insensitively. A
`*` matches one path component; excluded directories are not traversed.

| Paths | Purpose |
|---|---|
| `AppData/Local/Temp`, `AppData/Local/CrashDumps` | Temporary files and crash dumps |
| `AppData/Local/Microsoft/Windows/INetCache`, `WebCache`, `Explorer` | Internet and Explorer caches |
| `AppData/Local/Google/Chrome/User Data/*/Cache`, `Code Cache`, `GPUCache` | Chrome caches |
| `AppData/Local/Microsoft/Edge/User Data/*/Cache`, `Code Cache`, `GPUCache` | Edge caches |
| `AppData/Local/Mozilla/Firefox/Profiles/*/cache2` | Firefox cache |
| `.cache`, `.local/share/Trash`, `.Trash`, `Library/Caches` | Linux/macOS caches and trash |

The short entries in the table following the Windows prefix use that same
prefix. The desktop UI does not let the operator edit exclusions or select
individual data categories. Engine callers can supply different exclusions,
but the current CLI and desktop flows use defaults.

## Links, metadata, and special files

- Symbolic links and junctions are recorded rather than traversed, avoiding
  loops and unintentional traversal. They are not recreated during restore;
  inspect the report, especially for nonstandard links.
- Modification times and read-only state are represented in the package.
  Windows hidden/system attributes and creation time are handled where
  supported. Up to 64 KiB per supported NTFS alternate stream can be stored;
  the implementation has limits, so do not assume arbitrary stream coverage.
- Source ACLs are not migrated as a portable identity mapping. Verify that the
  destination user can access restored files.
- Windows cloud-only placeholders may be skipped to avoid hydration. Cloud
  provider behavior and macOS/iCloud placeholders have not been comprehensively
  tested. Reports and checks cannot be treated as complete cloud inventory.
- Locked, inaccessible, disappearing, unsupported or unreadable files may be
  skipped. The engine records reasons when it can. Live files can change while
  being copied; this is reported, not made transactionally consistent.

## Restore behavior

Package paths are validated and names that the target cannot use are adapted;
renames are reported. Standard source folders are mapped to detected target
folders, and other paths remain relative to the destination. The target must
exist. Destination links are not followed by the restore logic. Default
conflicts leave different existing content untouched; the CLI also offers
`--keep-both`. A file with matching content can be treated as already present.

The package records skipped and changed source files. Re-reading stored content
and matching hashes verifies the stored bytes, not that every source file was
captured or that an application can use its restored data. See
[troubleshooting](troubleshooting.md) and [package format](../package-format.md).
