# Screen Share Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** People in a group voice call can share a screen at its own size and up to 60 fps, and everyone else sees all shares in a panel with three layouts.

**Architecture:** Every new client's join offer carries one send-only video transceiver with no track, so the SFU sets up video m-lines once at join and nothing renegotiates when a share starts. Starting a share is `replaceTrack` on that sender. The SFU forwards a person's video only while the server says they are sharing, and only to viewers who subscribed ("watch"). `Voice` owns share presence, watch lists and watcher counts; the SFU owns forwarding, keyframes and a per-share rate ceiling. The web client keeps share state in `lib/share/` and renders a panel in the call room's page.

**Tech Stack:** Rust (str0m 0.24.1, axum, tokio, ts-rs), Svelte 5 + TypeScript (vitest, playwright-core e2e scripts).

**Spec:** `docs/superpowers/specs/2026-11-09-screen-share-design.md`. Read it first. This plan argues from it.

## Global Constraints

- Branch `feat/screen-share` from `origin/main` (which already contains v1.6.1). Commit per task. Do not push or open a PR unless Toby asks.
- Builds go through `scripts/capped.sh` (or `scripts/check.sh`, which uses it). Never a blanket `cargo update`. Toolchain stays 1.99.1.
- `unsafe` is forbidden in every crate.
- Wire types live in `crates/vesper-protocol`. After changing them run `cargo test -p vesper-protocol` and commit `web/src/lib/gen/*.ts`. Never edit `gen/` by hand.
- `user_id` format (32 lowercase hex) is unchanged. The screen track's MSID stream id is the sharer's `user_id`; clients tell it from voice by track kind.
- Capture order for audio is unchanged (mic, browser AEC, gain, DeepFilterNet, Opus).
- Codec for video: VP8 only.
- Share settings: `fps` is 30 or 60; `bitrate_kbps` is 500 to 30000. Server per-share ceiling: 35 Mbps sustained for 5 s.
- Watch grace: a viewer unsubscribes 10 s after they stop watching. Pointer markers show for 3 s.
- Firefox bitrate ceiling by size: 10000 kbps at 1920x1200 and above, 5000 at 1280x720 and above, 2500 at 800x480 and above, 2000 below that.
- Speed test: client posts 4 MiB to `POST /api/speedtest`; suggestion is half the measured upload rounded to 500 kbps, clamped to 500 and the ceiling; 8000 kbps (capped by the ceiling) when the test fails. Link: `https://speed.cloudflare.com`.
- Defaults: layout Grid, frame rate 60, "Keep text sharp". Layout and share settings are remembered in `localStorage` (`vesper.shareLayout`, `vesper.share`).
- No screen share in DMs: the server refuses it (`Forbidden`) and the UI has no button there.
- UI and doc copy: short plain sentences, no em dashes, nothing in `scripts/wording-banned.txt`.
- Docs live once and change in the same commit as the route, message or behaviour they describe.
- `scripts/check.sh` must pass before the branch is offered for review.

## Review Focus

1. **Late viewer:** someone joins the call while a share is already running. They must get the video and a keyframe within a couple of seconds. Tests: Task 1 `late_viewer_gets_video`, Task 3 `hello_lists_the_share`.
2. **Picker cancelled:** the sharer closes the browser's screen picker without choosing. Nothing is sent and no error shows. Test: Task 7 `cancelled_picker_sends_nothing`.
3. **Browser stop button:** the sharer ends the share from the browser's own "Stop sharing" bar. Vesper sends `share_stop` and the card goes away. Test: Task 7 `browser_stop_ends_the_share`.
4. **Rejoin while sharing:** a second tab takes the call over while the first was sharing. The share ends for everyone. Test: Task 3 `rejoin_ends_the_share`.
5. **Old cached client:** a browser still running the previous web build (audio-only offer) joins a call where someone shares. It joins, hears everyone, and gets no video m-lines. Test: Task 1 `audio_only_client_gets_no_video`.

---

### Task 1: SFU carries video, gated by sharing and watching

**Files:**
- Modify: `crates/vesper-sfu/src/lib.rs` (`SfuHandle`, `Command`, `check_offer` docs)
- Modify: `crates/vesper-sfu/src/room.rs` (`parse_offer`, builder, `handle_command`, join subscriptions, `propagate`)
- Modify: `crates/vesper-sfu/src/client.rs` (`TrackIn`, `Client`, `handle_output`, `negotiate_if_needed`)
- Modify: `crates/vesper-sfu/src/testing.rs` (`FakeClient`)
- Modify: `crates/vesper-sfu/tests/forwarding.rs`
- Create: `docs/decisions/0009-video-in-the-sfu.md`; Modify: `docs/decisions/README.md` (index), `docs/architecture.md` ("SFU ownership")

**Interfaces:**
- Produces:
  - `SfuHandle::set_sharing(&self, user: &str, on: bool)` and `SfuHandle::watch(&self, viewer: &str, sharer: &str, on: bool)`.
  - `check_offer` / `parse_offer` accept exactly one audio m-line plus at most one video m-line, nothing else.
  - `TrackIn { origin, user, mid, kind: MediaKind }`.
  - `Client` gains `video: bool` (its offer had a video m-line), `sharing: bool`, `watching: HashSet<String>` (sharer user ids).
  - `FakeClient::with_video() -> io::Result<FakeClient>` (audio + VP8 video, both send-only), `send_video(&self, frames: usize, bytes: usize)`, `received_video(&self) -> Vec<Vec<u8>>`, `remote_tracks_of(&self, kind: MediaKind) -> usize`. `received()` keeps meaning audio only.

- [ ] **Step 1: Write the failing tests** in `tests/forwarding.rs`. Add `Ctx::join_video(room, user) -> Peer` (same as `join` but `FakeClient::with_video()`; give `join_with` a `video: bool` argument).

```rust
#[tokio::test(flavor = "multi_thread")]
async fn video_reaches_only_watchers() {
    let t = Ctx::loopback();
    let a = t.join_video("r", "a").await;
    let b = t.join_video("r", "b").await;
    let c = t.join_video("r", "c").await;
    for p in [&a, &b, &c] { t.ready(p, 4).await; } // 2 audio + 2 video each
    t.h.set_sharing("a", true);
    t.h.watch("b", "a", true);
    a.client.send_video(60, 1200);
    t.wait_until("b got video", CONNECT, || b.client.received_video().len() >= 50).await;
    assert!(c.client.received_video().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn video_needs_a_share() { /* as above without set_sharing: after 2 s b.received_video() is empty */ }

#[tokio::test(flavor = "multi_thread")]
async fn late_viewer_gets_video() { /* a shares and keeps sending; c joins after 1 s, watches a; c receives >= 20 frames */ }

#[tokio::test(flavor = "multi_thread")]
async fn audio_only_client_gets_no_video() {
    // a with video and sharing; b is FakeClient::new() (old build)
    // assert b joins, b.client.remote_tracks() == 1, b hears a's audio, b.received_video() is empty
}

#[tokio::test(flavor = "multi_thread")]
async fn force_mute_keeps_video() { /* set_muted("a", true): b gets no new audio but still gets video */ }
```

