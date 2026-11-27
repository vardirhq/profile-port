# 0007 — Transfer code kept in the ticket; desktop without typing

**Status:** Accepted. Amends the key-management and "guarding against a lost
code" parts of [0002](0002-package-format.md).

**Implementation status:** The desktop and CLI wording, the desktop's
pick-from-lists screens and the code-written checkbox are implemented. The
ticket system clears the sensitive-data field when a ticket is set to finished;
that is the ticket system's behaviour, which ProfilePort relies on but can't
enforce.

## Context

ADR 0002 has the technician write the transfer code on a paper device tag that
stays with the customer's devices and is thrown away when they go back. That
ends the code's life without relying on any retention policy, which is what
makes a wipe final on flash media (see
[security](../development/security.md#wiping-backups-from-the-drive)).

In practice the store already keeps per-job secrets (passwords, PINs) in a
**sensitive-data field** of the ticket. It is the established place for such
data, and the ticket system clears it when the ticket is set to finished, as
data protection requires. A separate paper tag is a new habit,
can be lost, and puts the code on the bench where anyone can read it.

Separately, the desktop app should be usable without typing anything except
the ticket number and, at restore, the code. ADR 0002 also has the technician
type the code back before copying, so a mistranscribed code is caught while
the source is still there.

## Decision

1. **The code is kept in the ticket's sensitive-data field**, never in
   ordinary ticket notes, and never on the transfer drive. The app's wording
   says so. A paper device tag remains acceptable where a store has no such
   field.
2. **The code's life ends when the ticket is set to finished**, because the
   field is cleared then. Until then, anyone who can read that field and has
   the drive can open the backup, as with the passwords kept there.
3. **The desktop replaces the type-back with a checkbox**: "I've stored the
   code in the ticket and checked it". The CLI keeps the type-back.
4. **The desktop takes everything else from lists**: profiles, detected
   drives (`system::list_drives`), and the backups on a drive. A native
   folder dialog covers profiles the system doesn't list.

## Consequences

- **Wiping relies on the field being cleared.** On flash media a wipe removes
  the backup from the filesystem, but encrypted remnants may survive inside
  the drive. They are unreadable once no copy of the code exists, which is
  when the ticket is finished. A copy made elsewhere (a note, a photo) would
  undo that, so the app tells technicians not to.
- **The ticket links the customer to the key** while the job is open, in the
  same field and under the same access as the customer's passwords.
- **A mistyped code is no longer caught before copying.** Copying the code
  into a ticket field is usually by copy and paste or careful typing; the
  check symbols still catch most typos at restore. If the stored code is
  wrong, the backup can't be restored and must be made again from the source
  PC. Recommendation: open the backup's **Check backup** with the stored code
  while the source PC is still available.
- **Fewer lost codes.** A field in the ticket is harder to lose than a paper
  tag, and the person doing the restore can find it.
