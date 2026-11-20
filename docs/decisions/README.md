# Architecture decision records

Short records of the decisions that shape ProfilePort. Each one states the
problem, the options considered, the choice, and what it costs us.

Status values: **Proposed** (recommended, awaiting sign-off), **Accepted**,
**Superseded**.

| # | Decision | Status |
|---|----------|--------|
| [0001](0001-gui-framework.md) | GUI framework: Slint, not Tauri | Accepted |
| [0002](0002-package-format.md) | Transfer package format: encrypted chunked store | Accepted |
| [0003](0003-copy-engine.md) | Copy engine: native Rust, not Robocopy | Accepted |
| [0004](0004-privilege-model.md) | Always run elevated | Accepted |
| [0005](0005-locked-files-and-registry.md) | Locked files: avoid in Phase 1, VSS in Phase 2; registry archived, not merged | Accepted |

The order matters: 0002 decides most of 0003, and 0003 and 0005 both depend on 0004.

Related: [Windows gotchas](../windows-gotchas.md) — platform behaviours the
engine must handle regardless of these decisions.