Change `hostile_offers_are_rejected`: an audio+video offer is now accepted; add cases that must still be `BadOffer`: audio + two video m-lines, video only, audio + application (data channel).

- [ ] **Step 2: Run them and see them fail**

Run: `scripts/capped.sh cargo test -p vesper-sfu --features test-support --test forwarding`
Expected: compile errors (`join_video`, `set_sharing`, `watch`, `with_video` missing).

- [ ] **Step 3: Implement**
  - `room.rs` builder: `.clear_codecs().enable_opus(true, false).enable_vp8(true)`.
  - `parse_offer`: count m-lines by kind; accept `[audio]` or `[audio, video]` in either order; reject everything else.
  - `client.rs` `handle_output`: open a `TrackIn` for `MediaAdded` of `Audio` or `Video` (RecvOnly/SendRecv), storing `kind`. `negotiate_if_needed`: `add_media(track_in.kind, ...)` instead of the hardcoded `Audio`.
  - Join subscriptions (`room.rs` join): a newcomer gets a `TrackOut` for every audio `TrackIn`, and for video `TrackIn`s only if the newcomer's `video` is true. Likewise when a video `TrackIn` opens, only clients with `video` get it.
  - `propagate`: force-mute drops only audio. Video `MediaData` (look the kind up from the sender's `tracks_in` by `mid`) goes to a receiver only if `sender.sharing && receiver.watching.contains(&sender.user)`.
  - `Command::SetSharing { user, on }` sets `sharing` on that user's live client. `Command::Watch { viewer, sharer, on }` inserts or removes in the viewer's `watching`.
  - `FakeClient::with_video`: same as `new` plus a send-only VP8 video m-line (`enable_vp8(true)` on its `Rtc`). `send_video` writes `frames` frames of `bytes` bytes with the existing tag layout (byte 0 `0xF8`, client tag, sequence). If str0m's VP8 depacketizer rejects that payload, start each frame with a VP8 key frame tag (`0x10 0x02 0x00`) and keep the tag after it. `received_video` collects `MediaData` whose `mid` belongs to a video m-line. `remote_tracks_of(kind)` counts RecvOnly `MediaAdded` of that kind; `remote_tracks()` stays the total.

- [ ] **Step 4: Run the SFU tests**

Run: `scripts/capped.sh cargo test -p vesper-sfu --features test-support`
Expected: all pass, including the existing ones unchanged except `hostile_offers_are_rejected`.

- [ ] **Step 5: Write ADR 0009** (single layer, VP8 only, video m-lines fixed at join, forwarding gated by `set_sharing` and `watch`, no simulcast, why) and update "SFU ownership" in `docs/architecture.md` with the two new commands. Add the ADR to `docs/decisions/README.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/vesper-sfu docs/decisions docs/architecture.md
git commit -m "feat(sfu): forward VP8 video to watchers of a share"
```

### Task 2: SFU keyframes and the per-share ceiling

**Files:**
- Create: `crates/vesper-sfu/src/meter.rs`
- Modify: `crates/vesper-sfu/src/lib.rs` (`SfuConfig`, `SfuEvent`), `src/room.rs`, `src/client.rs`, `src/testing.rs`, `tests/forwarding.rs`, `docs/architecture.md`

**Interfaces:**
- Consumes: Task 1's `Client.sharing`, `watching`, `TrackIn.kind`, `FakeClient::with_video`.
- Produces:
  - `SfuConfig { share_ceiling_bps: u64 /* default 35_000_000 */, share_ceiling_secs: u32 /* default 5 */, .. }`.
  - `SfuEvent::ShareCut`: sent once to a sharer's session when the SFU stops forwarding their share; the SFU also sets their `sharing` to false.
  - `pub(crate) struct RateMeter` with `fn new() -> Self` and `fn add(&mut self, bytes: usize, now: Instant) -> Option<u64>` returning bits per second each time a window of at least 1 s closes.
  - `FakeClient::keyframe_requests(&self) -> usize` (count of `Event::KeyframeRequest` on its video sender) and `FakeClient::request_keyframes(&self)` (PLI on every received video mid).

- [ ] **Step 1: Write the failing tests**
  - `meter.rs` unit tests: `rate_is_reported_after_a_second` (add 1_000_000 bytes over 1.0 s of fake `Instant`s → `Some(8_000_000)`), `no_rate_inside_a_second` (→ `None`).
  - `forwarding.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn watching_asks_the_sharer_for_a_keyframe() {
    // a shares, b watches a; wait until a.client.keyframe_requests() >= 1
}

#[tokio::test(flavor = "multi_thread")]
async fn viewer_keyframe_request_reaches_the_sharer() {
    // a shares and sends, b watches; note n = a.keyframe_requests(); b.client.request_keyframes();
    // wait until a.keyframe_requests() > n
}

#[tokio::test(flavor = "multi_thread")]
async fn share_over_the_ceiling_is_cut() {
    // Ctx::new with share_ceiling_bps: 400_000, share_ceiling_secs: 1 (plus loopback setup)
    // a shares, b watches, a sends 60 frames/s of 2_000 bytes (~960 kbit/s) for 3 s
    // a's log gets Logged::ShareCut exactly once; b stops receiving new video within 1 s after it
}
```

  Add `Logged::ShareCut` to the test log and record `SfuEvent::ShareCut` in `join_with`.

- [ ] **Step 2: Run them and see them fail**

Run: `scripts/capped.sh cargo test -p vesper-sfu --features test-support`
Expected: compile errors for `RateMeter`, `ShareCut`, config fields, FakeClient methods.

- [ ] **Step 3: Implement**
  - On `Command::Watch { on: true }`: if the sharer has a video `TrackIn`, `rtc.writer(mid).request_keyframe(None, KeyframeRequestKind::Pli)` on the sharer's client.
  - `Event::KeyframeRequest { mid, .. }` on a viewer becomes a new `Propagated::KeyframeRequest(ClientId, Mid)`; `room.rs` maps the viewer's out `mid` to its `TrackIn` and requests a keyframe from that origin with the same kind (pattern: str0m `examples/chat.rs`).
  - Each sharer's client keeps a `RateMeter` fed with every incoming video `MediaData` length. When a closed window is above `share_ceiling_bps`, count one; below, reset to zero. At `share_ceiling_secs` in a row: `sharing = false`, send `SfuEvent::ShareCut`, reset the count. Ignore video while `sharing` is false.
  - `SfuConfig::default()` sets the two new fields. Thread them to clients as `answer_timeout` is.

- [ ] **Step 4: Run the SFU tests**

Run: `scripts/capped.sh cargo test -p vesper-sfu --features test-support`
Expected: all pass.

- [ ] **Step 5: Document** `SfuEvent::ShareCut` and keyframe handling in "SFU ownership" in `docs/architecture.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/vesper-sfu docs/architecture.md
git commit -m "feat(sfu): keyframes on watch and a per-share rate ceiling"
```

### Task 3: Share presence on the server

**Files:**
- Modify: `crates/vesper-protocol/src/lib.rs`, `web/src/lib/gen/*` (regenerated)
- Modify: `crates/vesper-server/src/voice.rs`, `crates/vesper-server/src/ws.rs`
- Modify: `crates/vesper-server/tests/signaling.rs`
- Modify: web fixtures that build `Peer` literals (add `share: null`) so `npm run check` and `npm test` still pass
- Modify: `docs/architecture.md` ("Message flow" lists)

**Interfaces:**
- Consumes: Task 1's `SfuHandle::set_sharing`.
- Produces:
  - `pub struct ShareInfo { pub fps: u8, pub bitrate_kbps: u32 }` (Serialize, Deserialize, TS, Debug, Clone, Copy, PartialEq, Eq).
  - `Peer.share: Option<ShareInfo>` with `#[serde(default)]`.
  - `ClientMsg::ShareStart { fps: u8, bitrate_kbps: u32 }` (also used to change settings while sharing) and `ClientMsg::ShareStop`.
  - `ServerMsg::PeerShare { user_id: String, share: Option<ShareInfo> }`.
  - `Voice::share_start(&self, user_id: &str, session: u64, info: ShareInfo) -> Result<(), ErrorCode>` and `Voice::share_stop(&self, user_id: &str, session: u64)`.
  - `fn valid_share(info: &ShareInfo) -> bool` in `voice.rs`.

- [ ] **Step 1: Write the failing tests**
  - `voice.rs` unit test `share_settings_are_checked`: `valid_share` is true for (30, 500), (60, 30000); false for (45, 8000), (60, 499), (60, 30001), (0, 8000).
  - `signaling.rs`:

```rust
#[tokio::test]
async fn share_reaches_the_room() {
    // alice and bob join general; alice sends ShareStart { fps: 60, bitrate_kbps: 8000 }
    // bob receives PeerShare { user_id: alice, share: Some(ShareInfo { fps: 60, bitrate_kbps: 8000 }) }
}
#[tokio::test]
async fn share_stop_reaches_the_room() { /* then ShareStop → bob gets PeerShare { share: None } */ }
#[tokio::test]
async fn share_in_a_dm_is_refused() { /* dm_room; alice joins its voice; ShareStart → expect_error(Forbidden) */ }
#[tokio::test]
async fn bad_share_settings_are_refused() { /* fps 45 → expect_error(BadMessage) */ }
#[tokio::test]
async fn share_needs_a_call() { /* ShareStart before Join → expect_error(BadMessage) */ }
#[tokio::test]
async fn hello_lists_the_share() { /* alice shares; carol opens a socket; Hello.voice has alice with share Some(..) */ }
#[tokio::test]
async fn rejoin_ends_the_share() {
    // alice shares; alice joins again from a second socket; bob ends up seeing alice with share None
    // (assert on bob's state after the presence messages that follow the rejoin)
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `scripts/capped.sh cargo test -p vesper-server --test signaling share` and `scripts/capped.sh cargo test -p vesper-server valid_share`
Expected: compile errors for the new types.

- [ ] **Step 3: Implement**
  - Protocol types as above. Run `scripts/capped.sh cargo test -p vesper-protocol` to regenerate `gen/`.
  - `Member` gains `share: Option<ShareInfo>`; `Member::peer` copies it.
  - `share_start`: ignore a stale session; `Forbidden` if the cached room's `kind` is `RoomKind::Dm`; `BadMessage` unless `valid_share`. Set `share`, call `sfu.set_sharing(user_id, true)` only when it was `None`, and `tell` the room `PeerShare` skipping the sender's socket (as `PeerState` does).
  - `share_stop`: if sharing, clear, `sfu.set_sharing(user_id, false)`, `tell` the room `PeerShare { share: None }` skipping the sender.
  - A member replaced by a rejoin or removed by `leave` loses its share with it. Make sure the presence messages that follow a rejoin leave other sockets with `share: None` for that user.
  - `ws.rs` dispatch: `(Ok(ClientMsg::ShareStart { fps, bitrate_kbps }), Some(j))` and `(Ok(ClientMsg::ShareStop), Some(j))`, passing `j.session`. Map `Forbidden` to the message "Screen share is only in group rooms." and `BadMessage` to "Bad share settings.". Without a join the existing "join first" arm answers.

- [ ] **Step 4: Run tests**

Run: `scripts/capped.sh cargo test -p vesper-server` then `cd web && npm run check && npm test`
Expected: all pass (fix `Peer` fixtures in web tests by adding `share: null`).

- [ ] **Step 5: Document** `share_start`, `share_stop`, `peer_share` and `Peer.share` in "Message flow" in `docs/architecture.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/vesper-protocol crates/vesper-server web/src docs/architecture.md
git commit -m "feat(server): share presence for group voice rooms"
```

### Task 4: Watching, watcher counts, pointer and share cut on the server

**Files:**
- Modify: `crates/vesper-protocol/src/lib.rs`, `web/src/lib/gen/*`
- Modify: `crates/vesper-server/src/voice.rs`, `crates/vesper-server/src/ws.rs`, `crates/vesper-server/tests/signaling.rs`, `docs/architecture.md`

**Interfaces:**
- Consumes: Task 1 `SfuHandle::watch`, Task 2 `SfuEvent::ShareCut`, Task 3 `ShareInfo`, `Member.share`.
- Produces:
  - `ClientMsg::Watch { user_id: String, on: bool }`, `ClientMsg::Point { user_id: String, x: f32, y: f32 }`.
  - `ServerMsg::Watchers { count: u32 }` (to the sharer), `ServerMsg::Point { user_id: String /* sharer */, from: String, x: f32, y: f32 }`.
  - `Voice::watch(&self, user_id: &str, session: u64, sharer: &str, on: bool)`, `Voice::point(&self, user_id: &str, session: u64, sharer: &str, x: f32, y: f32) -> Result<(), ErrorCode>`, `Voice::cut_share(&self, user_id: &str, session: u64)`.
  - `Member.watching: HashSet<String>`.

- [ ] **Step 1: Write the failing tests** (`signaling.rs`, plus one `voice.rs` unit test)

```rust
#[tokio::test]
async fn watch_count_reaches_the_sharer() {
    // alice shares → alice gets Watchers { count: 0 }
    // bob Watch { alice, on: true } → alice gets Watchers { count: 1 }
    // bob Watch { alice, on: false } → Watchers { count: 0 }
    // bob watches again, then leaves → Watchers { count: 0 }
}
#[tokio::test]
async fn watch_outside_the_room_is_ignored() { /* carol in another room watches alice: alice gets no Watchers change */ }
#[tokio::test]
async fn point_reaches_the_others_in_the_call() {
    // alice shares; bob and carol in the call; bob Point { alice, 0.25, 0.5 }
    // carol gets ServerMsg::Point { user_id: alice, from: bob, x: 0.25, y: 0.5 }; alice gets it too
}
#[tokio::test]
async fn bad_point_is_refused() { /* x: 1.5 → expect_error(BadMessage) */ }
// voice.rs unit test, with a real loopback SFU and outboxes as the existing tests do:
#[tokio::test]
async fn cut_share_tells_everyone() {
    // alice shares; voice.cut_share(alice, session)
    // bob's outbox gets PeerShare { alice, None }; alice's outbox gets PeerShare { alice, None } and Error { code: RateLimited, .. }
}
```

- [ ] **Step 2: Run them and see them fail**

Run: `scripts/capped.sh cargo test -p vesper-server watch point cut_share`
Expected: compile errors for the new messages and methods.

- [ ] **Step 3: Implement**
  - `watch`: ignore a stale session, or a `sharer` who is not a member in the same room. Update `watching`, call `sfu.watch(user_id, sharer, on)`. If the sharer is sharing and the count changed, send them `Watchers { count }` (count = members in the same room whose `watching` has the sharer).
  - `share_start` (from Task 3) sends the sharer `Watchers { count }` when a share begins.
  - When a member is removed, send fresh counts to every sharer they watched.
  - `point`: `BadMessage` unless `x` and `y` are finite and in `0.0..=1.0`. Ignore unless the sharer is in the same room and sharing. Send `Point` to every other member of that room's call (their `Member.tx`), not to the whole room's sockets.
  - `cut_share`: clear the share, `tell` the room `PeerShare { share: None }` without skipping the sharer, and send the sharer `Error { code: RateLimited, message: "Your screen share sent more than 35 Mbps and was stopped." }`.
  - `forward_events`: handle `SfuEvent::ShareCut` by calling `cut_share` for that session.
  - `ws.rs` dispatch arms for `Watch` and `Point` with `Some(j)`. The per-socket budget (5 a second) is the pointer's rate limit; no new `limits.rs` kind (the client only sends on a click).

- [ ] **Step 4: Run tests**

Run: `scripts/capped.sh cargo test -p vesper-server` and `cd web && npm run check`
Expected: all pass.

- [ ] **Step 5: Document** `watch`, `point`, `watchers`, the server `point`, and the cut in "Message flow" in `docs/architecture.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/vesper-protocol crates/vesper-server web/src/lib/gen docs/architecture.md
git commit -m "feat(server): watch lists, watcher counts, pointer and share cut"
```

### Task 5: Upload speed test endpoint

**Files:**
- Create: `crates/vesper-server/src/speedtest.rs`, `crates/vesper-server/tests/speedtest.rs`
- Modify: `crates/vesper-server/src/lib.rs` (mount routes next to `uploads::routes()`), `crates/vesper-server/src/limits.rs` (`Kind::Speedtest => (3.0, s(20))`), `docs/architecture.md` ("HTTP API")

**Interfaces:**
- Produces: `POST /api/speedtest`, raw body up to 25 MiB, needs a session and the `Origin` check, returns 204, stores nothing.
- `pub fn routes() -> Router<AppState>` in `speedtest.rs`; handler `async fn upload(State(st): State<AppState>, a: Authed, body: Body) -> Result<StatusCode, ApiError>` that calls `limit(&st, Kind::Speedtest, &a.user.user_id)?` then `uploads::read_body(body, &mut tokio::io::sink(), <idle>, <total>)` with the same idle and total limits uploads use (make them `pub(crate)` if needed).

- [ ] **Step 1: Write the failing tests** in `tests/speedtest.rs` using `common::TestServer` and the raw request style of `tests/uploads.rs`: `speedtest_accepts_and_discards` (4 MiB → 204; no new file under the uploads directory), `speedtest_needs_a_session` (401), `speedtest_needs_the_origin` (403 without `Origin`), `speedtest_is_rate_limited` (fourth post at once → 429), `speedtest_refuses_over_25_mib` (the status uploads return for an oversized body).

- [ ] **Step 2: Run them and see them fail**

Run: `scripts/capped.sh cargo test -p vesper-server --test speedtest`
Expected: 404s or compile errors.

- [ ] **Step 3: Implement** the module, the route and the limiter kind.

- [ ] **Step 4: Run tests**

Run: `scripts/capped.sh cargo test -p vesper-server`
Expected: all pass.

- [ ] **Step 5: Document** the route in "HTTP API" in `docs/architecture.md`.

- [ ] **Step 6: Commit**

```bash
git add crates/vesper-server docs/architecture.md
git commit -m "feat(server): upload speed test endpoint for share settings"
```

### Task 6: Web share logic (pure modules)

**Files:**
- Create in `web/src/lib/share/`: `ceiling.ts`, `prefs.ts`, `speedtest.ts`, `watch.ts`, `struggle.ts`, `geometry.ts`, each with a `*.test.ts` next to it

**Interfaces:**
- Produces:

```ts
// ceiling.ts
export type Engine = 'firefox' | 'other';
export const MIN_KBPS = 500, MAX_KBPS = 30_000;
export function engineOf(userAgent: string): Engine;               // /Firefox\/\d/ → 'firefox'
export function ceilingKbps(engine: Engine, width: number, height: number): number;
export function suggestKbps(uploadKbps: number | null, ceiling: number): number;
// prefs.ts
export type Prefer = 'sharp' | 'smooth';
export type Layout = 'grid' | 'focus' | 'dock';
export type ShareSettings = { fps: 30 | 60; kbps: number | null; prefer: Prefer }; // kbps null = use the suggestion
export const SHARE_KEY = 'vesper.share', LAYOUT_KEY = 'vesper.shareLayout';
export function loadShareSettings(): ShareSettings;                 // default { fps: 60, kbps: null, prefer: 'sharp' }
export function saveShareSettings(s: ShareSettings): void;
export function loadLayout(): Layout;                               // default 'grid'
export function saveLayout(l: Layout): void;
// speedtest.ts
export const SPEEDTEST_BYTES = 4 * 1024 * 1024;
export async function measureUploadKbps(post?: (b: Blob) => Promise<Response>, now?: () => number): Promise<number | null>;
// watch.ts
export const WATCH_GRACE_MS = 10_000;
export function wantsWatch(s: { inPanel: boolean; tabVisible: boolean; poppedOut: boolean }): boolean;
export class WatchGate { constructor(send: (on: boolean) => void); set(want: boolean): void; dispose(): void }
// struggle.ts
export type RecvSample = { packetsLost: number; packetsReceived: number; freezeCount: number };
export function struggling(samples: RecvSample[]): boolean;
// geometry.ts
export function pointIn(clientX: number, clientY: number, box: { left: number; top: number; width: number; height: number }, videoW: number, videoH: number): { x: number; y: number } | null;
```

- [ ] **Step 1: Write the failing tests** with these assertions:
  - `ceilingKbps('firefox', 1920, 1080) === 5000`; `(1728, 1080) === 5000`; `(1920, 1200) === 10000`; `(2560, 1440) === 10000`; `(1280, 720) === 5000`; `(1024, 576) === 2500`; `(640, 360) === 2000`; `ceilingKbps('other', 1920, 1080) === 30000`. Blocks are `Math.ceil(w / 16) * Math.ceil(h / 16)`, compared with the blocks of 1920x1200, 1280x720 and 800x480.
  - `suggestKbps(20_000, 30_000) === 10_000`; `suggestKbps(500_000, 5_000) === 5_000`; `suggestKbps(null, 30_000) === 8_000`; `suggestKbps(null, 5_000) === 5_000`; `suggestKbps(700, 30_000) === 500`; `suggestKbps(9_300, 30_000) === 4_500` (half, rounded to 500).
  - `engineOf` is `'firefox'` for a Firefox and a Zen user agent, `'other'` for Chrome, Edge and Brave ones.
  - Prefs: defaults when storage is empty, corrupt JSON, or values out of range (`fps: 45`, `kbps: 99`); a saved value round-trips; storage that throws does not throw out.
  - `measureUploadKbps`: posts a `Blob` of `SPEEDTEST_BYTES`; with a fake clock of 0 then 2000 ms returns `16_777` (rounded down); returns `null` on a 429 and when `post` throws.
  - `wantsWatch`: true when popped out whatever else; true when in panel and tab visible; false otherwise.
  - `WatchGate` (fake timers): `set(true)` sends `true` once; `set(false)` then 9 s → nothing; at 10 s → sends `false`; `set(false)`, 5 s, `set(true)` → no `false` ever sent and no second `true`.
  - `struggling`: false with fewer than 4 samples; true when over the last 4 samples lost/(lost+received) is above 0.02; true when `freezeCount` grew; false on a clean run.
  - `pointIn`: a 1920x1080 video in a 960x720 box (letterboxed top and bottom by 90 px): the box centre → `{x: 0.5, y: 0.5}`; a point in the top bar → `null`; the top-left of the picture → `{x: 0, y: 0}`.

- [ ] **Step 2: Run them and see them fail**

Run: `cd web && npx vitest run src/lib/share`
Expected: FAIL, modules missing.

- [ ] **Step 3: Implement the six modules.** Storage follows `ui/prefs.ts` (try/catch around `localStorage`). `measureUploadKbps`'s default `post` is `fetch('/api/speedtest', { method: 'POST', body, credentials: 'same-origin' })`.

- [ ] **Step 4: Run them**

Run: `cd web && npx vitest run src/lib/share && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/lib/share
git commit -m "feat(web): share ceilings, prefs, speed test, watch gate"
```

### Task 7: Web call wiring for sending and receiving shares

**Files:**
- Create: `web/src/lib/share/send.ts` (+ `send.test.ts`), `web/src/lib/share/state.svelte.ts` (+ `state.test.ts`)
- Modify: `web/src/lib/join.ts`, `web/src/lib/join.test.ts`, `web/src/lib/voice.svelte.ts`, `web/src/lib/voice.test.ts`, `web/src/lib/peers.ts`, `web/src/lib/peers.test.ts`, `web/vite.config.ts` (call chunk regex, if the new call-only files belong in it)

**Interfaces:**
- Consumes: Task 3 and 4 messages, Task 6 modules.
- Produces:

```ts
// join.ts: joinRoom's result gains `video: RTCRtpSender`; callbacks gain the kind
onRemoteStream?: (id: string, s: MediaStream, kind: 'audio' | 'video') => void;
onRemoteStreamEnd?: (id: string, kind: 'audio' | 'video') => void;
// send.ts
export function encodingFor(s: { fps: 30 | 60; kbps: number }, active: boolean): RTCRtpEncodingParameters; // active, maxBitrate kbps*1000, maxFramerate, scaleResolutionDownBy 1, priority 'low', networkPriority 'low'
export function degradationFor(p: Prefer): RTCDegradationPreference; // sharp → 'maintain-resolution', smooth → 'maintain-framerate'
export function hintFor(p: Prefer): 'detail' | 'motion';
export async function applyShare(sender: RTCRtpSender, s: { fps: 30 | 60; kbps: number; prefer: Prefer }, active: boolean): Promise<void>;
export async function capture(fps: 30 | 60, prefer: Prefer): Promise<MediaStreamTrack | null>; // null when the user cancels the picker; sets contentHint from hintFor(prefer) when supported
// state.svelte.ts
export type Marker = { share: string; from: string; x: number; y: number; until: number };
export type Mine = { track: MediaStreamTrack; fps: 30 | 60; kbps: number; prefer: Prefer; watchers: number };
export const shares: {
  streams: Map<string, MediaStream>;     // $state.raw, remote video by user_id
  hidden: Set<string>; shown: Set<string>; // shown = tiles mounted in an open panel (Task 9)
  popped: string | null; layout: Layout; focus: string | null; panelOpen: boolean; chatOpen: boolean;
  tabVisible: boolean; markers: Marker[]; mine: Mine | null;
};
export function syncWatches(sharing: string[], send: (userId: string, on: boolean) => void): void; // one WatchGate per sharer
// voice.svelte.ts
export async function startShare(s: ShareSettings): Promise<void>;
export async function updateShare(s: ShareSettings): Promise<void>;
export function stopShare(): void;
export function watchShare(userId: string, on: boolean): void; // sends `watch`
export function pointAt(userId: string, x: number, y: number): void; // sends `point`
```

- [ ] **Step 1: Write the failing tests**
  - `join.test.ts`: the offer comes from a pc with one audio track and one `addTransceiver('video', { direction: 'sendonly' })`; the result's `video` is that transceiver's sender; an `ontrack` for a video track with stream id `u1` calls `onRemoteStream('u1', stream, 'video')` and a later audio track for `u1` is still live (neither ends the other).
  - `peers.test.ts`: `peer_share` sets the peer's `share`; `share: null` clears it.
  - `send.test.ts`: `encodingFor({ fps: 60, kbps: 8000 }, false)` has `active: false`, `maxBitrate: 8_000_000`, `maxFramerate: 60`, `scaleResolutionDownBy: 1`; `applyShare` with a fake sender whose `setParameters` rejects `degradationPreference` still sets the encoding (retries without it) and never throws for that; `capture` returns `null` when `getDisplayMedia` rejects with `NotAllowedError` or `AbortError`, and sets `contentHint` only when the track has the property.
  - `voice.test.ts` (existing mocks):
    - `starts_a_share`: `startShare({ fps: 60, kbps: 8000, prefer: 'sharp' })` replaces the video sender's track and sends `{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }`.
    - `cancelled_picker_sends_nothing`: `capture` resolves `null` → no message sent, `voice.error` unchanged, `shares.mine` null.
    - `browser_stop_ends_the_share`: firing `ended` on the captured track sends `{ type: 'share_stop' }` and clears `shares.mine`.
    - `watchers_turn_the_encoder_on_and_off`: `watchers` 1 → `applyShare(..., true)`; 0 → `applyShare(..., false)`.
    - `server_cut_stops_locally`: `peer_share` for me with `share: null` while sharing → track stopped, `shares.mine` null, no `share_stop` sent.
    - `remote_video_never_reaches_playback`: a video `onRemoteStream` lands in `shares.streams` and `playback.addRemote` is not called.
    - `point_adds_a_marker`: a server `point` adds a marker with `until` 3000 ms ahead; it is gone after 3 s (fake timers).
  - `state.test.ts`: `syncWatches(['a'], send)` with `shown` containing `a` and `tabVisible` true sends `('a', true)` once; removing `a` from `shown` sends `('a', false)` after 10 s; `popped = 'a'` keeps it on with `shown` empty.

- [ ] **Step 2: Run them and see them fail**

Run: `cd web && npx vitest run src/lib/join.test.ts src/lib/peers.test.ts src/lib/voice.test.ts src/lib/share`
Expected: FAIL.

- [ ] **Step 3: Implement**
  - `join.ts`: add the video transceiver after the audio tracks; key `live` on `` `${kind}:${id}` ``; pass the kind to both callbacks; return `video`. After the answer is set, raise the audio sender's encoding `priority` and `networkPriority` to `'high'` (best effort, ignore failures).
  - `voice.svelte.ts`: keep the video sender from `joinRoom`. Branch `onRemoteStream`/`onRemoteStreamEnd` on kind (video goes to `shares.streams`). In `startShare`, clamp `kbps` (the setting, or `suggestKbps` of the last measurement) to `ceilingKbps(engineOf(navigator.userAgent), w, h)` using the captured track's `getSettings()`. Start with the encoding inactive; the server's `watchers` message turns it on. Handle `watchers`, `peer_share` (me) and `point` in the existing `startVoice` subscription. On call teardown, stop the local track and clear `shares` without sending.
  - `peers.ts`: handle `peer_share`.

- [ ] **Step 4: Run them**

Run: `cd web && npm test && npm run check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat(web): send and receive screen shares over the call"
```

### Task 8: Sharer UI: share button, card, settings dialog, cues, presence icon

**Files:**
- Create: `web/src/routes/ShareCard.svelte`, `web/src/routes/ShareDialog.svelte`
- Modify: `web/src/lib/ui/icons.ts`, `web/src/routes/CallBar.svelte`, `web/src/routes/callbar.test.ts`, `web/src/routes/PresenceList.svelte`, `web/src/lib/ui/cues.ts` (+ its test), `web/src/lib/ui/chime.ts`, `web/src/lib/ui/messages.ts`, `web/vite.config.ts` if needed

**Interfaces:**
- Consumes: Task 6 (`ceilingKbps`, `suggestKbps`, `measureUploadKbps`, prefs), Task 7 (`startShare`, `updateShare`, `stopShare`, `shares.mine`).
- Produces: icons `monitor`, `monitorUp`, `maximize`, `zoomIn`, `pictureInPicture2`, `mousePointerClick`, `eye`, `eyeOff`, `layoutGrid`, `galleryVerticalEnd`, `columns2` (Lucide 0.460.0, joined into one path each as the file's header says); cues `'share-start' | 'share-stop'`; `export function shareCue(prev: Peer[], next: Peer[], roomId: number): Cue | null`.

- [ ] **Step 1: Write the failing tests**
  - Cue test: `shareCue` returns `'share-start'` when a peer in `roomId` goes from `share: null` to a share, `'share-stop'` for the reverse, `null` for a change in another room or a settings change on a running share.
  - `callbar.test.ts`: in a group room call the "Share screen" button is present; in a DM call it is absent; while `shares.mine` is set the card shows "Stop" and the watcher count text ("2 watching"), and the live region holds no changing figures.

- [ ] **Step 2: Run them and see them fail**

Run: `cd web && npx vitest run src/lib/ui src/routes/callbar.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement**
  - CallBar row 1: after Deafen, an `IconButton` (`monitorUp`, label "Share screen", `tip="up"`) that opens `ShareDialog`; hidden when the joined room's `kind` is `dm`. While sharing, `ShareCard` replaces it.
  - `ShareCard`: a muted preview `<video>` of `shares.mine.track`, the line "`{w}`x`{h}` · `{fps}` fps · `{mbps}` Mbps · `{n}` watching" from the video sender's `outbound-rtp` stats polled every 2 s, a gear `IconButton` (opens `ShareDialog` in edit mode) and a "Stop" button.
  - `ShareDialog` (`c/Dialog.svelte`), copy exactly:
    - Title "Share your screen" (edit mode: "Share settings").
    - "Frame rate": 30 / 60.
    - "Bitrate": a `Range` from 500 to the ceiling in steps of 500, showing "`{n}` Mbps". Before capture the ceiling uses `screen.width * devicePixelRatio` by `screen.height * devicePixelRatio`.
    - While measuring: "Measuring your upload." After: "Your upload: about `{n}` Mbps. Suggested: `{m}` Mbps." On failure: "Could not measure your upload." The slider starts at the saved value, or at the suggestion when none is saved.
    - Link "Check your speed" to `https://speed.cloudflare.com` (`target="_blank" rel="noopener noreferrer"`).
    - For Firefox: "Firefox can send at most `{c}` Mbps at this screen size. A Chromium browser can go higher."
    - "When the connection struggles": "Keep text sharp" / "Keep motion smooth".
    - Buttons: "Start sharing" (edit mode: "Save") and "Cancel". Saving stores the settings with `saveShareSettings`.
  - `PresenceList`: a `monitor` icon with `aria-label="Sharing screen"` for a peer whose `share` is set.
  - Play `shareCue` results with `chime` where the other call cues are played; add tones for the two new cues.

