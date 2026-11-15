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
  the web: mic, echo cancellation, gain, DeepFilterNet, Opus. Echo cancellation is `sonora`, a pure Rust port of WebRTC's
  audio processing (AEC3), since D7; the app has no input gain yet **[call]**.
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
| D3 | Voice: join and leave, hear others, be heard, mute and deafen, per-person volume, who is speaking | Done 2026-11-14. Tests: engines exchange Opus through a real SFU and hear each other's tones |
| D4 | DeepFilterNet on the mic, global push to talk, tray icon, start with the computer, a Settings view | Done 2026-11-14. Tests: the real model takes noise out of a speech-like fixture (when the model is on disk), a slow fake falls back, a bad checksum is refused, push to talk with a fake key source |
| D5 | Watch a screen share (VP8 decode), share notices | Done 2026-11-14. Tests: a fake sharer sends VP8 through a real SFU; a desktop client watches, decodes the right colour, stops watching, and clears the view when the share ends |
| D6 | Packages. The friends use Windows **[Toby, 2026-11-14]**, so Windows comes first | **D6a done**: `desktop-package.yml` builds a release `Vesper.exe` and uploads a zip (exe, README, licence, third-party notices) as a 30-day artifact on every push to `v2/desktop`. Not signed, so SmartScreen warns. Not verified: running it. Still to do: an installer, signing (Toby's call), Linux and macOS packages |
| D7 | Echo cancellation, a device lost mid-call, saved per-person volumes | Done 2026-11-14. Tests: AEC3 on a synthetic room takes the echo down 50 dB once converged and keeps the near voice in double talk; the far end is exactly what was played; a fake device that is unplugged says so and is opened again; volumes saved and loaded |
| D8 | Chat parity with the web: Markdown, day rows and grouping, replies, edits, deletes, reactions, mentions, typing, uploads and files, pins | Done 2026-11-14. Tests: the web's Markdown cases ported; two clients against a real server see each other's edits, deletes, reactions, replies, typing and files; an image is fetched with the cookie and decoded; headless views for Markdown, reactions, the upload tray and the edit flow |
| Later | Search, settings parity, sharing a screen, input gain, inline audio and video players | |

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

## D3: what was built and how it was checked

- **Engine.** `src/voice/engine.rs` runs one call on its own thread (`vesper-voice`), not on the GPUI thread. It is a str0m
  0.24 client, like the SFU's `FakeClient`: one send and receive Opus m-line and no video m-line, so the SFU never offers it
  video (ADR 0009). D5 adds a receive-only video m-line (below). The socket is bound to `0.0.0.0`; after the answer, the engine adds a host candidate for each SFU address
  in it, using the local address the OS would route to that address. Every 20 ms it encodes what the microphone gave
  (`opus` 0.3 on the system libopus, Voip, the bitrate from `Call::bitrate_kbps`, 64 kbps by default like the web's setting;
  the desktop app has no settings screen yet) and sends it, then takes one frame per person from their jitter buffer,
  decodes it, mixes and plays it.
- **Signaling** goes through the model, as `join.ts` does it on the web: the engine makes the offer, the model sends `join`,
  hands the `joined` answer to the engine, passes each SFU `offer` to the engine and sends its `answer` back. The network
  thread writes `Cmd::Signal` messages on the socket and owns the running call (`Cmd::Voice`); signing out or a dropped
  session ends it. Engine events carry a call number, so a late event of an ended call is dropped.
- **Receiving.** Each forwarded track's MSID stream id is the sender's `user_id` (`accounts.id`); the engine keys a decoder,
  a jitter buffer and a speaking detector on it. The jitter buffer (`voice/jitter.rs`) holds frames by RTP time, starts
  after 60 ms, drops late packets, conceals a gap with Opus PLC (up to 5 frames, then waits to fill again), drops the oldest
  past 200 ms, and starts over when the timestamps jump (a rejoin or an unmute).
- **Mix** (`voice/mix.rs`). Per-person volume from 0 to 400 % on the web's curve (`volume.ts`), then a peak limiter at
  -3 dBFS (instant attack, about 50 ms release). Deafen plays silence. Speaking: the RMS of each 20 ms frame, on above 0.02 (the
  web's threshold) and off after 300 ms under 0.01. The engine sends who speaks only when it changes.
- **Controls** like `voice.svelte.ts` and `call.ts`: mute stops sending (the RTP clock runs on), deafen does not mute;
  both send `set_voice_state`. A force mute mutes and blocks unmuting; lifting it does not unmute. The call ends with the
  web's texts on my own `peer_left`, a room that is gone (with `leave`), an error at join, or an engine failure (no device,
  no UDP path, the connection lost). A dropped socket stops the call and joins the same room again after the next `hello`,
  unless it took over 60 s.
- **Devices** (`voice/device.rs`): cpal 0.18, the default input and output in their default format, turned into 48 kHz mono
  and back (downmix, linear resampling, upmix). A device that does not open shows why in the call bar, and the app goes on.
- **UI.** Join voice or Leave in the room header; the call bar above the composer with the room, Connecting or Connected,
  Mute, Deafen and Leave, then the people in the call: a ring while someone speaks, muted or deafened, and a volume slider
  for each other person. "Use headphones. Echo cancellation is not in the desktop app yet." shows at the first join.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display, microphone or speakers):

