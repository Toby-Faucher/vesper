# Environment variables

Every variable the code and scripts read. Source of truth: `crates/vesper-server/src/config.rs` (`Config::from_env`). The server
reads them once at start; changing one needs a restart. A bad value (for example an address that does not parse) stops the server with
`vesper-server: <VAR>=<value>: <error>`.

Fixed, not configurable: the room size cap (12, `max_room_size`) and the WebSocket ping period (25 s, `ping_interval`).

## Server (`vesper-server`)

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `VESPER_HTTP_BIND` | HTTP listen address: the web app, `/health`, the `/ws` signaling WebSocket. | `0.0.0.0:8080` | `config.rs` |
| `VESPER_UDP_BIND` | UDP address for all media. | `0.0.0.0:40000` | `config.rs` |
| `VESPER_PUBLIC_HOST` | Hostname or IP clients are told to send media to. Resolved at start, then every 60 s (`public_addr.rs`). Unset: the server advertises its bound address and logs a warning if that is `0.0.0.0`. Empty counts as unset. | unset | `config.rs`, `public_addr.rs`, `lib.rs` |
| `VESPER_UDP_PORT_PUBLIC` | UDP port clients reach from outside. Set it only if the router maps a different external port. | the `VESPER_UDP_BIND` port | `config.rs` |
| `VESPER_WEB_DIR` | Directory with the built web app, served at `/`. Empty counts as unset. The server warns at start if `index.html` is missing. Serving rules (SPA fallback, cache headers, dotfiles hidden): `lib.rs` `guard`. | `web/dist` | `config.rs`, `lib.rs` |
| `VESPER_MAX_SOCKETS` | Concurrent WebSockets. More get HTTP 503. | `256` | `config.rs`, `ws.rs` |
| `VESPER_DATA_DIR` | Directory for the database (`vesper.db`, with its WAL files) and uploads (`uploads/<first 2 of id>/<id>`, plus `uploads/tmp/` for uploads in progress, emptied at start). Created if missing (new dirs 0700, `vesper.db` and upload files 0600). Empty counts as unset. | `data` | `config.rs`, `cli.rs`, `lib.rs`, `uploads.rs` |
| `VESPER_ORIGIN` | The address people open in the browser: `http://` or `https://`, a host and an optional port, no path. Stored lowercase with no trailing slash and no default port, so it matches the browser's `Origin` header byte for byte. It is the base of every printed invite link. `https` turns on `Secure` cookies with the `__Host-` name. With Vite dev set it to `http://localhost:5173`. Passkeys (WebAuthn) use its host as the rp id and the whole value as the only allowed origin, so the host must be a domain name or `localhost`; an IP address turns passkeys off. Changing the host makes existing passkeys stop working. | `http://localhost:8080` | `config.rs`, `cli.rs`, `lib.rs`, `ws.rs` |
| `VESPER_UPLOAD_QUOTA_MB` | Total size of all uploads, in MiB. An upload that would pass it gets 507 `quota_full`. Empty counts as unset. | `10240` | `config.rs`, `uploads.rs` |
| `VESPER_PUSH_SUBJECT` | The contact sent to push services in every VAPID token (RFC 8292): `mailto:<address>` or `https://<host>`. Unset: `VESPER_ORIGIN` when it is https, else Web Push is off (Settings says so). Empty counts as unset. A value of another form stops the server. | `VESPER_ORIGIN` if https | `config.rs`, `push.rs` |
| `VESPER_BUILD_ID` | At run time: the build id the server reports in `hello`, `/api/me` and `/health`, in place of the one compiled in. 1 to 40 letters, digits, `.`, `-` or `_`. Leave it unset; the e2e test sets it to act as a new release. At build time (the Dockerfile's build arg): the id compiled in. Unset at build time, it is the git commit (`build.rs`), from `build-id.txt` in a `git archive` copy, else the version. The Docker build context has no `.git`, so a `git pull` checkout built without the arg gets the version. A tab compares it with the one it started with and asks for a reload when it changed. Empty counts as unset. | the compiled id | `config.rs`, `build.rs` |
| `VESPER_BACKUP_KEEP` | Automatic backups kept in `<data dir>/backups`, per kind (`pre-schema-*` before a migration, `auto-*` scheduled). Older ones of that kind are deleted after each new one, except the newest `pre-schema-*` of each schema number. Manual `vesper-*` backups and other names are never deleted. At least 1. Empty counts as unset. | `5` | `config.rs`, `cli.rs` |
| `VESPER_BACKUP_EVERY` | A scheduled backup every this many hours (0 to 8760), into `<data dir>/backups/auto-<UTC time>.db`, the first one that long after start. A failure is logged, the server keeps running. `0` is off. Empty counts as unset. | `0` | `config.rs`, `lib.rs` |
| `VESPER_BACKUP_UPLOADS` | `1`: automatic backups also copy `uploads/` (as `<name>-uploads/`). Off by default because uploads can be large (up to `VESPER_UPLOAD_QUOTA_MB`); the database, with profile pictures, is always copied. Each copy is a full copy, so a crash loop after a failed upgrade can use about the uploads size times (`VESPER_BACKUP_KEEP` plus the number of schema numbers kept) of disk; when the copy would leave less free disk than twice the uploads size, the server copies the database only and logs why. `0` or unset: off. | `0` | `config.rs`, `cli.rs` |
| `RUST_LOG` | Logs go to stderr. Log filter in `tracing_subscriber` `EnvFilter` syntax, for example `info,vesper_sfu=debug`. | `info` | `main.rs` |

The Origin check applies to `/api` requests that change something (not GET or HEAD) and to the `/ws` upgrade; a missing or
other `Origin` gets 403 (`require_origin` in `lib.rs`, `upgrade` in `ws.rs`). Browsers leave `Origin` out of same-origin GET
requests, so other GETs are not checked. Clients that are not browsers, such as `curl` and `vesper-loadtest`, must send
`Origin: <VESPER_ORIGIN>` themselves.

## Build and tooling

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `TS_RS_EXPORT_DIR` | Where `ts-rs` writes the generated TypeScript types. Set for every cargo command, made absolute from the repo root. | `web/src/lib/gen` | `.cargo/config.toml` |
| `RUSTFLAGS` | `scripts/build-dsp.sh` sets `-C target-feature=+simd128` for the wasm build only. Do not export it globally. | set by the script | `scripts/build-dsp.sh` |

## Deploy

| Variable | Meaning | Where |
|---|---|---|
| `VESPER_ORIGIN`, `VESPER_PUBLIC_HOST` | Put them in a `.env` next to `deploy/compose.yaml`. Compose refuses to start without either one. | `deploy/compose.yaml` |
| `VESPER_DATA_DIR` | Compose sets `/data`, a bind mount of `./data` next to `compose.yaml` (owned by uid 10001, mode 0700). The systemd unit sets `/var/lib/vesper` (`StateDirectory`). | `deploy/compose.yaml`, `deploy/vesper.service` |
| `VESPER_TAG` | Image tag compose starts (default `latest`). Set it to roll back. | `deploy/compose.yaml` |
| `VESPER_BUILD_ID` | Optional build arg, set on the command line at each build: `VESPER_BUILD_ID=$(git rev-parse --short HEAD) docker compose -f deploy/compose.yaml up -d --build`. Do not put a fixed value in `.env`: every build would then carry the same id and open tabs would never see an update. Unset, a `git archive` copy still gets its commit from `build-id.txt`; a `git pull` checkout gets the version, because the build context leaves `.git` out. | `deploy/compose.yaml`, `deploy/Dockerfile` |
| `VESPER_WEB_DIR` | The image sets it to `/srv/vesper/web`. The systemd unit sets `/opt/vesper/web`. | `deploy/Dockerfile`, `deploy/vesper.service` |
| `VESPER_ORIGIN`, `VESPER_PUBLIC_HOST` (systemd) | Read from `/etc/vesper/vesper.env` (mode 600). The unit also sets `VESPER_HTTP_BIND=127.0.0.1:8080` and `VESPER_UDP_BIND=0.0.0.0:40000`. | `deploy/vesper.service` |

## Dev machine only

| Variable | Meaning | Default | Read in |
|---|---|---|---|
| `VESPER_NO_CAP` | `1` runs builds and checks without the memory cap. CI sets it. | unset | `scripts/capped.sh` |
| `VESPER_CAP_MIB` | Memory cap for capped commands, in MiB. | `8192` | `scripts/capped.sh` |
| `VESPER_SERVER_BIN` | Server binary the e2e tests start. Relative paths start from `web/`. `push.mjs` needs a debug build with the `test-push` feature and defaults to `../target/debug/vesper-server`. | `../target/release/vesper-server` | `web/e2e/lib.mjs`, `web/e2e/push.mjs` |
| `E2E_CHROMIUM` | Path to a Chromium the e2e tests launch instead of the one Playwright downloaded. Local use only. | unset | `web/e2e/lib.mjs` |

## Not environment variables

`vesper-server` subcommands (`create-owner`, `invite`, `loadtest-accounts`, `help`) take flags, not variables. They read the
same variables as the server. See [dev.md](dev.md#accounts) and `crates/vesper-server/src/cli.rs`.

`vesper-loadtest` takes flags: `--url ws://127.0.0.1:PORT/ws --origin http://127.0.0.1:PORT --tokens FILE --room-id N --speakers N --seconds S [--pid PID]`
(`crates/vesper-loadtest/src/main.rs`; see `docs/testing/latency.md`).

## Keeping this file true

Change a variable in `config.rs`, change this table in the same commit. Check by hand: the variable names in `config.rs`
(`grep -o 'VESPER_[A-Z_]*' crates/vesper-server/src/config.rs | sort -u`) must all appear here.
