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
`-C target-feature=+simd128`, runs `wasm-bindgen`, and writes `web/public/dsp/` (the `.wasm`, the JS glue and `model.tar.gz`).
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
statically), the largest lazy JS chunk 35 kB (`import()` chunks and the workers), all JS 150 kB, first load CSS 12 kB (the stylesheets `dist/index.html`
links; a lazy chunk's CSS loads with the chunk) and all CSS 20 kB. Sizes are
KiB of Node's gzip, so they read a little lower than the kB figures Vite prints.

Keyboard shortcuts (one table in `web/src/lib/ui/keys.ts`) and the browser: Ctrl+Shift+M (Chrome profile menu, Firefox
responsive design mode) and Ctrl+Shift+D (bookmark all tabs) are the browser's own keys. The app takes them only while you are
in a call, and calls preventDefault then. Verified in Chromium by the e2e flows; Firefox and Safari are not verified. A
remap setting is a nice-to-have for the Settings task.

Other test commands:

| Command | What it covers |
|---|---|
| `scripts/check.sh dsp` | The denoiser tests (4 tests behind the `tract` feature, so a plain workspace run skips them) and the wasm build. Fetches the model if missing. |
| `npm --prefix web run e2e` | Two headless Chromium browsers join one room; checks that the second one receives and hears the first one's audio, then chat, upload and search. Then the UI flows test. The `e2e` CI job. See below. |
| `npm --prefix web run shots -- [scene...]` | Screenshots for review, 1440x900, full page, in `web/e2e/shots/` (git-ignored). Starts the release server like the e2e tests (same ports, needs `dist/`), signs up alice (owner) and bob, then runs the named scenes, or all of them. Scenes that need content seed two more rooms and 40 messages first. Scenes: `tokens` (the dev-only `/_gallery` page, from a Vite dev server on a spare port), `gallery` (`/_gallery?primitives`: writes `gallery.png`, `gallery-open.png` with a dialog and a menu open, and `gallery-nested`; the keyboard checks moved to `ui.mjs`), `login`, `signup`, `shell-empty` (before any seeding), `shell` (five people in voice, unread and mention pills; also `shell-folded`), `shell-collapsed`, `hidden-room` (also `room-restricted`), `not-found`, `call-bar` (alice in a five-person call; also `call-muted-deafened` and `call-narrow` at 1024x640), `call-ptt` (also `call-ptt-idle`), `call-force-muted`, `call-weak` (420 ms round trip injected through `window.__pcs`), `person-card` (a member sets a volume), `person-card-mod` (a moderator on a member; also `person-card-confirm`), `switcher-empty`, `switcher-query` ("gen"), `switcher-actions` ("> "), `switcher-none`, `switcher-loading`, `switcher-failed` (its chunk held back, or failing) and `shortcuts` (the Ctrl+K switcher and the shortcuts list on the five-person scene), `timeline` (a room over three days with groups, a reply, reactions, edited, pinned and deleted messages, a mention and the New row; older messages are moved back a day or two in the harness; also `timeline-read` (the New row folded away once read; fails if "Jump to latest" shows at the bottom), `timeline-hover`, `timeline-menu`, `timeline-row-focus` (Down from the toolbar), `timeline-copy-failed` (no clipboard: the plain note), `timeline-collapsed`, `timeline-expanded` and `timeline-jump`, the flash after a reply quote click), `timeline-older` (small pages: the unread pill, then an older page loading; also `timeline-unread-pill`), `timeline-empty`, `composer` (the composer above the call bar: `composer-empty`, `composer-multiline`, `composer-mention` with three people, `composer-reply`, `composer-edit-last`, `composer-typing`, `composer-help`, `composer-error` (a send answered 429) and `composer-limit`), `pins` (three pins in the header list; also `pins-empty`), `message-image` (a room "photos" with a wide and a tall image and a file card: `message-image-loading` with the images held back, then loaded with the view still, `image-viewer` and `image-viewer-next` after Right), `drop-overlay` (files dragged over the room), `upload-tray` (a fake XMLHttpRequest holds each upload: `upload-progress` with a 5 MB file at 40 percent, the tray with a failed, three running and one waiting file, the same at 1024 px as `upload-tray-narrow`, `upload-elsewhere` (another room while they upload: "Uploading in photos"), and `upload-errors`: too large, quota, rate limit, network; `uploads` names these three scenes), `room`, `settings`, `diagnostics`, `cpu-baseline` (no picture: prints bob's main-thread CPU in a call, see below), `composer-offline` (restarts the server and shoots the offline line; name it before `reconnecting`), and `reconnecting` (stops the server, shoots the banner, starts it again; runs last). Prints each file path. |
| `node scripts/bench-wasm.mjs chromium` | Real-browser speed of the built denoiser (needs `web/public/dsp` and Playwright; `PW_PKG` points at a `package.json` that has it). Prints median, p95, max and RTF over 20 s. Add `firefox`, or a CPU throttle rate such as `4` (Chromium, main thread). |
| `docs/testing/latency.md` | Load test (12 speakers), loopback delay, `tc netem` loss. Manual. |
| `docs/testing/checkpoint.md` | The listening test against Discord. Manual, Toby signs off. |

### Two-browser test

```bash
scripts/capped.sh cargo build --release -p vesper-server
npm --prefix web ci && npm --prefix web run build
npx --prefix web playwright-core install chromium   # once; CI adds --with-deps for the system libraries
npm --prefix web run e2e
```

`web/e2e/two-browser.mjs` starts `target/release/vesper-server` itself on 127.0.0.1:18090 (UDP 18091) on a new temp data dir with `VESPER_ORIGIN=http://127.0.0.1:18090`, and stops it and deletes the dir at the end
(helpers in `web/e2e/lib.mjs`). It runs `vesper-server create-owner`, signs alice up through that link, has alice's page make bob's
invite with `POST /api/invites`, signs bob up, and joins both to `general`.
Those ports must be free. It opens two Chromium browsers with fake microphones: alice plays `web/e2e/fixtures/speech.wav`, bob uses the default fake
device. It passes when bob's ICE is connected, his inbound audio packets and bytes rise, alice's speaking ring lights on bob's page, alice's inbound
packets rise too (only that count: audio content is proven alice to bob through the speaking ring, not bob to alice), each page lists the other in the people list, and neither page throws.
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
(`scripts/build-dsp.sh`, then `npm run build`), it also signs alice in with DeepFilterNet on and checks the denoiser starts.

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
worker shows neither, so only the denoiser step above catches one (a missing `'wasm-unsafe-eval'`). CI has no wasm build and skips that step.

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

UI Task 17 (2026-11-01), browser mode, alternating builds in one session: the baseline build (621e9c9, the Task 3 commit, built from
`git archive` and run against the current server) 269, 273 and 288 ms (median 273); this build 193, 305 and 269 ms (median 269). Before
the fix the same session gave 297 against 476 ms: the speaking ring faded in and out over 100 ms, which repainted the sidebar and the
call bar every frame while someone talked. The ring now toggles instantly, as the plan says.

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
