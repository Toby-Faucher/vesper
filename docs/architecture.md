# Architecture

One page on how the parts fit. Goals: [the design spec](superpowers/specs/2026-10-30-vesper-design.md). Decisions: [decisions/](decisions/README.md).
Deploying: [deploy.md](deploy.md). Variables: [env.md](env.md). Running and testing: [dev.md](dev.md).

Status: written from the code on 2026-10-30. Statements are about what the code does, not about what a browser or a deploy does.
The "Not verified" list at the end says what was only read.

## Processes

```
 browser                                          one machine
┌───────────────────────────┐   WebSocket /ws    ┌─────────────────────────────────────┐
│ Svelte UI (web/)          │◄──────────────────►│ vesper-server (axum)                │
│ mic ─► worklet ⇄ DSP      │   (via Caddy)      │   rooms.rs  presence, session ids   │
│ worker (DeepFilterNet     │                    │   ws.rs     one task per socket     │
│ WASM) ─► Opus (WebRTC)    │◄═ UDP 40000 ═════► │   vesper-sfu  own thread, str0m     │
└───────────────────────────┘   media, direct    └─────────────────────────────────────┘
```

- **vesper-server** is one binary. It serves the built web app, `/health` and `/ws` on one TCP port, and runs the SFU on one UDP port.
  It does no TLS; Caddy does. Media never goes through Caddy. (`crates/vesper-server/src/lib.rs`, `deploy/Caddyfile.example`)
- **The SFU** runs on its own OS thread (`vesper-sfu` thread, `crates/vesper-sfu/src/lib.rs`). The server talks to it through a cloneable
  `SfuHandle`. If the thread dies, `/health` returns 503 and the process exits with code 1 so Docker or systemd restarts it
  (`watch_sfu` in `lib.rs`).
- **The browser** captures the mic, cleans it with DeepFilterNet in WASM, and sends Opus over WebRTC (`web/src/lib/audio/`, `signaling.ts`).

## Crates

| Crate | Job |
|---|---|
| `vesper-protocol` | Wire types only: `ClientMsg`, `ServerMsg`, `Peer`, `ErrorCode`, and the limits `MAX_NAME_CHARS` (32) and `MAX_MSG_BYTES` (64 KiB). `ts-rs` generates `web/src/lib/gen/*.ts` from them. `lib.rs` |
| `vesper-sfu` | Audio-only SFU on str0m. One UDP socket, rooms keyed by name, Opus forwarded as is (no decode, mix or transcode). `room.rs` is the loop, `client.rs` one session, `lib.rs` the public handle. No signaling inside. |
| `vesper-server` | `config.rs` (env), `lib.rs` (router, static serving, `/health`, SFU watcher), `ws.rs` (one socket), `rooms.rs` (presence and session numbers), `public_addr.rs` (advertised address). |
| `vesper-dsp` | Wrapper over DeepFilterNet3 on the `tract` runtime, 480-sample (10 ms, 48 kHz) frames. Native for tests (feature `tract`), WASM for the browser (feature `wasm`). `scripts/build-dsp.sh` builds the WASM. |
| `vesper-loadtest` | Synthetic speakers over WebSocket and WebRTC, for load and loss checks. See `docs/testing/latency.md`. |

## Message flow

All messages are JSON on one WebSocket, tagged by `type` (`vesper-protocol/src/lib.rs`).

Client to server: `join { room, name, key, offer }`, `answer { sdp }`, `set_muted { muted }`, `leave`.
Server to client: `joined { user_id, answer, peers }`, `offer { sdp }`, `peer_joined`, `peer_left`, `peer_muted`, `error { code, message }`.
Error codes: `bad_key`, `room_full`, `bad_message`.

1. The client opens `/ws`. A socket that does not join within `VESPER_JOIN_DEADLINE_SECS` is closed. Five failed pre-join messages also close it
   (`ws.rs`, `MAX_PREJOIN_ERRORS`).
2. The client builds an SDP offer first, then sends `join` with the key, the name and that offer (`web/src/lib/signaling.ts`).
3. `Rooms::join` (`rooms.rs`) checks the key, trims and validates room and name (1 to 32 chars, no control characters), takes the join lock,
   replaces any earlier session of the same user, checks the room cap (12), then asks the SFU to join with the offer.
4. The server replies `joined` with the SFU's SDP answer and the current peers, and broadcasts `peer_joined` to the rest of the room.
   `joined` is queued under the members lock, so every later presence message for that room follows it.
5. When tracks are added or stopped, the SFU makes a new offer. It arrives as `offer`; the client answers with `answer`, one at a time.
6. `set_muted` updates presence and broadcasts `peer_muted`. `leave` or a closed socket removes the user and broadcasts `peer_left`.
7. The server pings every 25 s and drops a socket silent for two periods (`ws.rs`).

## Identity, sessions and the MSID rule

