# Known issues

Things found in review that were left on purpose, because they are small, rare or wait on a bigger piece of work.
Last updated 2026-11-10. Nothing here was run in a headed browser or a real deploy (the e2e runs headless Chromium). Fix an item, then delete its line in the same commit.

## Security and abuse

- **Plain HTTP connections are not capped in number.** Each one is closed after 10 s without a full request head, but there is no limit per
  address; the cap of 256 sockets counts upgraded sockets only.
- **There is no response timeout.** A client that reads a download (up to 25 MiB) very slowly keeps its connection until it is done.
- **An upload must average about 44 KB/s (350 kbit/s).** The 10 minute total limit means a 25 MiB file on a slower uplink fails near
  the end with `upload too slow`.
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
- **Passkeys are new and only tested with a software authenticator.** The server tests use a P-256 key made in the test, not a real
  authenticator or browser, so a real phone, security key or password manager is not verified. Passkeys are sign in only: the password
  stays and is the recovery path.
- **Passkeys stay when a password is reset.** A reset ends every session but does not remove passkeys, so someone who holds a passkey
  keeps being able to sign in. Remove it in Settings. A banned account cannot sign in with a passkey (it is refused at sign in), and
  deleting the account row deletes its passkeys. At most 10 per account. Passkeys need a domain name in `VESPER_ORIGIN`
  (an IP address host turns them off). Challenges are held in memory for 5 minutes, so a restart drops one in flight and the person
  tries again. The login page asks for the passkey with the browser's own dialog; there is no autofill (conditional UI) sign in.
  No attestation is checked (any authenticator is accepted).
- **A moderator disconnect does not keep anyone out.** The sessions stay valid and the client may reconnect at once; a ban is the way to keep someone out.

## Server behaviour

