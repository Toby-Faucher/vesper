# Theme plan: Game Night

- **Status:** Planned, not started.
- **Date:** 2026-11-02 (written against v1.4.0).
- **Releases:** follow the release notes convention in [versions.md](../versions.md#how-a-release-is-cut).

This is a plan, not a promise. Nothing here is built. Sizes are S, M or L and say nothing about time.

## Goal and the story

Show your screen and its sound to the call, with voice still first.

The whole group on a game or film evening: "Sam shares the game tab, everyone sees it and hears it, and Sam's voice still comes through clean."

## Why now

It is the one big capability Vesper lacks, and it has the strongest demand found in other open-source chat projects. Reaction counts are from the GitHub search API on 2026-11-01:

- Stoat [#313](https://github.com/stoatchat/stoatchat/issues/313) "Video Calling & Screen Share": 71 reactions (closed). A comment there says it is the only reason a group has not switched.
- Element Web [#29891](https://github.com/element-hq/element-web/issues/29891) "Screensharing should share audio on selected window": 199, open.
- Element Call [#3657](https://github.com/element-hq/element-call/issues/3657) "Desktop audio share": 20, open.
- Mattermost Calls [#158](https://github.com/mattermost/mattermost-plugin-calls/issues/158) "Increase Screen Sharing Quality": 6.

The ground work is done. v1.4 trimmed the first load, put the call code in its own chunk, takes a backup before each migration and makes open tabs reload after an upgrade (see [versions.md](../versions.md)). So a release that changes the wire protocol a lot can now ship without stranding open tabs.

The design spec lists screen share as "[Toby: v2]" and video as a non-goal ([spec](../superpowers/specs/2026-10-30-vesper-design.md)). The first decision below is about that line.

## Features

| # | Feature | Size |
|---|---|---|
| 1 | **Screen share, one sharer per room, video only.** VP8. Viewers opt in with "Watch", so nobody gets video they did not ask for. | L |
| 2 | **Share sound with the screen.** A second Opus track from `getDisplayMedia`, never sent through DeepFilterNet, stereo for that track only. | M |
| 3 | **Honest share status.** "This browser shares no sound" when the stream has no audio track. The sharing badge clears on `ended` and when the transport drops. | S |
| 4 | **Voice keeps priority.** The share sender gets a `maxBitrate` cap, a `contentHint` ("detail" for text, "motion" for games) and a lower priority than the mic. The share drops before voice does. | S |
| 5 | **Viewer controls.** Share volume apart from the person's voice, full screen, "Stop watching". | S |
| 6 | **Moderator "Stop share",** enforced in the SFU like force-mute. | S |
| 7 | **(Decision) Share a tab's sound only:** music to the room with no video. | S on top of #2 |

## What each needs

Checked against the code on 2026-11-02.

| # | Migration | Dependency | SFU change | CSP and headers | Size (gzip) |
|---|---|---|---|---|---|
| 1 | None | None new (str0m 0.24.1 has VP8) | Yes, the main work. See below. | CSP: none (a `<video>` with `srcObject` needs no source). Add `display-capture=(self)` to `Permissions-Policy`. | Call chunk or its own lazy chunk, about 6 to 9 kB. First load: only the sidebar badge. |
| 2 | None | None | A second audio mid per sharer | None | +1.5 kB lazy |
| 3 | None | None | None | None | Small, lazy |
| 4 | None | None | None | None | Small, lazy |
| 5 | None | None | None | None | +1 kB lazy |
| 6 | None | None | Drop the sharer's video and share-audio mids at the forward point | None | Small, Admin or person card |
| 7 | None | None | None beyond #2 | None | None beyond #2 |

What the code says today:

- **The SFU is audio only.** `room.rs` builds every session with `.clear_codecs().enable_opus(true, false)`, which turns all video codecs off. `client.rs` accepts only `MediaKind::Audio` in `MediaAdded`, and adds only audio media when it forwards a track. Video needs a new branch in both places, keyframe requests forwarded to the sharer (throttled, as the str0m [chat example](https://github.com/algesten/str0m/blob/main/examples/chat.rs) does), and a send bitrate cap.
- **Remote streams are keyed by user id.** The server's `user_id` is also the MSID stream id of a forwarded track, and `web/src/lib/signaling.ts` and `peers.ts` key remote streams on it (see CLAUDE.md). A second and third track per person (video, share sound) must be told apart by track, not by stream.
- **Protocol.** `Peer` in `crates/vesper-protocol/src/lib.rs` has `muted`, `deafened` and `force_muted`. Sharing adds a field (`sharing`) and `start_share` and `stop_share` messages. All additive.
- **Force-mute** is a set of muted users checked at the forward point in `room.rs` (`Command::SetMuted`), driven by `voice.rs`. "Stop share" can follow the same path.
- **Headers.** `with_security_headers` in `crates/vesper-server/src/lib.rs` sends `Permissions-Policy: microphone=(self), camera=(), geolocation=()`. `display-capture` is not listed. The Screen Capture spec gives it a default allowlist of self, so it should work as is (not tested). Listing it makes the intent plain; `tests/static_files.rs` and [ADR 0008](../decisions/0008-csp.md) change with it.
- **str0m upgrade.** It was planned together with RED. v1.4 measured RED under burst loss and did not ship it ([versions.md](../versions.md)). So stay on 0.24.1 unless the spike in the first slice shows a need (for example a BWE fix).

### Size, with v1.4.0 numbers

Measured on `main` at v1.4.0 (`npm run build`, `scripts/size-check.mjs`, gzip): first load 56.6 kB against a ratchet of 56.9 (hard cap 70), largest lazy chunk 14.7 (Settings, cap 35), call chunk 9.2, all JS 149.9 against 170.

- **All JS:** this theme adds about 8 to 11 kB of lazy code. That fits in the 20 kB of headroom, but it is about half of it, and Inside Jokes and Locks and Ledger want the rest.
- **First load:** only 0.3 kB is left under the ratchet. The Share button and the viewer belong in the call chunk (the call bar is already there), so they cost nothing on first load. The sidebar "sharing" badge is first load: keep it to a word and an icon, or raise the ratchet on purpose (a decision below).
- **Call chunk:** 9.2 kB plus about 8 to 11 kB stays under the 35 kB cap. Its own chunk, loaded on Share or Watch, keeps a voice-only join as light as today.

## Risks

- **Bandwidth.** One share at 2.5 Mbit/s watched by 11 people is about 27.5 Mbit/s up from the host. The share bitrate is an assumption and the arithmetic is simple multiplication. Opt-in watching and the cap bound it. The host's uplink decides the real limit.
- **Privacy.** The browser's picker may offer the whole screen first. Prefer tab or window (`monitorTypeSurfaces`, Chrome 119 and later).
- **Echo.** A sharer on speakers who shares tab sound plays the game into their own mic. Chrome's echo cancellation should handle it. Not verified.
- **SFU.** Video brings NACK, RTX, keyframes and bandwidth estimation, none of which run today. The force-mute SFU test is already flaky once in a full run ([#113](https://github.com/Toby-Faucher/vesper/issues/113)), and "Stop share" builds on the same path.
- **Browsers.** Share sound works in Chrome only, and on Linux and macOS only for a tab ([MDN browser-compat-data](https://github.com/mdn/browser-compat-data/blob/main/api/MediaDevices.json), `getDisplayMedia.audio_capture_support`). Firefox and Safari: no share sound.
- **Spec.** The spec marks screen share v2 and video a non-goal.

## Decisions for Toby

The all-JS budget is already decided: it was raised from 150 to 170 kB on 2026-11-02.

- [ ] Do screen share now, ahead of the spec's "v2" label? (yes/no)
- [ ] If yes, does that make it v2.0.0? (yes = v2.0.0, no = v1.x; [versions.md](../versions.md) calls additive protocol changes MINOR)
- [ ] Allow "share a tab's sound only" (it is close to the "no watch-together" line)? (yes/no)
- [ ] Accept that Firefox and Safari users can share video but no sound? (yes/no)
- [ ] Raise the first-load ratchet by up to 0.5 kB for the sidebar sharing badge, if it does not fit in 0.3 kB? (yes/no)

## Definition of done

- SFU unit test: a VP8 track from a fake sharer reaches the viewers who chose Watch, and no one else. A viewer's keyframe request reaches the sharer.
- e2e in headless Chromium with a fake display stream: share, watch, stop, a dropped transport clears the badge, a moderator stops the share.
- A manual real-browser check in Chrome and Firefox on two machines, written down.
- A loopback mouth-to-ear delay measurement ([latency.md](../testing/latency.md)) with and without a share running. Voice delay grows by less than a number agreed before the test (10 ms is the starting proposal).
- The size check passes, and the PR states all five numbers.
- `Permissions-Policy` test and ADR 0008 updated.

## Not included

Camera video, simulcast (one source, few viewers), more than one sharer per room, recording, remote control, share sound in Firefox (the platform has none) and a "Go Live" for people outside the call. Camera video, watch-together and recording stay spec non-goals.

## First slice that ships alone

Features #1 and #3: one sharer, video only, Watch opt-in and an honest badge. No share sound yet. Start with a spike that sends a VP8 track through the SFU in its unit test, to learn whether str0m 0.24.1 is enough.

## Sources

- Stoat [#313](https://github.com/stoatchat/stoatchat/issues/313), Element Web [#29891](https://github.com/element-hq/element-web/issues/29891), Element Call [#3657](https://github.com/element-hq/element-call/issues/3657), Mattermost Calls [#158](https://github.com/mattermost/mattermost-plugin-calls/issues/158). Seen 2026-11-01.
- str0m [`RtcConfig`](https://docs.rs/str0m/latest/str0m/struct.RtcConfig.html) (VP8, VP9, H.264 and AV1 are on by default) and the [chat example](https://github.com/algesten/str0m/blob/main/examples/chat.rs) (forwards video and keyframe requests; its README says it is not meant for production).
- MDN [`getDisplayMedia`](https://developer.mozilla.org/en-US/docs/Web/API/MediaDevices/getDisplayMedia): the stream "might contain no audio track even when `audio` is `true`".
- [MDN browser-compat-data](https://github.com/mdn/browser-compat-data/blob/main/api/MediaDevices.json): `audio_capture_support`, `windowAudio`, `monitorTypeSurfaces`.
- Chrome [screen sharing controls](https://developer.chrome.com/docs/web-platform/screen-sharing-controls): audio is offered with tabs and windows, not with screens.
- [Screen Capture spec](https://w3c.github.io/mediacapture-screen-share/) (the `display-capture` policy and its default allowlist).
- Mumble [#5626](https://github.com/mumble-voip/mumble/issues/5626) and [#2829](https://github.com/mumble-voip/mumble/issues/2829) (stereo music, for #7).
- In this repo: `crates/vesper-sfu/src/room.rs`, `crates/vesper-sfu/src/client.rs`, `crates/vesper-server/src/lib.rs` (headers), `crates/vesper-server/src/voice.rs`, `crates/vesper-protocol/src/lib.rs`, `web/scripts/size-check.mjs`, [dev.md](../dev.md) (what is in the first load).