- [ ] **Step 4: Run them**

Run: `cd web && npm test && npm run check && npm run build`
Expected: PASS, and the size budget in `npm run ci` still passes (`npm run ci`).

- [ ] **Step 5: Commit**

```bash
git add web
git commit -m "feat(web): share button, share card and share settings"
```

### Task 9: Viewer UI: the share panel, tiles, layouts and pop-out

**Files:**
- Create: `web/src/routes/SharePanel.svelte`, `web/src/routes/ShareTile.svelte`, `web/src/lib/share/pip.ts` (+ `pip.test.ts`)
- Modify: `web/src/routes/RoomView.svelte`, `web/src/routes/Shell.svelte`, `web/src/routes/CallBar.svelte`, `web/src/lib/ui/messages.ts`

**Interfaces:**
- Consumes: Task 6 (`pointIn`, `struggling`, prefs `Layout`), Task 7 (`shares`, `syncWatches`, `watchShare`, `pointAt`), Task 8 icons.
- Produces (`pip.ts`): `export function canPopOut(doc?: Document): boolean` (true only when `doc.pictureInPictureEnabled === true`); `export async function popOut(stream: MediaStream, onClose: () => void): Promise<void>` (one persistent hidden `<video>` appended to `document.body`, set `srcObject`, wait for `loadedmetadata`, `requestPictureInPicture()`, call `onClose` on `leavepictureinpicture`); `export function closePopOut(): void`.

