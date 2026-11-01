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
`deny` needs network for the advisory database. `web` runs `npm ci`, then `npm run ci` in `web/`: svelte-check, vitest, the build and a gzip size budget
(`web/scripts/size-check.mjs`, JS and CSS in `dist/assets` only; the wasm and the model are not counted).

Other test commands:

| Command | What it covers |
|---|---|
| `scripts/check.sh dsp` | The denoiser tests (4 tests behind the `tract` feature, so a plain workspace run skips them) and the wasm build. Fetches the model if missing. |
| `npm --prefix web run e2e` | Two headless Chromium browsers join one room; checks that the second one receives and hears the first one's audio, then chat, upload and search. Then the UI flows test. The `e2e` CI job. See below. |
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
and bob's page shows it with `naturalWidth` 1. Alice makes a restricted room `secret` with a `hello` message: her search for `hello` finds 2 hits,
bob's finds exactly his 1. After 30 newer messages push the hit out of view, the hit scrolls it back into view on an older slice. Bob
switches to another room and back, and sees the newest page with a live message again. The test sets the denoise mode to `browser`, so it needs no wasm build. Its two variables are in [env.md](env.md#dev-machine-only).

`web/e2e/flows.mjs` then starts a fresh server the same way and checks the UI in Chromium: a bad invite link, the invite code leaving the
address bar, autocomplete attributes, an empty password field and focus on the error after a failed sign in or signup, force-mute (also
after a rejoin, and lifted), a kick ending the call, and a room made private ending the call. If `web/dist/dsp/` exists
(`scripts/build-dsp.sh`, then `npm run build`), it also signs alice in with DeepFilterNet on and checks the denoiser starts.

Both tests fail on any CSP block: each page records `securitypolicyviolation` events and console lines about the Content Security
Policy, through sign up, sign in, voice, chat, upload and search ([decision 0008](decisions/0008-csp.md)).

The fixture is synthetic (voiced syllables with vowel formants), so it has no third-party licence. `node web/e2e/fixtures/make-speech.mjs` rebuilds it.
Not checked: the inbound `audioLevel` stat. It stays 0 because the app plays remote audio through Web Audio with the `<audio>` element muted.
Verified on the dev machine with Chromium 156 (Playwright 1.64.1). Firefox is not covered.

Debug-build note: `Cargo.toml` turns off debug assertions for `tract-core` only, because tract 0.21.4 trips one on the DeepFilterNet3 model
("duplicate name /convt3/Conv.bias"). Release builds are fine. Remove the override when tract moves past the bug.

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
