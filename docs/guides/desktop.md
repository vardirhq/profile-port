# Desktop transfer guide

ProfilePort's desktop app connects to the transfer engine, but it is an **early
implementation**. Its screens have been rendered and clicked through on Linux
(an X11 virtual display), not on Windows 10 or 11. Use disposable test data and
a spare drive until that work is complete. Keep the original data and an
independent backup.

Everything is picked from lists. The only things you type are the **ticket
number** when backing up and the **transfer code** when restoring.

The app has four pages: **Home** (the three tasks, the connected drive with its
free space, the backups on it, and a banner if one was interrupted), **Back
up**, **Restore** and **USB drive**.

- **Profiles** come from the running system: on Windows its registry's
  profile list, with the signed-in user first. **Other folder…** opens a
  folder dialog for anything not listed. An attached offline Windows
  installation is not discovered, and offline hive loading is not implemented.
- **Drives** are detected and refreshed every two seconds, so a USB drive
  appears shortly after it is plugged in. On Windows the list holds removable
  and fixed drives other than the system drive. External hard drives usually
  count as fixed. Network and optical drives are left out. Each entry shows
  free space and how many backups it holds.
- **Backups** on the chosen drive are listed by ticket number, date (UTC), size
  and state.

The app does not choose individual data categories, estimate the size of a
backup, or create an installed-program inventory.

## Before you begin

1. Make sure the transfer drive has enough free space. The drive list shows
   free space, but the app does not calculate the size of the backup.
2. Close applications that may change the files. The engine reads the live
   filesystem; it does not use Volume Shadow Copy (VSS), and it does not detect
   signed-in users or offer to sign them out.
3. Check which account is running ProfilePort and whether it can read the
   source and write the destination. The planned automatic elevation and backup
   privileges are not implemented, so another user's private files may be
   skipped as "access denied".
4. Obtain `ProfilePort.exe` from a trusted build and copy it onto the
   transfer drive; it needs no installation, no network and no Visual C++
   Redistributable, and writes no settings to the PC. It is not code-signed:
   if Windows SmartScreen warns, unblock the file once (Properties → Unblock)
   before copying it. The Windows CI artifact also contains
   `profileport-cli.exe`. See [building and testing](../development/building-and-testing.md).

For a controlled rehearsal, follow the [Windows VM guide](windows-vm-testing.md).

## Back up

1. Open **Back up**. Under **Whose files?**, pick the profile (or **Other
   folder…**).
2. Under **Back up to**, pick the drive. If none is listed, plug it in; it
   appears by itself.
3. Type the **ticket number**: 1–32 letters, digits, `_` or `-`. It is the only
   readable label on the drive, so never use a name; the field turns red if it
   isn't valid.
4. Choose **Create backup**. The app creates an empty backup and shows the
   **transfer code** *before* copying anything. Store it in the ticket's
   **sensitive-data field**, never in ordinary notes, and don't photograph it.
   Without the code the backup can't be opened or continued, and it is not
   stored on the drive. The code screen replaces the form until you start or
   cancel. See [security](../development/security.md).
5. Tick **I've stored the code in the ticket and checked it**, then choose
   **Start backup**. Progress shows files, size and the current path; there is
   no percentage, time estimate or pause button.
6. Review the result. A completed backup has not been verified: while the old
   PC is still here, choose **Check backup now** (it opens **Restore** with the
   new backup selected), enter the code *from the ticket* and choose **Check
   backup**. That proves the stored code is
   right and every stored file reads back. Skipped files, files changed during
   copying, exclusions and links are listed or counted in the result.

**Cancel** at the code step removes the empty backup from the drive; nothing
has been copied at that point. If the app or computer stops after copying
begins, keep the drive and code and continue the backup (below).

The source profile is copied with default cache/temp exclusions. Known folders
redirected outside the profile are included. Links are recorded, not followed.
See [coverage and exclusions](coverage.md).

## Check or continue a backup

1. Open **Restore** and pick the drive and the backup.
2. Enter the transfer code from the ticket. The field says when the code is
   well formed; whether it belongs to this backup is known only when used.
3. **Check backup** decrypts every stored file and compares it with its
   recorded hash. It does not prove that every file on the old PC was copied,
   or that files changed during copying are consistent; read the backup result
   for that.
4. An interrupted backup is marked *interrupted* and offers **Continue
   backup**. Run it on the old PC with the profile in the same place.

The app keeps the result on screen only; a durable, exportable report is not
provided. Record results by your organization's rules, without codes or
customer file names in ordinary notes.

## Restore

1. On **Restore**, pick the drive and the backup, and enter its code.
2. Under **Restore to**, pick the user's profile on this PC (or **Other
   folder…**). The user must have signed in once, so their profile exists.
3. If the destination has folders synced to OneDrive or similar, a warning
   lists them and **Restore files** stays disabled until you tick **Restore
   into them anyway** (the files will upload). Detection is path-based and
   incomplete; no warning doesn't prove a folder is local.
4. Choose **Restore files**. A different existing file is left in place and
   reported; identical files count as already present. Restored files are read
   back and compared; filesystem caches are not bypassed.
5. Review failures, items missing from the backup, conflicts, renamed files
   and links not restored. Check the destination folders and compare
   representative files before accepting the job.

The app offers no conflict preview or the CLI's `--keep-both`, does not
migrate application settings with specialized adapters, and does not merge
registry hives.

## Wipe the backup from the drive

When the restore is checked and the customer has checked their files:

1. After a fully successful restore, the result offers **Wipe backup from
   drive…**. Otherwise, select the backup on **Restore** or **USB drive** and
   choose **Wipe from drive…** (or **Wipe Ticket …**). No code is needed.
2. Read the question, optionally tick **Also overwrite all backup data first**
   (takes about as long as the backup did), and choose **Wipe backup**.
   **Cancel** changes nothing.
3. The backup's key is destroyed first, then its files are deleted, and the
   list updates.
4. Set the ticket to finished when the job is done; that clears the code
   from its sensitive-data field.

A wipe can't be undone. It refuses a folder holding anything other than backup
files. On flash drives it can't guarantee that no old blocks remain inside the
drive; those are encrypted and need the code, which is why the code must not
outlive the job. See
[security](../development/security.md#wiping-backups-from-the-drive). A backup
interrupted while being wiped is listed as *partly wiped*; choose **Finish
wiping**. Backups 14 or more days old are marked with their age, and Home
counts them.

**Prepare for next customer** (Home or **USB drive**) wipes every backup on
the drive after one confirmation that lists their ticket numbers. Damaged
backups and anything that isn't a backup are left alone and reported.

## When something goes wrong

See [troubleshooting](troubleshooting.md). Do not delete the original data or
wipe the backup merely because the app reports success. The transfer code is
required for recovery; there is no code reset or recovery.