- [ ] **Step 1: Write the failing tests** in `pip.test.ts`: `canPopOut` is false when `pictureInPictureEnabled` is undefined (Firefox) or false, true when true; `popOut` reuses one element across calls and calls `onClose` when `leavepictureinpicture` fires.

- [ ] **Step 2: Run them and see them fail**

Run: `cd web && npx vitest run src/lib/share/pip.test.ts`
Expected: FAIL.

- [ ] **Step 3: Implement**
  - **RoomView**: when `voice.joinedRoom === room.id` and at least one other peer in the room has `share`, lazy-load `SharePanel` (as `Chat` is loaded) between the error block and the chat.
    - Layout `grid` or `focus`: the panel takes the main area and `Chat` moves into a right-hand drawer opened by the panel's "Chat" button (`shares.chatOpen`).
    - Layout `dock`: panel, a divider (`role="separator"`, `aria-orientation="vertical"`, drag and ArrowLeft/ArrowRight in 5% steps, kept in memory only), then `Chat`.
  - **SharePanel header**: "`{n}` screens" (`n` = 1 → "1 screen"), three layout buttons (`layoutGrid` "Grid", `galleryVerticalEnd` "Focus", `columns2` "Dock", `pressed` on the current one, saved with `saveLayout`), the "Chat" button in grid and focus, and "Hide screens" (sets `panelOpen = false`; the panel then shows one row "`{n}` screens" with a "Show" button). Hidden shares are listed as rows with a "Show" button.
  - **Layouts**: grid uses `repeat(auto-fit, minmax(…))` and fits tiles as large as possible; focus shows `shares.focus` (or the first share) large and the rest as thumbnails, click to swap; dock stacks tiles in the panel column.
  - **ShareTile** (`{ userId }`):
    - A `<video autoplay muted playsinline>` with `srcObject` from `shares.streams`. Name chip.
    - On mount add `userId` to `shares.shown` when the panel is open; remove on unmount or when hidden.
    - Hover controls, each an `IconButton`:
      - "Full screen at 1:1" (`maximize`): `requestFullscreen()` on the tile; in full screen the video is sized `videoWidth / devicePixelRatio` by `videoHeight / devicePixelRatio` CSS pixels, centred, scrolling if larger.
      - "Zoom" (`zoomIn`): toggles a 2x `transform: scale(2)` with `transform-origin` at the click point; drag pans.
      - "Pop out" (`pictureInPicture2`), only when `canPopOut()`: `popOut(stream, () => shares.popped = null)` and `shares.popped = userId`, called inside the click handler.
      - "Point" (`mousePointerClick`): the next click on the video calls `pointAt(userId, x, y)` with `pointIn(...)`; clicks outside the picture do nothing.
      - "Hide" (`eyeOff`): add to `shares.hidden`.
    - Stats on hover: "`{w}`x`{h}` · `{fps}` fps · `{mbps}` Mbps" from this track's receiver `inbound-rtp`, polled every 2 s; the same poll feeds `struggling`.
    - When `struggling`: a warning chip "`{name}`'s upload is struggling."
    - Markers from `shares.markers` for this share: a dot in the pointer's avatar colour with their name at `x`, `y`.
  - **Shell**: track `document.visibilityState` into `shares.tabVisible`; an `$effect` while in a call that runs `syncWatches(sharingIds, watchShare)`.
  - **CallBar**: when shares exist in the call and the current route is not the call room, a link "`{n}` sharing" that opens the call room.
  - Copy goes in `lib/ui/messages.ts`.

