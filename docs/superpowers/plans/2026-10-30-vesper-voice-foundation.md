# Vesper Voice Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build voice rooms that sound good: a Rust SFU, a browser client with DeepFilterNet noise suppression in the capture path, user-tunable audio settings, diagnostics, and a quality checkpoint Toby signs off.

**Architecture:** One Rust binary (`vesper-server`, axum) serves a WebSocket for signaling and presence, and hands WebRTC sessions to an audio-only SFU (`vesper-sfu`, str0m, one UDP port). The Svelte client captures the mic, runs DeepFilterNet (`vesper-dsp` compiled to WASM) in a worker fed from an AudioWorklet, and sends Opus over WebRTC.

**Tech Stack:** Rust (stable, edition 2024), tokio, axum, str0m 0.24, serde, ts-rs, deep_filter (git rev `d375b2d`, `wasm` feature), wasm-bindgen; Svelte 5 + Vite + TypeScript, Tailwind, Bits UI, vitest; Docker + compose, Caddy.

**Spec:** `docs/superpowers/specs/2026-10-30-vesper-design.md`. This plan is build steps 1 and 2 plus the quality checkpoint. Accounts and rooms, text chat, and polish are separate plans written after the checkpoint, because its results change them. Reference code from the spike: `~/Projects/friendchat-spike/str0m/examples/chat.rs` (SFU loop, adapt audio-only) and `~/Projects/friendchat-spike/df-bench/` (WASM build and worker, `build.sh`).

## Global Constraints

- Workspace crates named `vesper-protocol`, `vesper-sfu`, `vesper-server`, `vesper-dsp`; every crate prefixed `vesper-` (crates.io `vesper` is taken).
- Desktop Chrome and Firefox only; no phone layouts, no light theme.
- SFU forwards Opus as separate streams: no decoding, mixing or transcoding. One UDP port for all media. No TURN.
- Opus 48 kHz, 20 ms frames, default 64 kbps mono, FEC on, DTX on at speech bitrates; bitrate user-configurable from 32 kbps up to music grade.
- One WebSocket per client carries signaling and presence, JSON messages.
- Capture order: mic, browser echo cancellation (always on), DeepFilterNet, gain, Opus. Browser noise suppression and auto gain default off when DeepFilterNet is on.
- DeepFilterNet3 built from upstream `Rikorose/DeepFilterNet` rev `d375b2d` with `RUSTFLAGS="-C target-feature=+simd128"`; the `wasm-bindgen` CLI version must equal the crate version in `Cargo.lock` exactly. Hop size 480 samples (10 ms).
- Voice room soft cap 12. Hostname `vesper.tfaucher.com`. No telemetry. Git repo private; do not commit model binaries over 10 MB without Toby's say-so (fetch script instead).
- Denoiser modes: `deepfilter`, `browser`, `off`; automatic fallback `deepfilter` to `browser` when processing falls behind real time.

## Review Focus

1. Same user joins twice (second tab, reconnect): newest session wins, no ghost in presence. Test in Task 4.
2. Client vanishes without leaving (tab killed, network dies): removed from presence within the ICE timeout and its tracks removed from others. Tests in Tasks 3 and 4.
3. Mic permission denied, no input device, or device unplugged mid-call: clear UI error, call stays alive, device can be switched. Test in Task 9.
4. DeepFilterNet fails to load or falls behind: call continues on browser suppression and the UI says so. Test in Task 8.
5. 13th person joins a full room, or sends malformed or oversized JSON: rejected with an error message, server does not panic or drop others. Test in Task 4.

## File Structure

```
vesper/
  Cargo.toml  rust-toolchain.toml  justfile  .gitignore
  crates/
    vesper-protocol/src/lib.rs        wire types, ts-rs export
    vesper-sfu/src/{lib,room,client}.rs   audio SFU over str0m
    vesper-sfu/tests/{support,forwarding}.rs
    vesper-server/src/{main,config,rooms,ws,public_addr}.rs
    vesper-server/tests/signaling.rs
    vesper-dsp/src/lib.rs             Denoiser + wasm bindings
    vesper-loadtest/src/main.rs       synthetic speakers
  web/                                Svelte app
    src/lib/{signaling,volume,diag,devices}.ts
    src/lib/audio/{opus,settings,ring,mode,capture}.ts
    src/lib/audio/{dsp.worker.ts,denoise.worklet.ts,rec.worklet.ts}
    src/lib/gen/                      generated protocol types
    src/routes/{Room,Settings,Diagnostics}.svelte
  scripts/{fetch-model.sh,build-dsp.sh}
  deploy/{Dockerfile,compose.yaml,Caddyfile.example,vesper.service}
  docs/testing/{latency.md,checkpoint.md}
```

