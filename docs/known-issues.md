# Known issues

Things found in review that were left on purpose, because they are small, rare or wait on a bigger piece of work.
Last updated 2026-11-01. Nothing here was run in a headed browser or a real deploy (the e2e runs headless Chromium). Fix an item, then delete its line in the same commit.

## Security and abuse

- **Plain HTTP connections before the upgrade are not limited.** A slow client can hold a connection on `/ws` or `/health`. The cap of 256 sockets counts upgraded sockets only.
- **An upload must average about 44 KB/s (350 kbit/s).** The 10 minute total limit means a 25 MiB file on a slower uplink fails near
  the end with `upload too slow`.
- **Only upload bodies have a read timeout** (30 s idle, 10 minutes in all). A slow client can still hold a JSON request open, but its body is
  at most 32 KiB.
- **Uploads in flight can pass the quota on disk for a while.** The quota is checked again under a lock before a file is kept, so it is never
  passed for good, but each upload streams to `uploads/tmp` first: up to 25 MiB per upload in flight, so about 250 MiB for one user's burst
  of 10. A `Content-Length` that cannot fit is refused before any byte is read.
- **A request cancelled during the upload row insert can leave a row with no file.** It counts against the quota until the 24 h sweep
  removes it (it is never attached, since the uploader's request failed).
- **Search timing shows a little about hidden rooms.** Matches in rooms you cannot see are skipped one by one while the index is walked,
  so many newer hidden matches make a search slower (measured 2 ms against 20 ms at 100k messages). A very weak signal at this scale,
  and the search limit (10, then 1 a second) slows any probing.
- **A very large `VESPER_MAX_SOCKETS` panics at start.** Keep it in the hundreds.
- **The client address trusts headers from private peers.** When the TCP peer is loopback or private, the server takes a valid `CF-Connecting-IP`,
  else the last `X-Forwarded-For` entry, else the peer; from any other peer it ignores both. Through Cloudflare this is the real client, because
  Cloudflare overwrites `CF-Connecting-IP`. If the app is ever reached on a private network without Cloudflare (the HTTP port open on the LAN, or
  Caddy without the tunnel), a LAN client can set `CF-Connecting-IP` itself and dodge or spend another address's login and invite limits. The
  login limit per username still allows each address only 5 tries for one name, then 1 a minute. Unit-tested (`client_ip` in `auth.rs`); not verified on the deploy.
  If Caddy ever drops `CF-Connecting-IP`, every client looks like the tunnel's address and all share one per-IP limit; the per-username limit
  still holds. Keep Caddy's default headers ([deploy.md](deploy.md#network)).
- **Password reset is by hand.** There is no email. The owner (or a moderator, for a member) makes a one-use reset link
  in Admin > Members and sends it; the owner's own way back in is `vesper-server reset-password` on the host
  ([deploy.md](deploy.md#password-recovery)). Whoever holds the link can set the password, so it must reach the right
  person. The reset page does not say whose account it is for.
- **A moderator can take over a member's account through a reset link.** They make a link for the member (typing their
  own password), use it themself and sign in as that member. It is not silent: the member is signed out everywhere and
  their password stops working; the owner's Members table lists every reset used in the last 30 days with who made it;
  the server log has both account ids. The taker sees the "your password was reset" notice first, so the member is told
  only after they get back in through a new link (that reset shows the earlier ones again). The owner can give the member
  a new link and ban the moderator, which ends every live link the moderator made. The owner can do the same to anyone.
- **Uploads are not scanned for viruses.** Images are served inline only after their magic bytes match, everything else is a
  download with `Content-Disposition: attachment` and a sandbox CSP, but the file itself is passed on as uploaded.
- **Rooms cannot be deleted, only archived.** An archived room's messages and files stay in the database and count toward disk use. Its search index rows stay too.
- **No passkeys yet.** Sign in is username and password only (plan Task 6, a stretch goal, not built).
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

- **Web Push is verified with a fake push service only.** Google, Mozilla, Apple and Microsoft were not reached; a notification on screen
  and a click on it were not seen (headless Chromium). iOS needs Vesper on the Home Screen. Desktop Firefox gets push but cannot install
  Vesper as an app (no manifest install in Firefox). There is no offline mode: the service worker caches nothing.
- **Past 256 pending sends, pushes are dropped**, with a debug log line only. A storm of mentions loses some notifications rather than delaying them all.
- **"Started voice" pushes are limited in memory.** A server restart forgets which people got one in the last 10 minutes.
- **A push can arrive while a tab is open but asleep.** A push goes only to accounts with no socket open, but a tab whose socket dropped
  (a laptop waking up) counts as closed until it reconnects.

- **"Your current password is not right." relies on the server's message.** A wrong current password is a 403 with the generic `forbidden` code; the client tells it apart from other 403s by the message text "wrong password". A server wording change would show the plain forbidden text instead.
- **Image boxes change shape when the image loads.** `Attachment` has no width or height, so the timeline keeps a 360 x 280 box and resizes it on load; the reader's place is kept, but rows under an image in view can move.
- **The keyboard, focus and motion checks run in headless Chromium only.** Safari does not focus a clicked button (the image viewer focuses it itself); other focus returns are not checked there.

- **A very slow first visit can still fall back.** The denoiser files (about 23 MB) start downloading when the page loads, and the worker waits 20 s after Join. A link that cannot fetch them in that time gets browser suppression for that call.
- **Not tried in real Chrome or Firefox.** Firefox may refuse a mic and AudioContext rate mismatch, which forces the browser fallback every time.
- **The noise gate and the limiter work in DeepFilterNet mode only.** In browser and off mode the raw mic track is sent; the gate switch is disabled there. After an automatic fallback to browser mode the setting stays on but does nothing. With the gate on, the Settings meter shows the gated sound, so the noise under the marker is not visible; turn the gate off to set the marker.
- **Soft word starts under the marker are muted.** The gate opens on the first frame above the marker, so a quiet "s" or "h" before it can lose up to about 1.5 dB over the first 20 ms (measured on corpus speech).
- **The automatic gate threshold does almost nothing after DeepFilterNet.** The floor under the shipped denoiser is near digital silence, so the threshold sits at its 3e-6 RMS clamp. On dev-0016 and dev-0022 it cut no noise at all (the marker cut 5.9 dB and all of it); speech and onsets were unchanged. It is an option, off by default.
- **The gate and limiter are not blind-listened.** They were measured on synthetic signals and on denoised corpus speech (see [dev.md](dev.md#noise-gate-and-limiter-lab-check)), not heard by Toby.
- **DTX applies at the next join.** The client cannot renegotiate it mid-call.
- **Retry microphone is tested with a fake device only.** The e2e ends Chromium's fake mic by hand; a real unplug, a revoked permission and an OS reclaim were not tried.
- **Stale mode reason on unmute.** The capture watcher can replay an old fallback reason after an unmute.
- **After a failed join, the page's guard against messages from an old socket stays set for a short window.** It clears when the next join starts.
- **No fade on dropped output frames.** An underrun fades out and back in over 2.5 ms, but a frame dropped from a full queue (above 80 ms, or the trim) is cut, which can click.
- **Boost is lost when the mode falls back to the browser**, and the status line says so.
- **A track swap can leave the new track out of sync with mute or push to talk for one tick.** Room re-applies the state after the mode change.
- **Firefox may fire a button click on Space key-up** after push to talk captured the key.
- **Stored per-person volumes are never pruned.** `navigator.locks` missing only logs a warning.
- **Unhandled edge cases:** a teardown that races a joining call can leave a poll timer running, and an `AudioContext` constructor that throws is not caught. Both are rare.
- **The connection test reports only the local candidate type.**
- **Chat history is trimmed only at the bottom.** A room keeps its newest 500 messages while it is read at the bottom (older ones load again on scroll up). A reader who scrolls far up keeps every page loaded until they come back down. Rooms visited earlier stay in memory, capped the same way, until the next reconnect.

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