- [ ] **Step 4: Run them**

Run: `cd web && npm test && npm run check && npm run ci`
Expected: PASS, including the gzip size budget. If the budget fails, keep `SharePanel` and `ShareTile` in their own lazy chunk.

- [ ] **Step 5: Look at it.** With a release server and `scripts/build-dsp.sh` done, run the app in two headless Chromium pages (the e2e helpers in `web/e2e/lib.mjs` help), share a stubbed canvas screen (as in Task 10), and screenshot the three layouts. Compare with the mockups at `~/Downloads/share-panel-shots/layout-{A,B,C}.png` on bawn. Fix layout gaps before committing.

- [ ] **Step 6: Commit**

```bash
git add web
git commit -m "feat(web): share panel with grid, focus and dock layouts"
```

### Task 10: End-to-end share test

**Files:**
- Create: `web/e2e/share.mjs`
- Modify: `web/package.json` (`e2e` script: add `node e2e/share.mjs` after `two-browser.mjs`)

**Interfaces:**
- Consumes: everything above. Uses `startServer`, `browser`, `openRoom` and `window.__pcs` from `web/e2e/lib.mjs`.

- [ ] **Step 1: Write the test.** An init script in both pages replaces `navigator.mediaDevices.getDisplayMedia` with a function returning a 1280x720 canvas `captureStream(30)` that redraws a counter every 33 ms. Flow:
  1. Alice and Bob open `general` and join voice.
  2. Alice clicks "Share screen", then "Start sharing".
  3. Bob sees a tile video for Alice with `videoWidth > 0` within 10 s; Bob's `inbound-rtp` video `bytesReceived` grows.
  4. Alice's card shows "1 watching". Alice's `candidate-pair` `availableOutgoingBitrate` passes 2 Mbps within 20 s, which shows the SFU's transport-cc feedback reaches the sender (if it stays low, check str0m's TWCC setup before going on).
  5. Bob hides the tile; within 15 s Alice's card shows "0 watching" and Alice's `outbound-rtp` video `bytesSent` stops growing over 3 s.
  6. Bob shows it again; video resumes within 5 s.
  7. Alice clicks "Stop"; Bob's panel goes away.
  8. In a DM's voice call there is no "Share screen" button.

