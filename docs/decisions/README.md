# Architecture decision records

Short records of the decisions that shape ProfilePort. Each one states the
problem, the options considered, the choice, and what it costs us.

Status values: **Proposed** (recommended, awaiting sign-off), **Accepted**,
**Superseded**.

| # | Decision | Status |
|---|----------|--------|
| [0001](0001-gui-framework.md) | GUI framework: Slint, not Tauri | Accepted |
| [0002](0002-package-format.md) | Transfer package format: encrypted chunked store | Proposed |
| [0003](0003-copy-engine.md) | Copy engine: native Rust, not Robocopy | Proposed |
| [0004](0004-privilege-model.md) | Always run elevated | Proposed |
| [0005](0005-locked-files-and-registry.md) | Locked files via VSS; user registry archived, not merged | Proposed |

The order matters: 0002 decides most of 0003, and 0003 and 0005 both depend on 0004.

Related: [Windows gotchas](../windows-gotchas.md) — platform behaviours the
engine must handle regardless of these decisions.
