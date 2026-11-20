# 0002 — Transfer package format

**Status:** Proposed — this is the most consequential open decision.

## Context

The original proposal adds encryption and "protected storage reset" in
Phase 4, but also says (correctly) that cryptographic erasure only works if
data was encrypted *before* it was written. Building Phases 1–3 on a plaintext
format means throwing that format away later.

The destination is usually a USB stick formatted **exFAT** or **FAT32**:

| Limitation | FAT32 | exFAT |
|---|---|---|
| Max file size | 4 GB | effectively none |
| NTFS ACLs | no | no |
| Alternate data streams | no | no |
| Timestamp precision | 2 s (modified) | 10 ms |
| Reserved/invalid names, case-insensitivity | yes | yes |

A plain copy of a profile tree onto these drives silently loses metadata and
fails on large files (VM images, Outlook PSTs, video).

## Options

### A. Plain folder tree + manifest

Copy files as-is under `transfers/<id>/files/…`, with a manifest alongside.

- ✅ Browsable in Explorer; recoverable without ProfilePort.
- ❌ FAT32 4 GB limit; metadata lost; no confidentiality at rest.
- ❌ "Prepare for next customer" can only delete, and on flash, deleting does
  not make data unrecoverable — on a shared store drive, every past customer
  stays potentially recoverable.

### B. Chunked, encrypted store (recommended)

Each package is a directory:

```text
transfers/<package-id>/
├── header.bin        # format version, KDF params, wrapped package key
├── manifest.db.enc   # encrypted SQLite: paths, metadata, hashes, chunk refs
└── packs/
    ├── 0000.pack     # ~64 MB append-only pack files of encrypted chunks
    └── …
```

- Files are split into chunks; each chunk is encrypted with
  XChaCha20-Poly1305 (or AES-256-GCM) under a random per-package key.
- File names, sizes, timestamps, attributes and ADS live only inside the
  encrypted manifest — nothing customer-identifying is in plaintext.
- BLAKE3 hash per file (and per chunk) recorded in the manifest.
- ✅ No file-size limit; full metadata preserved regardless of filesystem.
- ✅ Natural unit for resume (chunk-level) and verification.
- ✅ Confidential at rest from the first byte written.
- ❌ Not browsable in Explorer. Mitigation: "Open backup" / "Extract to
  folder" in ProfilePort, plus a documented format and a CLI extractor so
  data is never locked in.

### C. Per-file encryption mirroring the tree

Middle ground; leaks file count, sizes and (unless names are encrypted) the
directory structure, and still hits the FAT32 limit. Not recommended.

## Usage context and threat model

ProfilePort is used **in-store**, between customers' computers. The USB drive
belongs to the store and is never handed to a customer. Typically the same
technician runs export and restore, often the same day, and the source PC is
usually still on the bench.

So the main risks are:

1. **A lost or stolen store drive** exposing several customers at once (one
   drive holds many jobs over its life).
2. **Leftover data** from earlier customers remaining recoverable on flash.
3. **Casual access** by other staff to customer data not their job.

Because the drive is shared and long-lived, encryption at rest matters *more*
than for a single-customer drive.

## Key management

Crypto-erase means "destroy the key and the ciphertext is useless". But if the
package key is stored **on the same flash drive**, deleting it has the same
problem as deleting the data: wear-levelling means the old key block may
survive. The key must be protected by something **not stored on the drive**,
and **specific to the job** — a store-wide PIN protects a lost drive but
cannot erase past customers, because the PIN never changes and any surviving
wrapped key still opens with it.

**Proposed:**

- Each package has a random key, wrapped (Argon2id → key-encryption key) with
  a short **transfer code** generated at the end of export.
- The technician writes the code on the **job ticket / work order**. It is
  never written to the drive. Restore asks for it.
- When the job is closed and the code is no longer retained, that customer's
  data is unreadable even if deleted blocks are recovered. The erasure
  guarantee therefore depends on the store's **ticket-retention policy** for
  codes. Document this; don't hide it.
- **Optional store master key** (kept on the store's workstation, not the
  drive) as a second wrap, for jobs where a redo is impossible — chiefly
  recovery from a failing drive. Off by default; enabling it means past
  packages are only as erased as the master key is protected.
- A lost code otherwise just means redoing the backup from the source PC.

## Plaintext labels

The encrypted manifest hides names, so the dashboard cannot show
"j.miller on DESKTOP-4KQ7F2" without the code. Storing that in plaintext
would put customer-identifying data on the drive.

**Proposed:** packages carry a plaintext **ticket number** (entered at
export), date, size and status only. Profile and computer names appear only
after the code is entered. Ticket numbers are how technicians already match
jobs, and they identify no one without the store's own system.

## Lifecycle

- After a **verified** restore, the final step asks: "Delete this backup from
  the drive now?" (a prompt, not automatic deletion).
- Packages older than a configurable age (e.g. 14 days) are flagged on the
  dashboard for cleanup.
- "Prepare for next customer" deletes selected packages. With per-job codes
  this honestly makes their data inaccessible once the codes are discarded.

## Decision (proposed)

Option B with per-package keys wrapped by a transfer code recorded on the
job ticket, from Phase 1.

## Consequences

- Phase 1 includes the format, the manifest schema and the crypto — more up
  front, nothing to migrate later.
- Robocopy cannot write this format, which settles most of
  [0003](0003-copy-engine.md).
- The format must be versioned and documented (`docs/package-format.md`)
  before the first release, with a stand-alone extractor.
- A lost transfer code means redoing the backup (or using the optional master
  key). The UI must make this unmistakable before the source PC is released.
- The dashboard's backup list is keyed by ticket number, not by user or PC
  name (the current prototype shows names and needs updating).
- Real-world erasure depends on the store's ticket-retention policy for codes.
