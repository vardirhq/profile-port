# 0002 — Transfer package format

**Status:** Accepted


**Amended by [0007](0007-code-in-ticket-and-no-typing.md):** the code is
now kept in the ticket's sensitive-data field (cleared when the job closes)
rather than on a paper device tag, and the desktop replaces the type-back
with a checkbox. The reasoning below is kept as the original record.

**Implementation status:** The package format is implemented, but this record also contains design alternatives and future operational features. Use the [format specification](../package-format.md) and [security guide](../development/security.md) for the current format and threat boundaries. Plaintext status exposes aggregate file and byte counts; per-chunk authentication is provided by AEAD, while the manifest stores a BLAKE3 hash per file (not a separate BLAKE3 hash per chunk). Full metadata preservation, recovery-job flags and a configurable cleanup age are not implemented (both apps mark backups 14 or more days old). Wiping is implemented (CLI `wipe`, desktop **Wipe backup…**): the header is overwritten in place to destroy the wrapped key, data can optionally be overwritten, then the files are deleted ([format](../package-format.md#wiping)). That is crypto-erase within the filesystem; as this record explains, on flash media it relies on the discarded transfer code for remnants the drive keeps.

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
transfers/<ticket>-<id>/
├── header.bin        # format version, KDF params, wrapped package key
├── status.cbor       # plaintext progress counters for the dashboard
├── manifest.log      # encrypted append-only log: paths, metadata, hashes, chunk refs
└── packs/
    ├── 000000.pack   # ~64 MB append-only pack files of encrypted chunks
    └── …
```

*Implementation note:* the manifest was first planned as an encrypted SQLite
file. It is an **append-only log of individually encrypted records**
instead: SQLite would need a plaintext working copy somewhere while being
written (exactly the leak this format exists to prevent), and a log is
trivially recoverable after a pulled drive — a torn final record is simply
discarded. Full specification: [`docs/package-format.md`](../package-format.md).

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

**Decision:**

- Each package has a random key, wrapped (Argon2id → key-encryption key) with
  a **transfer code** generated when the package is created, **before any
  data is copied**. The code is never
  written to the drive. Restore asks for it.
- The technician writes the code on a **paper device tag** that stays with
  the customer's devices (taped to the old PC or its bag), **not in the
  ticket system**. Ticket systems retain records for years and are backed
  up; a paper tag is discarded when the devices go back to the customer,
  which ends the code's life without depending on any retention policy. The
  export screen says so explicitly: "Write this on the device tag. Don't
  enter it in the ticket system."
- Once the tag is discarded, that customer's data is unreadable even if
  deleted blocks are recovered from flash.
- **No store master key in v1.** It would be a single secret that, if it
  leaked, exposed every past customer, and it would weaken the erasure
  promise for all jobs to protect a few. Revisit only if lost codes prove to
  be a real problem in practice.
- A lost code means redoing the backup from the source PC, which is usually
  still on the bench.

### Transfer code format

A stolen drive can be attacked offline, so the code must resist guessing
even though Argon2id slows each attempt. Six digits (~20 bits) is too weak.

- **~50 bits of entropy:** 10 random characters from Crockford base32
  (no 0/O, 1/I/L ambiguity), plus 2 check characters, shown in groups of
  four: `7KQ2-M9XD-4PWH`.
- Case-insensitive; hyphens and spaces ignored on entry.
- The check characters catch typos before decryption is attempted, so a
  mistyped code gives "That code isn't valid — check the device tag", not a
  confusing decryption failure.
- Argon2id parameters stored in `header.bin` so they can be raised in later
  format versions.

### Guarding against a lost code

The real risk is a mistranscribed code while the source is about to be
released — worst for recovery from a failing drive, where a second backup
may be impossible. So:

- The code is shown and the technician **types it back once before copying
  starts**. A mismatch shows the code again. Showing it only at the end
  would make an interrupted export impossible to resume: the code is the
  only way back in. (Found while writing the crash-and-resume tests.)
- For packages marked as **recovery jobs**, ProfilePort never offers to
  delete the package until the restore is verified **and** the technician
  confirms the customer has checked their data.

## Plaintext labels

The encrypted manifest hides names, so the dashboard cannot show
"j.miller on DESKTOP-4KQ7F2" without the code. Storing that in plaintext
would put customer-identifying data on the drive.

**Decision:** packages carry a plaintext **ticket number** (entered at
export), date, size and status only. Profile and computer names appear only
after the code is entered. Ticket numbers are how technicians already match
jobs, and they identify no one without the store's own system.

## Lifecycle

- After a **verified** restore, the final step asks: "Delete this backup from
  the drive now?" (a prompt, not automatic deletion). Recovery jobs are the
  exception described above.
- Packages older than a configurable age (e.g. 14 days) are flagged on the
  dashboard for cleanup.
- "Prepare for next customer" deletes selected packages. With per-job codes
  this honestly makes their data inaccessible once the device tags are
  discarded.

## Decision

Option B with per-package keys wrapped by a transfer code recorded on a paper
device tag, from Phase 1. No master key in v1.

## Consequences

- Phase 1 includes the format, the manifest schema and the crypto — more up
  front, nothing to migrate later.
- Robocopy cannot write this format, which settles most of
  [0003](0003-copy-engine.md).
- The format must be versioned and documented (`docs/package-format.md`)
  before the first release, with a stand-alone extractor.
- A lost transfer code means redoing the backup. The type-back check and the
  recovery-job safeguard keep this rare.
- The dashboard's backup list is keyed by ticket number; profile and computer
  names appear only for packages unlocked in the current session.
- Staff must actually keep codes out of the ticket system. This is a store
  procedure, and the app reminds them at the moment the code is shown.
