# Desktop transfer guide

ProfilePort's desktop app connects to the transfer engine, but it is an **early
implementation**. The interface has not yet been visually or interactively
validated on Windows 10 or 11. Use disposable test data and a spare drive until
that work is complete. Keep the original data and an independent backup.

The app runs locally. The current UI does not automatically find a USB drive,
browse for folders, choose individual data categories, estimate the size of a
backup, or create an installed-program inventory. You enter paths yourself.
On Windows, a profile list comes from the running installation's registry. An
attached offline Windows installation is not discovered as an installation;
you can enter a folder path, but offline hive loading is not implemented.

## Before you begin

1. Prepare a source folder containing test files, a separate drive or folder
   to act as the transfer drive, and an existing destination folder.
2. Make sure the transfer location has sufficient free space. The app does not
   currently calculate the space required or check it in advance.
3. Close applications that may change the files. The engine reads the live
   filesystem; it does not use Volume Shadow Copy (VSS), and it does not detect
   signed-in users or offer to sign them out.
4. Check which account is running ProfilePort and whether it can read the
   source and write the destination. The planned automatic elevation and backup
   privileges are not implemented.
5. Obtain `ProfilePort.exe` from a trusted build. The Windows CI artifact also
   contains `profileport-cli.exe`. See [building and testing](../development/building-and-testing.md).

For a controlled rehearsal, follow the [Windows VM guide](windows-vm-testing.md).

## Back up

1. Open **Back up**. Choose a listed profile or type the source folder path.
   Check the path itself; a selected profile is not proof that all its files
   or redirected folders are accessible.
2. Enter the existing drive root, for example `E:\`, and a ticket identifier.
   The ticket must contain 1–32 ASCII letters, digits, `_`, or `-`. It is
   visible in plaintext in the package and folder name; do not put a name or
   other sensitive information there.
3. Select **Prepare backup**. The program creates an empty package and shows a
   transfer code *before* it copies files. Write the code on the device tag,
   store it according to your organization's process, and type it back.
   Without the code the package cannot be opened or resumed. The code is not
   stored in the package. Do not put it in the ticket system.
4. Choose **Confirm code and start backup**. The status area shows files,
   bytes and the current path. There is no percentage, time estimate or pause
   button. Changing screens is disabled while an operation is running.
5. Review the final status and every item in its details. A completed backup
   has not automatically been verified. Default exclusions, skipped files, files
   that changed during copying and links matter even when other files succeeded.
   The desktop export summary gives counts for exclusions and links, not a list
   of their paths.

The source profile is traversed with default cache/temp exclusions. Known
folders resolved outside the profile may also be included. Individual folder
selection is not currently available. Links are recorded, not followed. See
[coverage and exclusions](coverage.md).

**Cancel** at the code confirmation step removes the empty package from the
drive; nothing has been copied at that point. If it cannot be removed, the
status area names its folder. If the app or computer stops after copying begins, retain the
package and code and use the resume procedure below. If the drive was removed,
reconnect it and confirm its path before proceeding.

## Find, verify, and resume a backup

1. Open **Verify or restore**, enter the drive root and select **Find backups**.
   The list uses plaintext ticket, status, and folder information. You can
   instead enter the full package-folder path shown after export.
2. Select the package and enter its transfer code. To resume an incomplete
   export, reconnect the original source at the paths recorded in the package,
   then choose **Resume backup**. Resume is for unfinished packages, not a way
   to update a completed one.
3. Choose **Verify backup** to decrypt stored file content and compare it with
   its stored hashes. Verification does not prove that all intended source
   files were copied, that changed files represent a consistent point in time,
   or that data has survived a physical power loss. Review the original export
   report as well as the verification result.

The app keeps status and detail text on screen; a durable, exportable report is
not currently provided. Record results using your organization's rules. Avoid
recording transfer codes or customer file names in ordinary tickets.

## Restore

1. On **Verify or restore**, select or enter the backup folder and enter its
   code. Select the destination profile or type the path to an **existing**
   destination folder. Confirm the path before starting.
2. A detected cloud-synced target requires the checkbox acknowledging that
   restored files may upload. Detection is path-based and incomplete; absence
   of a warning does not establish that a destination is local.
3. Select **Restore files**. The default desktop policy leaves a different
   existing file in place and reports a conflict. Matching existing files can
   be counted as already present. Restored files are read back and compared by
   default; the implementation does not bypass filesystem caches.
4. Review failures, source skips and changes, conflicts, renamed files,
   incomplete items, and links not restored. Check the actual destination
   folders and compare representative file contents before accepting the job.

The UI does not offer a conflict preview or the CLI's `--keep-both` option.
It does not migrate application settings using specialized adapters or merge
registry hives. A restore of files is not the same as a complete working
application migration.

## Wipe the backup from the drive

When the restore is verified and the customer has checked their files:

1. On **Verify or restore**, select the backup (or enter its folder) and
   choose **Wipe backup…**. No transfer code is needed.
2. Type the ticket number shown in the instructions. For a backup listed as
   *partly wiped*, type `WIPE`. Optionally tick **Also overwrite all backup
   data before deleting**; it takes about as long as the backup did.
3. Choose **Wipe now**. The backup's key is destroyed first, then its files
   are deleted. **Cancel** leaves everything as it was.
4. Discard the device tag with the code when the job is closed.

A wipe can't be undone. It refuses a folder that contains anything other than
package files. On flash drives it can't guarantee that no old blocks remain
inside the drive; those are encrypted and need the code, which is why the
device tag must be discarded. See
[security](../development/security.md#wiping-backups-from-the-drive). Backups 14
or more days old are marked "wipe when done" in the **Find backups** list.
Wiping every backup on a drive at once is only available in the CLI
(`profileport-cli wipe --drive E:\ --all`).

This has been built and its logic tested on Linux; the wipe screen has not
been tried on Windows.

## When something goes wrong

See [troubleshooting](troubleshooting.md). Do not delete the original data or
wipe the package merely because the UI reports success. A transfer code is required
for recovery; there is no documented code-reset or recovery mechanism.
