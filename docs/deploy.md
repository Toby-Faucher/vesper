# Deploying Vesper

Vesper is one server binary (`vesper-server`): HTTP (the web app, `/health`, the `/ws` signaling WebSocket) on a TCP port and all media on one UDP port. HTTPS is not done by the server; put Caddy (or similar) in front of the TCP port. Media never goes through Caddy.

## Configuration

Every variable, with defaults, is in [env.md](env.md). To run on one machine first, see [dev.md](dev.md#run). The two you must set to deploy are `VESPER_ORIGIN` (the https address people open; the WebSocket and every change refuse other origins) and `VESPER_PUBLIC_HOST` (see Network below).

## Docker / Dockge

Files: `deploy/Dockerfile` (build from the repo root), `deploy/compose.yaml`.

1. Put `VESPER_ORIGIN` and `VESPER_PUBLIC_HOST` (see Network: it must resolve to the public IP from this host) in a `.env` next to `compose.yaml`. Compose refuses to start without either one.
2. Create the data dir next to `compose.yaml` and give it to the container user: `mkdir -m 700 data && sudo chown 10001:10001 data`. Compose mounts it at `/data` (`VESPER_DATA_DIR`); it holds the SQLite database with accounts, sessions and rooms.
3. `docker compose -f deploy/compose.yaml up -d --build` (in Dockge: create the stack from this repo and add the `.env`; it is not verified that the compose file's `context: ..` finds the repo inside a Dockge stack directory). The first build is slow: it compiles the denoiser to wasm and downloads the DeepFilterNet model (sha256-checked by `scripts/fetch-model.sh`).
4. `curl http://127.0.0.1:8080/health` returns `{"status":"ok","version":"...","build":"..."}`; `docker compose ps` shows `healthy`.
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
  ends, and they sign in with the new password. The next sign in after a reset shows who made the link and when, and
  lists every reset of that account used in the last 30 days again. So if someone else used a link first (and saw the
  notice), the person still hears about it once they are back in through a new link.
  A new link for the same person replaces the old one; a role change, a ban or unban, or the person changing their
  own password also ends a live link, and banning or demoting a moderator ends every live link they made. Moderators
  can not make one for the owner or a moderator, and nobody can make one for themself in the app. Each person can make
  10 links, then 1 every 6 minutes; a mistyped own password counts as one of them.
- **Who reset whom:** the owner sees, under each person in Admin > Members, their latest reset link and every reset used
  in the last 30 days ("Password reset by alice on 2 Nov (used)"; the date is the day it was used). Older used resets
  stay in the database (who made it, when, when it was used and whether from the LAN or the internet, never the code)
  and in the server log, but not in the app.
- **A taker who keeps changing the password:** someone who took over an account and is still signed in can change its
  password again, and a password change ends a live reset link, so your recovery link may stop working. Then: ban the
  account (that ends its sessions; you can still make a link for it), make the link, let the person use it, and only
  then unban. An unban before the link is used ends the link too.
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
- **Timeouts:** Cloudflare and Caddy have their own, but the server does not depend on them. It closes a
  connection that sends no full request head in 10 s (also an idle kept-alive one), cuts a JSON body after 15 s without bytes or 30 s in
  all, and an upload or profile picture after 30 s without bytes or 10 minutes in all. The WebSocket has none of these, only its own ping.
  Details in [architecture.md](architecture.md#connection-timeouts). Because the server closes idle connections after 10 s, the example
  Caddyfile keeps Caddy's idle upstream connections for 5 s, so Caddy does not send a request on a connection the server is closing (not
  verified with a real Caddy; the example was not run through `caddy validate`, since no Caddy is on the dev machine).
- **Existing deployments (upgrading to v1.3.1 or later):** add the same block to the live Caddyfile before or together with the upgrade.
  On the VM that is `/opt/stacks/caddy/Caddyfile`; in the Vesper site block, the bare `reverse_proxy vesper:8080` becomes:

  ```
  reverse_proxy vesper:8080 {
  	transport http {
  		keepalive 5s
  	}
  }
  ```

  The file is a
  single-file bind mount, so edit it in place with an editor (not `mv` or `sed -i`, which replace the file and leave the container on
  the old one), then reload: `docker exec caddy caddy reload --config /etc/caddy/Caddyfile`. Why: the server closes an idle connection
  after 10 s, while Caddy keeps idle upstream connections for 2 minutes by default. A request Caddy sends on such a connection just as the
  server closes it fails; Caddy retries a GET, but not a POST, PATCH or PUT with a body, so that request gets a 502. It is rare. To spot
  it: a 502 in Caddy's access log for a request that came after the app sat idle for a while.
- **`VESPER_UDP_PORT_PUBLIC`** only matters when the external UDP port differs from the one the container listens on.

- **Data dir permissions:** the server creates a new `VESPER_DATA_DIR` with mode 0700 and `vesper.db` (and its WAL files) with 0600. An existing directory keeps its mode, so a bind-mounted `/data` keeps its host permissions: set it to 0700 for the container user yourself. The database holds password and session hashes. Verified by a test on a fresh directory only.

## Verify

- Connectivity: open the app, join, open Diagnostics, press **Test connection**. Run it from a device on cellular (outside your LAN) and from the LAN. `ok: true` means the UDP path works.
- Load: `docs/testing/latency.md` section 1 (`vesper-loadtest`, 12 speakers). Run it on the server host against a throwaway data dir.

## Upgrade and rollback

After an image with a new migration has started on a database (v1.3 adds 0007 password resets, 0008 room archive and 0009 push; v1.4 adds 0010 call history), an older image will not start on that database: it stops with "migration 7 was previously applied but is missing". Rollback then means the older image and the backup taken before the upgrade.

From v1.4 the server takes that backup itself: at start, when `vesper.db` exists and the image carries migrations the database does not have yet, it writes `data/backups/pre-schema-<NNNN>-<UTC time>.db` and only then migrates. `<NNNN>` is the schema the copy holds: the number of the newest migration applied to it, for example `pre-schema-0009-20261102T040000.123Z.db` holds schema 9 (v1.3). Next to it is a small `.json` manifest (the new server's version and build, `schema`, the target `schema_to`, whether uploads were copied). Both are written as `<name>.partial` and renamed when complete, the database last, so a `pre-*.db` is a complete copy after a kill; the database, the manifest and the directory are also synced to disk, so the database copy survives a power cut too. Copied upload files (`-uploads/`, only with `VESPER_BACKUP_UPLOADS=1`) are not synced one by one: after a power cut some of them can be empty. A `.partial`, or a `.json` or `-uploads/` without its `.db`, is what a killed backup leaves; the server deletes those at its next start and before each automatic backup. Uploads are not copied unless `VESPER_BACKUP_UPLOADS=1` ([env.md](env.md)): they stay in `data/uploads` and an older image reads them as they are. Directories are mode 0700 and files 0600. If that backup fails (for example the disk is full), the server does not start and says why: free space and start again. The server deletes old `pre-*` copies itself and keeps the newest 5 (`VESPER_BACKUP_KEEP`) plus the newest copy of each schema number, so a crash loop never deletes the copy an older image can open (see below). A release with no migration takes no automatic backup. v1.4 adds migration 0010 (call history), so the upgrade from v1.3 to v1.4 writes `pre-schema-0009-*.db` before it migrates (verified on a copy of a v1.3.1 database, see the v1.4 release pull request). A manual backup is also the way to keep uploads: `docker compose exec vesper vesper-server backup /data/backups`.

sqlx commits each migration on its own. When a release carries several (say schema 9 to 11) and a later one fails on your data, the database is left at schema 10, which no older image opens, and `restart: unless-stopped` starts the server again and again. Each start backs up that schema-10 state as another `pre-schema-0010-*` copy; the `pre-schema-0009-*` copy taken at the first start is kept.

To roll back after a migration:

1. Find the old image's schema: the highest number in `crates/vesper-store/migrations/` of the old tag (`git ls-tree --name-only <old-tag> crates/vesper-store/migrations/`; v1.3 is 0009). An old image that refuses the database says "migration N was previously applied but is missing": its schema is below N.
2. List the copies: `sudo ls data/backups` (the data dir is mode 0700 and owned by uid 10001; for the binary install, `sudo ls /var/lib/private/vesper/backups`). Ignore `*.partial`. Of the `pre-schema-<NNNN>-<UTC time>.db` files whose `<NNNN>` is at most the old image's schema, take the one with the newest UTC time. In the crash loop above that is the `pre-schema-0009-*` copy, not the newest file.
3. Stop the stack, restore that copy as `vesper.db` (steps 1 to 4 of [Restore](#backup), with `data/backups/pre-schema-<NNNN>-<t>.db` in step 3 and the uploads left in place), and start the old image.

Everything written after the upgrade is lost. Files uploaded after it have no row in the old copy and are removed by the hourly sweep after 24 h; files deleted after it answer 404. Tested on a temporary dir (`crates/vesper-server/tests/cli.rs`: five starts of an upgrade that fails part way, then the copy step 2 picks opens with the old migrations); not verified with a real old image.

The account commands (`create-owner`, `invite`, `reset-password`, `loadtest-accounts`) never migrate a database: on a new image before its first start they stop and say to start the server once first, so the backup above is never skipped.

Upgrade: first tag the current image, `docker tag vesper:latest vesper:previous`; then `git pull` and `VESPER_BUILD_ID=$(git rev-parse --short HEAD) docker compose -f deploy/compose.yaml up -d --build`. The build context leaves `.git` out, so without that variable the build id is the version ([env.md](env.md)): two deploys of one version then look the same to open tabs, and they get no "Vesper was updated" banner. Everyone connected is dropped and rejoins. The database in the data dir stays; the server applies its schema migrations at start, so an older image may not read a database a newer one migrated: take a backup before an upgrade that changes the schema. Rollback: the image and `deploy/compose.yaml` must match, because compose only passes the variables it lists. Check out the previous commit's `deploy/compose.yaml` (`git checkout <old> -- deploy/compose.yaml`) and run `VESPER_TAG=previous docker compose -f deploy/compose.yaml up -d` (no `--build`), or `git checkout <old>` and rebuild. An image from before accounts needs its old compose file (or a local `VESPER_DEV_KEY: ${VESPER_DEV_KEY:?}` line) and the key in `.env`. Not verified. For the binary install keep the previous binary next to the new one and swap the file, then `systemctl restart vesper`.

If the voice engine thread dies (a bug on hostile input), `/health` returns 503 and the server process exits non-zero so Docker (`restart: unless-stopped`) or systemd (`Restart=on-failure`) restarts it; look for "voice engine (SFU thread) died" in `docker compose logs` / `journalctl -u vesper`. Covered by a test of the watcher, not verified with a real container.

## Backup

`vesper-server backup <dir>` writes two things into `<dir>` (created with mode 0700 if missing):

- `vesper-<UTC time>.db`, for example `vesper-20261101T040000Z.db`: a copy of the database made with SQLite's `VACUUM INTO`, so it is consistent while the server runs and needs no WAL file next to it. Profile pictures live in the database, so they are in this file.
- `vesper-<UTC time>-uploads/`: a copy of `uploads/` (attachments), without `uploads/tmp`. A file deleted while the copy runs is skipped and counted in the summary; its row is still in the database copy, so after a restore that one attachment answers 404.

Directories are mode 0700 and files 0600. It never overwrites: a second run in the same second fails. Automatic backups (before a migration, and every `VESPER_BACKUP_EVERY` hours when set) go to `<data dir>/backups` as `pre-schema-*` and `auto-*`, the database and a manifest only unless `VESPER_BACKUP_UPLOADS=1`; the server prunes them itself: the newest `VESPER_BACKUP_KEEP` (5) of each kind, plus the newest `pre-schema-*` of each schema number, are kept. It never deletes `vesper-*` ones or any other name ([Upgrade and rollback](#upgrade-and-rollback), [env.md](env.md)). `<dir>` must be outside `uploads/`. It reads `vesper.db` in `VESPER_DATA_DIR` as it is: a missing file is an error (a wrong data dir), and it never creates or migrates the database. Each output is written as `<name>.partial` and renamed when done, the database last. If any step fails, it removes both outputs and exits non-zero, so a pair left in `<dir>` is complete unless the machine lost power: the database copy is synced to disk, the copied upload files are not, so after a power cut some of them can be empty. A `vesper-*.partial` is from a run that was killed; the server deletes one older than an hour at its next start. It prints the paths, the file count and the bytes. The copy holds password and session hashes: keep it private.

In the container (Docker / Dockge), from the stack directory:

    docker compose exec vesper vesper-server backup /data/backups

`/data/backups` is `data/backups` on the host, owned by uid 10001. It sits on the same disk as the data, so copy it off the machine as well (the Proxmox snapshot, or `sudo rsync` to another host). A nightly run at 04:00 that keeps 14 days of the manual `vesper-*` backups (host crontab; `-T` because cron has no terminal; use the stack's path). The `find` matches only the names the command writes; never widen it to all of `/data/backups`, or it deletes the automatic `pre-*` copies, which only the server prunes:

    0 4 * * * cd /opt/stacks/vesper && docker compose exec -T vesper sh -c 'vesper-server backup /data/backups && find /data/backups -mindepth 1 -maxdepth 1 \( -name "vesper-*.db" -o -name "vesper-*-uploads" \) -mtime +14 -exec rm -r {} +' >> backup.log 2>&1

Binary install: run it as the service user with the `systemd-run` form from [Docker / Dockge](#docker--dockge), for example with `/var/lib/vesper/backups` as `<dir>`.

Restore (the stack stopped; `<t>` is the time in the file names):

1. `docker compose stop vesper`.
2. Move the current files aside: `sudo mv data/vesper.db data/vesper.db.old`, the same for `data/vesper.db-wal` and `data/vesper.db-shm` if present, and `sudo mv data/uploads data/uploads.old`.
3. `sudo cp data/backups/vesper-<t>.db data/vesper.db` and `sudo cp -r data/backups/vesper-<t>-uploads data/uploads`.
4. `sudo chown -R 10001:10001 data/vesper.db data/uploads && sudo chmod 600 data/vesper.db`.
5. `docker compose up -d`, then `docker compose ps` shows `healthy`. Sign in and open a room with an attachment.

Everything after the backup time is lost, and every session made later is gone (people sign in again). An upload made while the backup ran may be in the copy without a message; the server removes such files after 24 h.

Verified by tests on the dev machine: the copy opens as a valid database (`PRAGMA integrity_check`) with the same rows, a copy taken while another connection writes is consistent, the modes are 0700 and 0600, `uploads/tmp` is left out, an existing backup is never overwritten, a failed copy leaves nothing behind, a missing database is refused without creating one, and the database is not migrated. Not verified: the command through `docker compose exec`, the cron line, the `systemd-run` form, and the restore steps. The tests open the copy with the same store code the server uses, but no restored copy was started as a real server.

A backup now also holds the Web Push signing key (VAPID, table `server_secrets` in `vesper.db`): keep the copy private and mode 600,
like the database itself. Anyone with the key can sign pushes to the browsers that subscribed. After a restore, devices that turned on
notifications keep getting them. A database restored without that row makes a new key: the server deletes each old subscription when
its push service refuses it (401 or 403), and Settings asks each person to turn notifications on again.

## Web Push

Notifications while Vesper is closed ([architecture.md](architecture.md#web-push)). Nothing to set up when `VESPER_ORIGIN` is https:
the origin is the VAPID contact; set `VESPER_PUSH_SUBJECT=mailto:<you>` to give push services an address instead ([env.md](env.md)).

- **Outbound only.** The server opens HTTPS connections to `fcm.googleapis.com`, `updates.push.services.mozilla.com`,
  `*.push.apple.com` and `*.notify.windows.com` on port 443. A firewall that blocks outbound traffic from the VM must allow those. The
  Cloudflare tunnel is not involved: it carries only inbound requests, and the browser's subscription call is a normal `/api` request.
- **What the push services see:** that a notification went to a browser, when, and its size. Not who sent it, which room, or any text:
  the payload is encrypted for the browser.
- **Keep the clock right (NTP on).** Each VAPID token is valid for 12 hours from the server's clock. A clock off by more than that makes
  push services refuse it (401 or 403), and the server then deletes those subscriptions: people have to turn notifications on again.
- **iPhone and iPad:** only from Vesper added to the Home Screen (Share, Add to Home Screen), iOS 16.4 or later. Not verified.

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

- The pinned base image digests (read from Docker Hub's registry API on 2026-11-02 and checked against the sha256 of each index; no image was pulled).
- `docker build` / `docker compose up`, the image's healthcheck, non-root and read-only operation, the `VESPER_BUILD_ID` build arg and `build-id.txt` inside the image build.
- The automatic backup before a migration on the live data dir (tested on a temporary dir with an old-schema database: `crates/vesper-server/tests/cli.rs`), and starting an older image on a restored `pre-*` copy.
- The Dockerfile's `cargo install wasm-bindgen-cli` and wasm build inside the image.
- `compose.yaml` was read, not parsed by compose; `vesper.service` passes `systemd-analyze verify` apart from the binary not existing, and its hardening options were not run (check `MemoryDenyWriteExecute` and `SystemCallFilter` with a real start).
- Dockge deploy, router forward, split DNS, Caddy, certificates.
- The phone-on-cellular connectivity check.
- The server's headers through Caddy and Cloudflare (Cloudflare can add or change headers).
- The `data` bind mount, `create-owner` and `invite` through `docker compose exec`, the `systemd-run` form for the binary install, the backup command through `docker compose exec`, its cron line, the restore steps, and the "Upgrade from the shared key" steps.

Verified locally: the release server serves `web/dist` (index, SPA fallback, `application/wasm`, `text/javascript`, `nosniff`, dotfiles and `..` traversal refused) and `/health`/`/ws` still work (`crates/vesper-server/tests/static_files.rs`).