- **One lock serialises joins.** It is held while the SFU joins and while an old session closes (up to 2 s), for every room at once. Fine for 12 people. A signed-in user can slow others by rejoining in a loop, up to the socket limit of 5 messages a second.
- **A voice join that races a role change can keep access until the next change.** The join reads the role before it takes the join lock, and the change takes the user out of voice only if they were in it by then. The window is a few milliseconds. A room change in that window is caught: the join checks the newest copy of the room before it enters the user.
- **A second join refused by the SFU ends the first call.** The room cap, a dead SFU and a malformed offer are refused while the first call stays up. An offer that passes those checks but that the SFU still refuses, or a room change caught at the last check, comes after the old session ended, since the SFU keys sessions by user.
- **A socket keeps the role it had at the upgrade until the next user update.** A role change between the upgrade and the hub add reaches that socket's room filter only with the next change to that user.
- **A display name change does not reach voice presence** until the user joins again (`peer_joined` carries the name from the join).
- **An open socket outlives a session that expires.** A session unused for 90 days stops working for requests, but a socket opened on it stays until it closes. Logout, revoke, password change and ban do close it.
- **Admin > Server does not show the advertised address or its age.** The server does not record when the address last changed.
- **The Admin > Server push failure count lives in memory.** A restart starts it at 0.
- **The app badge from the service worker is a plain mark, not a number.** The worker does not know the count; the page sets the number when it opens.
- **A changed public IP needs a rejoin.** The name is re-resolved every 60 s, but only new sessions use the new address. A failed lookup at start advertises nothing until the next refresh, with a warning every 60 s.
- **Without `VESPER_PUBLIC_HOST`, `VESPER_UDP_PORT_PUBLIC` is ignored**, and a `0.0.0.0` bind is advertised as is, with a warning.
- **The ping interval (25 s) and the socket queue (256 messages) have no environment variable.**
- **Static files:** a missing extensionless path under `/assets/` returns `index.html` with the `immutable` cache header. Replies 206 and 304 under `/assets/` get `no-cache`. Base images in `deploy/Dockerfile` are pinned by digest (v1.4), so they get no security updates until someone bumps the digests on purpose. Precompressed `.br` and `.gz` files (v1.4) are served only for files read from `web/dist`; the SPA fallback page (`index.html` for a path like `/r/1`) goes out as it is. Whether this gains anything behind the Cloudflare tunnel, which may compress too, is not measured.
- **The SFU wakes every 10 ms when idle.** Cheap, but it is not zero.
- **Tests:** the rejoin race test is a regression guard, not a proof (the lock order is the proof).
- **A room's shares are capped in total.** Each share counts its bitrate once per other person in the call, and the sum may not pass 120 000 kbps (for example 4 people sharing 10 Mbps each). A share that would go over is refused with "Too many screens are being shared right now." The number is the highest tested load that held voice at 95% in the [video load test](testing/latency.md#video-load) on two cores. It is checked only when a share starts, so people who join later raise the total without a re-check. It scales with the number of viewers, so a 12-person room with one 10 Mbps share (110 000) is allowed, though only 4-person rooms were measured. It is measured on a Core i7-12700, not on the production VM. The suggested bitrate stops at 10 Mbps, so four people can share at the suggestion.
- **Screen share has one video layer for every viewer.** There is no simulcast. A viewer on a bad link asks for keyframes, and the SFU forwards each request with no throttle, so the other viewers pay a little for them.
- **Screen share carries no sound.** Game audio and tab audio are not sent. Firefox's `getDisplayMedia` captures no audio at all (from docs: Mozilla bug 1541425).
- **An exclusive fullscreen game can capture as a black picture.** Use borderless windowed mode. Not tried with a real game.
- **Screen share is VP8 only, and a client that joined without a video m-line cannot watch.** The video m-lines are fixed at join, so an audio-only client gets none and must rejoin to watch.
- **A share past 35 Mbps for 5 s is cut.** The SFU counts a second above 35 Mbps; one second at or under resets the count. Then it sends `ShareCut` and the share ends.
- **A viewer's watch list belongs to the viewer.** It survives the sharer stopping, restarting or leaving, so a new share from the same person is watched again without a click. `watch off` always applies.
- **Pointer messages use the per-socket budget** (5 messages a second shared with everything else the socket sends), not a limit of their own.
- **Firefox caps the share bitrate by video size** (10, 5 or 2.5 Mbps) and a page can only lower it. See the [design](superpowers/specs/2026-11-09-screen-share-design.md#firefox-zen-is-firefox).
- **Firefox has no pop-out button in the share panel.** `requestPictureInPicture` is missing there, so the button is hidden. Firefox's own pop-out on a video works but the page cannot see it. Where ours is missing, a hidden tab still counts as watching, so a sharer keeps encoding for a Firefox viewer who is not looking. Firefox's own pop-out closes when the viewer leaves the room page.
- **Not verified:** Firefox or Zen in a real share, a real network, a real call, the Dockge VM and Windows browsers. The e2e (`web/e2e/share.mjs`) runs three headless Chromium pages with a stubbed canvas screen; one joins after the share started.

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

- **A very slow first visit can still fall back.** The denoiser files (about 10 MB, see Denoiser) start downloading when the page loads, and the worker waits 20 s after Join. A link that cannot fetch them in that time gets browser suppression for that call.
- **Not tried in real Chrome or Firefox.** Firefox may refuse a mic and AudioContext rate mismatch, which forces the browser fallback every time.
- **The noise gate and the limiter work in DeepFilterNet mode only.** In browser and off mode the raw mic track is sent; the gate switch is disabled there. After an automatic fallback to browser mode the setting stays on but does nothing. With the gate on, the Settings meter shows the gated sound, so the noise under the marker is not visible; turn the gate off to set the marker.
- **Soft word starts under the marker are muted.** The gate opens on the first frame above the marker, so a quiet "s" or "h" before it can lose up to about 1.5 dB over the first 20 ms (measured on corpus speech).
- **In Chromium the hand-over from the browser filter to DeepFilterNet opens the mic a second time.** The old track runs until the new one has sound, so there is no gap, but the new track's echo canceller starts from scratch, so echo can leak for a moment if others talk then. A browser that asks for mic permission each time could ask again. When the second track is silent or fails, both filters stay stacked for the call (`swap skipped: ...`). Not heard in a real browser; not tried on Edge on Windows, where the first silent reopen was seen.
- **The switch from the passthrough to DeepFilterNet is a cut.** When the worker connects, the worklet goes from the passed-through mic to silence until 30 ms are queued, then fades in; the step to silence is not faded. It predates the underrun fade.
- **The gate has no automatic threshold.** Fluxer's rule (open above 3 times the 10th percentile of frame RMS over 2 s) was tried for #104 and not shipped: after DeepFilterNet the floor is near digital silence, so the threshold sat at its 3e-6 RMS clamp and the gate never closed. Gate lab check on dev-0016 and dev-0022: noise between words cut 0.0 dB (the marker: 5.9 dB and all of it), speech and onsets unchanged. The marker stays the only threshold.
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
- **Your recent calls misses a call that does not end in the app.** The summary is posted when the call ends in the page (Leave, a kick, a
  dropped socket, sign out); closing the tab or a crash loses it, and so does a failed post (not retried). Loss, jitter and round trip are
  averages of the 2 s polls; the concealed share counts only the people still in the call at the last poll. "Browser filter while it loaded"
  means the load took at least 1 s. The reason is the last fallback or mic problem of the call, also when it recovered.
- **The connection test reports only the local candidate type.**
- **Chat history is trimmed only at the bottom.** A room keeps its newest 500 messages while it is read at the bottom (older ones load again on scroll up). A reader who scrolls far up keeps every page loaded until they come back down. Rooms visited earlier stay in memory, capped the same way, until the next reconnect.
- **The light scheme was checked on screen on the component gallery only** (headless Chromium, `/_gallery?primitives`), not on the shell, a call, the call bar, toasts, images or markdown. Contrast is checked by numbers in `tokens.test.ts` and `accent.test.ts`. The scheme choice and the first-paint script were not tried in Firefox or Safari.
- **Persona's slanted buttons lose the slant while focused**, so the focus ring is not cut by the clip. Its fonts are the latin subset: names with other scripts fall back to the system face. Cozy and BBS use system faces, so they look different per OS.
- **The styles (Persona, Cozy, BBS, Aurora) were looked at on the component gallery and a mock sidebar with a dialog only** (headless Chromium, dev server, fake rooms), not on the real shell, messages, the call bar, search, settings pages or an image. Contrast is checked by numbers for each style (`tokens.test.ts`). Not verified: the Persona spring, tilt and hover motion (seen as still frames), Firefox and Safari (`clip-path`, `@starting-style`, `linear()`), how the lazy CSS and fonts load on a slow link (the page waits at most 2 s, then shows Vesper dark), and the accent colours other than the default under each style.

## Direct messages

- **The ring prompt and sound are not verified in a browser.** The server logic has integration tests; the prompt, the focus move and the tone were not seen or heard. A browser that has had no click or key on the page yet may keep the tone silent (autoplay policy); the prompt still shows.
- **A ring is not repeated.** If the other person misses the 30 s, only the push (when no tab was open) and the sidebar indicator are left, until the caller leaves and joins again.
- **A tab that is open gets no push.** The in-page notification (when the tab is hidden and permission is granted) is the only alert then, and it needs the mentions switch (or the direct messages switch, for a DM).
- **Not end-to-end encrypted.** The server's owner can read the database.
- **The sender's picture in a push is not verified in a browser.** The worker fetches it as a data URL; any failure shows the default icon.
- **Not verified:** DMs and DM voice in a real browser. The server side is covered by `tests/dms.rs` and unit tests; the web side by unit tests and `svelte-check` only.

## Denoiser

- **The first download is about 10 MB:** the wasm is 10.9 MB, sent as a 2.2 MB gzip copy (a 1.3 MB brotli copy where the browser accepts it, from `npm run build`), plus the 8.0 MB model, which is already gzip. It was 23.3 MB with the wasm sent as is. The model was never inside the wasm: the linker drops the unused copy, so leaving out `deep_filter`'s `wasm` feature saved only 4 kB. The 4.4 MB name section was the waste. The rest is tract's code. The gzip copy needs a browser that sends `Accept-Encoding: gzip`, and Caddy and Cloudflare must pass `Content-Encoding` through: not verified on the live host. The model is the larger part now and is not smaller.
- **No panic hook in the wasm**, so a Rust panic shows as an opaque error. The build drops the name section, so a wasm stack trace shows function numbers, not names.
- **Added delay is not measured.** The output queue starts at 30 ms and may grow to 80 ms, on top of the algorithm's lookahead. From the code we estimate about 30 ms more than before the larger buffers. The [checkpoint](testing/checkpoint.md) must measure it.
- **`ndarray` and the `tract-core` pin in `vesper-dsp` are not optional**, although only the `tract` feature needs them. A featureless build pulls them in for nothing.
- **The no-sound warnings work only in deepfilter mode.** In browser or off mode, also after a fallback, there are no pipeline numbers, so neither warning shows.
- **The output queue only grows after jitter.** After a bad patch it can sit at 60 to 80 ms until it drops a frame. Diagnostics shows it as `Queue`.
- **Thresholds are untuned:** the fallback trips at 20 late frames out of 100, with a 30 ms budget, after a 3 s warm-up. The numbers come from one laptop (2.1 ms average per frame there). The fallback is one-way for the rest of the call.

## Build and deploy

- **`scripts/build-dsp.sh` writes straight into `web/public/dsp`.** If `wasm-bindgen` fails half way, the folder is partial.
- **The load test assumes 100 clock ticks per second** when it computes server CPU, and reads `/proc`, so it is Linux only. The docs say so.
- **Not verified:** the Docker build, Dockge, the router forward, Caddy, and the systemd hardening options. See [deploy.md](deploy.md#not-verified).