---

### Task 1: Workspace and protocol types

**Files:** Create `Cargo.toml`, `rust-toolchain.toml`, `justfile`, `.gitignore`, `crates/vesper-protocol/{Cargo.toml,src/lib.rs}`.

**Interfaces:**
- Produces (`vesper-protocol`):
  - `pub struct Peer { pub user_id: String, pub name: String, pub muted: bool }`
  - `pub enum ErrorCode { BadKey, RoomFull, BadMessage }`
  - `pub enum ClientMsg { Join { room: String, name: String, key: String, offer: String }, Answer { sdp: String }, SetMuted { muted: bool }, Leave }`
  - `pub enum ServerMsg { Joined { user_id: String, answer: String, peers: Vec<Peer> }, Offer { sdp: String }, PeerJoined { peer: Peer }, PeerLeft { user_id: String }, PeerMuted { user_id: String, muted: bool }, Error { code: ErrorCode, message: String } }`
  - All `serde` internally tagged (`#[serde(tag = "type", rename_all = "snake_case")]`) and `ts_rs::TS`, exported to `web/src/lib/gen/`.
  - `pub const MAX_NAME_CHARS: usize = 32; pub const MAX_MSG_BYTES: usize = 64 * 1024;`

- [ ] **Step 1: Write failing tests** in `vesper-protocol/src/lib.rs`: `join_roundtrips_json` (serialize `ClientMsg::Join{..}`, assert JSON has `"type":"join"`, parse back equal); `unknown_type_is_error` (`{"type":"nope"}` fails to parse); `export_ts_bindings` (ts-rs generated test writes `web/src/lib/gen/*.ts`, assert file `ClientMsg.ts` exists).
- [ ] **Step 2:** `git init`, create the workspace (`cargo new` the crate), run `cargo test -p vesper-protocol`. Expected: FAIL (types missing).
- [ ] **Step 3:** Implement the types above. Workspace `Cargo.toml` pins shared dependency versions; `#![forbid(unsafe_code)]` in every Vesper crate.
- [ ] **Step 4:** `cargo test -p vesper-protocol`. Expected: PASS and `web/src/lib/gen/ClientMsg.ts` exists.
- [ ] **Step 5:** Commit `feat: workspace and protocol types`.

---

### Task 2: Denoiser crate (native)

**Files:** Create `crates/vesper-dsp/{Cargo.toml,src/lib.rs}`, `scripts/fetch-model.sh`.

**Interfaces:**
- Produces:
  - `pub const HOP: usize = 480;`
  - `pub struct Denoiser`
  - `Denoiser::new(model_tar_gz: &[u8], atten_db: f32, post_filter_beta: f32) -> Result<Denoiser, DspError>`
  - `Denoiser::process(&mut self, input: &[f32]) -> Result<Vec<f32>, DspError>` (input length must equal `HOP`)
  - `pub enum DspError { BadFrameLength { got: usize }, Model(String) }`
  - Cargo features: `tract` (native, tests), `wasm` (browser).

- [ ] **Step 1: `scripts/fetch-model.sh`** downloads `models/DeepFilterNet3_onnx.tar.gz` from the pinned upstream rev into `crates/vesper-dsp/model/` (gitignored) and prints its sha256; record the checksum in the script on first run and have the script verify it afterwards.
- [ ] **Step 2: Write failing tests** (`#[cfg(feature = "tract")]`): `rejects_wrong_frame_length` (`process(&[0.0; 100])` returns `BadFrameLength{got:100}`); `silence_stays_finite` (processing 100 frames of zeros yields only finite values, length `HOP` each); `reduces_white_noise` (500 frames of seeded white noise at about -30 dBFS: output RMS is below 0.5 times input RMS after the first 50 frames).
- [ ] **Step 3:** Run `cargo test -p vesper-dsp --features tract`. Expected: FAIL.
- [ ] **Step 4:** Implement `Denoiser` as a thin wrapper over `deep_filter` (git dep at rev `d375b2d`, `default-features = false`), using the crate's tract API to load the model from bytes. Spike result: speed is not a concern (about 1 to 2 ms per frame).
- [ ] **Step 5:** Run the tests. Expected: PASS. Commit `feat: denoiser wrapper`.