- There are no accounts. One shared key (`VESPER_DEV_KEY`) gates every join. A user is identified by display name.
- `user_id` is the first 16 bytes of sha256(name) as 32 lowercase hex characters (`rooms.rs` `user_id`). Hex keeps it a valid MSID stream id
  (RFC 8830).
- The SFU labels each forwarded track with the sender's `user_id` as its MSID stream id (`vesper-sfu/src/client.rs`, `TrackIn.user`). In the
  browser, `ontrack` gives `MediaStream.id` equal to that `user_id`, and the client keys remote streams on it (`signaling.ts`).
- A second join with the same name replaces the first. Each join gets a fresh `session` number (`next_session`, `rooms.rs`). `answer`,
  `set_muted` and `leave` carry the socket's session and are ignored if it is no longer current, because the SFU keys sessions by user alone.
  SFU offers for a stale session are not forwarded to the new socket.
- Joins are serialised by one lock, so a rejoin finishes closing the old SFU session (up to a timeout) before anyone else joins.

## SFU ownership

- The SFU thread owns the UDP socket, every str0m `Rtc` client and all media state. Nothing else touches them. The server sends it commands
  (`join`, `answer`, `leave`, `set_advertised`) over a channel and gets `SfuEvent::Offer` and `SfuEvent::Closed` back per session.
- `Rooms` owns presence: who is in which room, muted or not, and the socket for each session. It does not touch media.
- A session ends on ICE timeout, an error, an offer unanswered for `answer_timeout` (10 s), `leave`, or a new join by the same user
  (`SfuEvent` docs, `vesper-sfu/src/lib.rs`).
- Advertised address: with `VESPER_PUBLIC_HOST` set, the server resolves it at start and every 60 s and calls `set_advertised`. A failed
  resolve keeps the old address. Only sessions started afterwards use a changed address, so people in a call rejoin after the public IP changes
  (`public_addr.rs`).

## Audio pipeline (browser)

Order, fixed by the spec: microphone, browser echo cancellation (always on), DeepFilterNet, gain, Opus encode, WebRTC to the SFU.

- In `deepfilter` mode: mic into an AudioWorklet, which exchanges 10 ms frames with a DSP worker over a MessageChannel; worklet output goes
  through a gain node into a `MediaStreamDestination`, and that track is what WebRTC sends (`web/src/lib/audio/capture.ts`, `denoise.worklet.ts`,
  `dsp.worker.ts`, `dsp-handler.ts`).
- Buffers in `denoise-core.ts`: up to 4 frames at the worker, a 4 frame input ring, and an output queue that starts playing at 3 frames
  (30 ms) and holds at most 8 (80 ms). They absorb jitter in the worklet to worker messages. The delay they add is estimated, not measured.
- In `browser` and `off` modes the raw mic track is sent, so the fallback does not depend on the AudioContext or the worklet. Browser noise
  suppression is on only in `browser` mode, or in `deepfilter` mode if the user turned it on.
- Opus is tuned by editing the SDP (`web/src/lib/audio/opus.ts`): `maxaveragebitrate`, `usedtx`, `useinbandfec`, mono, ptime. The send bitrate is
  also set on the sender (`web/src/lib/devices.ts`). Remote SDP is tuned too, then applied (`signaling.ts`).

## Fallback logic

- Modes: `deepfilter`, `browser`, `off` (`web/src/lib/audio/mode.ts`).
- `ModeController` trips from `deepfilter` to `browser` when 20 of the last 100 frames each took longer than 30 ms. The first 300 frames (about 3 s) are ignored. It stays tripped until `reset()`.
  The numbers are set in `denoise-core.ts`.
- A load failure, a missing model or a worker that stalls also switches to `browser`, with a reason string the UI shows
  (`capture.ts` `fail` and `switchTo`). The call keeps going. The track is swapped in place and `replaceTrack` is used on the sender.
- The worker gets 20 s to report `ready` (`READY_TIMEOUT_MS` in `capture.ts`). The raw mic passes through, and the status line says `loading, unfiltered`, until then.
- The page starts fetching the denoiser files (`web/src/lib/audio/preload.ts`, called from `main.ts`) when it loads, before Join. The server answers `/dsp/*` with `no-cache`, so the worker's later fetch is a revalidation. Skipped when the browser asks to save data. Not verified in a real browser.
- The server side has no fallback. A dead SFU is not restarted in process; the process exits and the supervisor restarts it.

## Static serving

`guard` in `vesper-server/src/lib.rs`: dotfiles and `%2e` are 404; an extensionless 404 gets `index.html` (SPA routes); a missing file with
an extension stays 404; `/assets/*` is `immutable`, everything else `no-cache`; `nosniff` on all. Tests: `crates/vesper-server/tests/static_files.rs`.

## Not verified

Read from the code on 2026-10-30, not run:

- Behaviour in real Chrome and Firefox, including the `ontrack` stream id and `replaceTrack` during a mode swap.
- Everything in the SFU loop beyond its doc comments (`room.rs` forwarding, `client.rs` negotiation).
- Text chat, accounts and rooms beyond a name: not built (spec "Build order").
