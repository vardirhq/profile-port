# Desktop UI

The current desktop app is an early Slint interface connected to the transfer
engine. It has not been visually or interactively reviewed on Windows clients.
The checked-in dashboard screenshots show the earlier sample-data prototype
the current screens are modelled on, not the screens themselves. For operator instructions see the
[desktop guide](../guides/desktop.md); for the component and test boundaries see
[architecture](../development/architecture.md).

## Current screens and behavior

The rule ([ADR 0007](../decisions/0007-code-in-ticket-and-no-typing.md)): the
only things typed are the ticket number at backup and the transfer code at
restore. Everything else is picked from a list or confirmed with a button.

- **Layout:** sidebar navigation (Home, Back up, Restore, USB drive) and a
  page header naming the computer and the selected drive. It follows the
  earlier dashboard prototype's visual language: cards, numbered steps,
  selectable cards for profiles and drives, backup rows with status badges,
  Lucide icons.
- **Home:** an interrupted-backup banner, three action cards, the drive card
  (free space, space bar, old-backup count, **Prepare for next customer**)
  and the first four backups on the drive.
- **Back up:** whose files, which drive, ticket number, then **Create
  backup**. A dedicated code screen ("Store this transfer code in the
  ticket") enables **Start backup** only once "I've stored the code in the
  ticket and checked it" is ticked. An invalid ticket turns its hint red and
  disables the button. The result offers **Check backup now**.
- **Restore:** backup rows, the code (live format check), destination
  profile cards, a cloud warning when relevant, then **Restore files**,
  **Check backup**, **Continue backup** (interrupted only) and **Wipe from
  drive…**. A fully successful restore offers the wipe.
- **USB drive:** drive cards, every backup, per-backup wipe and **Prepare for
  next customer**. Wipes are confirmed inline, never by typing.
- Drives are re-read every two seconds while nothing is running or being
  confirmed. **Other folder…** opens the system folder dialog (`rfd`).
- The result card shows a spinner while working, then a title and details.
  After a fully successful restore it offers the wipe. Progress shows files,
  size and the current path, not a percentage, speed or ETA. There is no
  cancel for a running job, no category selection, size estimate, conflict
  preview, program inventory or sign-out detection. Verification of stored
  files does not prove that all source data was captured.

## Design intent

The intended direction is a calm professional tool with an obvious primary
action, clear status, and honest language about incomplete or unverified data.
Avoid showing profile/computer names from a package before it is unlocked.
Do not claim the current screens have already achieved the former prototype's
layout or polish.

`ui/theme.slint` contains theme tokens and the app uses the `Label` component
from `ui/components.slint` together with Slint standard widgets. The component
file retains reusable components from the earlier prototype; their presence is
not evidence that the current screens use them. `PROFILEPORT_THEME=dark`
selects dark mode; system theme detection is not implemented.

## Visual verification

`scripts/screenshot.sh [scale]` captures both themes on Linux with Xvfb,
ImageMagick and xdotool, overwriting the existing dashboard screenshot paths.
Current content comes from the executing machine's discovered profiles; it is
not sample data. Use synthetic profiles and inspect screenshots before sharing.
The helper only captures the initial view; it does not exercise each workflow.

Review both screens and their error, progress and completion states at the
minimum supported window size and common Windows display scales. Record the
Windows version, renderer and scaling used. A Linux screenshot or successful
Slint build is not evidence that Windows interaction and layout work. See the
[VM test plan](../guides/windows-vm-testing.md).