---

### Task 3: Audio SFU

**Files:** Create `crates/vesper-sfu/{Cargo.toml,src/lib.rs,src/room.rs,src/client.rs}`, `crates/vesper-sfu/tests/{support/mod.rs,forwarding.rs}`.

**Interfaces:**
- Produces:
  - `pub struct SfuConfig { pub bind: SocketAddr, pub advertised: Vec<SocketAddr> }`
  - `pub struct Sfu` with `Sfu::spawn(cfg: SfuConfig) -> std::io::Result<SfuHandle>` (runs the str0m loop on its own thread, as in the spike)
  - `pub struct SfuHandle` (cloneable): `async fn join(&self, room: String, user: String, offer_sdp: String) -> Result<JoinOk, JoinError>`, `fn answer(&self, user: &str, answer_sdp: String)`, `fn leave(&self, user: &str)`, `fn set_advertised(&self, addrs: Vec<SocketAddr>)`
  - `pub struct JoinOk { pub answer_sdp: String, pub events: tokio::sync::mpsc::UnboundedReceiver<SfuEvent> }`
  - `pub enum SfuEvent { Offer(String), Closed }`
  - `pub enum JoinError { BadOffer(String), Shutdown }`

- [ ] **Step 1: Write failing tests** in `tests/forwarding.rs` using `tests/support` (a fake client built from str0m `Rtc` talking over loopback UDP): `forwards_audio_between_two_clients` (A sends 50 Opus packets, B receives at least 45 with identical payload bytes, none transcoded); `sender_does_not_hear_itself`; `rooms_are_isolated` (a client in room "x" receives nothing from room "y"); `abrupt_disconnect_closes_client` (stop sending: `SfuEvent::Closed` arrives within the ICE timeout and the other client is offered an SDP that drops that track); `advertised_addresses_appear_as_candidates` (answer SDP contains every address from `SfuConfig::advertised`, including after `set_advertised`).
- [ ] **Step 2:** `cargo test -p vesper-sfu`. Expected: FAIL.
- [ ] **Step 3:** Implement by adapting the spike's `run`, `Client`, `propagate` and `negotiate_if_needed`. Audio only (drop keyframe and video code). SDP offers from the SFU go out as `SfuEvent::Offer`; the server returns answers through `answer()`. Rooms keyed by name; bind one UDP socket to `cfg.bind`.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit `feat: audio sfu`.

---

### Task 4: Server signaling, presence and public address

**Files:** Create `crates/vesper-server/{Cargo.toml,src/main.rs,src/config.rs,src/rooms.rs,src/ws.rs,src/public_addr.rs,tests/signaling.rs}`.

**Interfaces:**
- Consumes: `vesper_protocol::{ClientMsg, ServerMsg, Peer, ErrorCode, MAX_*}`, `vesper_sfu::{Sfu, SfuHandle, SfuConfig, SfuEvent}`.
- Produces:
  - `pub struct Config { pub http_bind: SocketAddr, pub udp_bind: SocketAddr, pub public_host: Option<String>, pub udp_port_public: u16, pub dev_key: String, pub max_room_size: usize }` loaded by `Config::from_env()` (`VESPER_HTTP_BIND`, `VESPER_UDP_BIND`, `VESPER_PUBLIC_HOST`, `VESPER_DEV_KEY`; `max_room_size` defaults to 12).
  - `pub struct Rooms` with `async fn join(&self, req: JoinReq) -> Result<JoinResult, ErrorCode>`, `fn leave(&self, user_id: &str)`, `fn set_muted(&self, user_id: &str, muted: bool)`, and presence broadcast to everyone else in the room.
  - `pub async fn resolve_public(host: &str, port: u16) -> Vec<SocketAddr>`; a background task calls it every 60 s and `SfuHandle::set_advertised`.
  - HTTP: `GET /health` returns 200 `ok`; `GET /ws` upgrades.
- Shortcut: `dev_key` is a single shared secret that stands in for accounts until the accounts plan; remove it there.

