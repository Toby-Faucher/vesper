# Environment variables

Every variable the code and scripts read. Source of truth: `crates/vesper-server/src/config.rs` (`Config::from_env`). The server
reads them once at start; changing one needs a restart. A bad value (for example an address that does not parse) stops the server with
`vesper-server: <VAR>=<value>: <error>`.

Fixed, not configurable: the room size cap (12, `max_room_size`) and the WebSocket ping period (25 s, `ping_interval`).

## Server (`vesper-server`)

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `VESPER_DEV_KEY` | The shared join key. Whatever you set is what people type into the join form. Required: unset or empty and the server refuses to start. Never logged (`Config`'s `Debug` prints `***`). Stands in for accounts until they exist. | none | `config.rs` |
| `VESPER_HTTP_BIND` | HTTP listen address: the web app, `/health`, the `/ws` signaling WebSocket. | `0.0.0.0:8080` | `config.rs` |
| `VESPER_UDP_BIND` | UDP address for all media. | `0.0.0.0:40000` | `config.rs` |
| `VESPER_PUBLIC_HOST` | Hostname or IP clients are told to send media to. Resolved at start, then every 60 s (`public_addr.rs`). Unset: the server advertises its bound address and logs a warning if that is `0.0.0.0`. Empty counts as unset. | unset | `config.rs`, `public_addr.rs`, `lib.rs` |
| `VESPER_UDP_PORT_PUBLIC` | UDP port clients reach from outside. Set it only if the router maps a different external port. | the `VESPER_UDP_BIND` port | `config.rs` |
| `VESPER_WEB_DIR` | Directory with the built web app, served at `/`. Empty counts as unset. The server warns at start if `index.html` is missing. Serving rules (SPA fallback, cache headers, dotfiles hidden): `lib.rs` `guard`. | `web/dist` | `config.rs`, `lib.rs` |
| `VESPER_JOIN_DEADLINE_SECS` | A socket that has not joined in this many seconds is closed. Whole seconds. | `10` | `config.rs`, `ws.rs` |
| `VESPER_MAX_SOCKETS` | Concurrent WebSockets. More get HTTP 503. | `256` | `config.rs`, `ws.rs` |
| `RUST_LOG` | Log filter in `tracing_subscriber` `EnvFilter` syntax, for example `info,vesper_sfu=debug`. | `info` | `main.rs` |

## Build and tooling

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `TS_RS_EXPORT_DIR` | Where `ts-rs` writes the generated TypeScript types. Set for every cargo command, made absolute from the repo root. | `web/src/lib/gen` | `.cargo/config.toml` |
| `RUSTFLAGS` | `scripts/build-dsp.sh` sets `-C target-feature=+simd128` for the wasm build only. Do not export it globally. | set by the script | `scripts/build-dsp.sh` |

## Deploy

| Variable | Meaning | Where |
|---|---|---|
| `VESPER_DEV_KEY`, `VESPER_PUBLIC_HOST` | Put them in a `.env` next to `deploy/compose.yaml`. Compose refuses to start without either one. | `deploy/compose.yaml` |
| `VESPER_TAG` | Image tag compose starts (default `latest`). Set it to roll back. | `deploy/compose.yaml` |
| `VESPER_WEB_DIR` | The image sets it to `/srv/vesper/web`. The systemd unit sets `/opt/vesper/web`. | `deploy/Dockerfile`, `deploy/vesper.service` |
| `VESPER_DEV_KEY`, `VESPER_PUBLIC_HOST` (systemd) | Read from `/etc/vesper/vesper.env` (mode 600). The unit also sets `VESPER_HTTP_BIND=127.0.0.1:8080` and `VESPER_UDP_BIND=0.0.0.0:40000`. | `deploy/vesper.service` |

## Dev machine only

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `VESPER_NO_CAP` | `1` runs builds and checks without the memory cap. CI sets it. | unset | `scripts/capped.sh` |
| `VESPER_CAP_MIB` | Memory cap for capped commands, in MiB. | `8192` | `scripts/capped.sh` |
| `VESPER_SERVER_BIN` | Server binary the two-browser test starts. Relative paths start from `web/`. | `../target/release/vesper-server` | `web/e2e/two-browser.mjs` |
| `E2E_CHROMIUM` | Path to a Chromium the two-browser test launches instead of the one Playwright downloaded. Local use only. | unset | `web/e2e/two-browser.mjs` |

## Not environment variables

`vesper-loadtest` takes flags: `--url ws://127.0.0.1:PORT/ws --key K --room R --speakers N --seconds S [--pid PID]`
(`crates/vesper-loadtest/src/main.rs`; see `docs/testing/latency.md`).

## Keeping this file true

Change a variable in `config.rs`, change this table in the same commit. Check by hand: the variable names in `config.rs`
(`grep -o 'VESPER_[A-Z_]*' crates/vesper-server/src/config.rs | sort -u`) must all appear here.
