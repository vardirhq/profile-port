# Desktop UI

The current desktop app is an early Slint interface connected to the transfer
engine. It has not been visually or interactively reviewed on Windows clients.
The checked-in dashboard screenshots show an earlier sample-data prototype,
not the current screens. For operator instructions see the
[desktop guide](../guides/desktop.md); for the component and test boundaries see
[architecture](../development/architecture.md).

## Current screens and behavior

- **Back up:** choose a discovered profile or type its path, enter an existing
  drive root and ticket, prepare a package, then record and type back the code.
  Canceling at confirmation leaves an empty interrupted package. The screen
  reports progress and export details.
- **Verify or restore:** enter a drive root and find packages, or enter a
  package folder directly; enter the code; optionally resume an incomplete
  backup, verify stored content, or restore to an existing destination. Cloud
  acknowledgment is required for detected cloud-like target paths.
- The status area lists relevant results and problems, but is not an
  exportable report. Progress shows files, bytes and a current path, not a
  total percentage, speed, or ETA. The app has no active-operation cancel.

Current limitations include manual drive/folder entry, no visual folder
picker, no category selection, no size estimate or conflict preview, no
program inventory, no user sign-out detection, and no USB-management flow.
The profile list is from the running system and is not an offline-Windows
installation browser. Verification of stored files does not prove that all
source data was captured.

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