- [ ] **Step 1: Write failing tests** in `tests/signaling.rs` (spawn the server on random ports, drive it with a websocket client and the Task 3 fake client): `join_returns_answer_and_peer_list`; `wrong_key_gets_bad_key_error`; `second_session_for_same_user_replaces_first` (join twice with the same `name`+`key`: first socket is closed, presence shows one entry, peers receive `PeerLeft` then `PeerJoined` once); `room_full_rejects_thirteenth` (12 fake clients succeed, 13th gets `Error{code: RoomFull}`, the other 12 are unaffected); `malformed_and_oversized_messages_do_not_crash` (garbage text gets `Error{BadMessage}`; a message over `MAX_MSG_BYTES` closes only that socket; name longer than `MAX_NAME_CHARS` gets `BadMessage`); `abrupt_close_removes_presence` (drop the socket without `Leave`: peers get `PeerLeft` within 15 s); `resolve_public_returns_loopback` (`resolve_public("localhost", 40000)` contains a loopback address).
- [ ] **Step 2:** `cargo test -p vesper-server`. Expected: FAIL.
- [ ] **Step 3:** Implement. A "user" is identified by display name for now (stable user ids arrive with accounts); `user_id` is a hash of the name. Presence lives in `Rooms`, media in the SFU; the websocket handler pumps `SfuEvent::Offer` out as `ServerMsg::Offer` and `ClientMsg::Answer` into `SfuHandle::answer`.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5:** Commit `feat: signaling and presence`.

---

### Task 5: Web client skeleton and Opus tuning

**Files:** Create `web/` (Vite + Svelte 5 + TypeScript + Tailwind + Bits UI + vitest), `web/src/lib/signaling.ts`, `web/src/lib/audio/opus.ts`, `web/src/routes/Room.svelte`, tests alongside.

**Interfaces:**
- Consumes: generated `ClientMsg`, `ServerMsg`, `Peer` types from `web/src/lib/gen/`.
- Produces:
  - `export function tuneOpus(sdp: string, o: { bitrateKbps: number; dtx: boolean; fec: boolean }): string` (sets `maxaveragebitrate`, `usedtx`, `useinbandfec`, `stereo=0`, `ptime=20` on the Opus fmtp line).
  - `export class Signaling { constructor(url: string, ws?: typeof WebSocket); send(m: ClientMsg): void; onMessage(cb: (m: ServerMsg) => void): void; onClose(cb: () => void): void }`
  - `export async function joinRoom(sig: Signaling, opts: { room: string; name: string; key: string; stream: MediaStream; bitrateKbps: number }): Promise<{ pc: RTCPeerConnection; peers: Peer[] }>`

- [ ] **Step 1: Write failing tests:** `tuneOpus` (a fixture Chrome SDP and a fixture Firefox SDP: output contains `maxaveragebitrate=64000`, `usedtx=1`, `useinbandfec=1`, `stereo=0`, `ptime=20`; unrelated codecs untouched; input without Opus is returned unchanged); `Signaling` with a fake WebSocket (sends JSON, parses incoming, calls `onClose`, ignores invalid JSON without throwing).
- [ ] **Step 2:** `npm --prefix web run test`. Expected: FAIL.
- [ ] **Step 3:** Implement. `joinRoom` creates the peer connection, adds the mic track, applies `tuneOpus` to the offer, sends `Join`, applies the answer, and answers server `Offer`s. `Room.svelte` is a minimal join form (name, key) and a peer list.
- [ ] **Step 4:** Run the tests. Expected: PASS. Manual check: run `vesper-server` with a dev key, open the app in two Chrome tabs, join the same room, hear yourself.
- [ ] **Step 5:** Commit `feat: web client skeleton`.

---

### Task 6: Voice controls and presence UI

**Files:** Create `web/src/lib/volume.ts`; modify `web/src/routes/Room.svelte`.

**Interfaces:**
- Produces: `export function volumeToGain(v: number): number` (v in 0 to 200 percent; 0 returns 0, 100 returns 1, 200 returns 2.5; quadratic `(v/100)^2` up to 100, linear from 1 to 2.5 above; monotonic); `export function restoreVolumes(store: Storage): Map<string, number>`.