- [ ] **Step 2: Run it**

Run: `scripts/capped.sh cargo build --release -p vesper-server && scripts/build-dsp.sh && cd web && npm run build && node e2e/share.mjs`
Expected: exits 0.

- [ ] **Step 3: Commit**

```bash
git add web/e2e/share.mjs web/package.json
git commit -m "test(e2e): two-browser screen share"
```

### Task 11: Video load test

**Files:**
- Modify: `crates/vesper-loadtest/src/main.rs`, `docs/testing/latency.md`
- Maybe modify: `crates/vesper-server/src/voice.rs` (only if the worst load fails, see Step 4)

**Interfaces:**
- Consumes: `FakeClient::with_video`, `send_video`, `received_video`, `remote_tracks_of`; protocol `ShareStart`, `Watch`.
- Produces: flags `--sharers N` (default 0) and `--share-mbps M` (default 10). Sharers are the first `N` clients, made with `with_video`, send `ShareStart { fps: 60, bitrate_kbps: M * 1000 }` and then video at `M` Mbps (60 frames a second of `M * 1_000_000 / 8 / 60` bytes). Every client sends `Watch { on: true }` for every sharer. The readiness gate counts audio tracks only (`remote_tracks_of(MediaKind::Audio) + 1 >= speakers`). PASS stays the audio rule (95%); video delivery per viewer is printed but does not decide PASS.

