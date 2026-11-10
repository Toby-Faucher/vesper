# Screen share

Status: design, not built. Written 2026-11-09 against `release/1.6.1` at `f8a9136`.

How each fact below is known:
- **verified**: read in the code on that commit, or in the browser's source where it says so.
- **measured**: seen with the test page (see "Test page" at the end) on the machine named.
- **from docs**: read in browser docs, bug trackers or caniuse. Not run.
- **not verified**: believed, not checked.

## Why

Toby and three friends play games together in Vesper voice calls, Terraria most often. Terraria is 2D pixel art with small text and
item counts. When they shared screens on Discord's free tier, the stream was 720p at 30 fps with a low bitrate. Downscaling blurred the pixel
art, and scrolling smeared. They could not read what they were being shown.

They have far more bandwidth than that stream used. Toby's connection (which also hosts the server) is 1 Gbit down and 500 Mbit up. One friend has
1 Gbit fiber. One has 100/20 cable. A shared screen should look like the sharer's own screen.

## What Toby asked for (2026-11-09)

- Screen share in group voice rooms. Not in DMs.
- A shared screen at full size and 60 fps, sharp enough to read small text.
- Each sharer picks their own quality. Vesper suggests a bitrate for their connection and links to a speed test to check it.
- Everyone's shares visible at once in a panel, with three layouts (grid, focus and strip, dock) the viewer can switch between.
- Per share: pixel-perfect full screen (1:1), zoom, a pop-out window, a "look here" pointer, and hide.
- A share nobody watches stops sending.
- Game audio is not needed.

## Goals

1. Anyone in a group voice call can share a screen or window. Several people can share at once, one share each.
2. A share is sent at the screen's own size (never rescaled to a fixed 1080p) at up to 60 fps, at a bitrate the sharer picks.
3. The sharer sees what they are sending: a preview, size, fps, bitrate and how many people are watching.
4. Viewers see every share in one panel and can switch between three layouts. The choice is remembered on that browser.
5. Viewers can open any share at 1:1 full screen, zoom into part of it, pop it out, point at a spot, or hide it.
6. A share with no watchers stops encoding and sending until someone watches again.
7. Voice keeps working while someone shares. A sharer's voice must not break up because of their own share.

## Non-goals

- Screen share in DMs. The server refuses it and the DM view has no share button.
- Game or system audio with a share.
- Webcams and any other video.
- Recording, clips or instant replay. The original design spec rules out recording.
- Remote control of a shared screen.
- Sharing part of a screen.
- Sizes above the sharer's screen size, and simulcast (a second, smaller copy of each share).
- Phones and mobile layouts.
- A moderator button to stop someone's share.
- Limits for large calls. The group is four people. See "Large calls" below.
- Watch together. It is parked as its own project. See "Parked".

## Anti-goals

These make the feature a failure even if the picture looks good:

- **A share hurts voice or the game's connection.** If the cable friend's voice breaks up or their game ping spikes while they share, it failed.
- **A share reaches someone who is not in that call.** Only people in the same call may receive it.
- **Someone shares without knowing.** The sharer must always see that they are sharing, with a Stop button in view.

## Constraints

### The group's browsers

| Person | OS | Browser | Engine |
|---|---|---|---|
| Toby | Arch Linux, Wayland | Zen 154 | Firefox |
| Friend | Nobara Linux | unknown | unknown |
| Friend | Windows | Brave | Chromium |
| Friend | Windows | Edge | Chromium |

### Firefox (Zen is Firefox)

- **Firefox caps the bitrate by video size, and a page can only lower it.** In `VideoStreamFactory.cpp`, `SelectBitrates` takes the
  table maximum, then applies `MinIgnoreZero` with the encoding's `maxBitrate` and with the negotiated cap. So `maxBitrate` can lower the
  ceiling, never raise it (verified in Firefox source on searchfox, `firefox-main`, 2026-11-09). The table:

  | Size sent (in 16x16 blocks) | Ceiling |
  |---|---|
  | 1920x1200 and above | 10 Mbps |
  | 1280x720 up to below 1920x1200 (this includes 1920x1080) | 5 Mbps |
  | 800x480 up to below 1280x720 | 2.5 Mbps |

  Measured: at 1728x1080 with `maxBitrate` 20 Mbps, Zen sent about 3 Mbps and dropped to 34 fps during heavy scrolling. Not yet measured:
  whether 1920x1200 reaches 10 Mbps.
