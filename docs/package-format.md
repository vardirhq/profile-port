# Package format, version 1

The on-disk format of a ProfilePort transfer package. The rationale is in
[ADR 0002](decisions/0002-package-format.md). Reference implementation:
`crates/package`.

This document exists so that customer data is never locked into
ProfilePort. With a package and its transfer code, the contents can be
recovered by any implementation of this spec.

## Layout

```text
<drive>/ProfilePort/transfers/<ticket>-<hex>/
├── header.bin
├── status.cbor
├── manifest.log
└── packs/
    ├── 000000.pack
    ├── 000001.pack
    └── …
```

All integers are little-endian. CBOR is RFC 8949 as produced by
`ciborium` from the Rust types in `crates/package/src/format.rs`
(externally tagged enums, struct fields by name).

## Cryptography

| Purpose | Algorithm |
|---|---|
| Encryption of records and chunks | XChaCha20-Poly1305, 24-byte random nonce, 16-byte tag |
| Key derivation from the transfer code | Argon2id v1.3, parameters in the header (default 64 MiB, t=3, p=1), 32-byte output |
| File and code-check hashes | BLAKE3 |

`seal(key, aad, m)` = `nonce ‖ XChaCha20-Poly1305(key, nonce, aad, m)`.

## Transfer code

12 symbols from the Crockford base32 alphabet
`0123456789ABCDEFGHJKMNPQRSTVWXYZ`, shown as `XXXX-XXXX-XXXX`.

- Symbols 1–10: random (50 bits).
- Symbols 11–12: the first two bytes of
  `BLAKE3("profileport transfer code check v1" ‖ s₁…s₁₀)`, each masked to 5
  bits, where `sᵢ` are the symbol *values* (0–31).
- Parsing ignores `-` and whitespace, is case-insensitive, and maps
  `O→0`, `I→1`, `L→1`.
- The Argon2id password is the 10 data symbols as ASCII characters.

## header.bin (plaintext)

```text
"PPORTPKG" (8 bytes) ‖ version: u16 = 1 ‖ CBOR(Header)
```

`Header` fields: `package_id` (16 random bytes), `ticket` (1–32 of
`[A-Za-z0-9_-]`), `created_unix`, `kdf` {`m_cost_kib`, `t_cost`, `p_cost`,
`salt` (16 bytes)}, `wrapped_key`, `chunk_size`.

`wrapped_key = seal(KEK, AAD_h, package_key)` where
`KEK = Argon2id(code, kdf)` and

```text
AAD_h = "profileport header v1" ‖ package_id ‖ created_unix: i64 ‖ chunk_size: u32 ‖ ticket (UTF-8)
```

Binding the ticket in the AAD means a relabelled package refuses its code.

## status.cbor (plaintext)

`Status` {`state`: InProgress | Complete | CompleteWithIssues, `files`,
`bytes`, `skipped`, `updated_unix`}. Informational only. It is rewritten
atomically on each commit, and the manifest is authoritative.

## manifest.log (encrypted)

A sequence of frames:

```text
len: u32 ‖ seal(package_key, AAD_i, CBOR(Record))
AAD_i = "profileport log v1" ‖ package_id ‖ i: u64      (i = 0, 1, 2, …)
```

Binding the index means frames can't be reordered, removed from the
middle, or moved between packages.

**Reading rule:** a frame that is truncated, or that fails authentication
*and* ends exactly at end-of-file, is a torn write. Discard it and stop.
Any other failure means the manifest is corrupt.

### Records

| Record | Meaning |
|---|---|
| `Begin` {source_root, computer, profile, started_unix, tool_version, source_os, known_folders} | Always first. `source_os` is `windows`, `linux`, `macos`…; `known_folders` lists {folder, path, source_path}, see below. Both default to empty when absent |
| `Dir` {path, meta} | A folder |
| `FileStart` {id, path, meta, size} | A file begins; `size` is the size at start |
| `Chunks` {id, chunks: [ChunkRef]} | Further chunks of file `id`, in order |
| `FileEnd` {id, blake3, changed_during_copy} | File complete; `blake3` is the hash of all its plaintext |
| `FileAbort` {id} | Reading the source failed; discard file `id` |
| `Link` {path, target, meta} | A symlink or junction, recorded but not followed |
| `Skipped` {path, reason, detail} | Not copied; `reason` is one of Excluded, PermissionDenied, Locked, Vanished, InvalidName, CloudOnly, ReadError |
| `AltStream` {path, name, data} | An NTFS alternate data stream (≤ 64 KiB, e.g. `Zone.Identifier`) of the file at `path`, stored inline |
| `End` {finished_unix} | Export finished. Without it the package is incomplete |

