# Deploying Vesper

Vesper is one server binary (`vesper-server`): HTTP (the web app, `/health`, the `/ws` signaling WebSocket) on a TCP port and all media on one UDP port. HTTPS is not done by the server; put Caddy (or similar) in front of the TCP port. Media never goes through Caddy.

## Configuration

Every variable, with defaults, is in [env.md](env.md). To run on one machine first, see [dev.md](dev.md#run). The two you must set to deploy are `VESPER_ORIGIN` (the https address people open; the WebSocket and every change refuse other origins) and `VESPER_PUBLIC_HOST` (see Network below).

## Docker / Dockge

Files: `deploy/Dockerfile` (build from the repo root), `deploy/compose.yaml`.

1. Put `VESPER_ORIGIN` and `VESPER_PUBLIC_HOST` (see Network: it must resolve to the public IP from this host) in a `.env` next to `compose.yaml`. Compose refuses to start without either one.
2. Create the data dir next to `compose.yaml` and give it to the container user: `mkdir -m 700 data && sudo chown 10001:10001 data`. Compose mounts it at `/data` (`VESPER_DATA_DIR`); it holds the SQLite database with accounts, sessions and rooms.
3. `docker compose -f deploy/compose.yaml up -d --build` (in Dockge: create the stack from this repo and add the `.env`; it is not verified that the compose file's `context: ..` finds the repo inside a Dockge stack directory). The first build is slow: it compiles the denoiser to wasm and downloads the DeepFilterNet model (sha256-checked by `scripts/fetch-model.sh`).
4. `curl http://127.0.0.1:8080/health` returns `ok`; `docker compose ps` shows `healthy`.
5. Make the owner account (below).

If Docker's UDP NAT misbehaves, `network_mode: host` is the alternative (see the comment in `compose.yaml`): drop `ports:`, the server binds the host directly, and the `VESPER_HTTP_BIND`/`VESPER_UDP_BIND` values are the real host listeners.

Publishing: TCP 8080 is bound to `127.0.0.1` by default, which suits Caddy on the same host. If Caddy runs elsewhere, change the mapping to `"8080:8080"` and firewall it to the proxy, or attach both containers to one docker network and point Caddy at `vesper:8080` (then drop the 8080 mapping). UDP `40000:40000/udp` must stay published on all interfaces. The container runs as a non-root user with a read-only root filesystem and no capabilities.

Plain binary instead: `deploy/vesper.service` (systemd, hardened, `DynamicUser`). Install the binary to `/opt/vesper/vesper-server`, the built `web/dist` to `/opt/vesper/web`, and put `VESPER_ORIGIN=...` and `VESPER_PUBLIC_HOST=...` in `/etc/vesper/vesper.env` (mode 600). Then `systemctl enable --now vesper`. The unit keeps the database in `/var/lib/vesper` (`StateDirectory`). Run the admin commands as the same dynamic user, for example
`sudo systemd-run --pipe --wait -p DynamicUser=yes -p User=vesper -p StateDirectory=vesper -p EnvironmentFile=/etc/vesper/vesper.env -E VESPER_DATA_DIR=/var/lib/vesper /opt/vesper/vesper-server create-owner` (not verified).

## First owner and invites

There is no sign-up without an invite. On the host:

1. `docker compose exec vesper vesper-server create-owner` prints a one-use owner link, valid 24 h, and works only while no owner exists.
   Open it on your own device and create the owner account. The link is a secret: do not paste it anywhere else.
2. `docker compose exec vesper vesper-server invite` prints a member link (1 use, 168 h; `--role moderator`, `--uses N`, `--hours H`).
   Signed-in owners and moderators can also make invites through `POST /api/invites`. Send each friend their own link.

`vesper-server help` lists the commands. They print on stdout and never log the link.

## Password recovery

There is no email, so a reset is a link someone hands over, like an invite.

- **A forgotten password:** the owner, or a moderator for a member, opens Admin > Members, presses Reset link on that
  person's row and types their own password to confirm. The link (`<origin>/reset#<code>`) is shown once. It works once,
  for 24 hours. Send it to that person only. On the page they set a new password; every session of that account then
  ends, and they sign in with the new password. Their next sign in tells them once who made the link and when.
  A new link for the same person replaces the old one; a role change, a ban or unban, or the person changing their
  own password also ends a live link. Moderators can not make one for the owner or a moderator, and nobody can make one
  for themself in the app. Each person can make 10 links, then 1 every 6 minutes.
- **Who reset whom:** the owner sees each person's latest reset in Admin > Members ("Password reset by alice on 2 Nov
  (used)"). The database keeps every reset (who made it, when, when it was used and whether from the LAN or the
  internet), never the code.
- **The owner's own password:** on the host, `docker compose exec vesper vesper-server reset-password <username>` prints
  the same kind of link, valid 24 h. It works for any account, the owner's too. Open it on your own device.
  For the binary install use the `systemd-run` form above with `reset-password <username>` (not verified).

The database keeps only a hash of the code. The server logs that a link was made or used, with the account ids of the
person and of who made it, never the link.

## Network

- **Router:** forward **UDP 40000** to the machine running the container (the Docker host). Not TCP 3000, and no TCP forward to Vesper at all: HTTPS goes through Caddy on 443.
- **HTTPS:** `deploy/Caddyfile.example` proxies `vesper.tfaucher.com` to the HTTP port; Caddy gets the certificate and handles the WebSocket upgrade by itself.
- **`VESPER_PUBLIC_HOST`** is the name or IP the server puts into its ICE candidates, for everyone. The server resolves it with the Docker host's own resolver, so it must resolve to the **public** IP *from the server's point of view*. Use the literal public IP, or a public-only name (for example `media.tfaucher.com`) that your LAN DNS does not override. Do not point it at `vesper.tfaucher.com` if you use split DNS for that name: the server would resolve it to the LAN address and advertise that to internet clients, whose media would then fail.
- **Split DNS (HTTPS name only):** make your LAN resolver answer `vesper.tfaucher.com` with the Caddy host's LAN address so LAN browsers reach HTTPS directly. This does not affect media: LAN clients still send media to the advertised public IP, so they rely on the router's hairpin NAT (loopback of UDP 40000 to the Docker host). If the router has no hairpin NAT, LAN clients cannot get media through that path; test from outside instead (phone on cellular) and treat LAN-to-LAN calls as unsupported until that is solved.
- Known limit: the name is re-resolved every 60 s, but only sessions started afterwards pick up a changed IP; people already in a call keep the old address and must rejoin after your public IP changes.
- **Client address:** the chain is Cloudflare, `cloudflared` on the VM, Caddy, Vesper, so Vesper's TCP peer is a loopback or private address. From such a peer (loopback, 10/8, 172.16/12, 192.168/16, IPv6 unique-local) the server takes a valid `CF-Connecting-IP` first (Cloudflare overwrites it, so a client cannot fake it through Cloudflare), then the last `X-Forwarded-For` entry (the one Caddy adds), then the peer. From any other peer it ignores both headers. Caddy's default `reverse_proxy` sets `X-Forwarded-For` to its own peer and passes `CF-Connecting-IP` through unchanged, which is what the server expects; do not add `header_up` lines that drop or rewrite them (not verified with a real Caddy). **Keep the HTTP port reachable only from Caddy**: a LAN client that reaches it directly can set either header and pick its own address, and so dodge the login and invite rate limits. See [known-issues.md](known-issues.md#security-and-abuse).
- **`VESPER_UDP_PORT_PUBLIC`** only matters when the external UDP port differs from the one the container listens on.

- **Data dir permissions:** the server creates a new `VESPER_DATA_DIR` with mode 0700 and `vesper.db` (and its WAL files) with 0600. An existing directory keeps its mode, so a bind-mounted `/data` keeps its host permissions: set it to 0700 for the container user yourself. The database holds password and session hashes. Verified by a test on a fresh directory only.

## Verify

- Connectivity: open the app, join, open Diagnostics, press **Test connection**. Run it from a device on cellular (outside your LAN) and from the LAN. `ok: true` means the UDP path works.
- Load: `docs/testing/latency.md` section 1 (`vesper-loadtest`, 12 speakers). Run it on the server host against a throwaway data dir.

## Upgrade and rollback

Upgrade: first tag the current image, `docker tag vesper:latest vesper:previous`; then `git pull` and `docker compose -f deploy/compose.yaml up -d --build`. Everyone connected is dropped and rejoins. The database in the data dir stays; the server applies its schema migrations at start, so an older image may not read a database a newer one migrated: take a backup before an upgrade that changes the schema. Rollback: the image and `deploy/compose.yaml` must match, because compose only passes the variables it lists. Check out the previous commit's `deploy/compose.yaml` (`git checkout <old> -- deploy/compose.yaml`) and run `VESPER_TAG=previous docker compose -f deploy/compose.yaml up -d` (no `--build`), or `git checkout <old>` and rebuild. An image from before accounts needs its old compose file (or a local `VESPER_DEV_KEY: ${VESPER_DEV_KEY:?}` line) and the key in `.env`. Not verified. For the binary install keep the previous binary next to the new one and swap the file, then `systemctl restart vesper`.

If the voice engine thread dies (a bug on hostile input), `/health` returns 503 and the server process exits non-zero so Docker (`restart: unless-stopped`) or systemd (`Restart=on-failure`) restarts it; look for "voice engine (SFU thread) died" in `docker compose logs` / `journalctl -u vesper`. Covered by a test of the watcher, not verified with a real container.

## Backup

Back up the data dir (`data/` next to `compose.yaml`, or `/var/lib/private/vesper`). With the Proxmox snapshot of the VM, check that the snapshot includes that directory. For a copy while the server runs, use SQLite's own backup so the WAL is included: `sqlite3 data/vesper.db ".backup vesper-backup.db"` (needs the `sqlite3` tool; the data dir is owned by uid 10001 with mode 0700, so on the host run it with `sudo`, or run it inside the container), or stop the stack and copy the whole directory. The copy holds password and session hashes: keep it private. Not verified.

## Upgrade from the shared key

For the instance that still runs the voice-only build with `VESPER_DEV_KEY`. Not verified (no Docker on the dev machine). In the stack directory (from a clone, add `-f deploy/compose.yaml`):

1. Tag the running image: `docker tag vesper:latest vesper:previous`.
2. Create the data dir next to `compose.yaml` and give it to uid 10001: `mkdir -m 700 data && sudo chown 10001:10001 data`.
3. In `.env`: remove the `VESPER_DEV_KEY` line and add `VESPER_ORIGIN=https://vesper.tfaucher.com`.
4. Check that Caddy passes `X-Forwarded-For` and `CF-Connecting-IP` (the default `reverse_proxy` does; see Client address above) and that port 8080 is reachable only from Caddy.
5. `docker compose up -d --build`, then `docker compose ps` shows `healthy`.
6. `docker compose exec vesper vesper-server create-owner`; open the printed link on your own device and create the owner account.
7. Make invites with `docker compose exec vesper vesper-server invite` (or the API) and send each friend their link. The old key no longer works for anyone.
8. Backups: the Proxmox snapshot now has to include the `data` directory.

Rolling back to the shared-key image needs the old `deploy/compose.yaml` too (the current one does not pass `VESPER_DEV_KEY`), plus the key in `.env`. Not verified. That image ignores the data dir, and the accounts stay there for the next try.

## Security headers

The server sets the security headers itself, on every response: a strict Content-Security-Policy, `Referrer-Policy`, `Permissions-Policy`
(microphone for this site only), `Cross-Origin-Opener-Policy`, `X-Content-Type-Options`, and HSTS when `VESPER_ORIGIN` is https. The policy
and why each part is there: [decision 0008](decisions/0008-csp.md). Uploads keep their own sandbox policy. `deploy/Caddyfile.example` sets
only HSTS (the same value), so the headers do not depend on Caddy. Verified in headless Chromium by the e2e test; not verified through Caddy.

## Not verified

Last updated 2026-11-01. Written and checked for syntax and against the real files, but not executed (no Docker or browser on the dev machine):

- `docker build` / `docker compose up`, the image's healthcheck, non-root and read-only operation.
- The Dockerfile's `cargo install wasm-bindgen-cli` and wasm build inside the image.
- `compose.yaml` was read, not parsed by compose; `vesper.service` passes `systemd-analyze verify` apart from the binary not existing, and its hardening options were not run (check `MemoryDenyWriteExecute` and `SystemCallFilter` with a real start).
- Dockge deploy, router forward, split DNS, Caddy, certificates.
- The phone-on-cellular connectivity check.
- The server's headers through Caddy and Cloudflare (Cloudflare can add or change headers).
- The `data` bind mount, `create-owner` and `invite` through `docker compose exec`, the `systemd-run` form for the binary install, the backup commands and the "Upgrade from the shared key" steps.

Verified locally: the release server serves `web/dist` (index, SPA fallback, `application/wasm`, `text/javascript`, `nosniff`, dotfiles and `..` traversal refused) and `/health`/`/ws` still work (`crates/vesper-server/tests/static_files.rs`).
