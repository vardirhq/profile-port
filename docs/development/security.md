# Security and data handling

This describes the implementation and its limits; it is not a security audit or
a guarantee of suitability for handling customer data. Use only test data
until the Windows client, USB and operational workflows are validated.

## What the package protects

The package uses a randomly generated package key, authenticated encryption
(XChaCha20-Poly1305) for manifest records and file chunks, and Argon2id to
wrap the key using a transfer code. File contents and source profile/computer
labels are intended to remain encrypted on the transfer drive. Details and
parameters are specified in [the package format](../package-format.md).

The 12-symbol code has 10 random base32 symbols (50 bits) and two check
symbols. The check symbols catch many entry mistakes; they add no secrecy. The
code is the practical key to the backup: anyone who can obtain both package
and code can attempt to open it offline. Protect the code and the drive
separately using the organization's approved process. Do not treat a device
tag as a secret if it is stored with the drive. Do not put a code in a ticket,
issue, shared log or screenshot. There is no implemented code recovery/reset.

The CLI can accept `--code`; command-line arguments can appear in process
listings and shell history. Prefer the interactive prompt. Avoid collecting
unredacted program output and traces: path names can identify customers even
when the code is hidden. The desktop displays the code and collects it as text;
physical screen visibility and memory exposure remain operational concerns.

## What remains visible

`header.bin` contains plaintext ticket, creation time and format/KDF metadata;
`status.cbor` contains state and counters. Package folder names include the
ticket and a generated suffix. Do not put customer names, email addresses or
other private details in tickets. The transfer drive's filesystem metadata,
size and timestamps can also reveal activity. Status is not authoritative or
a substitute for checking the encrypted manifest/content.

## Access, cloud and permissions

- No implementation of automatic elevation, `SeBackupPrivilege`, VSS or
  offline hive loading is present, despite accepted future-oriented ADRs.
  Normal OS permissions apply and inaccessible files may be skipped.
- Desktop cloud detection uses path-name heuristics on the target and detected
  known folders. The CLI uses detected known folders. They can miss synced
  folders and cannot determine whether an upload is authorized.
- Verification reads stored content and compares hashes. It does not establish
  source completeness, application consistency or independent recovery; cache
  bypass is not implemented.
- Restore validates package paths and avoids following destination links; it
  leaves conflicts alone by default. Still verify the selected target and test
  the behavior on the intended filesystem and Windows account before use.
- `--no-verify` on the CLI disables restore read-back. The restore report
  records this (`read_back_skipped`), so such a restore is never counted as
  fully successful: the CLI exits with `1` and does not offer to delete the
  package.

## Operational handling

Use synthetic test data and isolated test drives. Confirm all destinations
before writing. Review skipped and changed files, retain the original and an
independent copy, and perform an independent restore check.

## Wiping backups from the drive

Both apps can wipe a package: the CLI's `wipe` command (and its offer after a
fully successful restore) and the desktop's **Wipe backup…** action. A wipe
first overwrites `header.bin` in place, which destroys the wrapped package
key, then deletes the package's files. Optionally (`--overwrite`, or the
desktop checkbox) it overwrites the manifest and pack data with random bytes
before deleting them. Wiping needs no transfer code, only the ticket number
typed back (or `WIPE` for several backups, or one already partly wiped). It
refuses folders holding anything that isn't part of a package. The details
are in [the package format](../package-format.md#wiping).

What a wipe does and doesn't achieve:

- **Within the filesystem**, the key is overwritten and the files are gone.
  The package can't be opened with the code any more, and the drive no longer
  lists it.
- **On flash media**, wear levelling can keep earlier copies of overwritten
  blocks where only the drive's controller can read them. Everything in a
  package was encrypted before it was written, so such remnants are
  ciphertext, and an old copy of the header still needs the transfer code.
  **Discarding the device tag with the code is part of the wipe.** Neither
  ProfilePort nor this document can prove that a specific drive keeps no
  remnants; treat the encryption plus the discarded code as the protection,
  and the wipe as removing the package from normal reach.
- **Wear levelling is not something `--overwrite` defeats.** It mainly helps
  on media that overwrite in place (hard drives) and against simple undelete
  tools. It takes about as long as the backup did.
- The wipe removes the package from the transfer drive only. Restored files on
  the target PC are plaintext by design, and the source PC still holds the
  original data. Nothing here removes data from either, or from cloud folders
  a restore was allowed to write into.
- Filesystem metadata (directory entries, timestamps, the ticket-based folder
  name) can outlive a wipe in the filesystem's own structures. Ticket numbers
  should therefore identify no one outside the store's own system.
- There is no recovery from a wipe. Only wipe once the restore is verified and
  the customer has checked their files. "Recovery job" protection (ADR 0002)
  is not implemented; the technician decides.

Losing a package's only code also makes recovery impossible, so protect the
device tag until the job is closed.

Report suspected vulnerabilities privately through the repository owner's
established contact route if available; do not include real customer data.
No formal security reporting channel or audit is documented in this repository.
