# 0003 — Copy engine: native Rust, not Robocopy

**Status:** Proposed (largely follows from [0002](0002-package-format.md))

## Context

The proposal planned to start with Robocopy, but also requires a per-file
manifest, resumable transfers and BLAKE3 verification. Robocopy works against
all three:

- Per-file state must be scraped from its log output.
- Hashing needs a second full read of every file.
- It cannot produce an encrypted, chunked package (0002).

Robocopy's genuine strengths — backup-privilege reads, long paths, retries —
are all available directly from Win32.

## Decision (proposed)

A native Rust engine, using plain OS threads (no Tokio; Windows file I/O here
is blocking anyway) and a channel to report progress to the UI.

### Export (read side)

- Open with `CreateFileW` + `FILE_FLAG_BACKUP_SEMANTICS` (with
  `SeBackupPrivilege` enabled) so ACL-protected files in other profiles are
  readable, and `FILE_FLAG_SEQUENTIAL_SCAN`.
- Use `FILE_FLAG_OPEN_REPARSE_POINT`: **never follow** junctions/symlinks;
  record them as links (see [gotchas](../windows-gotchas.md) — profiles
  contain legacy junctions that loop).
- Enumerate alternate data streams with `FindFirstStreamW`.
- Always use `\\?\` paths (no 260-character limit).
- Hash (BLAKE3) and encrypt **while streaming**: each byte is read once.
- Read from a VSS snapshot where available ([0005](0005-locked-files-and-registry.md)).
- Detect change-during-copy by comparing size + last-write time before and after.

### Restore (write side)

- Write to a temporary name, flush, then rename into place; never overwrite
  an existing destination file without an explicit conflict decision.
- Restore timestamps and attributes; do **not** restore source ACLs.
  Destination files inherit the target folder's ACLs and are owned by the
  target user.

### Verification

Three levels, reported distinctly:

1. **Copied** — write completed.
2. **Metadata checked** — size and timestamps match.
3. **Content verified** — BLAKE3 of the read-back data matches the source
   hash.

For level 3 the read-back must bypass the Windows cache
(`FILE_FLAG_NO_BUFFERING`, sector-aligned buffers). Otherwise we verify RAM,
not the USB stick.

## Consequences

- More engine code to own and test than wrapping Robocopy.
- Full control over progress, resume granularity, error classification and
  partial-success reporting — the things the product is judged on.
- Robocopy remains a reasonable *fallback* for an unencrypted "plain folder
  export" feature, if one is ever wanted. Its exit codes are a bitmask
  (0–7 are success variants; ≥8 is failure).
