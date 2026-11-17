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
| [0009](0009-video-in-the-sfu.md) | The SFU forwards one VP8 screen per person | accepted |
| [0010](0010-gpui-desktop-client.md) | A native desktop client in GPUI, beside the web app | accepted |
| [0011](0011-desktop-screen-share-send.md) | The desktop app sends its screen as VP8 from a pure Rust encoder | accepted, encoder superseded by 0012 |
| [0012](0012-desktop-share-libvpx.md) | The desktop app encodes its screen with libvpx, linked statically | accepted |

The design itself is in [the spec](../superpowers/specs/2026-10-30-vesper-design.md).
