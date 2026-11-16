# 0011. The desktop app sends its screen as VP8 from a pure Rust encoder

Status: accepted, 2026-11-15. Builds on [ADR 0009](0009-video-in-the-sfu.md) (one VP8 screen per person) and
[ADR 0010](0010-gpui-desktop-client.md) (the GPUI desktop client).

Context: The desktop app could watch shares but not send one. Toby's friends use Windows, so Windows matters most, and Linux must
work under Wayland and X11. The SFU forwards VP8 only, one layer, and every viewer (web or desktop) decodes VP8. `unsafe` is
forbidden in our crates and the desktop binary may not take GPL code.

Decision:

- **Capture, one backend per system** (`desktop/src/voice/capture.rs`):
  - Windows: Windows Graphics Capture through `windows-capture` (MIT), the screen or window chosen in the share dialog (its
    list from the crate's monitor and window enumeration), else the primary monitor, with the cursor, BGRA.
  - Linux under Wayland: the xdg-desktop-portal ScreenCast through `ashpd` (MIT, already in GPUI's tree). Its picker chooses the
    screen or window. Then a PipeWire stream through `pipewire` 0.9 (MIT), with the cursor embedded.
  - Linux under X11: `GetImage` through `x11rb` (MIT or Apache-2.0, already in GPUI's tree) of the monitor (RandR) or window
    (`_NET_CLIENT_LIST`) chosen in the share dialog, else the primary monitor, polled at the frame rate. No cursor.
  - Not `zed-scap`: its Wayland side needs `pipewire` 0.8, which does not build against PipeWire 1.6 headers (Arch), and its
    `links` clash keeps `pipewire` 0.9 out of the same lockfile. GPUI's optional `scap` dependency is dropped in `desktop/vendor/gpui`
    for the same reason.
- **Encoder: `vp8-encoder`** (pure Rust, `(MIT OR Apache-2.0) AND BSD-3-Clause`), the same team as the `vp8-decoder` the app watches
  with. VP8 is what the SFU forwards and what the web sends. No system library, no build step on Windows.
- **Threads.** Capture runs on `vesper-capture` and keeps only the newest picture. `vesper-encode` converts it to I420 (BT.601 limited
  range), scales it down when it is bigger than the preference allows, and encodes at the share's frame rate and bitrate. The call
  thread (`engine.rs`) only takes the encoded frames and writes them to str0m. Nothing of it runs on the UI thread. A slow encode
  costs frames, never voice.
- **Peer connection.** The join offer's video m-line is send-only, as the web's. Starting a share needs no renegotiation: the
  engine opens the screen, the model sends `share_start` once it is open (so a cancelled picker sends nothing), and the encoder runs
  only while the server's `watchers` count is above 0, as the web turns its encoder off. A viewer's PLI comes through the SFU and makes
  the next frame a keyframe; a still screen's last picture is encoded again for it.
- **Quality options are the web's**: 30 or 60 fps, 0.5 to 30 Mbps (8 Mbps until set: the web's suggestion without a measured upload),
  and "Keep text sharp" (up to 1920x1080 worth of pixels) or "Keep motion smooth" (up to 1280x720). The server's answers are the web's
  too: a lowered bitrate is used and said, a refusal puts the old settings back, a cut or a moderator's stop ends the share.

Why:

- One encoder in pure Rust builds the same on Windows and Linux in CI, with no libvpx to ship or link.
- The capture crates are thin, maintained wrappers of each system's own API, and the two Linux ones were already in the tree.

Consequences:

- The encoder is single threaded with no SIMD. Its own figures: about 47 fps at 720p on one Ryzen 7 3800X core, so a 1080p share runs at
  roughly 20 fps. "Keep motion smooth" exists for that. A faster encoder (libvpx, or the GPU's) is the upgrade if the group finds 1080p
  too slow.
- The encoder comes in under its target on flat content (its quantizer has a floor), so the figures line can show less than the
  bitrate chosen.
- Linux builds need the PipeWire headers and libclang (for `pipewire-sys`'s bindgen), listed in [dev.md](../dev.md) and CI.
- On Windows and X11 the share dialog lists the screens and windows by name (no thumbnails); the last choice is kept until the
  app quits. No share audio, as on the web.

Verified: on Linux, in a debug build. `desktop/tests/share.rs` (`a_desktop_client_shares_its_screen_with_another`) sends the test pattern
from one desktop client through the real encoder, engine, server and SFU to another, which decodes the four colour bars. The parity
harness's `share-send` scene (`web/e2e/parity.mjs`) has the desktop app in a headless sway share the pattern (`VESPER_DEV_SHARE`) and
Chromium watch it. Not verified: the Wayland portal and PipeWire path (no ScreenCast portal backend on the dev machine), X11 capture,
and the Windows capture, which CI only compiles. Not verified on a real screen, across a real network, or for how fast a 1080p share
runs in a release build.
