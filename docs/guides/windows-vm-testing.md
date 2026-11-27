# Windows VM test plan

A Windows 10 or 11 client VM can test the desktop app on Windows before trying
physical PCs. A VM adds useful Windows-client coverage; it does not replace
real-device, real-USB and customer-workflow testing. No VM execution is claimed
by this document. Record the exact build and what you actually observed.

## Setup

- Use a disposable Windows client VM with sufficient disk space and an
  up-to-date ProfilePort Windows build. Record Windows edition, build, VM
  platform and ProfilePort commit or artifact run.
- Use two VMs, or two separate disposable local accounts and directories in one
  VM. A second VM better represents a different PC. Creating a second user alone
  does not test moving a drive between computers.
- Attach a separate disposable virtual disk, format and mount it in Windows,
  and note its drive letter. Check its contents before testing. If using USB
  passthrough, dedicate a spare physical USB drive and follow your VM
  platform's instructions for safely passing it through and reconnecting it.
- Take a VM snapshot before testing. A VM snapshot is a recovery aid, not proof
  of backup integrity; do not restore a snapshot in the middle of a test unless
  that is the scenario being tested. If the transfer disk is external to the
  snapshot, record its independent state.
- Populate the source account with clearly fictional data. Do not use customer
  files, personal accounts, recovery keys, or production tickets. Keep the
  original fixture separately so it can be compared after restoring.

Example fixture: `Documents\report.txt`, `Pictures\image.png`, a nested
folder with a Unicode name, a large disposable file, a hidden file and a
read-only file. Include an empty folder and a file whose content differs at the
destination. For a locked-file scenario, use a test program that opens a
fixture in a way that denies reads, and keep it open through export.

## Record every run

For each scenario record: source and destination account/path, transfer drive
path and filesystem, whether the source account was signed in, the operations
performed, the result text and relevant non-sensitive details, expected versus
actual contents, and any problem or error. Never include transfer codes or
sensitive filenames in shared logs, screenshots, issues or tickets.

The desktop app currently displays reports in its window and does not export a
report. Capture non-sensitive evidence deliberately, checking it before sharing.

## Core scenarios

1. **Profile discovery and export.** Start the desktop app. Choose the source
   profile or enter the fixture path. Enter the transfer drive root and a
   non-sensitive ticket such as `VM-001`. Prepare the backup, write down the
   code securely, type it back and run it. Confirm the displayed package path
   is on the test drive. Record every skipped, excluded or changed item.
2. **Verification.** On **Verify or restore**, find the backup on the drive,
   enter the code and choose **Verify backup**. Record the number verified and
   any failure. A clean verification does not prove the source was complete;
   compare the export report with the fixture inventory.
3. **Restore on a second VM or account.** Safely detach the virtual drive and
   attach it to the destination VM if using two. Select the package and an
   existing destination folder. Restore; compare contents and folder placement
   with the fixture, including Unicode names and metadata where applicable.
   Check that the destination account can actually open the files.
4. **Conflict and repeat restore.** Put a different file at the intended
   destination path before restoring. Confirm that the default desktop policy
   leaves it untouched and reports the conflict. Re-run the restore and check
   that already-identical files are counted without overwriting anything.
5. **Wrong and missing code.** Enter an incorrect code and verify that opening,
   verification and restore fail without modifying the destination. Test
   cancelling at the code confirmation screen: the empty package should be
   removed from the drive, and nothing else in `ProfilePort/transfers` touched.
6. **Locked or inaccessible file.** Keep the disposable fixture locked or deny
   read access using an appropriate test account. Confirm that the actual
   report distinguishes skipped/failed data, and that a successful verification
   of stored files does not imply the skipped file was included. Access
   behavior depends on the account and permissions: document the setup.
7. **Interrupted export.** Start a sufficiently large backup. Detach the
   disposable *transfer* disk using the VM platform's supported controls while
   copying, then reattach it. Record whether the operation reports failure or
   stops and whether any OS cache affects the observed result. Use the saved
   code and **Resume backup** with the source at its original path, then verify
   and restore. Repeat at different stages. Do not simulate removal on a disk
   containing anything valuable.
8. **Cloud destination.** If using a dedicated test cloud account, test a
   detected synced path with and without the acknowledgment, and record whether
   files actually upload. Path-based detection is incomplete: a missing warning
   does not establish that a folder is local. Do not use a real customer cloud
   account.

9. **Wipe.** After a restore, select the backup and choose **Wipe backup…**.
   Type a wrong ticket number: nothing should change. Type the right one and
   wipe: the package folder should be gone, other packages and the restored
   files untouched. Repeat with the overwrite option on a larger backup and
   record how long it takes. Then detach the transfer disk in the middle of
   an overwrite wipe, reattach it, and check that **Find backups** lists the
   package as partly wiped, that the code no longer opens it, and that wiping
   it again (typing `WIPE`) finishes. This doesn't show what a physical flash
   drive keeps internally; that can't be checked from Windows.

For each scenario, write PASS only for observed checks, FAIL for an unexpected
result, and NOT TESTED when the environment does not support it. The included
`scripts/windows-smoke.ps1` covers CLI scenarios, not GUI interaction; it uses
and deletes its `pp-smoke` directory under `RUNNER_TEMP` or the system temp
directory. Inspect the script before running it in any environment.

## Limits and follow-up

Also test physical USB removal, different Windows client versions, OneDrive
placeholders, BitLocker, and ARM64 Windows separately when hardware and safe
test accounts are available. A virtual disk detach cannot fully reproduce a
USB controller, cable, device write cache, power failure or real-world drive
performance. Verify the desktop layout at common Windows display scaling
levels before treating it as finished.