- **Capture and encode hold 60 fps.** Zen 154 on Arch (Wayland, PipeWire portal) captured and sent about 59 fps of 1728x1080, VP8 in software, at
  7 to 9 ms encode time per frame (measured; desktop scrolling, not a game).
- **Missing APIs and stats.** `MediaStreamTrack.contentHint` is not supported (measured). `degradationPreference` is accepted (measured; whether
  it is honoured is not verified). `getStats` gives no `encoderImplementation`, `qualityLimitationReason`, `powerEfficientEncoder` or
  `availableOutgoingBitrate` (measured). `HTMLVideoElement.requestPictureInPicture()` does not exist (from docs: caniuse). Firefox has its own
  pop-out button on videos. `getDisplayMedia` captures no audio at all (from docs: Mozilla bug 1541425).

### Chromium (Brave, Edge)

- Honours `maxBitrate` above its defaults (not verified in source). Its default ceiling for a 1080p stream, if the page sets nothing, is about
  2.5 Mbps (not verified).
- Reports `availableOutgoingBitrate`, `qualityLimitationReason` and encode stats (from docs).
- Supports `contentHint` and `requestPictureInPicture()` (from docs).
- Brave's WebRTC privacy setting may block connections. Voice already works for the group, so this is probably fine (not verified).

### Bandwidth

- The server sends one copy of each share to every viewer. Four people, everyone sharing at 20 Mbps: each viewer receives up to 60 Mbps and the
  server sends up to 240 Mbps of Toby's 500 up. It fits.
- The cable friend has 20 Mbit up. Their voice and the game share it, and cable upload gets laggy near full. Keep their share near 10 Mbps.

### Vesper today (verified at `f8a9136`)

- **SFU is audio only.** `vesper-sfu` uses str0m 0.24. `room.rs` builds each `Rtc` with `.clear_codecs().enable_opus(true, false)`.
  `SfuHandle::join` requires exactly one audio m-line and returns `JoinError::BadOffer` for anything else. `client.rs` only opens a
  `TrackIn` for `MediaKind::Audio`.
- **The SFU drives renegotiation.** After join, offers go from the SFU to the client (`SfuEvent::Offer`), the client answers
  (`SfuHandle::answer`), and no new offer is made while one is outstanding (`answer_timeout`, 10 s).
- **MSID rule.** A forwarded track's MSID stream id is the sender's `user_id`, and the web client keys remote streams on it
  (`signaling.ts`, `peers.ts`, `docs/architecture.md`). A screen track from the same user must be told apart from their voice track
  without changing the `user_id` format.
- **DMs are rooms** with `rooms.kind = 'dm'`. `RoomKind` is in `vesper-protocol` and `voice.rs` already branches on it.
- **Wire types** live in `crates/vesper-protocol`. `web/src/lib/gen/*.ts` is generated by `ts-rs`.
- **Rate limits** live in `crates/vesper-server/src/limits.rs` (`Kind`).
- **Headers.** `Permissions-Policy` is `microphone=(self), camera=(), geolocation=()`. `display-capture` is not listed, so it defaults
  to `self` and capture is allowed. CSP has `media-src 'self' blob:`. A `MediaStream` set through `srcObject` is not a URL load, so CSP does
  not block it (not verified in a browser).
- **No `unsafe`**, toolchain pinned, wording rules: see `CLAUDE.md`.
- **One SFU thread.** Every room's media runs on the single SFU thread (`room.rs`), so video and voice share it. The only load test is 12 voice speakers:
  17.5% of one i7-12700 core over loopback, release build (`docs/testing/latency.md`). Video at the worst case below (about 80 Mbps in and 240 Mbps out,
  all SRTP) is far more packets than that.
