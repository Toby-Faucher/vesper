# Decisions

One short record per decision that is costly to undo. Each has a status, the context, what was decided, why, and what it costs.
To change a decision, add a new record that supersedes the old one and set the old status to "superseded by NNNN". Do not rewrite history.

| # | Decision | Status |
|---|---|---|
| [0001](0001-str0m-sfu.md) | A Rust SFU on str0m | accepted |
| [0002](0002-deepfilternet-wasm.md) | DeepFilterNet runs in the browser as WASM | accepted |
| [0003](0003-shared-join-key.md) | One shared join key until accounts exist | superseded by 0007 |
| [0004](0004-no-csp-yet.md) | No Content-Security-Policy yet | superseded by 0008 |
| [0005](0005-sqlite-sqlx.md) | SQLite with sqlx for accounts and text chat | accepted, built in 2026-11-01 plan |
| [0006](0006-quality-checkpoint-gate.md) | Later plans wait for the manual quality checkpoint | accepted |
| [0007](0007-accounts-and-sessions.md) | Accounts, invites and session cookies | accepted |
| [0008](0008-csp.md) | A strict Content-Security-Policy sent by the server | accepted |

The design itself is in [the spec](../superpowers/specs/2026-10-30-vesper-design.md).
