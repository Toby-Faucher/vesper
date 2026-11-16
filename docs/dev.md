# Development

How to build, run and test Vesper on one machine. Variables: [env.md](env.md). How the parts fit: [architecture.md](architecture.md).
Deploying: [deploy.md](deploy.md). Last updated 2026-11-01.

## Prerequisites

- Rust through rustup. `rust-toolchain.toml` pins an exact version (1.99.1) and adds `rustfmt`, `clippy` and the
  `wasm32-unknown-unknown` target. Move the pin on purpose, in its own commit.
- Node and npm for `web/`. `.nvmrc` says 26. `web/package.json` allows Node 22.12 or newer, which is what the Docker web stage and CI use.
- `cargo-nextest` (the test runner) and `cargo-deny` (the supply-chain check). `cargo install cargo-nextest cargo-deny --locked`.
- The `wasm-bindgen` CLI at exactly the version of the `wasm-bindgen` crate in `Cargo.lock`. `scripts/build-dsp.sh` compares them and prints
  the install command: `cargo install wasm-bindgen-cli --version <ver> --locked`.
- `curl` and `sha256sum`, for `scripts/fetch-model.sh`.
- Optional: `shellcheck` (`scripts/check.sh shell` skips without it, CI runs it) and `just` (a thin wrapper over the scripts).

## Build the denoiser first

```bash
scripts/build-dsp.sh
```

This fetches the DeepFilterNet3 model if it is missing (`scripts/fetch-model.sh`, pinned revision, sha256 checked), builds `vesper-dsp` to WASM with
`-C target-feature=+simd128`, runs `wasm-bindgen` without the name section, and writes `web/public/dsp/` (the `.wasm`, its gzip copy `.wasm.gz` that the server sends to gzip clients, the JS glue and `model.tar.gz`).
Neither the model nor `web/public/dsp/` is committed. Run it again after any change to `crates/vesper-dsp` or a `wasm-bindgen` bump.
The Docker build does the same inside the image.

## Run

