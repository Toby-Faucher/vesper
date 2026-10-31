# Deploying Vesper

Vesper is one server binary (`vesper-server`): HTTP (the web app, `/health`, the `/ws` signaling WebSocket) on a TCP port and all media on one UDP port. HTTPS is not done by the server; put Caddy (or similar) in front of the TCP port. Media never goes through Caddy.

## Configuration (environment variables)

| Variable | Default | Meaning |
|---|---|---|
| `VESPER_DEV_KEY` | none, required | The shared join key. Whatever you set here is what people type into the join form. Empty or unset: the server refuses to start. Pick a passphrase; there are no accounts yet. |
| `VESPER_HTTP_BIND` | `0.0.0.0:8080` | HTTP listen address. |
| `VESPER_UDP_BIND` | `0.0.0.0:40000` | Media UDP listen address. |
| `VESPER_PUBLIC_HOST` | unset | Hostname or IP clients are told to send media to. Resolved at start and re-resolved periodically. Unset: the server advertises its bound address, which is useless to other machines when bound to 0.0.0.0 or 127.0.0.1. |
| `VESPER_UDP_PORT_PUBLIC` | the `VESPER_UDP_BIND` port | The UDP port clients reach from outside (set only if the router maps a different external port). |
| `VESPER_WEB_DIR` | `web/dist` | Directory with the built web app, served at `/` (extensionless unknown paths fall back to `index.html`, a missing file with an extension is a 404; `/assets/*` is `Cache-Control: immutable`, everything else `no-cache`; dotfiles and anything outside the dir are not served). |
| `VESPER_JOIN_DEADLINE_SECS` | `10` | A socket that has not joined in this time is closed. |
| `VESPER_MAX_SOCKETS` | `256` | Concurrent WebSockets. |
| `RUST_LOG` | `info` | Log filter. |

## Run locally

From the repo root. The join key is whatever you put in `VESPER_DEV_KEY`.

```bash
# 1. denoiser wasm + model into web/public/dsp/ (needs the wasm32 target and a wasm-bindgen CLI
#    whose version equals the one in Cargo.lock; the script tells you the exact install command)
scripts/build-dsp.sh

# 2a. dev: server in one terminal ...
VESPER_DEV_KEY=pick-a-passphrase VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  cargo run -p vesper-server --release
# ... Vite dev server in another (proxies /ws to localhost:8080, prints a localhost URL)
npm --prefix web install && npm --prefix web run dev

# 2b. or no Vite: build the app and let the server serve it (http://127.0.0.1:8080)
npm --prefix web run build
VESPER_DEV_KEY=pick-a-passphrase VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
  VESPER_WEB_DIR=web/dist cargo run -p vesper-server --release
```

Browsers only allow the mic on `localhost` or HTTPS. For another machine on the LAN or friends, also set `VESPER_HTTP_BIND=0.0.0.0:8080`, `VESPER_UDP_BIND=0.0.0.0:40000`, `VESPER_PUBLIC_HOST=<address they can reach>`, and use the deployed setup below (HTTPS is needed for their mic).

## Docker / Dockge

Files: `deploy/Dockerfile` (build from the repo root), `deploy/compose.yaml`.

