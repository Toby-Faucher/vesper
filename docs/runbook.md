# Vesper runbook

For the operator. Setup details: [deploy.md](deploy.md). Every env var: [env.md](env.md). This page is what you do when you run it and when it breaks.
Nothing here was run against a real deploy (no Docker on the dev machine): treat the commands as not verified until you have used them once. Last updated 2026-11-01.

Shape: one server (`vesper-server`). HTTP/WebSocket on TCP 8080, media on UDP 40000. Caddy does HTTPS in front of 8080. Media never goes through Caddy. Accounts, sessions and rooms live in one SQLite database in the data dir; restarting loses only the live calls.

## Where things are

| Thing | Docker (Dockge VM) | systemd (plain binary) |
|---|---|---|
| Config | `.env` next to `compose.yaml` (`chmod 600`) | `/etc/vesper/vesper.env` (mode 600, root) |
| Logs | `docker compose logs vesper` | `journalctl -u vesper` |
| Health | `curl -fsS http://127.0.0.1:8080/health` prints `ok` | same |
| Data dir (database) | `data/` next to `compose.yaml`, mounted at `/data`, owned by uid 10001 | `/var/lib/private/vesper` (`StateDirectory`) |
| Admin CLI | `docker compose exec vesper vesper-server <command>` | see [deploy.md](deploy.md#docker--dockge) (`systemd-run`) |
| Binary / image | image `vesper:latest`, or the tag in `VESPER_TAG` | `/opt/vesper/vesper-server`, previous one as `.prev` |

CLI commands: `create-owner` (one-use owner link, only while no owner exists), `invite [--role member|moderator] [--uses N] [--hours H]`, `backup <dir>`, `help`. They print the link on stdout; it is a secret, send it only to the person it is for. Details: [deploy.md](deploy.md#first-owner-and-invites).

Log level: set `RUST_LOG` (default `info`), for example `RUST_LOG=vesper_server=debug,info`, then recreate the container or restart the unit.

## Start, stop, status

Docker, from a clone of the repo (the `-f deploy/compose.yaml` form; in Dockge use the stack's own compose.yaml and drop `-f`):

    docker compose -f deploy/compose.yaml up -d      # start
    docker compose -f deploy/compose.yaml stop       # stop
    docker compose -f deploy/compose.yaml ps         # want: healthy
    docker compose -f deploy/compose.yaml logs -f --tail 100 vesper

In Dockge the same buttons exist (Start, Stop, Restart); Logs shows the same output.

systemd:

    sudo systemctl start|stop|restart vesper
    systemctl status vesper
    journalctl -u vesper -n 100 --no-pager

Quick end-to-end check from outside the LAN (phone on cellular): open the site, join, open Diagnostics, press Test connection. `ok: true` means signaling and the UDP path both work.

## Update

Coming from the shared-key build: follow [deploy.md "Upgrade from the shared key"](deploy.md#upgrade-from-the-shared-key) once instead. In short: tag the image, `mkdir -m 700 data && sudo chown 10001:10001 data`, drop `VESPER_DEV_KEY` from `.env` and add `VESPER_ORIGIN=https://vesper.tfaucher.com`, `up -d --build`, `docker compose exec vesper vesper-server create-owner`, then send invites. The Proxmox snapshot must include `data/` from then on.

1. On the host: `git pull`. Back up first: `docker compose exec vesper vesper-server backup /data/backups` ([deploy.md](deploy.md#backup)).
2. Tag what runs now, so you can go back: `docker tag vesper:latest vesper:previous`.
3. `docker compose -f deploy/compose.yaml up -d --build`. Everyone connected is dropped and rejoins.
4. `docker compose -f deploy/compose.yaml ps` wants `healthy`, and `curl -fsS http://127.0.0.1:8080/health` wants `ok`.
5. Open the site from a second device and join.

The first build is slow (it compiles the denoiser to wasm). Later builds reuse Docker layers.

Binary install: copy the new binary next to the old one, then

    cp /opt/vesper/vesper-server /opt/vesper/vesper-server.prev
    install -m 755 new-vesper-server /opt/vesper/vesper-server.new && mv -T /opt/vesper/vesper-server.new /opt/vesper/vesper-server
    systemctl restart vesper
    curl -fsS http://127.0.0.1:8080/health

Replace the web app by copying the new `web/dist` to `/opt/vesper/web.new`, then `mv /opt/vesper/web /opt/vesper/web.old && mv /opt/vesper/web.new /opt/vesper/web`. The server reads files on each request, so this needs no restart.

## Roll back

Docker: start the image you tagged before the update. No rebuild. Use the `deploy/compose.yaml` of that same version, because compose only passes the variables it lists (not verified).

    docker image ls vesper
    git checkout <old> -- deploy/compose.yaml
    VESPER_TAG=previous docker compose -f deploy/compose.yaml up -d

Binary: `mv -T /opt/vesper/vesper-server.prev /opt/vesper/vesper-server && systemctl restart vesper`.

A shared-key image runs without the data dir and leaves it alone; an accounts image keeps using it. A database migrated by a newer build may not open in an older one; restore the backup in that case ([deploy.md](deploy.md#backup)).

Prune old images only when disk is short: `docker image prune`. Keep the `previous` tag.

## When something breaks

Start with: `ps` shows healthy? `/health` says `ok`? Logs for the last 100 lines? Then match the symptom.

| Symptom | Likely cause | Check and fix |
|---|---|---|
| Page loads, join works, no audio, Test connection fails | UDP 40000 not reaching the server | Router forwards UDP 40000 to the Docker host (UDP, not TCP). Host firewall allows it. `docker compose ps` shows `0.0.0.0:40000->40000/udp`. Test from cellular first. |
| One person is silent, or the room says `No sound from your microphone` | The mic sends digital silence: wrong device, hardware mute, or no input reaches the denoiser | Open Diagnostics on their machine. Input level silent or many missing input blocks: fix the device or its mute switch. Underruns or drops rising while input blocks keep arriving: the denoiser cannot keep up; Copy report and file it. |
| The room says `The audio pipeline stopped` | The denoiser stopped reporting for over 2 s, or the capture context is not running | Leave and join again. If it repeats, Copy report from Diagnostics and file it. |
| Works on cellular, no audio from the LAN | Router has no hairpin NAT | LAN clients send media to the public IP and need loopback to it. Test from outside; treat LAN to LAN as unsupported or turn hairpin NAT on. |
| No audio for anyone; Diagnostics shows a LAN or Docker address | `VESPER_PUBLIC_HOST` resolves to the wrong address from the server | It must resolve to the public IP as the server sees it. Use the literal IP or a public-only name, not a name your LAN DNS overrides. Check with `docker compose exec vesper getent hosts <name>`. Fix `.env`, `up -d`. |
| Log says `VESPER_PUBLIC_HOST unset and UDP bound to ...` | Variable missing | Set it in the env file, restart. |
| It worked, then stopped working for new calls after the ISP changed your IP | Public IP changed | The name is re-resolved every 60 s, but running calls keep the old address. New joins recover by themselves; ask everyone to rejoin. |
| `/health` returns 503 `sfu down`, log has `voice engine (SFU thread) died` | The media thread crashed | The process exits non-zero and Docker or systemd restarts it within seconds. If it repeats, keep the log lines before it and stop the stack. Check `docker compose ps` for a restart loop. |
| Container restarts forever, log says `vesper-server: ...` and exits | Bad config (bad bind address or `VESPER_ORIGIN`, port in use) | Read the first log line. Fix the env file. An empty `VESPER_UDP_PORT_PUBLIC` is a parse error: remove the line instead. |
| Site loads blank or 404 | Web app missing from the image or `VESPER_WEB_DIR` wrong | Log has `.../index.html not found`. Rebuild the image. |
| Browser says it cannot use the microphone | Not a secure context, or permission denied | Use the HTTPS name or localhost, never the bare IP. Click the padlock, allow the microphone, reload. Another app may hold the device. |
| The app cannot connect, the browser shows the WebSocket refused with 403 | `VESPER_ORIGIN` is not the address in the browser bar | Set it to that address (for example `https://vesper.tfaucher.com`). `up -d` recreates the container with the new value. |
| 502 from Caddy, or WebSocket fails only through the proxy | Caddy cannot reach 8080 | If Caddy is on another host, the compose mapping must be `8080:8080`, not `127.0.0.1:8080:8080`. Test `curl http://<vm>:8080/health` from the Caddy host. |
| Join works, then the call drops after about 10 seconds | Proxy idle timeout | The server closes a socket only when it is silent for 2 x 25 s (the ping interval). Check the proxy does not cut idle WebSockets. |
| Log says it cannot open or create the database (permission denied) | The `data` dir is not owned by uid 10001 | `sudo chown 10001:10001 data && chmod 700 data`, then `up -d`. |
| Nobody can sign in after a move to a new VM | The data dir was not copied or not in the snapshot | Restore `data/` from the backup ([deploy.md](deploy.md#backup) has the steps). Without it every account is gone: run `create-owner` again and send new invites. |
| Disk filling on the VM | Container logs or old images | Compose caps logs at 3 x 10 MB. `docker image prune` for images. |

## Before you ask for help

Collect: `docker compose ps`, `docker compose logs --tail 200 vesper`, the git commit (`git rev-parse --short HEAD`), what the Diagnostics panel says, and whether the client was on the LAN or cellular. Session cookies and invite links are not in the logs; do not paste the env file.
