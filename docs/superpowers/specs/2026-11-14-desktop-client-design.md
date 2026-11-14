# Vesper v2: the desktop client (GPUI)

Status: in progress. Started 2026-11-14. Items marked **[Toby]** are Toby's decisions; items marked **[call]** are the author's
calls and are open to veto. Decision record: [ADR 0010](../../decisions/0010-gpui-desktop-client.md).

## Goal

A native desktop Vesper, written in Rust with GPUI **[Toby, 2026-11-14]**. It talks to the same server as the web app. What it
gives friends that a browser cannot: push to talk while a game has focus, voice that does not depend on a browser tab, and a
lighter app that starts with the computer.

The web app stays. It is still how people sign up, use passkeys, and join from a computer without the app **[call]**.

## What v2 means

v2.0.0 is the first release with a desktop client that can sign in, chat and talk. The server and web app keep shipping 1.x
fixes until then. Nothing in this plan changes a wire message in a way an older web tab cannot read **[call]**; if a stage needs
one, it says so and that stage is the MAJOR bump.

## Shape

```text
desktop/                      its own Cargo workspace and lockfile (GPUI's tree never moves the server's pins)
  src/main.rs                 window, theme, routing between screens
  src/net/                    HTTP (reqwest, cookie jar) and the WebSocket, on a tokio thread; vesper-protocol types
  src/state/                  GPUI entities fed by net events: session, rooms, messages, peers, voice
  src/ui/                     screens and views (gpui-component widgets)
  src/voice/                  call engine: str0m client, Opus, audio in and out, DeepFilterNet, push to talk
crates/vesper-protocol        shared wire types (path dependency)
```

- **GPUI 0.2.2 from crates.io and gpui-component 0.5.1** **[call]**. Newer gpui-component releases follow a GPUI fork
  (`gpui-pre`). Both pinned exactly. The lockfile pins `libc` 0.2.189 and `mio` 1.2.4, because `xattr` 0.2.3 (through
  GPUI's HTTP client) needs `libc::ENOATTR`, which libc 0.2.190 removed.
- **One tokio runtime thread for the network**, talking to GPUI through channels; GPUI owns all state and the UI thread.
- **Auth like the web app**: `POST /api/login` with a password, the session cookie kept by the client. Requests send
  `Origin: <server origin>` so the server's Origin check passes, as a browser would. The cookie is stored in the OS
  keyring so a restart stays signed in, and never logged **[call]**. Passkeys: not in the first version.
- **Voice**: a str0m client (the same library as the server's SFU), Opus encode and decode, cpal for the microphone and
  speakers, DeepFilterNet through `vesper-dsp` (native tract, as the audio lab already does), and the same capture order as
  the web: mic, echo cancellation, gain, DeepFilterNet, Opus. Echo cancellation needs a native library (WebRTC's audio
  processing) and comes after the first working call; until then the app says headphones are needed **[call]**.
- **Push to talk** with a global hotkey that works while another app has focus.

## Stages

Each stage lands as commits on `v2/desktop`, passes `scripts/check-desktop.sh`, and says what was verified and how.
This machine has no display, so UI work is verified with GPUI's headless test context and the network and voice layers
against a real `vesper-server` started inside the tests. Anything that needs a screen, a microphone or speakers is
"not verified" until Toby runs it.

| Stage | Scope | Done when |
| --- | --- | --- |
| D0 | Crate, GPUI window, pins, a check script and a CI job | `cargo build` and the check pass |
| D1 | Server address and sign in, signed-in state kept across restarts, sign out | tests sign in against a real server, a bad password shows the server's error |
| D2 | Rooms and DMs in a sidebar, a room's history, live new messages, sending, unread counts | tests: two clients, one sends, the other sees it |
| D3 | Voice: join and leave, hear others, be heard, mute and deafen, per-person volume, who is speaking | tests: two engines exchange Opus through a real SFU |
| D4 | DeepFilterNet on the mic, global push to talk, tray icon, start with the computer | engine tests with a fake hotkey and recorded audio |
| D5 | Watch a screen share (VP8 decode), share notices | tests decode the SFU's forwarded VP8 |
| D6 | Packages: Linux first, then Windows and macOS through CI | CI builds them; running them is not verified here |
| Later | Uploads, reactions, edits, search, settings parity, sharing a screen, echo cancellation | |

## Not in v2.0

Signup, invites and passkeys (the web app does them), admin pages, phones, webcams.

## Open questions for Toby

- Which systems do the friends use? Linux is the only one that can be built and tested here.
- Should the desktop app ever replace the web app, or always sit beside it?