A file is in the package only if its `FileStart` is followed by a
`FileEnd`. `ChunkRef` = {`pack`, `offset`, `stored_len`, `plain_len`}.

`meta` = {`modified`, `created`: optional {secs: i64, nanos: u32} since the
Unix epoch, `readonly`, `win_attributes`: optional u32}.

### Paths

Relative, `/`-separated, UTF-8. Readers **must** reject (and restore
nothing from) a package containing a path that is empty, starts with `/`,
or has a component that is empty, `.` or `..`, or that contains a control
character. `Skipped` paths are informational and are never restored.

Components may contain characters that are legal on the source system
but not on the target: `:` and `\` are ordinary characters in Linux file
names. A reader **must** adapt every component to the target's rules
before using it as a path. On Windows that means replacing
`< > : " | ? * \`, appending `_` after a trailing dot or space, and
prefixing `_` to device names (`CON`, `NUL`, `COM1`, …). This is not
cosmetic: unadapted, `\` would act as a separator and `:` as a
drive or stream designator.

Stream names (`AltStream.name`) must be a single plain name: no
`: \ / < > " | ? *` or control characters.

### Known folders

`Begin.known_folders` says where the source kept its standard folders
(`Desktop`, `Documents`, `Downloads`, `Music`, `Pictures`, `Videos`,
`Templates`, `Public`), as package paths:

- a folder inside the profile keeps its relative path, e.g.
  `OneDrive/Documents` when Documents was redirected into OneDrive;
- a folder outside the profile (redirected to another drive) is stored
  under a top-level `@Name` directory, e.g. `@Music`. `source_path` is its
  absolute path on the source, used to resume the export.

On restore, a path under a known folder goes under the *target's* folder
of the same kind (e.g. `~/Dokumenter` on a Norwegian Linux desktop),
matching the prefix case-insensitively when `source_os` is `windows` or
`macos`. Other paths stay relative to the profile, and an unmapped `@Name`
becomes `Name`.

## packs/NNNNNN.pack (encrypted)

Concatenated sealed chunks with no framing. They are located only through
`ChunkRef`s.

```text
chunk = seal(package_key, AAD_c, plaintext)        |plaintext| ≤ chunk_size
AAD_c = "profileport chunk v1" ‖ package_id ‖ file_id: u64 ‖ chunk_index: u64
```

Pack bytes not referenced by any committed record (left over from an
interruption) are garbage. Writers truncate them on resume.

## Write protocol (durability)

1. Write chunk data to the current pack.
2. **Commit:** `fsync` the pack(s), *then* append the buffered records to
   the log, *then* `fsync` the log, then rewrite `status.cbor` atomically.

A record that refers to chunk data is therefore never durable before the
data. After a crash, everything up to the last commit is intact.

**Resume:** read the log, truncate it to the valid prefix, find the
furthest committed chunk, truncate that pack there, delete later packs,
and continue. A file with `FileStart` but no `FileEnd` is continued from
its committed chunks if the source file is unchanged (same size and
modified time). Otherwise it is aborted and copied again.

## Wiping

A wipe (`profileport_package::wipe`) removes a package once the job is done.
It does three things, syncing each to the drive before the next:

1. **Destroy the key.** `header.bin` is overwritten in place, at its
   existing length, with `"PPORTWIP"` (8 bytes) followed by random bytes. The
   wrapped package key is gone, so the package can't be opened even with its
   code. Readers report such a folder as *partly wiped* rather than ignoring
   it, so an interrupted wipe stays visible.
2. **Overwrite the data** (optional). `manifest.log` and every pack are
   overwritten in place with random bytes.
3. **Delete** the packs, `packs/`, `manifest.log`, `status.cbor` (and
   `status.tmp`), then `header.bin`, then the package folder.

Before changing anything, the wipe checks that the folder is a real
directory (not a link) holding only a `header.bin` that starts with
`"PPORTPKG"` or `"PPORTWIP"`, `status.cbor`/`status.tmp`, `manifest.log`, and
a `packs/` directory of regular files named `NNNNNN.pack`. Anything else, and
nothing is touched. An empty folder (a wipe interrupted after deleting the
header) is removed. Rerunning an interrupted wipe finishes it.

**Limits.** An in-place overwrite reaches the same blocks *as the
filesystem sees them*. Flash drives and SSDs remap writes internally (wear
levelling), so earlier copies of a block can survive where only the drive's
controller can read them. Those are ciphertext. A surviving copy of the old
header still opens with the transfer code, which is why the code lives only
on the paper device tag: once the tag is discarded, remnants are unreadable.
See [ADR 0002](decisions/0002-package-format.md#key-management).

