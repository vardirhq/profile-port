# Platform gotchas

This is a working list of platform requirements, risks, and future design
ideas. Many bullets describe intended behavior, not implemented behavior or
verified coverage. Consult [current coverage](guides/coverage.md) and the
[build and test guide](development/building-and-testing.md) before relying on
any item. In particular, offline hive loading, VSS, `SeBackupPrivilege`,
volume-identity tracking, sleep prevention, hard-link preservation, and a
cloud-hydration option are not implemented. Cloud and hardware cases require
additional testing.

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

## Moving between operating systems

See [ADR 0006](decisions/0006-cross-platform-restore.md).

- **Names.** Linux allows `: ? * \ " < > |`, trailing dots, device names
  (`CON`) and names differing only by case. Restoring to Windows adapts them
  (`NameRules::Windows`) and reports each rename; case twins on Windows or
  macOS become `name (restored).ext`.
- **Unicode normalisation.** macOS file systems treat `é` (one code
  point) and `e` + combining accent as the same name; Linux does not. Two
  such Linux files would collide on a Mac. Not handled yet (rare).
- **AppData** is restored as files on Linux/macOS but no program there
  reads it. Firefox and Thunderbird profiles are portable and are
  candidates for migration adapters.

## macOS

- **Profiles** are the folders in `/Users` (except `Shared` and `Guest`).
  People live in Directory Services, not `/etc/passwd`.
- **Standard folders** have fixed English names on disk (`Documents`,
  `Movies`…). Finder shows them translated via `.localized`, so never
  match on the displayed name.
- **Privacy protection (TCC).** Without *Full Disk Access* for the app or
  terminal running ProfilePort, parts of `~/Library` (Mail, Messages,
  Safari) and sometimes Desktop/Documents/Downloads cannot be read. They
  are reported as *access denied*, not silently missed; grant Full Disk
  Access before exporting.
- **iCloud Drive "Desktop & Documents".** When enabled, `~/Desktop` and
  `~/Documents` keep their paths but are synced, and files may be
  cloud-only ("optimised storage"). Paths under
  `~/Library/Mobile Documents` are detected as cloud-synced; the
  Desktop & Documents setting itself is not detected yet.
- **Extended attributes** (`com.apple.quarantine`, the macOS
  "downloaded from the internet" marker, and Finder tags) are not carried
  yet.
