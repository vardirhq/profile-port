# Windows gotchas

Platform behaviours the engine must handle explicitly. Each one turns into a
test fixture (see the proposal's testing strategy) and, where relevant, a
distinct line in the transfer report.

## Profile structure

- **Legacy junctions.** Profiles contain compatibility junctions such as
  `Application Data`, `Local Settings`, `My Documents`, `Cookies` and
  `NetHood`, some of which point back into the profile. Following them causes
  duplicate data or infinite loops. Never follow reparse points; record them.
- **Known-folder redirection.** Desktop, Documents, Pictures etc. may be
  redirected (Group Policy, OneDrive, or the user moving them). Resolve
  locations with `SHGetKnownFolderPath` for the live user, or from the
  `User Shell Folders` key in the user's hive for offline profiles. Never
  assume `C:\Users\<name>\Documents`.
- **Profile list.** Discover profiles from
  `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList` (each SID →
  `ProfileImagePath`). For an offline Windows drive, load that installation's
  `SOFTWARE` hive with `RegLoadKey` first. Skip system SIDs (S-1-5-18/19/20)
  and `.bak` / temporary profiles, but show temporary ones because they
  often hold the data people think they lost.

## OneDrive

- **Cloud-only placeholders (export).** With Files On-Demand, many files are
  stubs (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS` /
  `FILE_ATTRIBUTE_RECALL_ON_OPEN`). Reading them triggers a download — or
  fails offline. Detect them, do not hydrate by default, and report
  "in OneDrive, not copied" with a count and size. Offer hydration as an
  explicit option.
- **Redirected destination (restore).** If the destination's Documents is
  backed by OneDrive, restoring into it uploads the customer's data to the
  cloud. Warn and require confirmation; offer restoring to a local folder
  instead.

## Encryption and access

- **BitLocker** on an attached drive: the volume is unreadable until unlocked
  with the recovery key. Detect and explain; never report "no profiles found".
- **EFS-encrypted files** cannot be read on another machine or by another
  account without the user's certificate. Report them individually.
- **ACLs.** Reading other profiles needs `SeBackupPrivilege`
  ([0004](decisions/0004-privilege-model.md)). On restore, do not copy source
  ACLs: they reference SIDs from the old machine.

## Filesystem edge cases

- **Long paths:** always use `\\?\` prefixed paths internally.
- **Alternate data streams:** `Zone.Identifier` (Mark of the Web) and others.
  Preserve them; MOTW affects whether Office opens documents in Protected View.
- **Hard links:** preserve as one stored file + link references, don't store
  twice.
- **Sparse and compressed files:** store logical content; report if a large
  sparse file expands significantly.
- **Per-directory case sensitivity** (used by WSL): two files differing only
  by case can exist. Detect collisions before restoring to a case-insensitive
  directory.
- **Reserved names** (`CON`, `NUL`, `COM1`…), trailing dots/spaces: possible
  in NTFS via `\\?\`, invalid in many APIs. Handle via the package format, not
  the destination filesystem.
- **Timestamps:** FAT32 has 2-second precision, exFAT 10 ms. With the
  chunked package format ([0002](decisions/0002-package-format.md)),
  timestamps live in the manifest and survive regardless.

## Source drive behaviour

- **Drive letters change** between sessions and machines. Identify volumes by
  volume GUID / serial, and the package by its ID, never by letter.
- **Surprise removal** of the USB stick: every write path must assume it can
  vanish mid-write. Resume works from the last committed chunk.
- **Sleep and power.** Long transfers on laptops: request
  `SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)` during
  transfers so the PC doesn't sleep mid-copy.
