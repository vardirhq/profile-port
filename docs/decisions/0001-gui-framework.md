# 0001 — GUI framework: Slint, not Tauri

**Status:** Accepted

## Context

ProfilePort runs from a USB stick on whatever Windows PC is in front of the
technician. Those are disproportionately the machines in bad shape: stripped,
unpatched, offline, LTSC/IoT editions, or PCs where policy has removed
components. The UI layer must not add a runtime dependency that can be
missing at launch.

## Options

**Tauri (WebView2).** Fast UI development with HTML/CSS and a huge component
ecosystem. WebView2 ships with Windows 11 and has been pushed to Windows 10
through Windows Update since about 2021, so it is present on *most* machines.
A fixed-version runtime can be bundled for true portability.

**Slint.** Rust-native declarative UI that renders itself. Single executable,
no runtime dependency.

## Decision

Slint.

The common objection to Tauri ("WebView2 isn't everywhere") is weaker than it
sounds. The stronger reasons are:

1. **The edge cases are our core market.** Repair and recovery PCs are exactly
   where WebView2 is most likely to be missing, broken or blocked.
2. **Footprint on the customer's PC.** WebView2 creates a user-data folder
   under `%LOCALAPPDATA%` and spawns `msedgewebview2.exe` processes. It can be
   redirected, but it is one more thing that could leave traces in a tool whose
   promise is to leave none.
3. **WinPE.** WebView2 does not run in WinPE. If we later boot ProfilePort from
   a recovery stick, only a self-rendering UI has a chance. *Slint under WinPE
   is unverified — prototype before relying on it.*
4. **Bundling the fixed WebView2 runtime** adds roughly 150–200 MB to the USB
   payload and must be kept patched by us.
5. **One small binary** is easy to verify, sign and reason about.

## Consequences

- Slower UI development; fewer ready-made widgets. Polished tables, rich
  progress views and theming are built by hand (see
  [`docs/design/ui.md`](../design/ui.md)).
- Screen-reader support on Windows (via AccessKit) is less mature than a
  browser engine's. Test with Narrator early.
- **Licensing:** Slint is available under GPLv3, a royalty-free licence, or a
  commercial licence. ProfilePort uses the **royalty-free licence**: it
  permits closed-source desktop apps at no cost, provided we show
  attribution (an "About" screen entry with Slint's badge/notice). This
  fits an in-store tool. **Revisit before selling or distributing
  ProfilePort to other businesses.**
- The engine must stay GUI-independent (a requirement anyway) so a CLI and,
  later, a WinPE front-end can share it.
