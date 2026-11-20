# 0006 — Cross-platform restore and standard folders

**Status:** Proposed. The foundation is built and tested; which platforms to
ship for is open.

## Context

The question: can a backup made on Windows be restored on Linux, and vice
versa, with files ending up where that system keeps them?

The engine and package format are already OS-neutral: Rust, relative
UTF-8 paths, timestamps as Unix time, and CI runs the full test suite on
Linux and Windows. What made a plain path-for-path restore wrong across
systems was:

1. **Standard folders live in different places.** Windows `Documents` may
   be `C:\Users\anna\Documents`, `…\OneDrive\Documents`, or `D:\Data`.
   On Linux it is whatever `~/.config/user-dirs.dirs` says, often
   translated (`~/Dokumenter`, `~/Bilder`). On macOS, Videos is `~/Movies`.
2. **File-name rules differ.** Linux allows `:` `?` `*` `\`, names like
   `CON`, trailing dots, and two names differing only by case. Windows
   allows none of these. On Windows `\` and `:` are also structural (a
   separator and a stream/drive designator), so this is a safety issue as
   well as a compatibility one.
3. **Some metadata only exists on one side**: hidden/system attributes and
   alternate data streams (Windows), permission bits (Unix).

## Decision

- **Record meaning, not just paths.** The package stores which folder is
  Documents, Pictures, etc. on the source (`Begin.known_folders`), and
  folders outside the profile under `@Name`. Restore puts each into the
  target's folder of that kind, using the Windows Shell or registry
  ("User Shell Folders") on Windows and XDG user-dirs on Linux.
- **Adapt names to the target, always.** The package format only checks
  structure. Every component goes through the target's name rules
  (`NameRules`) before touching disk. Renames (illegal characters, case
  collisions) are restored under a safe name and **reported**, never
  dropped or overwritten.
- **Carry platform metadata, apply where meaningful.** Windows attributes,
  creation time and small alternate data streams are stored. They are
  applied on Windows and counted as "not applicable" elsewhere.
- **Application data is not translated.** `AppData\…` is restored as files
  but is meaningless on Linux. Cross-platform apps with portable profiles
  (Firefox, Thunderbird; Chromium bookmarks) are candidates for migration
  adapters later. Saved passwords stay non-portable on every platform.
- **Cloud-synced targets need consent.** If a target folder is under
  OneDrive or similar, restore asks first (or needs
  `--allow-cloud-folders`), because writing there uploads the files.

## Consequences

- The format change is backward compatible: older packages lack these
  fields, and restore then keeps paths relative to the profile.
- The engine and CLI build and test on Linux today. The desktop app
  (Slint) also runs on Linux and macOS, but the product is Windows-first
  and nothing is packaged for other platforms yet.
- **Open questions for the product:**
  - Is Linux a real destination for the store's customers? Windows → **macOS**
    is probably the more common retail migration. It needs macOS profile
    discovery (`dscl`), testing on APFS (case-insensitive by default), and
    a macOS CI runner.
  - Should the restore UI let the technician review the folder mapping
    ("Documents → ~/Dokumenter") before writing? Recommended: yes, the CLI
    already prints it.
