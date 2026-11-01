# Known issues

Things found in review that were left on purpose, because they are small, rare or wait on a bigger piece of work.
Last updated 2026-11-01. Nothing here was run in a real browser or a real deploy. Fix an item, then delete its line in the same commit.

## Security and abuse

- **Plain HTTP connections before the upgrade are not limited.** A slow client can hold a connection on `/ws` or `/health`. The cap of 256 sockets counts upgraded sockets only.
- **A very large `VESPER_MAX_SOCKETS` panics at start.** Keep it in the hundreds.
- **The client address trusts headers from private peers.** When the TCP peer is loopback or private, the server takes a valid `CF-Connecting-IP`,
  else the last `X-Forwarded-For` entry, else the peer; from any other peer it ignores both. Through Cloudflare this is the real client, because
  Cloudflare overwrites `CF-Connecting-IP`. If the app is ever reached on a private network without Cloudflare (the HTTP port open on the LAN, or
  Caddy without the tunnel), a LAN client can set `CF-Connecting-IP` itself and dodge or spend another address's login and invite limits. The
  login limit per username still allows each address only 5 tries for one name, then 1 a minute. Unit-tested (`client_ip` in `auth.rs`); not verified on the deploy.
  If Caddy ever drops `CF-Connecting-IP`, every client looks like the tunnel's address and all share one per-IP limit; the per-username limit
  still holds. Keep Caddy's default headers ([deploy.md](deploy.md#network)).
- **Force-mute is enforced by the client only.** The server marks the user muted and tells everyone, but the SFU still forwards their audio,
  so a modified client can keep talking. A moderator then disconnects them, or the owner bans them. A server restart lifts every force-mute.
- **A moderator disconnect does not keep anyone out.** The sessions stay valid and the client may reconnect at once; a ban is the way to keep someone out.

## Server behaviour

- **One lock serialises joins.** It is held while the SFU joins and while an old session closes (up to 2 s), for every room at once. Fine for 12 people. A signed-in user can slow others by rejoining in a loop, up to the socket limit of 5 messages a second.
- **A voice join that races a role change can keep access until the next change.** The join reads the role before it takes the join lock, and the change takes the user out of voice only if they were in it by then. The window is a few milliseconds. A room change in that window is caught: the join checks the newest copy of the room before it enters the user.
- **A second join refused by the SFU ends the first call.** The room cap, a dead SFU and a malformed offer are refused while the first call stays up. An offer that passes those checks but that the SFU still refuses, or a room change caught at the last check, comes after the old session ended, since the SFU keys sessions by user.
- **A socket keeps the role it had at the upgrade until the next user update.** A role change between the upgrade and the hub add reaches that socket's room filter only with the next change to that user.
- **A display name change does not reach voice presence** until the user joins again (`peer_joined` carries the name from the join).
- **An open socket outlives a session that expires.** A session unused for 90 days stops working for requests, but a socket opened on it stays until it closes. Logout, revoke, password change and ban do close it.
- **A changed public IP needs a rejoin.** The name is re-resolved every 60 s, but only new sessions use the new address. A failed lookup at start advertises nothing until the next refresh, with a warning every 60 s.
- **Without `VESPER_PUBLIC_HOST`, `VESPER_UDP_PORT_PUBLIC` is ignored**, and a `0.0.0.0` bind is advertised as is, with a warning.
- **The ping interval (25 s) and the socket queue (256 messages) have no environment variable.**
- **Static files:** a missing extensionless path under `/assets/` returns `index.html` with the `immutable` cache header. Replies 206 and 304 under `/assets/` get `no-cache`. Base images in `deploy/Dockerfile` are not pinned by digest, and assets are not precompressed.
- **The SFU wakes every 10 ms when idle.** Cheap, but it is not zero.
- **Tests:** the rejoin race test is a regression guard, not a proof (the lock order is the proof).

## Web client

- **A very slow first visit can still fall back.** The denoiser files (about 23 MB) start downloading when the page loads, and the worker waits 20 s after Join. A link that cannot fetch them in that time gets browser suppression for that call.
- **Not tried in real Chrome or Firefox.** Firefox may refuse a mic and AudioContext rate mismatch, which forces the browser fallback every time.
- **The VAD threshold in Settings is display-only.** Nothing gates on it. DTX still drops silence. The UI says so. See [decision 0006](decisions/0006-quality-checkpoint-gate.md).
- **DTX applies at the next join.** The client cannot renegotiate it mid-call.
- **A mic that ends mid-call has no "Retry microphone" button.** The status line says what happened, but you leave and rejoin to recover.
- **Stale mode reason on unmute.** The capture watcher can replay an old fallback reason after an unmute.
- **After a failed join, the page's guard against messages from an old socket stays set for a short window.** It clears when the next join starts.
- **No fade on underrun or drops.** A starved output can click.
- **Boost is lost when the mode falls back to the browser**, and the status line says so.
- **A track swap can leave the new track out of sync with mute or push to talk for one tick.** Room re-applies the state after the mode change.
- **Firefox may fire a button click on Space key-up** after push to talk captured the key.
- **Stored per-person volumes are never pruned.** `navigator.locks` missing only logs a warning.
- **Unhandled edge cases:** a teardown that races a joining call can leave a poll timer running, and an `AudioContext` constructor that throws is not caught. Both are rare.
- **The connection test reports only the local candidate type.**

## Denoiser

- **The wasm is about 15 MB** because the `wasm` feature of `deep_filter` embeds a second copy of the model. A build on `deep_filter/tract` plus `getrandom` with the `js` feature should halve it. Not tried.
- **No panic hook in the wasm**, so a Rust panic shows as an opaque error.
- **Added delay is not measured.** The output queue starts at 30 ms and may grow to 80 ms, on top of the algorithm's lookahead. From the code we estimate about 30 ms more than before the larger buffers. The [checkpoint](testing/checkpoint.md) must measure it.
- **`ndarray` and the `tract-core` pin in `vesper-dsp` are not optional**, although only the `tract` feature needs them. A featureless build pulls them in for nothing.
- **The no-sound warnings work only in deepfilter mode.** In browser or off mode, also after a fallback, there are no pipeline numbers, so neither warning shows.
- **The output queue only grows after jitter.** After a bad patch it can sit at 60 to 80 ms until it drops a frame. Diagnostics shows it as `Queue`.
- **Thresholds are untuned:** the fallback trips at 20 late frames out of 100, with a 30 ms budget, after a 3 s warm-up. The numbers come from one laptop (2.1 ms average per frame there). The fallback is one-way for the rest of the call.

## Build and deploy

- **`scripts/build-dsp.sh` writes straight into `web/public/dsp`.** If `wasm-bindgen` fails half way, the folder is partial.
- **The load test assumes 100 clock ticks per second** when it computes server CPU, and reads `/proc`, so it is Linux only. The docs say so.
- **Not verified:** the Docker build, Dockge, the router forward, Caddy, and the systemd hardening options. See [deploy.md](deploy.md#not-verified).