- [ ] **Step 1: Implement the flags** and run the existing voice run to show nothing changed: `--speakers 12 --seconds 30` still PASSes.

- [ ] **Step 2: Run the two loads** on the dev machine limited to two cores, release builds, as `docs/testing/latency.md` §1 describes, with the server under `taskset -c 0,1`:
  - Usual: `--speakers 4 --sharers 2 --share-mbps 10 --seconds 30`
  - Worst: `--speakers 4 --sharers 4 --share-mbps 20 --seconds 30`
  Expected: the usual load PASSes. Record both results, CPU model and server CPU in a new "Video load" section of `docs/testing/latency.md`. Running it on the Dockge VM itself is Toby's call; say it was not run there.

- [ ] **Step 3: If the usual load fails,** stop and report to Toby with the numbers. Do not ship.

- [ ] **Step 4: If the worst load fails,** add the cap from the spec: in `Voice::share_start`, refuse a new share with `RateLimited` and the message "Too many screens are being shared right now." when the sum over sharing members in the room of `bitrate_kbps * (members in the room - 1)`, plus the new share's, is above `MAX_ROOM_SHARE_KBPS`. Set that constant to the highest total that passed, rounded down to 10_000. Add a `signaling.rs` test `share_over_the_room_total_is_refused`, and note the cap in `docs/known-issues.md`.