- [ ] **Step 1: Write failing tests:** `volumeToGain(0) === 0`, `volumeToGain(100) === 1`, `volumeToGain(200) === 2.5`, monotonic over 0 to 200; out-of-range input is clamped; `restoreVolumes` ignores corrupt JSON in storage.
- [ ] **Step 2:** Run, expect FAIL. 
- [ ] **Step 3:** Implement the functions. In `Room.svelte`: mute (disables the mic track and sends `SetMuted`), deafen (mutes all remote audio), per-peer volume slider through a `GainNode` per remote stream, saved per listener in `localStorage`, speaking ring from an `AnalyserNode` on each stream (local and remote), live join and leave from `PeerJoined` and `PeerLeft`. Voice stays up when the tab is hidden: take a Web Lock for the call's lifetime.
- [ ] **Step 4:** Run tests; manual check with two tabs: mute, deafen, volume, speaking ring, tab switching does not cut audio.
- [ ] **Step 5:** Commit `feat: voice controls`.

---

### Task 7: DeepFilterNet in the browser (WASM build)

**Files:** Modify `crates/vesper-dsp/{Cargo.toml,src/lib.rs}`; create `scripts/build-dsp.sh`, `web/src/lib/audio/dsp.worker.ts`.

**Interfaces:**
- Consumes: `vesper_dsp::{Denoiser, HOP}`.
- Produces: wasm-bindgen exports `WasmDenoiser::new(model: &[u8], atten_db: f32, post_filter_beta: f32) -> WasmDenoiser`, `process(&mut self, input: &[f32]) -> Vec<f32>` (throws on wrong length), `setAttenLimit`, `setPostFilter`; worker message protocol `{ type: 'init', modelUrl, atten, beta } | { type: 'frame', data: Float32Array } | { type: 'set', atten?, beta? }` answered by `{ type: 'ready' } | { type: 'frame', data: Float32Array, ms: number } | { type: 'error', message }`.

- [ ] **Step 1:** `scripts/build-dsp.sh` mirrors the spike's `build.sh`: reads the `wasm-bindgen` version from `Cargo.lock`, fails with a clear message if `wasm-bindgen --version` differs, builds with `RUSTFLAGS="-C target-feature=+simd128"` for `wasm32-unknown-unknown`, writes to `web/public/dsp/`, copies the model.
- [ ] **Step 2: Write failing test** in `web`: `dsp.worker` unit test with a stubbed wasm module: `init` then 3 `frame` messages return 3 frames of length 480 and non-negative `ms`; a frame of wrong length returns `{type:'error'}` and the worker keeps serving; a failed model fetch returns `{type:'error'}` and no `ready`.
- [ ] **Step 3:** Run, expect FAIL. Implement the wasm bindings and the worker; the worker owns timing (`performance.now()` per frame).
- [ ] **Step 4:** Run the web test and `scripts/build-dsp.sh`. Expected: PASS and `web/public/dsp/` contains the wasm and model.
- [ ] **Step 5:** Commit `feat: denoiser wasm and worker`.

---

### Task 8: Denoised capture pipeline with fallback

**Files:** Create `web/src/lib/audio/{settings.ts,ring.ts,mode.ts,capture.ts,rec.worklet.ts,denoise.worklet.ts}`.

**Interfaces:**
- Consumes: the worker protocol from Task 7.
- Produces:
  - `export type AudioSettings = { mode: DenoiseMode; attenDb: number; postFilter: number; inputBoostDb: number; bitrateKbps: number; dtx: boolean; echoCancellation: true; noiseSuppression: boolean; autoGain: boolean; vad: { thresholdDb: number }; pushToTalk: { enabled: boolean; key: string }; micId: string | null; speakerId: string | null }`
  - `export const DEFAULT_AUDIO_SETTINGS: AudioSettings` (mode `deepfilter`, `attenDb` 100, `postFilter` 0.02, `inputBoostDb` 0, `bitrateKbps` 64, `dtx` true)
  - `export function clampSettings(s: Partial<AudioSettings>): AudioSettings` (fills defaults; `attenDb` 6 to 100, `postFilter` 0 to 0.05, `inputBoostDb` 0 to 20, `bitrateKbps` 32 to 510; `echoCancellation` always `true`)
  - `export class FrameRing { constructor(capacityFrames: number); push(block: Float32Array): void; pop(): Float32Array | null; get queued(): number }` (collects 128-sample worklet blocks, hands out 480-sample frames; overflow drops the oldest).
  - `export type DenoiseMode = 'deepfilter' | 'browser' | 'off'`
  - `export class ModeController { constructor(o: { budgetMs: number; windowFrames: number; maxLate: number }); report(frameMs: number): DenoiseMode; reset(): void }` (once `maxLate` frames in the last `windowFrames` exceed `budgetMs`, returns `'browser'` and stays there until `reset()`).
  - `export async function createCapture(s: AudioSettings): Promise<{ stream: MediaStream; setSettings(s: AudioSettings): void; onModeChange(cb: (m: DenoiseMode, reason: string) => void): void; close(): void }>`