- Against a real `vesper-server` and SFU on loopback (`desktop/tests/voice.rs`), with a sine generator as the microphone and a
  recorder as the speakers: two clients join the same call; ann's 440 Hz tone reaches bob (about 0.29 RMS at 438 Hz measured
  by zero crossings) and bob's 660 Hz reaches ann; ann's mute silences her at bob and bob sees her muted; bob's deafen
  silences what he plays; leave stops ann's playback and bob sees her go. With three clients, bob's speaking list names ann
  when only she plays and carol when only she plays, so each stream maps to its own `user_id`; carol at 0 % is silent for bob
  while ann still hears her, and at 400 % the peak stays under the limiter's ceiling. A force mute from the server mutes bob and
  blocks unmuting, and lifting it leaves him muted; a socket the server drops brings the call back after the reconnect; a
  join from another device of the same account ends the first call with the "Your call ended" text.
- Unit tests: the jitter buffer (order, gaps, late and early packets, underrun, reset, overflow), the mixer, the volume curve
  and the limiter, the speaking hysteresis, the resampler, the SDP candidate parsing, and the model's call flow (join, answer,
  renegotiation, mute, deafen, volume, force mute, the endings, rejoin and its 60 s limit).
- GPUI's headless window (`ui/view_tests.rs`): Join voice sends the start, the call bar and the people draw with a speaking
  ring and a slider for the other person only, Mute, Deafen and Leave send the right messages, a force mute disables Mute,
  and a device error shows in the bar.

Not verified:

- Real devices: cpal with ALSA, PulseAudio or PipeWire, other sample rates and formats, a device unplugged mid-call, and how
  the call sounds. The dev machine has none.
- A real network: NAT, a firewall, Wi-Fi loss and jitter, an SFU behind `VESPER_PUBLIC_HOST`, IPv6. Only loopback ran.
- A call with the web app on the other end. The SFU forwards the same Opus, but it was not run here.
- Windows and macOS.

Left for later on purpose: echo cancellation, device choice, a settings screen (bitrate, saved volumes), DTX and in-band FEC,
the call sounds, connection quality, screen share.

## D4: what was built and how it was checked

- **Noise filter** (`src/voice/denoise.rs`). Capture order: mic, then the filter, then Opus (no echo cancellation and no input
  gain yet). Three modes, like the web's: DeepFilterNet, the noise gate, off. The web's `browser` mode is the browser's own
  suppression, which has no native match, so the light mode is the web's noise gate (`dynamics.ts`: open above -50 dBFS peak,
  close 200 ms after 4 dB under it, 2 ms and 120 ms ramps) on the raw mic **[call]**. DeepFilterNet is `vesper-dsp` with tract,
  with the root workspace's `tract-core = "=0.21.4"` pin and its debug-assertions override, copied into `desktop/Cargo.toml`;
  the lockfile pins (`libc` 0.2.189, `mio` 1.2.4) still hold. tract's state cannot move between threads and its build takes
  about 1.9 s in a debug build, so DeepFilterNet runs on its own thread (`vesper-denoise`): it builds there, and the call thread
  hands it each 10 ms frame and waits for the answer. Until it is ready the gate runs.