Browsers only allow the mic on `localhost` or HTTPS. You sign in with an account; make the first one with `create-owner`
(see [Accounts](#accounts)).

One terminal (the server serves the built app at http://127.0.0.1:8080):

```bash
npm --prefix web install && npm --prefix web run build
VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  VESPER_WEB_DIR=web/dist cargo run -p vesper-server --release
```

`just run-local` does the same steps. Not verified: `just` is not installed on the dev machine.

Two terminals (hot reload for the web app):

```bash
# terminal 1: the server
VESPER_ORIGIN=http://localhost:5173 VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  cargo run -p vesper-server --release

# terminal 2: Vite (proxies /ws and /api to localhost:8080, prints a localhost URL)
npm --prefix web install
npm --prefix web run dev
```

Open the page. It sends you to `/login`. To make the first account, run `create-owner` (below) and open the link it prints.
The page at `/invite` reads the code from the part after `#`, which the browser never sends to the server, and removes it from the address bar and history at once.

### Accounts

The server keeps its database in `VESPER_DATA_DIR` (default `data`, relative to where you start it). The subcommands below read
the same variables as the server. Each prints its result and exits.

```bash
# a one-use owner invite link, valid 24 h; works until someone signs up as owner
cargo run -p vesper-server -- create-owner
# an invite for friends (default: member, 1 use, 168 h)
cargo run -p vesper-server -- invite --role member --uses 5 --hours 48
# load test accounts: no password, one session token per line. Throwaway data dir only:
# it refuses a dir that already has accounts unless you add --force-nonempty yes.
VESPER_DATA_DIR=/tmp/vesper-load cargo run -p vesper-server -- loadtest-accounts --count 12
```

Links and tokens go to stdout. Logs go to stderr, so piping stdout stays clean.

The links start with `VESPER_ORIGIN`. Set it to the address you open in the browser (`http://localhost:5173` with Vite).
The link is printed once and never logged; the server keeps only a hash of it.

For another machine on the LAN, also set `VESPER_HTTP_BIND=0.0.0.0:8080`, `VESPER_UDP_BIND=0.0.0.0:40000` and
`VESPER_PUBLIC_HOST=<address they can reach>`, and put HTTPS in front (their mic needs it). See [deploy.md](deploy.md).

## Desktop client (v2, in progress)

`desktop/` is the GPUI desktop client, a Cargo workspace of its own with its own `Cargo.lock`
([ADR 0010](decisions/0010-gpui-desktop-client.md), [design](superpowers/specs/2026-11-14-desktop-client-design.md)).

- Build: `cd desktop && ../scripts/capped.sh cargo build`. Run: `cargo run` there, on a machine with a display.
- Linux needs the X11, Wayland, Vulkan, fontconfig, ALSA and Opus development headers (the package list is in the `desktop` CI
  job). Arch: `opus` and `alsa-lib`; Debian and Ubuntu: `libopus-dev` and `libasound2-dev`. The `opus` crate links the system
  libopus through pkg-config. Watching a screen share needs nothing more: the VP8 decoder (`vp8-decoder`) is pure Rust.
- Sending a screen share on Linux needs the PipeWire headers and libclang (the `pipewire` crate generates its bindings with
  bindgen). Arch: `pipewire` and `clang`; Debian and Ubuntu: `libpipewire-0.3-dev` and `libclang-dev`. Under Wayland it needs
  xdg-desktop-portal with a backend that has ScreenCast (GNOME, KDE or wlr). The encoder (`vp8-encoder`) is pure Rust. Windows needs
  nothing more ([ADR 0011](decisions/0011-desktop-screen-share-send.md)).
- `VESPER_DEV_SHARE=1280x720` (debug builds, with `VESPER_DEV_VOICE`) shares moving colour bars in place of the screen through the
  real encoder and sender. `desktop/src/ui/dev.rs` lists the `VESPER_DEV_*` variables.
- Check: `scripts/check-desktop.sh` (fmt, clippy `-D warnings`, tests). It is not part of `scripts/check.sh`; CI runs it as its own job.
- `desktop/vendor/` holds patched copies of `gpui` 0.2.2 and `gpui-component` 0.5.1 (`[patch.crates-io]` in
  `desktop/Cargo.toml`). Each `PATCHED.txt` says what changed and when to drop it. Both patches stop redraws in an idle
  window: GPUI's Wayland frame loop committed about 60 times a second, and an input that was not focused redrew the
  window on every caret blink. Measured in a headless sway with `WAYLAND_DEBUG=1` (surface commits per 10 idle seconds):
  sign in 616 before, 0 after; a room with no focused input 577 before (29 real draws), 13 after (one 0.5 s animation);
  a room in voice 569 before, 15 after (a stats redraw every 2 s). A focused input still blinks: 2 redraws a second.
  Windows was not measured.
- Windows is the main target (the friends use it). CI's `desktop-windows` job builds the app and runs the unit and headless GPUI tests
  on Windows; the real-server tests in `desktop/tests/` are Unix only, because the server crates use Unix file permissions.
  Running the app on Windows is not verified by CI.
- Builds to try: the `desktop-package` workflow uploads `vesper-desktop-windows-x64-<commit>` (a zip with `Vesper.exe`) and
  `vesper-desktop-linux-x64-<commit>` (a tarball built on Ubuntu 24.04, glibc 2.39 or newer) for every push to `v2/desktop`. Download it from the workflow run's Artifacts. `desktop/package/README.txt` and `README-linux.txt` are what each package's README says.
  The tests start a real `vesper-server` in process (a path dev-dependency, so the first build compiles the server too) on a free
  loopback port over plain http, and draw the views in GPUI's headless test window. No display, keyring or network is needed.
  The voice tests (`desktop/tests/voice.rs`) run whole calls through that server's SFU with a sine for a microphone and a
  recorder for speakers, so no audio devices are needed either. The share test (`desktop/tests/share.rs`) has a fake sharer
  send VP8 made by `vp8-encoder` (a dev-dependency), so it needs no screen or video file.
  The chat tests (`desktop/tests/chat.rs`) run two clients through edits, deletes, reactions, replies, typing and files,
  with the files in a temp folder.
- Files from chat: Save opens the system's save dialog in the Downloads folder. Open (images, audio and video only) downloads
  to `Vesper/open/<id>.<ext>` in the user's cache folder (`%LOCALAPPDATA%`, `~/Library/Caches`, `$XDG_CACHE_HOME`) and hands it
  to the system's app. A download goes to `<name>.part` and is renamed when whole; it is capped at the server's upload limit.
  A pasted picture is first written to `Vesper/paste/` in the cache folder and deleted once uploaded (one never sent goes
  after a day).
- Voice uses the microphone and speakers chosen in Settings, by name, or the system defaults (cpal: ALSA on Linux, WASAPI on
  Windows). Echo cancellation is `sonora` (pure Rust AEC3), so neither Linux nor Windows needs a system package for it.
- Settings (the gear next to your name; Admin for staff and Log out are in the menu on your name; Settings > App holds Start with the computer) are a JSON file, `Vesper/settings.json` in the OS config dir (`%APPDATA%`,
  `~/Library/Application Support`, `$XDG_CONFIG_HOME`). It holds no secrets. It also keeps whether the sidebar is hidden
  (the web keeps that in localStorage), the style and colour scheme, and the ring's switch and volume. It is written to a temporary file and renamed. A value the app cannot read falls
  back to its default alone; a file that is not JSON is kept as `settings.json.bad`.
- DeepFilterNet runs natively (`vesper-dsp` with tract, the same `=0.21.4` pin and debug-assertions override as the root
  workspace). The app does not ship the model: at the first call it downloads `/dsp/model.tar.gz` from the server it is signed
  in to (the file the web app loads), checks its sha256 against the pin in `scripts/fetch-model.sh`, and keeps it as
  `Vesper/DeepFilterNet3_onnx.tar.gz` in the OS data dir. A server without `/dsp/` (a dev server without `scripts/build-dsp.sh`)
  gives a call with the noise gate and a note in the call bar. RNNoise (`vesper-dsp`'s `Rnnoise`) is built into the app and
  needs no model file.
- The DeepFilterNet test with the real model runs only when `crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz` exists
  (`scripts/fetch-model.sh`); without it the test prints `SKIPPED` and passes. CI has no model, so it skips there.
- Push to talk watches its key globally on Windows, macOS (with Accessibility access) and X11, only during a call. On Wayland it
  works only while the window has focus. As on the web, a press while a text field in Vesper has focus is ignored, and a
  release always counts. The tray uses KSNI on Linux (no GTK headers needed); a desktop without a tray host shows none.
- Sign in: the app asks for the server address (the one you open Vesper at in a browser, e.g. `https://vesper.example.com`), a
  username and a password. Accounts are made in the web app. The session is kept in the OS keyring (Secret Service on Linux,
  for example GNOME Keyring or KWallet); without one the app works but asks you to sign in at every start.
- Look: the desktop app draws the web app's tokens (`desktop/src/ui/theme.rs`, a `Tokens` global with every role of
  `web/src/styles/`, the styles and the accent maths of `accent.ts`), its icons (read from `web/src/lib/ui/icons.ts` at build
  time, `ui/icons.rs`) and its base components (`ui/c.rs`, after `web/src/lib/ui/c/`). The type is the web's: Segoe UI on
  Windows, and on Linux the font `fc-match sans-serif` names, which is also what Chromium draws for `system-ui`.
- Times follow the OS locale as the web's follow the browser's: "7:41 PM" and "Mon Oct 27" for the English locales that write
  them so (en-US and a few more, `timeline::Locale`), "19:41" and "Mon 27 Oct" for the rest. Linux reads `LANGUAGE`,
  `LC_ALL`, `LC_MESSAGES` or `LANG` (the `sys-locale` crate), Windows the user's locale. Verified on Linux only.
- Parity with the web app: `desktop/scripts/parity.sh [scene...]` builds what is missing, then `web/e2e/parity.mjs` starts one
  server, shoots each scene in Chromium and in the desktop app (a debug build in a headless sway of its own, 1440x900, scale 1,
  a throwaway home with no keyring) and writes `web/e2e/parity/<scene>-web.png`, `-desktop.png`, `-side.png` and `-diff.png`
  with the share of pixels that differ. Scenes: `login`, `shell-empty`, `timeline-general` (the 40 seeded messages),
  `timeline` (shots.mjs's plans room over three days, with the New row), `timeline-empty`, `composer` (a multi-line draft),
  `composer-typing` (bob typing), `pins`, `message-image`, `upload-tray`, `composer-mention`, `motion-send` (also
  `motion-send-{web,desktop}-{60,150}ms.png`: a sent message 60 and 150 ms into its rise), `shell` (four people in voice
  in music, one muted, one deafened, unread and mention pills; alice is not in the call, the call bar is its own scene),
  shots.mjs's call scenes `call-bar`, `call-ptt` (the key held), `call-force-muted` (shot from bob), `call-weak` (420 ms
  round trip), `call-mic-lost`, `person-card` (bob opens alice's card from the call bar at 140 %), `person-card-mod` (dave,
  a moderator, opens carol's from the sidebar) and `voice-quality` (the connection icons), `composer-offline` and
  `reconnecting` (both stop the server for each picture), `motion-ring` (bob calls alice in their DM: also
  `motion-ring-{web,desktop}-{60,150,800}ms.png`, the prompt's rise and its pulse half way), `shell-collapsed`,
  `logo-accent` (alice in teal: the logo follows the accent), `call-pill` (the call bar made small), `motion-reveal` (Dark
  to Light with a click: also `motion-reveal-{web,desktop}-{100,250}ms.png`) and `motion-slide` (Settings, Voice to
  Appearance: also `motion-slide-{web,desktop}-{60,150}ms.png`), and one per Settings and Admin section. One
  account is in one call at a time, so in the call scenes the web account leaves the call after its picture and the
  desktop's same account joins it. The desktop runs with the web scenes' clock (7:41 PM) and an en-US locale. It needs `sway` and `grim`; verified on the
  dev machine (Arch, no X). A debug build reads `VESPER_DEV_SERVER`, `VESPER_DEV_USER`, `VESPER_DEV_PASSWORD` and
  `VESPER_DEV_SCENE` (`sign-in`, `room:<name>`, `settings`, `settings:<section>`, `admin:<section>`) to get into a scene without typing, and `VESPER_DEV_CLOCK`,
  `VESPER_DEV_SHIFT`, `VESPER_DEV_EMPTY`, `VESPER_DEV_DRAFT`, `VESPER_DEV_PINS`, `VESPER_DEV_TYPING` and `VESPER_DEV_TRAY` for
  the clock, older days, an empty room, a draft, the pins list, someone typing and the upload tray, and `VESPER_DEV_VOICE`
  (join voice in a room on a silent fake microphone and speakers that play nothing: a real call through the server, no
  audio device opened), `VESPER_DEV_LINKS`, `VESPER_DEV_RTT`, `VESPER_DEV_MIC_LOST`, `VESPER_DEV_DEAFEN`,
  `VESPER_DEV_PTT_HELD`, `VESPER_DEV_PERSON`, `VESPER_DEV_VOLUME` and `VESPER_DEV_RING` for the call scenes, and
  `VESPER_DEV_THEN` for a step a motion scene films on a Settings page, and `VESPER_DEV_FRAMES` (once a second on
  stderr: frames drawn, share pictures, and the longest the UI thread was busy)
  (`desktop/src/ui/dev.rs` says how each is written); a
  release build does not have that code. The server takes 5 sign ins per name and then one a minute, so the app waits 13 s and signs in
  again when it is refused. To add a scene, add an entry to `SCENES` in `parity.mjs`: the web steps, and the
  `VESPER_DEV_*` state (a new kind of state goes in `dev.rs`), and `settings` for a `settings.json` written before the app starts
  (`shell-collapsed` sets `sidebar_collapsed`). The rooms the scenes share with `shots.mjs` (plans, photos) are in
  `web/e2e/shotlib.mjs`. The desktop scenes drop the no-keyring note, which no web scene has.
- Its lockfile pins `libc` 0.2.189 and `mio` 1.2.4 on purpose (see the ADR). Do not `cargo update` it as a whole either.

## Check before a PR

```bash
scripts/check.sh          # everything CI runs except the wasm build and `npm audit`
scripts/check.sh fast     # fmt, clippy, tests
scripts/check.sh dsp      # opt in: denoiser tests and the wasm build (needs the CLI above)
scripts/check.sh --help   # the step list
```

The full run stops at the first failure. Steps: `fmt`, `clippy` (`-D warnings`), `test` (nextest, `ci` profile), `bindings` (committed `ts-rs`
output matches the Rust types), `deny`, `secrets`, `wording`, `scripts` (every `scripts/test-*.sh`), `shell` and `web`.
`deny` needs network for the advisory database. `web` runs `npm ci`, then `npm run ci` in `web/`: svelte-check, vitest, the build and gzip size budgets
(`web/scripts/size-check.mjs`, files in `dist/assets` only; the wasm and the model are not counted). It prints five numbers and
fails above each budget: first load JS 70 kB (the entry chunk in `dist/index.html`, its modulepreloads and every chunk they import
statically), the largest lazy JS chunk 35 kB (`import()` chunks and the workers), all JS 200 kB, first load CSS 12 kB (the stylesheets `dist/index.html`
links; a lazy chunk's CSS loads with the chunk) and all CSS 24 kB. Sizes are
KiB of Node's gzip, so they read a little lower than the kB figures Vite prints.
First load JS also has a ratchet (`RATCHET_GZIP_KB` in `size-check.mjs`): the measured value after the 2026-11-02 trim plus
1 kB. Lower it when first load shrinks.

What is in the first load (measured 2026-11-02, gzip): the Svelte runtime chunk (21.2 kB) and `index` (the shell, the sidebar,
the room view, the socket, the call store and sign in). Out of it, loaded on use: anime.js (`motion.ts` imports it right
after start; until it arrives every preset is instant), the call chunk (`lib/call-stack.ts`: playback, capture, `join.ts`,
the stats, the call bar and the screen share panel with its tiles and icons (`lib/share/icons.ts`), one chunk by a
`codeSplitting` group in `vite.config.ts`; a hover, focus or press of Join voice
loads it, `join()` at the latest, and the AudioContexts are still made inside the click), the search parser (with the
search dialog) and the page texts (`messages.ts`; the first load uses `messages-first.ts`). To see what a chunk holds, build
with `npx vite build --sourcemap` and add up the mapped bytes per source.

Keyboard shortcuts (one table in `web/src/lib/ui/keys.ts`) and the browser: Ctrl+Shift+M (Chrome profile menu, Firefox
responsive design mode) and Ctrl+Shift+D (bookmark all tabs) are the browser's own keys. The app takes them only while you are
in a call, and calls preventDefault then. Verified in Chromium by the e2e flows; Firefox and Safari are not verified. A
remap setting is a nice-to-have for the Settings task.

Other test commands:

| Command | What it covers |
|---|---|
| `scripts/check.sh dsp` | The denoiser tests (4 tests behind the `tract` feature, so a plain workspace run skips them), the audio lab tests and the wasm build. Fetches the models if missing. The steps `denoiser`, `audiolab` and `wasm` run one part each. |
| `npm --prefix web run e2e` | Two headless Chromium browsers join one room; checks that the second one receives and hears the first one's audio, then chat, upload and search. Then the UI flows test. The `e2e` CI job. See below. |
| `npm --prefix web run shots -- [scene...]` | Screenshots for review, 1440x900, full page, in `web/e2e/shots/` (git-ignored). Starts the release server like the e2e tests (same ports, needs `dist/`), signs up alice (owner) and bob, then runs the named scenes, or all of them. Scenes that need content seed two more rooms and 40 messages first. Scenes: `tokens` (the dev-only `/_gallery` page, from a Vite dev server on a spare port), `gallery` (`/_gallery?primitives`: writes `gallery.png`, `gallery-open.png` with a dialog and a menu open, and `gallery-nested`; the keyboard checks moved to `ui.mjs`), `login`, `signup`, `shell-empty` (before any seeding), `shell` (five people in voice, unread and mention pills; also `shell-folded`), `shell-collapsed`, `hidden-room` (also `room-restricted`), `not-found`, `call-bar` (alice in a five-person call; also `call-muted-deafened` and `call-narrow` at 1024x640), `call-ptt` (also `call-ptt-idle`), `call-force-muted`, `call-weak` (420 ms round trip injected through `window.__pcs`), `person-card` (a member sets a volume), `person-card-mod` (a moderator on a member; also `person-card-confirm`), `switcher-empty`, `switcher-query` ("gen"), `switcher-actions` ("> "), `switcher-none`, `switcher-loading`, `switcher-failed` (its chunk held back, or failing) and `shortcuts` (the Ctrl+K switcher and the shortcuts list on the five-person scene), `timeline` (a room over three days with groups, a reply, reactions, edited, pinned and deleted messages, a mention and the New row; older messages are moved back a day or two in the harness; also `timeline-read` (the New row folded away once read; fails if "Jump to latest" shows at the bottom), `timeline-hover`, `timeline-menu`, `timeline-row-focus` (Down from the toolbar), `timeline-copy-failed` (no clipboard: the plain note), `timeline-collapsed`, `timeline-expanded` and `timeline-jump`, the flash after a reply quote click), `timeline-older` (small pages: the unread pill, then an older page loading; also `timeline-unread-pill`), `timeline-empty`, `composer` (the composer above the call bar: `composer-empty`, `composer-multiline`, `composer-mention` with three people, `composer-reply`, `composer-edit-last`, `composer-typing`, `composer-help`, `composer-error` (a send answered 429) and `composer-limit`), `pins` (three pins in the header list; also `pins-empty`), `message-image` (a room "photos" with a wide and a tall image and a file card: `message-image-loading` with the images held back, then loaded with the view still, `image-viewer` and `image-viewer-next` after Right), `drop-overlay` (files dragged over the room), `upload-tray` (a fake XMLHttpRequest holds each upload: `upload-progress` with a 5 MB file at 40 percent, the tray with a failed, three running and one waiting file, the same at 1024 px as `upload-tray-narrow`, `upload-elsewhere` (another room while they upload: "Uploading in photos"), and `upload-errors`: too large, quota, rate limit, network; `uploads` names these three scenes), `room`, `settings`, `settings-picture` (Account with a profile picture and a bio, the error for an SVG, and the shell with the picture), `accent` (Appearance with Rose chosen as `settings-accent`, a refused custom colour as `settings-accent-refused`, then a room in rose, sky and lime as `accent-<key>`; ends on amber), `diagnostics`, `motion` (the five-person scene with each animation frozen half way by the Web Animations API: `motion-arrive`, `motion-react`, `motion-room-switch` with the sidebar glide, `motion-menu` and `motion-leave`; then prints rAF frame gaps over a room switch, three arrivals and a reaction), `cpu-baseline` (no picture: prints bob's main-thread CPU in a call, see below; `cpu-level` the same with Even out voices on), `voice-quality` (the connection icons in the five-person call; also `voice-quality-card`), `composer-offline` (restarts the server and shoots the offline line; name it before `reconnecting`), and `reconnecting` (stops the server, shoots the banner, starts it again; runs last). Prints each file path. |
| `node scripts/bench-wasm.mjs chromium` | Real-browser speed of the built denoiser (needs `web/public/dsp` and Playwright; `PW_PKG` points at a `package.json` that has it). Prints median, p95, max and RTF over 20 s. Add `firefox`, or a CPU throttle rate such as `4` (Chromium, main thread). |
| `docs/testing/latency.md` | Load test (12 speakers), loopback delay, `tc netem` loss. Manual. |
| `docs/testing/checkpoint.md` | The listening test against Discord. Manual, Toby signs off. |

### Two-browser test

```bash
scripts/capped.sh cargo build --release -p vesper-server
scripts/capped.sh cargo build -p vesper-server --features test-push   # push.mjs only: a debug build
npm --prefix web ci && npm --prefix web run build
npx --prefix web playwright-core install chromium   # once; CI adds --with-deps for the system libraries
npm --prefix web run e2e
```

`web/e2e/two-browser.mjs` starts `target/release/vesper-server` itself on 127.0.0.1 on a free HTTP and a free UDP port picked at start (`VESPER_ORIGIN` follows), on a new temp data dir, and stops it and deletes the dir at the end
(helpers in `web/e2e/lib.mjs`). It runs `vesper-server create-owner`, signs alice up through that link, has alice's page make bob's
invite with `POST /api/invites`, signs bob up, and joins both to `general`.
A crash (an uncaught exception or rejection, SIGINT, SIGTERM, SIGHUP) also stops the server, the Vite dev server and the browsers before the script exits: `lib.mjs` runs each spawned server in its own process group and kills the group. `node web/e2e/cleanup.mjs` (about 15 s, the first step of `npm run e2e`) checks this: it ends a child script by an uncaught exception, an unhandled rejection, SIGTERM and SIGINT, and fails if any of its processes or its port is left. It opens two Chromium browsers with fake microphones: alice plays `web/e2e/fixtures/speech.wav`, bob uses the default fake
device. It passes when bob's ICE is connected, his inbound audio packets and bytes rise, alice's speaking ring lights on bob's page, alice's inbound
packets rise too (only that count: audio content is proven alice to bob through the speaking ring, not bob to alice), each page lists the other in the people list, and neither page throws.
Then bob leaves and joins again three times (after 2 s, within a second, and after a page reload) while alice stays: each time both pages'
inbound packets and bytes rise and the loudest sample of the other's track in the page's own playback graph is above 0.02 (a
ScriptProcessor tap; the speaking ring misses most of the short fake beeps). Chromium only: the Firefox bug behind #45 is covered by `rejoin_plays_the_new_track` in `web/src/lib/join.test.ts`.
Then chat on the same pages: alice sends `hello **bob** <b>x</b>` and a link, bob sees a bold `bob`, the `<b>x</b>` as text and a link with
`rel="noopener noreferrer nofollow"`; bob replies, alice edits (the edit box has focus, Escape cancels, Enter saves), and bob sees both live. Alice attaches a 1x1 PNG built in the test (nothing committed)
and bob's page shows it with `naturalWidth` 1. Alice makes a restricted room `secret` with a `hello` message: her search for `hello` (the
header's Search button) finds 2 hits, Esc gives focus back to the button; bob's (Ctrl+Shift+F) says `No one called nobody.` for an unknown
`from:`, asks for 2 letters on `h`, and finds exactly his 1. Down then Enter jumps. After 30 newer messages push the hit out of view, the hit scrolls it back into view on an older slice. Bob
switches to another room and back, and sees the newest page with a live message again. The test sets the denoise mode to `browser`, so it needs no wasm build. Its two variables are in [env.md](env.md#dev-machine-only).

`web/e2e/flows.mjs` then starts a fresh server the same way and checks the UI in Chromium: a bad invite link, the invite code leaving the
address bar, autocomplete attributes, an empty password field and focus on the error after a failed sign in or signup, force-mute (also
after a rejoin, and lifted), a kick ending the call, and a room made private ending the call and dropping out of bob's search
(alice still finds its message), and a room made private while bob's search is open dropping its hit from his list. It also opens search by
Ctrl+Shift+F after a failed chunk load and from the switcher, and picks a `from:` and an `in:` name from the box's lists with Enter and Tab. If `web/dist/dsp/` exists
(`scripts/build-dsp.sh`, then `npm run build`), it also signs up a new account from an invite with DeepFilterNet on and checks the denoiser
starts (the call bar and Diagnostics say `deepfilter`). CI copies the dsp-wasm job's build into the e2e job, so it runs there.

`cargo test --release` (or nextest `--release`) on vesper-server fails to compile on purpose: the tests turn on `test-push`, which is a
compile error without debug assertions. Run the tests in the debug profile, as `scripts/check.sh` and CI do.

`web/e2e/push.mjs` runs last, on the debug server built with `test-push` (it may POST to 127.0.0.1; a release build refuses the
feature) and a fake push service on a spare port. Bob runs in full Chromium in its new headless mode (the headless shell refuses the
notification permission), with a stand-in `PushManager` that hands out a subscription holding keys the test made. He turns push on in
Settings > Notifications and sends a test; the test checks the VAPID JWT against the key the page was given and decrypts the body. A
push delivered through the DevTools protocol shows as a notification from the worker. Then bob leaves the app, alice mentions him and
joins voice, and both pushes arrive encrypted with no message text in the clear. Logging out removes the worker. Not exercised: a real
push service, the browser's own subscribe, a notification on screen, a click on it.

`web/e2e/ui.mjs` runs third, on its own fresh server. First it opens the dev-only `/_gallery?primitives` page from a Vite dev server on
a spare port and checks the primitives by keyboard (`web/e2e/primitives.mjs`): a focus ring on every Tab stop, Dialog focus in and back
(Esc, backdrop, a late close event, removed while open), with Chromium's own focus return taken away so the components' code is what
is checked, nested dialogs and the scroll lock, Menu arrows, Home, End and typeahead,
Popover Esc, `aria-haspopup`, `aria-controls` and `aria-expanded`, and the Popover fallback without `showPopover({ source })`. Then the
admin pages: alice creates an invite in the form (one request
for two clicks), copies the link and bob signs up with it; a revoked link shows the bad-link text; a member who opens `/admin` sees
that it is not allowed; after a confirmed role change bob has the Admin menu entry and the moderator view without role or ban controls; a
confirmed ban ends carol's session. On Rooms alice creates, renames, moves (by button and by Alt+Up) and restricts a room to bob: bob's
sidebar follows each change live and carol's never shows it once restricted; Disk use shows two meters and the database size; a failed Members load offers Try again, which moves focus to the table. It never prints an invite link. Then: layout shift on bob's page after he joins a call (opening a
room, a message arriving, an image loading, hovering, the typing line) stays below 0.02, every shift counted (also those right after
his click, which CLS leaves out); with `prefers-reduced-motion: reduce` a dialog, the call bar and a search jump animate no transform; and a keyboard-only walk
(sign in, the skip link is the first Tab stop and moves focus to main, Ctrl+K to a room, the socket dropped: Join voice keeps focus,
says why and ignores Enter, then the socket back and Join voice by Enter, Ctrl+Shift+M, send, Up edits, Esc cancels, Ctrl+Shift+F and Enter jumps,
Ctrl+K to Settings and Esc back) checks a visible focus ring after every Tab.

All three tests fail on any CSP block in a page: each page records `securitypolicyviolation` events and console lines about the Content
Security Policy, through sign up, sign in, voice, chat, upload and search ([decision 0008](decisions/0008-csp.md)). A block inside a
worker shows neither, so only the denoiser step above catches one (a missing `'wasm-unsafe-eval'`). CI runs that step with the dsp-wasm job's build.

The fixture is synthetic (voiced syllables with vowel formants), so it has no third-party licence. `node web/e2e/fixtures/make-speech.mjs` rebuilds it.
Not checked: the inbound `audioLevel` stat. It stays 0 because the app plays remote audio through Web Audio with the `<audio>` element muted.
Verified on the dev machine with Chromium 156 (Playwright 1.64.1). Firefox is not covered.

### CPU baseline

`npm --prefix web run shots -- cpu-baseline` puts alice (speech file mic) and bob in `general`, waits 2 s, then reads the CDP
`Performance.getMetrics` `TaskDuration` on bob's page at the start and end of 10 s and prints the difference. Run it three times and take
the median. Baseline, measured 2026-11-01 on the dev machine (headless Chromium, fake mics, browser noise suppression, with the 2 s
connection quality poll): runs of 312, 261 and 238 ms, median **261 ms per 10 s**. The UI plan's Task 17 must stay within 25 percent of it
(at most 326 ms). The runs vary by about 30 percent, so compare medians, not single runs. The baseline ran in
browser noise suppression mode, because the shots run has no wasm denoiser. Task 17 must compare in the same mode. When `dist/dsp` is
present, measure both modes and compare the browser mode run against this baseline.

With the call bar (UI Task 5, 2026-11-01): runs of 379, 400 and 329 ms, median 379 ms, against 355, 356 and 305 ms (median 355 ms) for the
Task 4 build measured in the same session. So the bar adds about 7 percent; the machine was busier than on the baseline day, which is
why both medians are above 326 ms. Compare against a fresh run of the previous build, not only against the number above.

`cpu-level` is the same scene with "Even out voices" on for bob (one remote track levelled). Connection quality per person (#56,
2026-11-01): median 286 ms (8 runs of the build before) against 266 ms (5 runs). Even out voices (#57), interleaved in one session:
cpu-baseline median 237 ms, cpu-level 236 ms; the runs ranged from 106 to 308 ms on a busy machine, so the difference is under the noise.
Measured alone (vitest, Node), one levelling step costs about 4 microseconds, ten steps a second per remote track.

UI Task 17 (2026-11-01), browser mode, alternating builds in one session: the baseline build (621e9c9, the Task 3 commit, built from
`git archive` and run against the current server) 269, 273 and 288 ms (median 273); this build 193, 305 and 269 ms (median 269). Before
the fix the same session gave 297 against 476 ms: the speaking ring faded in and out over 100 ms, which repainted the sidebar and the
call bar every frame while someone talked. The ring now toggles instantly, as the plan says.

Motion everywhere (issue #26, 2026-11-01), browser mode, one session: origin/main 280, 241 and 257 ms (median 257); with the motion
presets 258, 315 and 267 ms (median 267), so about 4 percent more, inside the 25 percent. The runs vary more than that difference.
That no animation runs on a speaking change is checked in `two-browser.mjs`, which counts `Element.animate` calls while alice talks.

Debug-build note: `Cargo.toml` turns off debug assertions for `tract-core` only, because tract 0.21.4 trips one on the DeepFilterNet3 model
("duplicate name /convt3/Conv.bias"). Release builds are fine. Remove the override when tract moves past the bug.

### Noise gate and limiter lab check

`web/src/lib/audio/dynamics.test.ts` runs in `npm test` on synthetic signals. `web/scripts/gate-lab.test.mjs` repeats the gate and
limiter checks on real speech and is skipped unless both variables are set. It needs the v2 corpus ([research-loop.md](research-loop.md))
and a release `audiolab`:

```bash
touch /tmp/empty.toml   # the shipped denoiser settings
target/release/audiolab listen --config /tmp/empty.toml --items dev-0016,dev-0022 --out /tmp/gate-lab \
  --corpus research/corpus48/built --model crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz
cd web && VESPER_LAB_DIR=/tmp/gate-lab VESPER_CORPUS_DIR=../research/corpus48/built/dev \
  npx vitest run scripts/gate-lab.test.mjs --silent=false
```

It replays each denoised item through `DenoiseCore` as the worker's replies, sets the marker 6 dB over the lead-in noise peak and prints,
per item, the speech level change, word onsets, the lead-in noise cut and the largest sample step. The limiter part boosts the clean
speech 6 and 12 dB over full scale and compares it with a hard clip.

## CI

`.github/workflows/ci.yml` runs on every pull request and every push to `main`. The jobs call `scripts/check.sh` steps:

| Job | Runs |
|---|---|
| `lint-rust` | `fmt clippy shell scripts secrets wording` |
| `test-rust` | `test bindings` |
| `supply-chain` | `deny` |
| `web` | `web`, then `npm audit --audit-level=high` |
| `dsp-wasm` | `wasm`, uploaded as the `dsp` artifact, then `denoiser audiolab` |
| `e2e` | the release server and `npm run e2e`, with the `dsp` artifact in `web/dist/dsp` |
| `changes` | decides whether `e2e` and `dsp-wasm` run (below) |

- `dsp-wasm` caches the built `web/public/dsp`, keyed on a hash of `crates/vesper-dsp/**`, `Cargo.toml`, `Cargo.lock` (which holds the
  wasm-bindgen version), `.cargo/config.toml`, `rust-toolchain.toml`, `scripts/build-dsp.sh` and `scripts/fetch-model.sh` (which holds the
  model's sha256). On a hit it uploads the artifact without building. The denoiser and audio lab tests run either way. Only green `main`
  runs save the cache.
- `e2e` polls for the `dsp` artifact instead of `needs: dsp-wasm`. `dsp-wasm` uploads it before its tests, so `e2e` rarely waits.
- A pull request that only bumps the release version gets the light pipeline: `e2e` and `dsp-wasm` are skipped, the other jobs run.
  `scripts/version-only.py` decides it: the only changed files are `Cargo.toml`, `Cargo.lock`, `web/package.json`, `web/package-lock.json`
  and `docs/versions.md`, and the first four differ only in the release version (the workspace members in `Cargo.lock`, the root package
  in the npm files). Any other change, a push to `main`, an error in the script, or a failed `changes` job means the full pipeline.
  `scripts/test-version-only.sh` tests it. The `changes` job runs the base branch's copy of the script, so a PR that edits the script is
  judged by the old one (and gets the full pipeline, since the script is not on the list). `ci.yml` itself comes from the PR, so the
  gate guards against mistakes, not against a PR that rewrites the workflow.
  A job skipped by `if:` reports "skipped", which a required status check counts as passing. Whether `main` has branch protection could
  not be checked: the protection API answers 403 for this repo.
- Rust caches (Swatinem/rust-cache) are saved by `main` runs only; pull requests restore them. The repo's caches are close to the 10 GB limit.
- A newer push to the same pull request cancels the running one.

## Memory on a small machine

A release build of `tract`, `str0m` and the wasm target is heavy. `scripts/capped.sh` runs a command in a systemd user scope with a memory cap
(8 GiB, `VESPER_CAP_MIB` changes it), no swap, low CPU weight, `nice` and idle IO. It needs no root. `scripts/check.sh` and the `just` recipes use it.
For your own commands: `scripts/capped.sh cargo build --release`. If user systemd is missing it warns and falls back to `nice`.
`VESPER_NO_CAP=1` turns the cap off (CI does). Memory cap behaviour was measured on the dev machine only.

## Generated files

`web/src/lib/gen/*.ts` comes from the Rust types in `vesper-protocol` through `ts-rs`. `.cargo/config.toml` sets `TS_RS_EXPORT_DIR`. After a type change,
run `cargo test -p vesper-protocol` and commit the regenerated files. `scripts/check.sh bindings` fails if they drift.

## Dependencies

- Rust: do not run a blanket `cargo update`. `tract` stays at 0.21.4 because newer 0.21.x breaks the `deep_filter` build. `wasm-bindgen` moves together with its CLI.
  Update one crate: `cargo update -p <crate> --precise <ver>`, then `scripts/check.sh`.
- npm: `web/package.json` uses exact versions (`web/.npmrc` sets `save-exact`). Run `npm audit` after a change.
- Dependabot opens weekly PRs for cargo, npm and GitHub Actions, waits 14 days after a release, and ignores `tract-*` and `deep_filter`.

## Docs and wording

README, docs and UI strings use short, plain sentences. No em dashes, no stock AI phrases. `scripts/check-wording.sh` enforces it; the list is
`scripts/wording-banned.txt`. Reword a line instead of allowing it. Env vars live in [env.md](env.md) only, and run-locally steps here only.

## Repo layout

See the table in [README.md](../README.md#layout).
