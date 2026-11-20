# 0005 — Locked files: avoid, then VSS; user registry archived, not merged

**Status:** Accepted — VSS deferred to Phase 2

## Context

On a running PC several important files are locked or changing:

- `NTUSER.DAT` — the user's registry hive (`HKCU`), always locked while the
  user is logged on, and often while background processes hold it.
- Outlook data files (`.pst`, `.ost`).
- Browser profile databases (SQLite, LevelDB) while the browser runs.

The original proposal says "close applications or use an appropriate
consistency mechanism" without naming one, and does not mention the user's
registry settings at all, although many settings live there.

## Decision

### Phase 1: avoid the locks instead of working around them

In-store, the technician controls how the source PC is signed in to. If they
sign in with a **separate administrator account** rather than as the
customer, the customer's hive isn't loaded and their Outlook and browsers
aren't running — so most locks never happen. Phase 1 therefore:

- **Detects whether the source user is signed in** (active or disconnected
  session) and, if so, recommends signing them out before export, with a
  one-click "Sign out user" where permitted.
- Warns if known applications (Outlook, browsers, Thunderbird) are running
  under that user.
- Reports anything still locked as **skipped (locked)** — individually, in
  the report — never silently.

### Phase 2: Volume Shadow Copy

For the cases Phase 1 can't avoid (the customer must stay signed in, a
service holds files open, background sync):

- Export reads from a **VSS snapshot** of the source volume
  (`IVssBackupComponents`). This gives a crash-consistent view of every file,
  including locked ones, without asking the customer to close anything.
- Works for the system volume and for attached NTFS volumes (including an
  old PC's drive in a USB enclosure).
- If VSS is unavailable (FAT/exFAT source, service disabled, snapshot fails),
  fall back to live reads and report locked files as **skipped (locked)**
  rather than failing silently.
- ProfilePort must be built **native** for each architecture (x64, ARM64):
  VSS refuses to run from a 32-bit process on 64-bit Windows.

Browser adapters still warn when the browser is open, because a
crash-consistent copy of an open browser database is valid but may lose the
last few seconds of state.

### User registry: archive always, restore selectively

- Always **archive** `NTUSER.DAT` (from the snapshot) and `UsrClass.dat` in the
  package, so nothing is lost and a technician can extract settings later.
- **Never merge a whole hive** into the destination: it contains SIDs, paths,
  machine-specific and app-version-specific state, and merging can break the
  destination profile.
- Restore specific keys only through **migration adapters** (for example
  mapped drives, wallpaper, mouse/keyboard preferences, specific app
  settings). Each adapter declares exactly which keys it reads and writes.
- To write to the destination user's `HKCU`, load their hive with
  `RegLoadKey` while they are logged off. If they are logged on, write
  through `HKEY_USERS\<SID>` instead.

## Consequences

- VSS requires admin ([0004](0004-privilege-model.md)) and adds
  a COM-heavy Windows-specific module. It is worth it: locked files are the
  most common cause of "the backup looked fine but Outlook is missing".
- The archived hive makes "Archival backup vs managed restore" (proposal
  §4.2) concrete for registry data too.
- Snapshot creation takes a few seconds and needs free space on the source
  volume for the diff area; surface both in the UI.