- **Fallback**, the web's `ModeController` logic: past a warm-up of 50 frames, 20 late frames in the last 100 turn
  DeepFilterNet off for the rest of the call and the gate takes over. The budget is 10 ms per 10 ms frame, not the web's 30 ms
  (that is the depth of the web's output queue; here a frame must be done in its own time). A frame not answered in 100 ms
  also falls back. The call bar says which filter runs and why DeepFilterNet does not.
- **Model.** Not in the binary and not committed. At a call in DeepFilterNet mode the network thread reads the cached copy
  (`<data dir>/Vesper/DeepFilterNet3_onnx.tar.gz`) or downloads `/dsp/model.tar.gz` from the signed-in server (the file the web
  app loads: the upstream DeepFilterNet3 tar.gz, which tract reads natively; nothing in it is wasm specific), at most 32 MB.
  It must match the sha256 pinned in `scripts/fetch-model.sh` (a test checks the copy in the code is the same), or it is
  refused and not cached. A bad cached copy is downloaded again. Without a model the call runs with the gate and says why.
- **Push to talk** (`src/voice/ptt.rs`). `device_query` 4.0.1 asks the OS whether the key is down every 10 ms on a thread of its
  own: `GetAsyncKeyState` on Windows, `CGEventSourceKeyState` on macOS, `XQueryKeymap` on X11. It reports press and release and
  never takes the key away from the game **[call]**: `global-hotkey` registers a system hotkey, which on Windows swallows a
  key without modifiers in every other app, and `rdev` hooks the keyboard and has had no release since 2023. The window's own
  key events also count, which is all there is on Wayland; there the Settings view says global keys need X11. Release delay
  200 ms: the web app has none, so this is new **[call]**. The key is chosen by pressing it, like the web's Settings; Ctrl,
  Alt, Meta, Tab, Enter and Backspace are refused. The default is the key left of 1 (`Grave`), not the web's Space, which would
  open the mic on every jump in a game **[call]**. As on the web, the mic sends only when not muted and the key is held;
  a mute, a force mute or turning push to talk off closes it, and the key never unmutes. The model tells the engine
  `Muted(!live)`; the server's mute state is untouched by the key, as on the web.
- **Tray** (`src/ui/tray.rs`, `tray-icon` 0.26.1): Show, Mute, Deafen, Quit. On Windows it needs a message loop on its thread;
  GPUI 0.2.2 runs `GetMessageW`/`DispatchMessageW` for the whole main thread, so the icon is made there, inside
  `Application::run`. On Linux it uses the KSNI backend (D-Bus, no GTK or appindicator headers), so CI's apt list is
  unchanged and no cargo feature is needed. Closing the window during a call minimises it (GPUI 0.2.2 has no hide on
  Windows); outside a call it quits as before.
- **Start with the computer** (`auto-launch` 0.6.0): the Run key on Windows, a login item on macOS, an autostart entry on Linux.
  Off by default; set when the setting changes.
- **Settings view**, from the sidebar: the noise filter, voice activity or push to talk and its key, microphone and speakers
  (cpal's lists, saved by name, the default when the name is gone), the Opus bitrate (32 to 510 kbps, 64 by default), start
  with the computer. Saved on each change as `Vesper/settings.json` in the OS config dir (no secrets). Push to talk applies at
  once; the filter, devices and bitrate at the next call.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display, microphone, speakers or tray):

- With the real model (`crates/vesper-dsp/model/`, copied from the main checkout; the test skips without it): a speech-like
  signal (120 Hz harmonics with vibrato, 300 ms syllables and 200 ms pauses) plus white noise at 0.02 RMS through the call's
  filter. Noise in the pauses dropped 57.6 dB (the model outputs near silence there); the syllables kept their level within
  12.7 dB (the model takes part of a synthetic voice for noise). A pure sine plus noise comes out as silence, so it is not the
  fixture. Debug build: 1.9 s to build the denoiser, 3.3 ms per 10 ms frame with the hand-over to its thread.
- A fake filter that takes 12 ms per frame falls back to the gate after its third late frame and says so; a missing model, a
  failed build and a refused checksum each give the call the gate and the reason. A wrong-checksum download is not cached, a
  tampered cached file is not used.
- Against a real server with no `/dsp/`: a call in DeepFilterNet mode shows "DeepFilterNet is not available (the server has no
  model: 404 Not Found) ..." and the other side still hears the tone through the gate.