- **Host** (verified over SSH, 2026-11-09). The `vesper` container runs on the Dockge VM at 10.0.0.163: 2 vCPU (KVM on a Ryzen 7 3700X),
  3.6 GiB RAM with about 800 MiB of swap in use, shared with about 20 other containers. The container has no CPU or memory limit, is on the `proxy`
  network, and publishes UDP 40000 through Docker's NAT. Vesper used 75 MiB and almost no CPU at rest. HTTP reaches it through Cloudflare,
  `cloudflared` and Caddy (`docs/deploy.md`). Media goes direct over UDP.
- **More vCPUs do not lift the SFU's ceiling.** The SFU is one thread, so extra cores only reduce contention with the other containers.
- **Rough estimate** (not verified, from the voice load test): that test moved about 7,200 packets per second at 17.5% of a 12700 core, roughly
  25 µs per packet. If video costs the same per packet, two shares at 10 Mbps to three viewers is about 8,000 packets per second (about the voice
  test), and four shares at 20 Mbps is about 33,000 (more than one 3700X core). So the usual case should fit and the worst case may not.

## Design

### Who can share, and where

- Anyone in a group room's voice call. One share per person at a time.
- The server refuses a share in a room with `RoomKind::Dm`, and the DM view shows no share button.
- A share ends when the sharer presses Stop, stops it from the browser's own control, leaves the call, or loses the connection.

### Sending

