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
  not make data unrecoverable.

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

## Key management — the important subtlety

Crypto-erase means "destroy the key and the ciphertext is useless". But if the
package key is stored **on the same flash drive**, deleting it has the same
problem as deleting the data: wear-levelling means the old key block may
survive. The key must therefore be protected by something **not stored on
the drive**:

- **Recommended:** the package key is wrapped (Argon2id → key-encryption key)
  with a short **transfer code** generated at backup time, shown to the
  technician and printed on the customer's receipt, and never written to the
  drive. Restore requires the code. Once the code is discarded, the package
  is unreadable even if every deleted block is recovered.
- Optional: a technician master key (stored on the technician's workstation,
  not the stick) as a second wrap, for shops that need recovery if the code
  is lost.

With this, "Prepare for next customer" is simply *delete the packages*, and it
is honest to describe it as making previous customer data inaccessible.

## Decision (proposed)

Option B with transfer-code key wrapping, from Phase 1.

## Consequences

- Phase 1 includes the format, the manifest schema and the crypto — more up
  front, nothing to migrate later.
- Robocopy cannot write this format, which settles most of
  [0003](0003-copy-engine.md).
- The format must be versioned and documented (`docs/package-format.md`)
  before the first release, with a stand-alone extractor.
- A lost transfer code means a lost backup (unless the optional master key is
  used). The UI must make this unmistakable.