- Push to talk: the state machine (press, release, the 200 ms delay, a press inside it), the key thread with a fake key source
  and window keys, turning it off; the model's mute rules (mute wins, the key never unmutes, force mute, a new call starts
  closed). GPUI's headless window: Settings opens from the sidebar, each choice lands in the model, a missing device says so,
  the key is taken from a key press (Backspace and Ctrl+V refused), V in the window opens and closes push to talk, and Join
  voice starts with the chosen filter and device, closed until the key.
- The settings file round trip, clamping and a corrupt file.

Not verified:

- Real keys: `device_query` on Windows, macOS and X11, a game in front (also one running as administrator, which may hide
  keys from a normal app), and the Wayland note on a real Wayland session.
- A real tray on Windows, macOS or a Linux desktop, the close-to-tray behaviour, and start with the computer: none can run here.
- Real audio through DeepFilterNet, its time per frame in a release build, and whether a slow laptop trips the fallback.
- The model download from a real server behind Caddy or Cloudflare (a proxy that adds `Content-Encoding` would fail the
  checksum and fall back), and the data dir on Windows and macOS.
- Anything on Windows beyond what CI builds and the unit and headless tests it runs.

Left for later on purpose: the gate's threshold setting and the gate after DeepFilterNet (the web has both), input gain, saved
per-person volumes, a mouse button for push to talk.

## D5: what was built and how it was checked

- **Video m-line.** The join offer has a receive-only VP8 m-line after the audio one. The web's is send-only, for its own
  share. The SFU marks any client whose offer has a video m-line as able to get video and offers it one m-line per other client
  that sends video (ADR 0009), but it makes an incoming track only from an m-line the client sends on. So the desktop client gets
  others' screens, holds no track at the SFU, and no one is offered video from it **[call]**. Renegotiation is unchanged: the
  engine answers the SFU's offers as before. The desktop cannot share yet.