- `getDisplayMedia` with video only, no size constraint (so the screen's own size is kept), and a frame rate constraint of 30 or 60.
- Bitrate and frame rate go in the video sender's encoding: `maxBitrate`, `maxFramerate`, `scaleResolutionDownBy: 1`.
- **Sharp vs smooth.** "Keep text sharp" sets `degradationPreference: 'maintain-resolution'` and `contentHint: 'detail'`. "Keep motion smooth"
  sets `'maintain-framerate'` and `'motion'`. Each only where the browser supports it.
- **Voice first.** The audio sender gets the higher `priority` and `networkPriority`, the video sender the lower (where supported; not verified
  in Firefox). The suggested bitrate leaves room under the measured upload.
- **Codec: VP8 only, to start** (my call). It is the one measured in Firefox, and every browser in the group can send and play it. The SFU does not
  transcode, so all receivers must handle the sender's codec. Revisit H.264 if a Windows sharer's game FPS drops, since Chromium may encode it on
  the GPU.
- **Suggested approach for the sender's track** (builder may choose otherwise): the join offer always carries one video transceiver with no track.
  Starting a share is `replaceTrack(screen)` plus `setParameters`, stopping is `replaceTrack(null)`. The sharer never renegotiates, which fits the
  SFU-driven negotiation above. Old cached clients that offer audio only must still be able to join.

### Server

- **Accept one video m-line** next to the audio one, VP8 only.
- **Forward a share only to its subscribers.** A viewer subscribes to a share while it is visible in their panel, and unsubscribes when they hide it or
  close the panel.
- **Keyframes.** A new subscriber needs a keyframe right away, so the SFU asks the sharer for one on subscribe. Viewers' keyframe requests (PLI/FIR) go
  back to the sharer.
- **Feedback for the sharer's bitrate.** The sharer's browser only raises its bitrate if the SFU sends feedback on what arrives (transport-cc or
  REMB). Check what str0m 0.24 does by default and turn it on if needed. Without it a share may sit near its start bitrate.
- **One layer, no simulcast.** Every viewer gets the same stream. A second, smaller copy would cost the sharer more encoding, which is what hurts Linux
  sharers. Known cost: a viewer with a bad link can trigger keyframe requests that cost everyone a little. Fine for four people on fast links.
- **Watcher count.** The server tells the sharer how many people are subscribed. At zero, the sharer's client sets the encoding to `active: false`, which
  stops encoding and sending. When someone subscribes again, it sets `active: true` and the SFU asks for a keyframe.
- **Telling tracks apart.** The screen track keeps the sharer's `user_id` as its MSID stream id. The client tells it from voice by `kind == 'video'`.
  One share per person makes that enough.
- **Server ceiling per share** (my call). The SFU stops forwarding a share whose incoming rate stays above a fixed ceiling (about 35 Mbps, above the
  30 Mbps slider maximum) and tells the sharer. The browser's settings are the client's word; this is the check at the trust boundary.
- **Load test before shipping** (my call: measure first, then decide). Extend the existing latency test to video, release build, with voice loss and
  latency measured at the same time. Pass bar: voice stays within the existing latency test's limits. Two loads:
  - **Usual:** two sharers at 10 Mbps, each to three viewers. This must pass to ship.
  - **Worst:** four sharers at 20 Mbps, each to three viewers. If it fails, add a server cap on total video before shipping (refuse a new share or
    lower the ceiling while the total is high). Moving video off the voice thread is the larger fix, kept for if the usual load fails.
  - Run it on the Dockge VM, or on a 2 vCPU VM like it, and record where it ran.

### Wire messages

Names are the builder's. Needed:

- Start and stop sharing, and a broadcast of who is sharing (with the sharer's chosen size, fps and bitrate, so viewers can tell when a share is
  struggling).
- Subscribe and unsubscribe to a share.
- Watcher count to the sharer.
- Pointer: share owner, `x` and `y` as fractions of the frame, sent to the other people in the call. Rate limited with a new `limits.rs` kind.
- Errors for: sharing in a DM, sharing while not in the call.

### Suggested bitrate

- When the share settings open, the client runs a short upload test to the Vesper server: it posts a few MB to a new endpoint that discards the body and times
  it in the browser. The request goes through Cloudflare, not the UDP media path, so it measures the person's own upload rather than the exact route. That is
  the number that matters. (My call. Toby asked for an estimate, and Firefox exposes no bandwidth estimate.)
- The endpoint needs a session, the same `Origin` check as every `/api` change, a body size cap and a rate limit. It stores nothing.
- Suggested bitrate: about half the measured upload, capped by the browser's ceiling. A "Check your speed" link opens `https://speed.cloudflare.com`.
- **Browser ceiling.** Firefox: the size table above, for the size actually being captured. Chromium: no engine ceiling; cap the slider at 30 Mbps.
  For a Firefox sharer the dialog shows the ceiling and says a Chromium browser can go higher.

### Watching

- **Panel.** Every share the viewer has not hidden. Three layouts, switched from the panel header and remembered on that browser (`localStorage`).
  The default is Grid (my call).
  - **Grid:** shares fill the main area in equal tiles. Chat becomes a drawer.
  - **Focus and strip:** one big share, the others as thumbnails. Click one to swap it in. Chat becomes a drawer.
  - **Dock:** shares and chat side by side, with a divider to drag.
  - Vesper has no right-hand panel today: chat is the main area. That is why Grid and Focus and strip turn chat into a drawer.
- **Per share:**
  - **1:1 full screen:** Fullscreen API, with the video sized to `videoWidth / devicePixelRatio` CSS pixels so one video pixel is one screen pixel.
  - **Zoom:** magnify part of the share and drag to pan.
  - **Pop-out:** `requestPictureInPicture()` where it exists. In Firefox, hide the button. Firefox's own pop-out button works instead.
  - **Pointer:** click a spot, and a marker with the person's name and avatar colour shows on that share for the other viewers for about
    3 seconds. The sharer only sees it if they look at Vesper.
  - **Hide:** unsubscribe. A hidden share shows as a small row with a button to show it again.
  - **Stats on hover:** size, fps and bitrate, from the viewer's own receive stats.
  - **Struggling warning:** when received fps or bitrate stays well under what the sharer set, for a few seconds.
- **When the panel shows** (my call). The panel belongs to the call room's page. It opens by itself there when the first share in the call starts. On
  any other page (another room, a DM, Settings) the call bar shows "2 sharing", which goes back to the call room.
- **What counts as watching** (my call). A viewer watches a share while its tile is in the panel and the tab is visible, or while it is popped out.
  Otherwise they unsubscribe after a 10 second grace, so a quick alt-tab costs no keyframe wait. In the group's normal use (everyone in the game,
  Vesper behind it) a share stops encoding until someone looks, and a popped-out share keeps the sharer sending. A popped-out video must outlive the
  room page, so it cannot be owned by that page's component.
