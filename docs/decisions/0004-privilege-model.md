# 0004 — Always run elevated

**Status:** Accepted

## Context

The proposal asks for "minimal privileges" and elevation only when needed. In
practice, nearly every technician workflow needs administrator rights:

| Task | Needs admin? |
|---|---|
| Reading another user's profile | Yes (ACLs; `SeBackupPrivilege`) |
| Reading a profile on an external/offline Windows drive | Yes, usually (ACLs carry over) |
| Loading an offline registry hive (`RegLoadKey`) | Yes |
| VSS snapshots | Yes |
| Restoring into another user's profile | Yes |
| Backing up *your own* profile while logged in as yourself | No — but this is the rare case |

Elevating mid-flow (re-launching with `runas`) means splitting the app into
two processes or restarting it, which complicates state, progress reporting
and resume.

## Decision

Embed an application manifest with `requestedExecutionLevel
level="requireAdministrator"`. One UAC prompt at launch; the whole session
runs elevated. Enable `SeBackupPrivilege` / `SeRestorePrivilege` only around
the operations that use them.

The principle of least privilege is kept *inside* the app instead: every
destructive operation is an explicit, previewed, confirmed operation with a
validated target (see the proposal's "explicit operations" principle).

## Consequences

- Simple mental model: if ProfilePort is running, it can do its job.
- Cannot be used by a standard user without admin credentials. Acceptable for
  the target audience (technicians, IT); individual users can enter their own
  admin password.
- An elevated process owning files it writes: restored files must be
  explicitly re-owned to the target user (see [0003](0003-copy-engine.md)).
- **Code signing.** An elevated, unsigned EXE shows "Unknown publisher" in the
  UAC prompt and is more likely to be flagged by Defender or other AV. Buy an
  Authenticode certificate before ProfilePort goes onto store drives, so the
  UAC prompt shows a real publisher name and technicians learn to expect it.