- [ ] **Step 1: Write failing tests:** `clampSettings({})` equals defaults; out-of-range values clamp (bitrate 10 gives 32, 9999 gives 510); `echoCancellation` cannot be set false. `FrameRing`: 4 pushes of 128 yield one 480 frame with samples in order and 32 left queued; `pop` on empty is `null`; overflow drops oldest and never throws. `ModeController`: 10 frames at 2 ms stay `'deepfilter'`; 5 of 20 frames at 15 ms switch to `'browser'`; stays there until `reset()`.
- [ ] **Step 2:** Run, expect FAIL.
- [ ] **Step 3:** Implement. Pipeline: `getUserMedia` with echo cancellation on and browser noise suppression and auto gain off when mode is `deepfilter`; a MessageChannel connects the AudioWorklet directly to the worker so the main thread is not in the audio path; denoised audio returns to the worklet, which outputs through a `MediaStreamAudioDestinationNode` whose track is sent over WebRTC. Added delay target is at most 30 ms. If the worker errors or does not become ready in 5 s, switch to `'browser'`: re-request the mic with browser suppression on, swap the track with `RTCRtpSender.replaceTrack`, call `onModeChange` with the reason.
- [ ] **Step 4:** Run tests. Manual check in Chrome and Firefox: call a second tab, confirm denoised audio; kill the worker from devtools and confirm the call continues with a visible mode change.
- [ ] **Step 5:** Commit `feat: denoised capture with fallback`.

---

### Task 9: Audio settings, mic test and device handling

**Files:** Create `web/src/lib/devices.ts`, `web/src/routes/Settings.svelte`.

**Interfaces:**
- Consumes: `AudioSettings`, `clampSettings`, `DEFAULT_AUDIO_SETTINGS` from Task 8.
- Produces:
  - `export type DeviceError = 'denied' | 'no-device' | 'in-use' | 'unplugged' | 'unknown'`; `export function classifyMediaError(e: unknown): DeviceError`; `export function watchDevices(cb: (list: MediaDeviceInfo[]) => void): () => void`.

- [ ] **Step 1: Write failing tests:** `classifyMediaError` maps `NotAllowedError` to `denied`, `NotFoundError` to `no-device`, `NotReadableError` to `in-use`, anything else to `unknown`.
- [ ] **Step 2:** Run, expect FAIL.
- [ ] **Step 3:** Implement. `Settings.svelte`: mode selector, the three suppressor sliders, bitrate, DTX, noise suppression and auto gain toggles, VAD threshold with a live level meter and clipping and too-quiet hints, push to talk (in-tab key), device pickers with hot swap, a mic test that loops the mic back through the pipeline, and a Bluetooth warning when the device label suggests a headset profile. Settings persist in `localStorage`. On `denied`, `no-device` or `unplugged` show an inline error and keep the call connected; `watchDevices` re-offers a device when one appears.
- [ ] **Step 4:** Run tests; manual check: block the mic permission mid-call, unplug a USB mic, switch devices.
- [ ] **Step 5:** Commit `feat: audio settings and devices`.

---

### Task 10: Diagnostics and connectivity check

**Files:** Create `web/src/lib/diag.ts`, `web/src/routes/Diagnostics.svelte`.

**Interfaces:**
- Produces: `export type Diag = { rttMs: number | null; jitterMs: number | null; lossPct: number | null; sendKbps: number | null; codec: string | null; iceState: string; selectedPair: 'host' | 'srflx' | 'relay' | null }`; `export function summarize(prev: RTCStatsReport | null, cur: RTCStatsReport, dtSec: number): Diag`; `export async function connectivityCheck(signalingUrl: string, key: string): Promise<{ ok: boolean; reason: string; ms: number }>` (joins a throwaway room, succeeds when ICE reaches `connected` within 5 s).