- **Your own share** sits in the call bar, not in a tile: preview, size, fps, bitrate, watcher count, Stop, and a gear for settings.
- **Voice list:** a screen icon next to each person sharing.
- **Sounds:** the existing sound cues play when a share starts and stops.

### Share settings

- Frame rate: 30 or 60. Default 60.
- Bitrate slider up to the browser ceiling, starting at the suggested value.
- "Check your speed" link.
- When the connection struggles: "Keep text sharp" (default, my call: the group's complaint was unreadable text) or "Keep motion smooth".
- Remembered on that browser.
- Then the browser's own screen picker opens.

### Large calls

Rooms hold up to 12 people. In a full room, two shares at 20 Mbps is 2 x 11 x 20 = 440 Mbps, nearly all of Toby's upload. Not handled now. If the
group grows, the place for a cap is the server's share start: refuse a new share past a per-room count, or lower the suggested bitrate as viewers
grow. Note this in `docs/known-issues.md`.

## Docs to change in the same work

- New decision record `docs/decisions/0009-video-in-the-sfu.md`: single layer, VP8 only, subscriptions, no simulcast, and why.
- `docs/architecture.md`: video forwarding, subscriptions, the MSID rule for the screen track, and the new endpoint and messages.
- `docs/known-issues.md`: Firefox bitrate ceilings, no game audio, exclusive fullscreen games can capture black, the single layer cost, no large call
  limits.
- `docs/env.md`: only if a variable is added. None is planned.
- `README.md`: the "Next" line says screen share is v2.

## How it gets checked

- **Rust:** SFU tests with `FakeClient` (`crates/vesper-sfu/src/testing.rs`, `tests/forwarding.rs`): an audio and video offer is accepted, an audio-only
  offer still works, video goes only to subscribers, a keyframe is requested on subscribe, and a share in a DM is refused. Protocol types regenerate
  `gen/`.
- **Web:** unit tests for the pure parts: the ceiling table, the suggested bitrate, the watch and hide state, the layout memory.
- **`scripts/check.sh`** passes.
- **Real browsers are not available on the dev machine.** The group runs one call where each person shares for two minutes in their own browser,
  and the result goes in this doc. Until then, the browser behaviour above is "measured" only for Zen on Arch.

## Left to the builder

- Wire message names and shapes.
- Whether viewers get a video m-line when a share starts or when they first subscribe.
- How str0m 0.24 is set up for feedback and keyframe requests.
- Stats polling interval, pointer rate limit, upload test size and endpoint path.
- When "struggling" triggers.

## Parked

- **Watch together.** A separate project after this one. Start from Cloudberry (Toby's shared radio): its server keeps a shared clock and every
  browser plays the same file on one timeline. Video instead of MP3 is the same idea. The original design spec lists watch-together as out of scope,
  so this is Toby's decision to change.
- **Limits for large calls.** See above.

## Test page

A throwaway page outside the repo, `/home/toby/Downloads/screen-test.html`. It shares the screen back to itself in one page and reports capture, encode
and decode stats, so each person can check their PC without a call. It does not test the network.

Results so far (Toby, Zen 154, Arch, 2026-11-09):

| Run | Max bitrate | Size | Avg fps | Lowest fps | Avg bitrate | Encode per frame |
|---|---|---|---|---|---|---|
| 1, desktop scrolling | 12 Mbps | 1728x1080 | 59.0 | 56.9 | 2.4 Mbps | 7.2 ms |
| 2, heavy scrolling, keep sharp | 20 Mbps | 1728x1080 | 56.9 | 34.0 | 3.4 Mbps | 8.8 ms |

Both runs captured at 1728x1080 because the page then asked for 1080 lines. It now captures at the screen's own size (Toby's is 1920x1200). Still to
run: Toby at 1920x1200, and the three friends.