- **Watching**, like the web's `watch.ts`. The call's room shows a share panel while someone in the call shares: "Bob is sharing
  their screen." with Watch, or Stop watching. One share is watched at a time **[call]**; Watch on another switches. The share
  counts as watched while it is chosen and its view is on screen (the call's room shown, not Settings). Then `watch` on goes to
  the server and the engine at once; `watch` off goes `WATCH_GRACE` (10 s, the web's `WATCH_GRACE_MS`) after that stops, so a quick
  look at another room costs no keyframe wait. A call that ends drops the gates without a `watch` off, as the server forgets
  them with the session. GPUI 0.2.2 has no call that says whether the window is visible, so a minimised window or one behind a
  game still counts as watching **[call]**: close the view (Stop watching) to let the sharer stop sending.
- **Keyframes.** The server's `watch` makes the SFU send the sharer a PLI. The engine also sends a PLI on the share's m-line
  (str0m's `request_keyframe`, which the SFU passes to the sharer) when the decoder waits for a keyframe, when a frame does not
  decode, or when a frame had to be dropped. At most one a second per share.
- **Decoding** on its own thread (`vesper-video`, `src/voice/video.rs`), so a big frame never delays the 20 ms audio turn.
  `vp8-decoder` 0.1.1 **[call]**: pure Rust, no C, no system package, so the Windows build needs nothing new. It decodes
  keyframes and interframes with every VP8 feature, and its README says it matches the WebM project's 59 test vectors bit for
  bit. Checked here as well (once, not in the tests): three streams encoded by the system's libvpx through ffmpeg
  (720p60 realtime with four token partitions and error resilience, 854x480 with alt-ref frames, 1080p60 at 10 Mbps) decoded
  byte for byte the same as libvpx's own output. The 1080p60 stream decoded at about 236 fps on one core of the dev machine in a
  release build. The other options: libvpx through `env-libvpx-sys`, `vpx-sys` or `libvpx-native-sys` links a system libvpx
  (vcpkg on Windows), and `shiguredo_libvpx`'s source build needs libvpx's configure and an assembler on Windows. Its only
  `unsafe` is in its SSE2 and NEON loop filters; ours stays `forbid(unsafe_code)`. A debug build optimises it alone
  (`[profile.dev.package.vp8-decoder]`), as a debug decode of a big screen cannot keep up.
- **Frames to the screen.** The decoder turns each picture from I420 into BGRA (BT.601 limited range, VP8's colour space) and
  hands it to the model as an event. Only the newest picture waits: the next goes out once the view has taken the last one, so a
  busy UI skips pictures instead of queueing them. Compressed frames queue for the decoder up to 30; past that the engine drops
  them and asks for a keyframe. The view moves the pixels into a GPUI `RenderImage` without a copy, draws it with `img` (object
  fit Contain: the whole screen at its own aspect ratio) and frees the previous one's GPU copy (`drop_image`).
- **Notices**, the web's texts: "Bob stopped sharing." when the watched share ends (the picture goes at once), "Bob's upload is
  struggling." while the watched share lost over 2 % of packets in a receiver report (str0m's stats every 2 s) or had a frame
  that did not decode or was dropped, in the last 6 s (the web's window), and "A moderator stopped your screen share." for
  `share_stopped`, which only a sharer gets. The cap texts ("Too many screens are being shared right now." and the 35 Mbps
  cut) go only to a sharer, so a viewer never sees them. The call bar notes "sharing" next to each person sharing.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display):

- Against a real `vesper-server` and SFU on loopback (`desktop/tests/share.rs`): a fake sharer (a str0m peer with send-only Opus
  and VP8, sending a 160x90 solid colour at 30 fps from `vp8-encoder` 0.1.1, with a keyframe when asked) joins after the desktop
  client, so the client's video m-line comes by renegotiation. The client sees the share, watches it, and decodes 160x90 pictures:
  red came out as RGB (255, 0, 0), and after the sharer turned blue, (0, 0, 255). The sharer got a keyframe request and a watcher
  count of 1. Stopping watching sent `watch` off (the sharer's count went to 0) and no picture came in the next 500 ms. Watching
  again brought pictures back. The sharer stopping cleared the view and showed "bob stopped sharing.", and the call went on. The
  SFU offered the sharer no video m-line from the desktop client. Six runs in a row passed. The voice tests pass with the new
  m-line.
- Unit tests: the colour conversion, the decoder thread (only watched shares, one picture held at a time and then the newest,
  a gap waits for a keyframe and asks for one, a broken frame asks too, a full queue drops), and the model's watch rules (on
  only when chosen and on screen, off after the grace, back within it sends nothing, the share's end, the call's end, the
  notices).
- GPUI's headless window (`ui/view_tests.rs`): the panel shows with a sharer, Watch sends the watch to the server and the engine,
  the view waits, then draws a 16x9 fake picture as a `RenderImage` of that size, the struggling note shows, and the share's
  end clears the picture and shows the note.

Not verified:

- A real browser sharing to the desktop client. The SFU forwards the browser's VP8 as it is, and libvpx streams decoded right
  here, but Chromium's and Firefox's own encoders (and their temporal layers) were not run against it.
- How it looks and how fast it is on a screen: the picture's sharpness at fit size, a real 1080p or 1440p share at 60 fps in a
  release build, and GPU memory over a long share.
- Windows beyond what CI builds and tests (the real-server tests are Unix only).

Left for later on purpose: sharing from the desktop, several shares at once, the web's layouts, 1:1 full screen, zoom, pop-out,
pointer, hide, the share stats and the share sounds.

## D7: what was built and how it was checked

- **Echo canceller: `sonora` 0.3.0 [call].** A pure Rust port of WebRTC's audio processing module, BSD-3-Clause, with only
  its echo canceller (AEC3) turned on: no noise suppression or gain control, as DeepFilterNet and the noise gate do that. It has
  no C and no build script, so Windows needs nothing new; `cargo check -p sonora --target x86_64-pc-windows-msvc` passed on the
  dev machine. Its crates use `unsafe` for SIMD, in the FFT and to turn off denormals; ours stays `forbid(unsafe_code)`. The others looked at:
  - `webrtc-audio-processing` 2.1 (the C++ AEC3): its bundled build runs meson and ninja, copies sources with `cp -a` and needs
    bindgen (libclang), so the Windows MSVC job would need meson, ninja and LLVM set up, and the C++ library is far heavier.
  - SpeexDSP: `speexdsp-sys` 0.1.2 (2018) links a system library through pkg-config, which Windows does not have; `aec-rs-sys`
    bundles it but builds with CMake and bindgen. Speex's echo canceller is an older and weaker design than AEC3 and has no
    delay estimator of its own.
  - `aec3` 0.4 (another pure Rust AEC3 port): read, not measured. Its README says the graph API is a work in progress and
    expected to change.
- **Where it runs.** On the call thread, in each 20 ms turn: the mix of everyone (after the volumes and the limiter, zero
  while deafened or with no speakers) goes to the speakers and, the same samples, to AEC3 as the far end; then each 20 ms
  captured is cleaned in two 10 ms blocks before the noise filter and Opus. AEC3 finds the delay between the far end and the
  microphone itself, with its matched filters, and follows it when the output queue or the room changes it; the engine sets
  no delay. It runs while muted too, so it stays in step. A debug build optimises `sonora`, `sonora-aec3` and `sonora-fft`
  (`[profile.dev.package]`), as the voice path does for the VP8 decoder.
- **Falls back** like DeepFilterNet: with 20 of the last 100 blocks over 5 ms (render and capture work together), after 0.5 s,
  it turns off for the rest of the call and the call bar says "Echo cancellation could not keep up on this computer, so it is
  off for the rest of the call. Use headphones, or the others may hear themselves."
- **Setting.** Settings > Echo cancellation, on by default, applies from the next call. The headphones note at the first join
  now shows only when it is off.
- **A device lost mid-call.** A cpal stream error that means the device is gone (anything but an underrun, a route change or
  refused real-time scheduling), or a microphone that gives no samples for 2 s, makes the device lost. The call bar says "Your
  microphone was disconnected.", "Your speakers were disconnected." or both, and every second the engine opens the saved
  device again, else the system default. The call goes on: without a mic nothing is sent, without speakers the far end is
  silence. The note clears when both are back.
- **Per-person volumes** are saved in `settings.json` as `volumes`, percent by user id, clamped to 0 to 400 on load, and
  the call starts with them. The slider saves on each change.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display, no audio devices):

- `voice::echo` test, a synthetic room: a speech-like far voice (140 Hz harmonics with vibrato, in syllables) through a
  room of a direct path 30 ms late at -6 dB and 60 reflections dying away over 60 ms, plus a near voice (210 Hz) from second 6.
  Echo down 34 dB in the first second and 50 dB from 3 to 6 s (asserted over 15 dB). In double talk, with the near voice 6.8 dB
  over the echo, the near voice comes out at -3.9 dB (measured as the difference between runs with and without it; asserted
  over -6 dB) and the echo is down 54 dB. 0.54 ms per 10 ms block on the dev machine with `sonora` optimised.
- `voice::engine` tests: what the fake speakers got is, sample for sample, the far end the canceller got, with a voice at 300 %
  through the limiter and zeros while deafened; a fake mic and speakers unplugged and refusing to open give the right notes,
  are tried as the saved device then the default, only once a second, and the note clears when they open; the call never
  fails.
- The voice tests through the real SFU pass with echo cancellation on in both clients.
- Settings: `echo_cancel` and `volumes` round trip, an old file turns echo cancellation on, and volumes out of range are
  clamped. The headless view tests draw the device note and the echo choice.

Not verified: a real room, real speakers and microphones (how well AEC3 does with a laptop's speakers and mic, clock drift
between two devices, a Bluetooth headset's long delay), unplugging a real device on Windows or Linux (which errors cpal's
WASAPI and ALSA backends really give), and the CPU cost on a slow Windows laptop.

## D8: what was built and how it was checked

- **Markdown** (`src/markdown.rs`) is a port of `web/src/lib/markdown.ts`: the same subset, the same rules and the same tree
  (bold, italic, strike, inline code, code blocks with a language label, headings, one level of quote, bullet and numbered
  lists nested up to 4 levels, rules, bare links, mentions). Raw HTML, `[text](url)` links and images stay text. A link is
  kept only when it parses as http or https with no user name or password, and its text is the parsed URL, so it reads as
  where it goes. `mailto:` is not a link, as on the web. `src/ui/md.rs` draws each run of inline nodes as one `StyledText` with
  highlights (bold, italic, strike, code, link, mention) and each block as its own element. A click on a link calls
  `cx.open_url`, the system browser. Code blocks are not coloured by language (the web's highlighter is not ported) **[call]**.
- **Timeline** (`src/timeline.rs`): a day row ("Today", "Yesterday", "Thu 12 Nov", "Fri 19 Dec 2025" for another year, in
  local time) before the first message of each day, and grouping as `timeline.ts`: a message heads a group unless the one
  before has the same author, is less than 5 minutes older, and neither is deleted; a reply and a new day always head one.
  The labels are English, not the system locale **[call]**. There is no "New" row.
- **Messages.** A row shows the name and time on a group's head, "(edited)" and "Pinned", the reply's quote (a click scrolls
  to the original when it is loaded; the web also loads it from the server, the desktop app does not yet), the body as
  Markdown, files, and reactions. A message that mentions me has an amber bar, as on the web. On hover a bar shows React, Reply,
  Pin or Unpin (rooms only: a DM has no pins), Edit (my own) and Delete (my own, or anyone's for a moderator or the owner, as
  the server allows). Delete asks first with the web's text. React opens the web's three quick reactions; a reaction under a
  message toggles mine and shows who reacted on hover. The web's "any symbol" box is not there **[call]**.
- **Editing.** Up in an empty composer with no files waiting edits my newest loaded message, as on the web; Edit does the same
  for any of mine. The edit box sits in the row, Enter saves, Shift+Enter is a new line, Esc cancels, and a failed save shows
  why and stays open. Focus goes back to the composer after.
- **Composer.** Above the box: "X is typing" (the web's lines for one, two and more), the reply bar ("Replying to X: ...",
  Esc or Cancel reply), the `@` list, and the upload tray. Typing is sent at most every 3 s and only for real text; a typing
  line lasts 5 s or until that person's message comes. The `@` list follows `mentionQuery` and `mentionable` from the web
  (the switcher's scorer, only people who can see the room, not banned, not me, at most 8): Up and Down move, Enter or Tab
  picks, Esc closes it for that `@`. Up, Down, Tab and Esc fall through to the text box when the list is shut.
- **Uploads** (`src/uploads.rs`, the web's `tray.svelte.ts`): the Attach button (the system's file dialog), files dropped on
  the room (GPUI's `ExternalPaths`), and a picture pasted with Ctrl+V (Cmd+V on macOS) when the clipboard holds one, saved as
  "Pasted image YYYY-MM-DD HH.MM.png" in the temp folder first. Folders are left out. A file over `Me.upload_max` fails at
  once with "Files can be up to N MB." and is never sent. Each file is its own `POST /api/rooms/{id}/uploads?name=`, the file
  streamed as the raw body with its `Content-Length`, at most 3 at a time; the tray shows the progress, Waiting, Ready, or why
  it failed (the web's texts) with Retry, and Cancel or Remove. Cancel drops the request. Send waits while a file of the room
  is still going ("Wait for the files to finish uploading."), then sends the finished ones with the text (or their names,
  as the web does when there is no text); a 409 says "That file could not be attached. Upload it again.".
- **Files in messages.** An image (png, jpeg, gif first frame, webp, at most 20 MB) is fetched from `/api/uploads/{id}` with
  the session cookie once its row is drawn, decoded off the UI thread with limits (16384 a side, 256 MB), made at most
  720 x 560 and drawn at the web's box (at most 360 x 280); the last 64 are kept. A click opens it in the system's viewer.
  Any other file is a card with the name, kind and size. Save opens the system's save dialog in Downloads (straight to
  Downloads with a free name if there is no dialog) and then says where it went, with Show in folder. Audio and video
  cards also have Open, which hands the file to the system's player; inline players are later. Open is only for images,
  audio and video, saved under the temp folder as `<id>.<ext>` with the extension from the type the server found in the
  bytes, never from the name; a plain file can only be saved, so nothing sent in chat runs with a click **[call]**. A saved
  name is made safe for Windows (no separators or reserved characters, no device names). An upload id must be 32 hex
  characters before it goes in a URL or a path.
- **Pins.** A Pins button in a room's header opens a panel of its pins (`GET /api/rooms/{id}/pins`), loaded again when a pin
  changes; a click scrolls to the message when it is loaded. Pinning and unpinning is in the hover bar.
- **Dependencies.** No new crate. Features: `reqwest` `stream` (for the upload body; it adds `wasm-streams` 0.5.0 to the
  lockfile, used only on wasm), `tokio` `fs` and `io-util`, and `image` `png`, `jpeg`, `gif` and `webp` (already on through
  GPUI). `libc` 0.2.189 and `mio` 1.2.4 still hold.

Verified, by `scripts/check-desktop.sh` on the dev machine (no display):

- Unit tests: the web's Markdown cases (`markdown_is_inert`, the link rules, bidi and IDN hosts, fences, basic, unclosed,
  depth 4, blocks, nested lists, hostile blocks) and the linear time cases (28 hostile 100k-character inputs, each under 2 s
  in a debug build); day labels and grouping; `mentionQuery` and `mentionable`; the typing throttle and the 5 s line; the tray
  (3 at a time, cancel, retry, too large, send takes the finished ones); safe names, picture decoding and the id check; the
  model's reply, edit, delete, reaction and pin flows, with a member and a moderator.
- Against a real `vesper-server` (`desktop/tests/chat.rs`): ann edits and bob sees the new text marked edited; bob reacts and
  ann sees who, then it comes off; bob cannot delete ann's message (the app offers no Delete, and the server answers 403 to
  one sent anyway); ann deletes hers and bob sees it deleted. A reply round trip with the right quote. A mention in a room not
  shown counts. Typing from ann reaches bob, and a second keystroke within 3 s sends nothing. A small text file uploaded,
  attached and seen by bob as a `file` with its name and size, then saved by bob to disk with the same bytes. A png is seen
  as an `image`, fetched by bob's client with his cookie and decoded (BGRA, 800 x 400 made 720 x 360), and the same route
  without the cookie is refused (401). A cancelled upload leaves the tray.
- GPUI's headless window (`ui/view_tests.rs`): a Markdown message draws its blocks; a day row heads the first message; a
  click on my reaction takes it off; the picker adds one; a click on a link opens it (`opened_url`); Save asks for a path
  and downloads there; Pin and the pins panel; an image is fetched once and drawn when it comes; the upload tray shows
  each file, takes progress, cancels, and Send waits and then sends with the file; Up edits my last message, Enter saves,
  Esc cancels; Delete asks first; Reply shows the bar and sends `reply_to`; the `@` list offers bob and Tab puts `@bob `;
  someone typing shows the line.

Not verified: anything on a screen (how the rows, hover bar, tray and pictures look; that the hover bar shows on hover),
a real drop of files from Explorer or a file manager, a real paste of a picture (which formats each system's clipboard
gives GPUI), the save and open dialogs and the system viewer or player on Windows, a large upload over a slow link, and
emoji drawing with Windows fonts. GPUI 0.2.2 never clears its test `debug_bounds`, so the view tests check that something
went away in the model, not on screen.

## Not in v2.0

Signup, invites and passkeys (the web app does them), admin pages, phones, webcams.

## Open questions for Toby

- Which systems do the friends use? Linux is the only one that can be built and tested here.
- Should the desktop app ever replace the web app, or always sit beside it?
