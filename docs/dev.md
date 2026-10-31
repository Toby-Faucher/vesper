# Development

How to build, run and test Vesper on one machine. Variables: [env.md](env.md). How the parts fit: [architecture.md](architecture.md).
Deploying: [deploy.md](deploy.md). Last updated 2026-10-30.

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

The join key is whatever you put in `VESPER_DEV_KEY`. Browsers only allow the mic on `localhost` or HTTPS.

One terminal (the server serves the built app at http://127.0.0.1:8080):

```bash
npm --prefix web install && npm --prefix web run build
VESPER_DEV_KEY=pick-a-passphrase VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  VESPER_WEB_DIR=web/dist cargo run -p vesper-server --release
```

`just run-local` does the same steps with a throwaway key (`pick-a-passphrase` unless `VESPER_DEV_KEY` is set). Not verified: `just` is not installed on the dev machine.

Two terminals (hot reload for the web app):

```bash
# terminal 1: the server
VESPER_DEV_KEY=pick-a-passphrase VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  cargo run -p vesper-server --release

# terminal 2: Vite (proxies /ws to localhost:8080, prints a localhost URL)
npm --prefix web install
npm --prefix web run dev
```

Open the page, type any name, and use your passphrase as the key.

For another machine on the LAN, also set `VESPER_HTTP_BIND=0.0.0.0:8080`, `VESPER_UDP_BIND=0.0.0.0:40000` and
`VESPER_PUBLIC_HOST=<address they can reach>`, and put HTTPS in front (their mic needs it). See [deploy.md](deploy.md).

## Check before a PR

```bash
scripts/check.sh          # everything CI runs except the wasm build
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
| `docs/testing/latency.md` | Load test (12 speakers), loopback delay, `tc netem` loss. Manual. |
| `docs/testing/checkpoint.md` | The listening test against Discord. Manual, Toby signs off. |

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

- Rust: do not run a blanket `cargo update`. `tract` stays at 0.21.4 until the debug-assertion bug is gone. `wasm-bindgen` moves together with its CLI.
  Update one crate: `cargo update -p <crate> --precise <ver>`, then `scripts/check.sh`.
- npm: `web/package.json` uses exact versions (`web/.npmrc` sets `save-exact`). Run `npm audit` after a change.
- Dependabot opens weekly PRs for cargo, npm and GitHub Actions, waits 14 days after a release, and ignores `tract-*` and `deep_filter`.

## Docs and wording

README, docs and UI strings use short, plain sentences. No em dashes, no stock AI phrases. `scripts/check-wording.sh` enforces it; the list is
`scripts/wording-banned.txt`. Reword a line instead of allowing it. Env vars live in [env.md](env.md) only, and run-locally steps here only.

## Repo layout

See the table in [README.md](../README.md#layout).