- [ ] **Step 5: Commit**

```bash
git add crates/vesper-loadtest docs/testing/latency.md crates/vesper-server docs/known-issues.md
git commit -m "test(loadtest): video share load and results"
```

### Task 12: Docs and the full check

**Files:**
- Modify: `docs/known-issues.md`, `README.md`, `docs/superpowers/specs/2026-11-09-screen-share-design.md`

- [ ] **Step 1: Write the docs**
  - `docs/known-issues.md`: the Firefox bitrate ceilings, no game audio, exclusive fullscreen games can capture black, one layer for every viewer (a viewer on a bad link causes keyframe requests that cost the others a little), no limits for large calls (unless Task 11 added one), Firefox has no pop-out button in the panel.
  - `README.md`: replace "Screen share is v2." in the "Next" line with what now exists, in one short sentence.
  - Spec: add "Built, and where it differs" at the end: the pointer uses the per-socket budget instead of a new limiter kind, video m-lines are fixed at join, any Task 11 cap, and what was and was not verified (real browsers: only the e2e headless Chromium run and the stubbed canvas, not a real call).

- [ ] **Step 2: Run the full check**

Run: `scripts/check.sh`
Expected: every step passes (`deny` needs the network; if it is offline, say so).

- [ ] **Step 3: Commit**

```bash
git add docs README.md
git commit -m "docs: screen share known issues and what was built"
```

- [ ] **Step 4: Ask Toby for the real call.** The group shares in one real call, one person at a time for two minutes each, in their own browsers. Their results go in the spec's "Test page" section. Nothing in this plan verifies Firefox on a real network.
