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
│ mic ─► worklet ⇄ DSP      │   (via Caddy)      │   voice.rs  presence, session ids   │
│ worker (DeepFilterNet     │                    │   ws.rs     one task per socket     │
│                           │                    │   hub.rs    open sockets, pushes    │
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
| `vesper-protocol` | Wire types only: `ClientMsg`, `ServerMsg`, `Peer`, `ErrorCode`, the limit `MAX_MSG_BYTES` (64 KiB) and the close codes `CLOSE_SESSION_ENDED` (4001) and `CLOSE_DISCONNECTED` (4003). `api.rs` holds REST bodies and shared structs (`Role`, `UserInfo`, `Device`, `InviteInfo`, `RoomInfo`, `Unread`). `ts-rs` generates `web/src/lib/gen/*.ts` from them. `lib.rs` |
| `vesper-store` | SQLite through sqlx (ADR 0005), the only crate that imports sqlx. WAL, foreign keys on, a pool of 4, migrations in `migrations/`. Accounts, sessions, invites and rooms (`accounts.rs`, `sessions.rs`, `invites.rs`, `rooms.rs`). It stores only hashes of session tokens and invite codes. |
| `vesper-sfu` | Audio-only SFU on str0m. One UDP socket, rooms keyed by name, Opus forwarded as is (no decode, mix or transcode). `room.rs` is the loop, `client.rs` one session, `lib.rs` the public handle. No signaling inside. |
| `vesper-server` | `config.rs` (env), `lib.rs` (router, static serving, `/health`, SFU watcher, the Origin check), `api_accounts.rs` (account routes, `ApiError`, the `Authed` extractor), `api_admin.rs` (users, invites, roles, bans, `require`), `api_rooms.rs` (rooms, `can_see`), `auth.rs` (passwords, names, cookies, client address), `limits.rs` (rate limits), `cli.rs` (admin subcommands), `ws.rs` (one socket), `hub.rs` (open sockets, pushed events, closing sockets), `voice.rs` (voice presence, session numbers, force-mute, the connection check), `public_addr.rs` (advertised address). |
| `vesper-dsp` | The capture `Pipeline` around DeepFilterNet3 on the `tract` runtime, 480-sample (10 ms, 48 kHz) frames. Native for tests (feature `tract`), WASM for the browser (feature `wasm`). `scripts/build-dsp.sh` builds the WASM. |
| `vesper-loadtest` | Synthetic speakers over WebSocket and WebRTC, for load and loss checks. See `docs/testing/latency.md`. |

## Message flow

All messages are JSON on one WebSocket per tab, tagged by `type` (`vesper-protocol/src/lib.rs`).

Client to server: `join { room_id, offer }`, `answer { sdp }`, `set_voice_state { muted, deafened }`, `leave`, `check { offer }`,
`typing { room_id }`. `typing` is ignored until text chat.
Server to client: `hello { me, users, rooms, voice, unread }`, `joined { user_id, answer, peers }`, `offer { sdp }`, `peer_joined`,
`peer_left`, `peer_state { user_id, muted, deafened, force_muted }`, `rooms_changed { rooms }`, `user_updated { user }`,
`force_muted { muted }`, `error { code, message }`. `unread` is empty until text chat.
Error codes: `room_full`, `bad_message`, `forbidden`, `rate_limited`.

1. The client opens `/ws` with its session cookie. The upgrade gets 403 unless `Origin` equals `VESPER_ORIGIN`, 401 unless the
   cookie names a live session, and 503 past `VESPER_MAX_SOCKETS` (`ws.rs` `upgrade`).
2. The server writes `hello` first: the user and session, every user, the rooms this user can see, and who is in voice in them.
   The socket joins the hub before `hello` is read, so no change in between is lost; changes queued meanwhile follow `hello`.
3. The client builds an SDP offer, then sends `join` with a room id. The server reads the user and the room again, so a role,
   ban or member list change counts (`bad_message` if the room is unknown). `Voice::join` (`voice.rs`) refuses a banned user or
   one who cannot see the room (`forbidden`).
4. It takes the join lock, replaces any earlier voice session of the same account, checks the room cap (12, the same for
   everyone), then asks the SFU to join SFU room `room-<id>` with the offer.
5. The server replies `joined` with the SFU's SDP answer and the peers already in that room. `joined` is queued under the voice
   lock, so every later presence message for that room follows it.
6. When tracks are added or stopped, the SFU makes a new offer. It arrives as `offer`; the client answers with `answer`, one at a time.
7. `set_voice_state` updates presence. `leave`, a closed socket, a kick, a lost room or the SFU ending the session removes the user.
8. The server pings every 25 s and drops a socket silent for two periods (`ws.rs`).

Presence goes to every socket whose user can see the room (`Hub::broadcast_room`), in voice or not, so the room list shows who
is talking. `peer_joined` and `peer_state` skip the session's own socket. `peer_left` also reaches the session's own socket: that
is how a tab learns its call ended (kick, another tab took over, lost access, SFU). The socket stays open for the rest.
A restricted room's presence reaches only sockets that can see it. When a room's member list or a role changes, everyone in
voice who can no longer see their room is taken out (`Voice::refresh`, called from `api_rooms::rooms_changed`).

The connection check: `check { offer }` starts an SFU session under a random 32 hex id in an SFU room of its own
(`check-<id>`) and answers `joined` with that id and no peers. It is not in presence and does not touch the user's call, since
the SFU keys sessions by that id, not the account. A new `check` ends the last one; so do `leave` and a closed socket.

Force-mute (`Voice::force_mute`): sets `force_muted` on the account, sends `force_muted { muted }` to all of that user's sockets
and `peer_state` to the room. While it holds, `set_voice_state` cannot clear `muted`. It holds across leave and rejoin until a
moderator lifts it, and lifting does not unmute. It lives in memory, so a restart lifts it. Only the client stops sending audio
([known-issues.md](known-issues.md#security-and-abuse)).

Pushed from REST (`hub.rs`): a room change sends each socket `rooms_changed` with only the rooms its user can see; a role
change also does, since the role decides that. A role, ban or profile change sends everyone `user_updated`.

Closing (`hub.rs`): logout, revoking a device, a login that replaces a presented cookie, and a password change (other sessions
only) close the sockets of the ended sessions with code 4001. A ban closes all of that user's sockets with 4001, which also
takes them out of voice. A moderator disconnect closes all of that user's sockets with 4003; the sessions stay valid, so the
client may reconnect. Each socket has a queue of 256 messages and every send is non-blocking: a send to a full queue closes that
socket with 1013, through a signal outside the queue, so a client that stopped reading cannot hold memory or the task. A closing
socket leaves the hub and voice first, then waits up to 1 s for the client's close reply. Ending a voice session never closes
the socket.

Tests: `crates/vesper-server/tests/signaling.rs` (a real server and sockets, `FakeClient` as the browser's WebRTC side) and the
unit tests in `hub.rs` and `voice.rs`.

## Identity, sessions and the MSID rule

- A user is an account. The WebSocket takes the session cookie on the upgrade; no message carries a name or a key.
- `user_id` is `accounts.id`: 16 random bytes as 32 lowercase hex characters. Hex keeps it a valid MSID stream id (RFC 8830).
- The SFU labels each forwarded track with the sender's `user_id` as its MSID stream id (`vesper-sfu/src/client.rs`, `TrackIn.user`). In the
  browser, `ontrack` gives `MediaStream.id` equal to that `user_id`, and the client keys remote streams on it (`signaling.ts`).
- A second voice join by the same account (another tab) takes the call over. The first tab's socket stays open and gets
  `peer_left` for itself; it may join again, which takes the call back. Each join gets a fresh `session` number
  (`next_session`, `voice.rs`). `answer`, `set_voice_state` and `leave` carry the socket's session and are
  ignored if it is no longer current, because the SFU keys sessions by user alone. SFU offers for a stale session are not
  forwarded to the new socket.
- Joins are serialised by one lock, so a rejoin finishes closing the old SFU session (up to a timeout) before anyone else joins.

## SFU ownership

- The SFU thread owns the UDP socket, every str0m `Rtc` client and all media state. Nothing else touches them. The server sends it commands
  (`join`, `answer`, `leave`, `set_advertised`) over a channel and gets `SfuEvent::Offer` and `SfuEvent::Closed` back per session.
- `Voice` owns presence: who is in which room, muted, deafened or force-muted, and the socket for each session. It keeps the
  latest copy of each room for visibility checks. It does not touch media.
- A session ends on ICE timeout, an error, an offer unanswered for `answer_timeout` (10 s), `leave`, or a new join by the same user
  (`SfuEvent` docs, `vesper-sfu/src/lib.rs`).
- Advertised address: with `VESPER_PUBLIC_HOST` set, the server resolves it at start and every 60 s and calls `set_advertised`. A failed
  resolve keeps the old address. Only sessions started afterwards use a changed address, so people in a call rejoin after the public IP changes
  (`public_addr.rs`).

## Audio pipeline (browser)

Order, fixed by the spec: microphone, browser echo cancellation (always on), gain (the input boost), DeepFilterNet, Opus encode, WebRTC to the SFU.

- In `deepfilter` mode: mic through a gain node (the input boost) into an AudioWorklet, which exchanges 10 ms frames with a DSP worker over a MessageChannel; worklet output goes
  into a `MediaStreamDestination`, and that track is what WebRTC sends (`web/src/lib/audio/capture.ts`, `denoise.worklet.ts`,
  `dsp.worker.ts`, `dsp-handler.ts`).
- Buffers in `denoise-core.ts`: up to 4 frames at the worker, a 4 frame input ring, and an output queue that starts playing at 3 frames
  (30 ms) and holds at most 8 (80 ms); if it stays above 5 for 2 s it drops one frame a second down to 3. They absorb jitter in the worklet to worker messages. The delay they add is estimated, not measured.
- In `browser` and `off` modes the raw mic track is sent, so the fallback does not depend on the AudioContext or the worklet. Browser noise
  suppression is on only in `browser` mode, or in `deepfilter` mode if the user turned it on.
- Opus is tuned by editing the SDP (`web/src/lib/audio/opus.ts`): `maxaveragebitrate`, `usedtx`, `useinbandfec`, mono, ptime. The send bitrate is
  also set on the sender (`web/src/lib/devices.ts`). Remote SDP is tuned too, then applied (`signaling.ts`).

## DSP pipeline (Rust)

`vesper-dsp` runs each 10 ms frame through a `Pipeline`: input gain, pre stages, DeepFilterNet, post stages. A `PipelineConfig`
sets each step, and its default gives the same output as the shipped denoiser bit for bit (a golden hash test pins it). The WASM
`WasmDenoiser` is backed by the same `Pipeline`, so an audio experiment changes Rust only and never needs a change under `web/`.
DeepFilterNet3 delays the output by 1440 samples (30 ms), which a test checks against a measured delay to within one sample; `latency_samples()` reports what our own stages add on top.
The `serde` feature lets the audio lab load configs from TOML. It is off by default and the WASM build does not use it.

## Fallback logic

- Modes: `deepfilter`, `browser`, `off` (`web/src/lib/audio/mode.ts`).
- `ModeController` trips from `deepfilter` to `browser` when 20 of the last 100 frames each took longer than 30 ms. The first 300 frames (about 3 s) are ignored. It stays tripped until `reset()`.
  The numbers are set in `denoise-core.ts`.
- A load failure, a missing model or a worker that stalls also switches to `browser`, with a reason string the UI shows
  (`capture.ts` `fail` and `switchTo`). The call keeps going. The track is swapped in place and `replaceTrack` is used on the sender.
- The worker gets 20 s to report `ready` (`READY_TIMEOUT_MS` in `capture.ts`). The raw mic passes through, and the status line says `loading, unfiltered`, until then.
- The page starts fetching the denoiser files (`web/src/lib/audio/preload.ts`, called from `main.ts`) when it loads, before Join. The server answers `/dsp/*` with `no-cache`, so the worker's later fetch is a revalidation. Skipped when the browser asks to save data. Not verified in a real browser.
- The server side has no fallback. A dead SFU is not restarted in process; the process exits and the supervisor restarts it.

## HTTP API

JSON over HTTP under `/api` (`api_accounts.rs`, `api_admin.rs`, `api_rooms.rs`). Bodies are the `vesper-protocol` types in `api.rs`. A failed call returns
`ApiError { code, message }`, and the status follows the code: `bad_request` 400, `unauthorized` 401, `forbidden` 403,
`not_found` and `invite_invalid` 404, `conflict` 409, `too_large` 413, `rate_limited` 429 (with `Retry-After` in seconds),
`internal` 500, `quota_full` 507.

| Route | Needs | Returns |
|---|---|---|
| `GET /api/invites/preview?code=` | Invite limit by address | `InvitePreview` or 404 `invite_invalid` |
| `POST /api/signup` (`SignupReq`) | Invite limit by address | 201 `Me` and the session cookie |
| `POST /api/login` (`LoginReq`) | Login limits by address, then by address and username | `Me` and a new session cookie |
| `POST /api/logout` | a session | 204, the session is deleted and the cookie cleared |
| `GET /api/me` | a session | `Me` |
| `PATCH /api/me` (`ProfilePatch`) | a session | the new `UserInfo` |
| `POST /api/me/password` (`PasswordChange`) | a session, the old password, Login limit | 204, every other session is deleted |
| `GET /api/sessions` | a session | `Device[]`, `current` marks this one |
| `DELETE /api/sessions/{id}` | a session | 204 (the cookie is cleared if it is this session), or 404 if the id is not one of yours |
| `GET /api/users` | a session | `UserInfo[]` |
| `GET /api/invites` | moderator or owner | `InviteInfo[]`, newest first |
| `POST /api/invites` (`InviteCreate`) | moderator or owner; a moderator invite needs the owner | 201 `InviteCreated` with the link `<origin>/invite#<code>` |
| `DELETE /api/invites/{id}` | moderator or owner | 204, or 404 for an unknown id |
| `PATCH /api/users/{id}/role` (`RolePatch`) | owner | the new `UserInfo` |
| `POST /api/users/{id}/ban` | owner | the new `UserInfo`; every session of that user is deleted |
| `POST /api/users/{id}/unban` | owner | the new `UserInfo` |
| `POST /api/users/{id}/kick` | moderator or owner, on a lower role | 204, the user leaves voice; 404 if not in voice |
| `POST /api/users/{id}/force-mute` (`ForceMute`) | moderator or owner, on a lower role | 204; `muted: false` lifts it; works when not in voice too |
| `POST /api/users/{id}/disconnect` | moderator or owner, on a lower role | 204, every socket of the user closes with 4003 |
| `GET /api/rooms` | a session | `RoomInfo[]` by position, only the rooms you can see |
| `POST /api/rooms` (`RoomCreate`) | owner | 201 `RoomInfo`, open, placed last |
| `PATCH /api/rooms/{id}` (`RoomPatch`) | owner | the new `RoomInfo` |
| `PUT /api/rooms/order` (`RoomOrder`) | owner | 204; `ids` must hold every room id once, else 409 |
| `PUT /api/rooms/{id}/members` (`RoomMembers`) | owner | the new `RoomInfo`; the list replaces the old one, an unknown account is 404, a banned one 400 |

Rooms (`api_rooms.rs`, store `rooms.rs`): every room has text and voice. A fresh database has `general` and `hangout`
(migration `0002_rooms.sql`). Room names follow the display name rule (NFC, 1 to 32 characters, no control or hidden format
characters, else 400) and are unique ignoring case (409). A restricted room is seen only by the owner, the moderators and the
accounts on its member list (`can_see`); `members` is filled only while the room is restricted. Opening a room clears its member list, so restricting it again
starts empty. A banned account can not be put on a list (400). Every change pushes `rooms_changed` to open sockets.

A ban ends the user's sessions but does not revoke invites they already created; the owner revokes those by hand. `InviteCreate` needs all three fields (`role`, `max_uses`, `expires_hours`), so the web client must send them.

Roles (`require`, `api_admin.rs`; owner > moderator > member):

| Action | Member | Moderator | Owner |
|---|---|---|---|
| List users | yes | yes | yes |
| List and revoke invites | no | yes | yes |
| Mint a member invite | no | yes | yes |
| Mint a moderator invite | no | no | yes |
| Change a role, ban, unban | no | no | yes, not on themselves or an owner |
| Mint or grant the owner role | no | no | no, only `vesper-server create-owner` |
| See a restricted room | only when listed | yes | yes |
| Create, rename, reorder, restrict a room or set its members | no | no | yes |
| Kick from voice, force-mute, disconnect | no | on members only | yes, not on themselves |

A refused role is 403. An `owner` role in `InviteCreate` or `RolePatch`, the owner acting on their own account, `max_uses` outside
1 to 50 and `expires_hours` outside 1 to 720 are 400. The voice actions on yourself, a fellow moderator or the owner are 403. A role change counts from the user's next request and their next voice join. A banned user's cookie
gets 401 at once and login fails until an unban; the unban does not bring old sessions back.

Rules every route keeps:

- **Origin.** A request that is not GET or HEAD needs `Origin` equal to `VESPER_ORIGIN`, else 403 before the handler runs
  (`require_origin`, `lib.rs`).
- **Body cap.** JSON bodies over 32 KiB get 413 (`API_BODY_LIMIT`).
- **Sessions.** The cookie holds a random token; the database holds its SHA-256. `Authed` turns the cookie into the user and
  session id, or 401 when it is missing, malformed, unknown, expired or the user is banned.
- **Login.** Every failure is the same 401 "wrong username or password". An unknown user still costs one argon2 check, so the
  time taken should not tell whether the name exists; this is reasoned from the code, not measured. A username that fails the
  format check is refused before any argon2 check. A cookie sent with a login or signup is deleted, and a new token is always minted.
- **Client address.** Rate-limit keys use the client address (`client_ip`, `auth.rs`), and IPv6 clients are keyed on their /64.
  When the TCP peer is loopback or private (10/8, 172.16/12, 192.168/16, IPv6 unique-local), the server takes a valid
  `CF-Connecting-IP`, else the last `X-Forwarded-For` entry, else the peer. From any other peer it ignores both headers.
  Behind Cloudflare, `cloudflared` and Caddy the peer is private and Cloudflare overwrites `CF-Connecting-IP`, so the key is the real
  client. Reached on a private network without Cloudflare, a LAN client could set that header itself
  ([known-issues.md](known-issues.md#security-and-abuse)). The login limit per username
  is keyed on address and username, so each faked address gets that username 5 more tries, then 1 a minute.
- **Invites.** Unknown, used up, expired, revoked and changed codes all give the same 404 body. An owner invite stops working
  once an owner exists; the store checks this in the signup transaction.
- **Caching.** Every `/api` response has `Cache-Control: no-store`.
- Handlers never log request bodies, and the request types with a password or invite code print `***` in `Debug`. A body that does
  not parse gets a fixed "invalid request body" message. The WebSocket upgrade takes the same session cookie and Origin check.

Tests: `crates/vesper-server/tests/accounts.rs`, `tests/admin.rs` and `tests/rooms.rs` (every route for every caller and target role), through
`tests/common/mod.rs`; the voice actions in `tests/signaling.rs` (`voice_actions_check_roles`).

## Static serving

`guard` in `vesper-server/src/lib.rs`: dotfiles and `%2e` are 404; an extensionless 404 gets `index.html` (SPA routes); a missing file with
an extension or under `/api/` stays 404; `/assets/*` is `immutable`, `/api/*` `no-store`, everything else `no-cache`; `nosniff` on all. Tests: `crates/vesper-server/tests/static_files.rs`.

## Not verified

Read from the code on 2026-10-30, not run:

- Behaviour in real Chrome and Firefox, including the `ontrack` stream id and `replaceTrack` during a mode swap.
- Everything in the SFU loop beyond its doc comments (`room.rs` forwarding, `client.rs` negotiation).
- Text chat and rooms beyond a name: not built (spec "Build order").
- The accounts and admin REST API is built and tested in process only (`tests/accounts.rs`, `tests/admin.rs`). No real browser run, and no https run of the
  `Secure` `__Host-` cookie beyond the in-process test.