- [ ] **Step 1: Write failing tests:** `summarize` on fixture reports computes loss percent, kbps from the byte delta, and tolerates missing fields (returns `null` rather than `NaN`); `connectivityCheck` with a fake peer connection that never connects returns `{ok:false}` after 5 s and a reason naming UDP.
- [ ] **Step 2:** Run, expect FAIL. 
- [ ] **Step 3:** Implement the functions and the page (live numbers every second, a "Copy report" button, a "Test connection" button).
- [ ] **Step 4:** Run tests. Manual check from a phone on cellular against the deployed server (after Task 12).
- [ ] **Step 5:** Commit `feat: diagnostics`.

---

### Task 11: Load test and latency harness

**Files:** Create `crates/vesper-loadtest/{Cargo.toml,src/main.rs}`, `docs/testing/latency.md`.

**Interfaces:**
- Produces: binary `vesper-loadtest --url <ws-url> --key <k> --room <r> --speakers <N> --seconds <S>`; prints per-speaker packets sent, packets received by every other speaker, and server CPU if `--pid` is given; exits non-zero if any speaker receives under 95% of the others' packets.

- [ ] **Step 1: Write failing test** (`cargo test -p vesper-loadtest`): `summary_fails_below_95_percent` for the pure summary function with fixture counts.
- [ ] **Step 2:** Run, expect FAIL. Implement the summary, then the synthetic clients (reuse the Task 3 fake client approach, 20 ms Opus-sized packets).
- [ ] **Step 3:** Run against a local server with 12 speakers for 30 s. Expected: exit 0.
- [ ] **Step 4:** Write `docs/testing/latency.md`: acoustic loopback method (play a click on machine A, record it on machine B through a call, measure the offset), when to use WebRTC stats instead, and a `tc netem` recipe for loss and jitter.
- [ ] **Step 5:** Commit `feat: loadtest and latency docs`.

---

### Task 12: Packaging and deployment

**Files:** Create `deploy/{Dockerfile,compose.yaml,Caddyfile.example,vesper.service}`, `docs/deploy.md`.

**Interfaces:**
- Produces: a Docker image that serves the web app and runs the server; compose file publishing TCP 8080 internally and **UDP 40000** to the host; `docs/deploy.md` covering router forward (UDP 40000), Caddy reverse proxy for `vesper.tfaucher.com`, split DNS for Toby's LAN, and `VESPER_PUBLIC_HOST`.

- [ ] **Step 1:** Write the Dockerfile (multi-stage: build web, build server, minimal runtime) and compose file with a health check on `GET /health`.
- [ ] **Step 2:** `docker compose build` then `docker compose up -d`; `curl localhost:8080/health` returns `ok`.
- [ ] **Step 3:** Write `Caddyfile.example` (reverse proxy including WebSocket upgrade) and `vesper.service` for the plain-binary path; write `docs/deploy.md`.
- [ ] **Step 4:** Deploy to the Dockge VM; from Toby's phone on cellular, run the diagnostics connectivity check. Expected: `ok: true`. Toby does the router forward himself.
- [ ] **Step 5:** Commit `feat: packaging and deployment docs`.

---

### Task 13: Quality checkpoint (manual, Toby signs off)

**Files:** Create `docs/testing/checkpoint.md`.

- [ ] **Step 1:** Write the checkpoint procedure: three noisy scenarios (fan, keyboard, TV), each recorded through Vesper and Discord with the same mic and room; at least three listeners compare blind; record notes on robotic or clipped speech.
- [ ] **Step 2:** Run the procedure with a wired or USB mic (the spike used a Bluetooth headset), in Chrome and Firefox, once on a weaker laptop.
- [ ] **Step 3:** Tune defaults for `attenDb`, `postFilter`, `inputBoostDb` and evaluate the low-latency model variant; update `DEFAULT_AUDIO_SETTINGS`; measure total added delay.
- [ ] **Step 4:** Toby listens and signs off, or lists what to fix. Do not start the accounts, text or polish plans until he does.
- [ ] **Step 5:** Commit `docs: checkpoint results and tuned defaults`.
