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
  src/main.rs                 starts the app (ui::run)
  src/net.rs                  HTTP (reqwest, cookie jar) and the WebSocket, on a tokio thread; vesper-protocol types
  src/store.rs                the stored session: the OS keyring behind the SessionStore trait
  src/state.rs                the reducer for server messages (a copy of web state.ts, peers.ts, chat.svelte.ts)
  src/model.rs                what the app shows and what it asks the network for; no GPUI, so tests drive it
  src/ui/                     one GPUI view that owns the model: sign in, sidebar, room view, composer, theme
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
| D1 | Server address and sign in, signed-in state kept across restarts, sign out | Done 2026-11-14. Tests sign in against a real server, a bad password shows the server's error |
| D2 | Rooms and DMs in a sidebar, a room's history, live new messages, sending, unread counts | Done 2026-11-14. Tests: two clients, one sends, the other sees it |
| D3 | Voice: join and leave, hear others, be heard, mute and deafen, per-person volume, who is speaking | tests: two engines exchange Opus through a real SFU |
| D4 | DeepFilterNet on the mic, global push to talk, tray icon, start with the computer | engine tests with a fake hotkey and recorded audio |
| D5 | Watch a screen share (VP8 decode), share notices | tests decode the SFU's forwarded VP8 |
| D6 | Packages: Linux first, then Windows and macOS through CI | CI builds them; running them is not verified here |
| Later | Uploads, reactions, edits, search, settings parity, sharing a screen, echo cancellation | |

## D1 and D2: what was built and how it was checked

- **Network.** `net.rs` runs one tokio runtime on its own thread. The UI sends `Cmd`s and gets `Event`s back over channels;
  GPUI applies them on the UI thread (`cx.spawn_in`). Each sign in gets a fresh reqwest cookie jar. Every request and the
  `/ws` upgrade send `Origin` equal to the server's origin. The address typed is reduced to its origin (lowercase host, no
  default port, no path, `https://` when no scheme is given), so it matches `VESPER_ORIGIN` byte for byte.
- **Errors.** API error codes become the web app's texts (`web/src/lib/api.ts`). A 401 at sign in is "Wrong username or
  password.", a 403 at sign in means the Origin check failed (the address is not the one the server is set up with), and
  no answer is "Could not reach the server".
- **Stored session.** The cookie and the server origin go to the OS keyring (`keyring` 4: Secret Service on Linux, Keychain,
  Credential Manager), behind the `SessionStore` trait, and never into a file or a log line (`Saved` and the password print
  `***` in `Debug`). With no keyring, sign in works, nothing is remembered, and the app says so once. At start a stored
  cookie is tried with `GET /api/me`: a 401 forgets it and shows sign in with the address filled in; no answer keeps it.
  Sign out calls `POST /api/logout` and forgets the cookie. A 401 on any request, or the socket closing with 4001, goes back
  to sign in and forgets it; 4003 says a moderator disconnected you and does not reconnect.
- **Secure cookies.** The server sets `Secure` and the `__Host-` name only for an https origin. The tests run plain http on
  127.0.0.1, so their cookie has no `Secure` flag; nothing in the client relaxes it. A cookie restored from the keyring for
  an https server is put back as `Secure`, and a unit test checks the jar does not send it over http.
- **Socket.** After `hello` the state is kept current from server messages the way `state.ts` and `chat.svelte.ts` do it:
  rooms and DMs (DMs named after the other person and sorted by unread, newest message, then name; each DM's newest message
  is fetched once after `hello`, like `seedActivity`), users, unread and mention counts, presence, new messages, edits and
  deletes (a copy with a smaller `updated_at` is dropped). A drop reconnects after the web app's backoff (random 50 to 100%
  of 2 s doubling to 30 s, reset after 10 s up), asking `/api/me` first. Every `hello` drops the other rooms' history and
  fetches the shown room's newest page again, merged into what is held, like `resync()`.
- **Room view.** The newest 50 messages, then 50 older ones each time the list is scrolled to the top. Author, time, body as
  plain text, a reply's author and excerpt, attachment names. The shown room is marked read with `PUT /api/rooms/{id}/read`
  when its newest message is on screen and the window has focus.
- **Composer.** Enter sends, Shift+Enter makes a new line. A failed send shows why and puts the text back.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display):

- The reducer, the model and the list updates: unit tests (`state.rs`, `model.rs`, `ui/mod.rs`, `net.rs`).
- Against a real `vesper-server` started in the test (`desktop/tests/server.rs`, plain http): sign in with a good and a bad
  password, rate limited after repeated bad passwords, an address that does not answer, the stored session used at the next
  start and forgotten at sign out (the old cookie then gets 401), a stale stored cookie, no keyring (a fake store), two
  clients where one sends and the other gets it live with its unread count rising and then marked read, an older page, a
  socket the server drops (the missed message comes back from the refetch), and a session revoked from another device.
- GPUI's headless test window (`desktop/src/ui/view_tests.rs`): the sign in view and the room view with fake data, a click
  on Sign in sending the login, Enter and Shift+Enter in the composer, a click on another room.

Not verified:

- Anything on a screen: how the app looks, fonts, scrolling feel, focus and Tab order, window activation.
- A real keyring (Secret Service, Keychain, Credential Manager): only the fake store ran. The dev machine has no Secret
  Service on its session bus (`busctl --user list`, 2026-11-14), so a real run there would take the "not remembered" path.
- https and `wss://` against a real server, Caddy or Cloudflare; the TLS setup (rustls with ring and the system's roots)
  was not run.
- Windows and macOS builds.

Left for later on purpose: typing indicators, day separators and grouping, Markdown, previews, the build change check of
the web app (a message kind this build does not know is skipped instead), and remembering the server address after sign
out (it is stored only with the session).

## Not in v2.0

Signup, invites and passkeys (the web app does them), admin pages, phones, webcams.

## Open questions for Toby

- Which systems do the friends use? Linux is the only one that can be built and tested here.
- Should the desktop app ever replace the web app, or always sit beside it?
