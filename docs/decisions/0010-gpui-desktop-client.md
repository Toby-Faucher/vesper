# 0010. A native desktop client in GPUI, beside the web app

Status: accepted, 2026-11-14. Design: [the desktop client spec](../superpowers/specs/2026-11-14-desktop-client-design.md).

Context: Toby wants v2 to be a desktop app written with GPUI (Zed's Rust UI framework). The web app does everything today, but a
browser cannot take a push-to-talk key while a game has focus, and calls depend on a tab staying alive. A Tauri shell around the
web app was the cheaper option; Toby chose a native GPUI client.

Decision:

- **A new client in `desktop/`, with its own Cargo workspace and lockfile.** It depends on `crates/vesper-protocol` by path for the
  wire types. The server's workspace and lockfile do not change, so its pins (tract, sqlx, time) stay where they are.
- **GPUI 0.2.2 and gpui-component 0.5.1 from crates.io, pinned exactly.** Later gpui-component releases depend on a GPUI fork.
- **The web app stays** for signup, invites, passkeys, admin and anyone without the app.
- **Same server, same rules**: password sign in, the session cookie, `Origin` set to the server's origin, the same WebSocket and
  the same SFU.
- **No `unsafe` in Vesper's own code**, as in every other crate (`unsafe_code = "forbid"`); dependencies may use it.

Why:

- A separate workspace keeps a fast-moving GUI dependency tree from touching the server, which already holds careful pins.
- Sharing `vesper-protocol` means a wire change is a compile error in the client, not a runtime surprise.
- str0m on both sides keeps one WebRTC implementation to understand.

Costs:

- The browser gave WebRTC, echo cancellation, screen capture and codecs for free; the client must build each one.
- Two clients to keep in step. The desktop app starts with a subset and grows.
- GPUI is pre-1.0 and changes; the exact pins hold it still until an upgrade is chosen on purpose.
- `libc` is pinned at 0.2.189 and `mio` at 1.2.4 in the desktop lockfile, because GPUI's HTTP stack pulls `xattr` 0.2.3, which does
  not build against libc 0.2.190 or later. Remove the pins when GPUI drops that dependency.
