# Rust style

Only rules that a tool or a test checks. Run `scripts/check.sh` to check them all.

- **Format:** `cargo fmt --all`. `scripts/check.sh fmt` fails on any diff.
- **Warnings are errors:** `cargo clippy --workspace --all-targets -- -D warnings`.
- **No `unsafe`.** `unsafe_code = "forbid"` in `[workspace.lints.rust]`, and every crate sets `[lints] workspace = true`.
  Each crate also has `#![forbid(unsafe_code)]`.
- **Leftover debugging:** `dbg!`, `todo!` and `unimplemented!` warn in clippy, so `-D warnings` rejects them.
  Plain `println!` is allowed on purpose in `vesper-loadtest` and the server `main`.
- **Locks and invariants:** use `expect("<what> lock poisoned")` or another message that names the invariant, not a bare `unwrap()`.
  This is a convention, not checked by a tool. Existing examples: `voice.rs` and the socket timeout in `vesper-sfu/src/room.rs`.
- **Dependencies:** a crate used by two or more members goes in the root `[workspace.dependencies]`. Members add their own features.
- **Shortcuts:** a known limit gets a `// shortcut: <limit>, <when to fix>` comment (see `config.rs`).
- **Wire types** live in `vesper-protocol`. `scripts/check.sh bindings` fails if `web/src/lib/gen` is out of date.
- **Tests:** run by nextest (`.config/nextest.toml`). A test that waits on a deadline has a deadline in the config, not a retry.
  Debug assertions are off for `tract-core` only (`Cargo.toml`).