1. Put `VESPER_DEV_KEY=<passphrase>` and `VESPER_PUBLIC_HOST` (see Network: it must resolve to the public IP from this host) in a `.env` next to `compose.yaml`. Compose refuses to start without the key; the image never contains it.
2. `docker compose -f deploy/compose.yaml up -d --build` (in Dockge: create the stack from this repo and add the `.env`; it is not verified that the compose file's `context: ..` finds the repo inside a Dockge stack directory). The first build is slow: it compiles the denoiser to wasm and downloads the DeepFilterNet model (sha256-checked by `scripts/fetch-model.sh`).
3. `curl http://127.0.0.1:8080/health` returns `ok`; `docker compose ps` shows `healthy`.

If Docker's UDP NAT misbehaves, `network_mode: host` is the alternative (see the comment in `compose.yaml`): drop `ports:`, the server binds the host directly, and the `VESPER_HTTP_BIND`/`VESPER_UDP_BIND` values are the real host listeners.

Publishing: TCP 8080 is bound to `127.0.0.1` by default, which suits Caddy on the same host. If Caddy runs elsewhere, change the mapping to `"8080:8080"` and firewall it to the proxy, or attach both containers to one docker network and point Caddy at `vesper:8080` (then drop the 8080 mapping). UDP `40000:40000/udp` must stay published on all interfaces. The container runs as a non-root user with a read-only root filesystem and no capabilities.

Plain binary instead: `deploy/vesper.service` (systemd, hardened, `DynamicUser`). Install the binary to `/opt/vesper/vesper-server`, the built `web/dist` to `/opt/vesper/web`, and put `VESPER_DEV_KEY=...` and `VESPER_PUBLIC_HOST=...` in `/etc/vesper/vesper.env` (mode 600). Then `systemctl enable --now vesper`.

## Network

- **Router:** forward **UDP 40000** to the machine running the container (the Docker host). Not TCP 3000, and no TCP forward to Vesper at all: HTTPS goes through Caddy on 443.
- **HTTPS:** `deploy/Caddyfile.example` proxies `vesper.tfaucher.com` to the HTTP port; Caddy gets the certificate and handles the WebSocket upgrade by itself.
- **`VESPER_PUBLIC_HOST`** is the name or IP the server puts into its ICE candidates, for everyone. The server resolves it with the Docker host's own resolver, so it must resolve to the **public** IP *from the server's point of view*. Use the literal public IP, or a public-only name (for example `media.tfaucher.com`) that your LAN DNS does not override. Do not point it at `vesper.tfaucher.com` if you use split DNS for that name: the server would resolve it to the LAN address and advertise that to internet clients, whose media would then fail.
- **Split DNS (HTTPS name only):** make your LAN resolver answer `vesper.tfaucher.com` with the Caddy host's LAN address so LAN browsers reach HTTPS directly. This does not affect media: LAN clients still send media to the advertised public IP, so they rely on the router's hairpin NAT (loopback of UDP 40000 to the Docker host). If the router has no hairpin NAT, LAN clients cannot get media through that path; test from outside instead (phone on cellular) and treat LAN-to-LAN calls as unsupported until that is solved.
- Known limit: the name is re-resolved every 60 s, but only sessions started afterwards pick up a changed IP; people already in a call keep the old address and must rejoin after your public IP changes.
- **`VESPER_UDP_PORT_PUBLIC`** only matters when the external UDP port differs from the one the container listens on.

## Verify

- Connectivity: open the app, join, open Diagnostics, press **Test connection**. Run it from a device on cellular (outside your LAN) and from the LAN. `ok: true` means the UDP path works.
- Load: `docs/testing/latency.md` section 1 (`vesper-loadtest`, 12 speakers). Run it on the server host against a throwaway key.

## Upgrade and rollback

Upgrade: first tag the current image, `docker tag vesper:latest vesper:previous`; then `git pull` and `docker compose -f deploy/compose.yaml up -d --build`. Everyone connected is dropped and rejoins; there is no state to migrate (presence is in memory). Rollback: `VESPER_TAG=previous docker compose -f deploy/compose.yaml up -d` (no `--build`), or `git checkout <old>` and rebuild. For the binary install keep the previous binary next to the new one and swap the file, then `systemctl restart vesper`.

If the voice engine thread dies (a bug on hostile input), `/health` returns 503 and the server process exits non-zero so Docker (`restart: unless-stopped`) or systemd (`Restart=on-failure`) restarts it; look for "voice engine (SFU thread) died" in `docker compose logs` / `journalctl -u vesper`. Covered by a test of the watcher, not verified with a real container.

## Security headers

The server sets `X-Content-Type-Options: nosniff` only. `deploy/Caddyfile.example` adds HSTS, `Referrer-Policy` and `Permissions-Policy`. Nothing sends a Content-Security-Policy: a suggested CSP is commented out in the Caddyfile; it is **untested** against the real app (WASM, the audio worklet and WebRTC all break silently under a wrong CSP), so verify in a browser before enabling it.

## Not verified

Written and checked for syntax and against the real files, but not executed (no Docker or browser on the dev machine):

- `docker build` / `docker compose up`, the image's healthcheck, non-root and read-only operation.
- The Dockerfile's `cargo install wasm-bindgen-cli` and wasm build inside the image.
- `compose.yaml` was read, not parsed by compose; `vesper.service` passes `systemd-analyze verify` apart from the binary not existing, and its hardening options were not run (check `MemoryDenyWriteExecute` and `SystemCallFilter` with a real start).
- Dockge deploy, router forward, split DNS, Caddy, certificates.
- The phone-on-cellular connectivity check.
- The CSP in the Caddyfile.

Verified locally: the release server serves `web/dist` (index, SPA fallback, `application/wasm`, `text/javascript`, `nosniff`, dotfiles and `..` traversal refused) and `/health`/`/ws` still work (`crates/vesper-server/tests/static_files.rs`).
