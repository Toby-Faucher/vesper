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
- **The web app** has three pages (`web/src/App.svelte`): `/login`, `/invite` (signup; the code is read from the part after `#`, then removed from the URL) and
  `/`, the voice screen, which needs a session (`GET /api/me`, else `/login`). `web/src/lib/session.svelte.ts` keeps one socket per
  page and the state it pushes (`state.ts`). After a close it reconnects after a random wait that doubles from 1 to 2 s up to 15 to 30 s, asking `/api/me` first; the wait
  starts over only after a socket stayed up for 10 s.
  It does not reconnect after 4001 (it goes to `/login`) or 4003 (it says a moderator disconnected you). A `peer_left` with your
  own `user_id` ends the call in that tab; the socket stays open.

## Crates

| Crate | Job |
|---|---|
| `vesper-protocol` | Wire types only: `ClientMsg`, `ServerMsg`, `Peer`, `ErrorCode`, the limit `MAX_MSG_BYTES` (64 KiB) and the close codes `CLOSE_SESSION_ENDED` (4001) and `CLOSE_DISCONNECTED` (4003). `api.rs` holds REST bodies and shared structs (`Role`, `UserInfo`, `Device`, `InviteInfo`, `RoomInfo`, `Unread`). `ts-rs` generates `web/src/lib/gen/*.ts` from them. `lib.rs` |
| `vesper-store` | SQLite through sqlx (ADR 0005), the only crate that imports sqlx. WAL, foreign keys on, a pool of 4, migrations in `migrations/`. Accounts, sessions, invites, rooms, messages, search and upload rows (`accounts.rs`, `sessions.rs`, `invites.rs`, `rooms.rs`, `messages.rs`, `search.rs`, `uploads.rs`). Messages are never hard-deleted: a delete empties the body and keeps a tombstone. Triggers keep the FTS5 table `messages_fts` in step with live bodies. Reactions, mentions (user ids, parsed by the server), pins and per-room read markers have their own tables; the `uploads` table is in the same migration (`0003_messages.sql`). It stores only hashes of session tokens and invite codes. |
| `vesper-sfu` | Audio-only SFU on str0m. One UDP socket, rooms keyed by name, Opus forwarded as is (no decode, mix or transcode). `room.rs` is the loop, `client.rs` one session, `lib.rs` the public handle. No signaling inside. |
| `vesper-server` | `config.rs` (env), `lib.rs` (router, static serving, `/health`, SFU watcher, the Origin check), `api_accounts.rs` (account routes, `ApiError`, the `Authed` extractor), `api_admin.rs` (users, invites, roles, bans, `require`), `api_rooms.rs` (rooms, `can_see`), `api_messages.rs` (chat routes, body, mention and reaction rules), `uploads.rs` (uploads, the sweep, disk use), `auth.rs` (passwords, names, cookies, client address), `limits.rs` (rate limits), `cli.rs` (admin subcommands), `ws.rs` (one socket), `hub.rs` (open sockets, pushed events, closing sockets), `voice.rs` (voice presence, session numbers, force-mute, the connection check), `public_addr.rs` (advertised address). |
| `vesper-dsp` | The capture `Pipeline` around DeepFilterNet3 on the `tract` runtime, 480-sample (10 ms, 48 kHz) frames. Native for tests (feature `tract`), WASM for the browser (feature `wasm`). `scripts/build-dsp.sh` builds the WASM. |
| `vesper-loadtest` | Synthetic speakers over WebSocket and WebRTC, for load and loss checks. See `docs/testing/latency.md`. |

## Message flow

All messages are JSON on one WebSocket per tab, tagged by `type` (`vesper-protocol/src/lib.rs`).

Client to server: `join { room_id, offer }`, `answer { sdp }`, `set_voice_state { muted, deafened }`, `leave`, `check { offer }`,
`typing { room_id }`.
Server to client: `hello { me, users, rooms, voice, unread }`, `joined { user_id, answer, peers }`, `offer { sdp }`, `peer_joined`,
`peer_left`, `peer_state { user_id, muted, deafened, force_muted }`, `rooms_changed { rooms }`, `user_updated { user }`,
`force_muted { muted }`, `message_new { message }`, `message_updated { message }`, `typing { room_id, user_id }`,
`read { room_id, last_read_id }`, `error { code, message }`. `unread` in `hello` has one `Unread` per room the user can see.
Error codes: `room_full`, `bad_message`, `forbidden`, `rate_limited`.

1. The client opens `/ws` with its session cookie. The upgrade gets 403 unless `Origin` equals `VESPER_ORIGIN`, 401 unless the
   cookie names a live session, and 503 past `VESPER_MAX_SOCKETS` (`ws.rs` `upgrade`).
2. The server writes `hello` first: the user and session, every user, the rooms this user can see, and who is in voice in them.
   The socket joins the hub before `hello` is read, so no change in between is lost; changes queued meanwhile follow `hello`.
   Right after it joins the hub, the server checks the session again: a logout, ban or expiry that landed after the upgrade
   closes the socket with 4001 before `hello`. If `hello` cannot be built (a store error), it is logged and the socket closes with 1011.
3. The client builds an SDP offer, then sends `join` with a room id. The server reads the user and the room again, so a role,
   ban or member list change counts (`bad_message` if the room is unknown). `Voice::join` (`voice.rs`) refuses a banned user or
   one who cannot see the room (`forbidden`).
4. It takes the join lock and checks the room cap (12, the same for everyone; the user's own place does not count), that the
   SFU runs and that the offer is well formed. Only then does it end any earlier voice session of the same account, so a
   join refused there leaves that call up. Then it asks the SFU to join SFU room `room-<id>` with the offer. Before it enters
   the user, it checks access again against the newest copy of the room, in case the room changed meanwhile.
5. The server replies `joined` with the SFU's SDP answer and the peers already in that room. `joined` is queued under the voice
   lock, so every later presence message for that room follows it.
6. When tracks are added or stopped, the SFU makes a new offer. It arrives as `offer`; the client answers with `answer`, one at a time.
7. `set_voice_state` updates presence. `leave`, a closed socket, a kick, a lost room or the SFU ending the session removes the user.
8. The server pings every 25 s and drops a socket silent for two periods (`ws.rs`).

Each socket may send a burst of 20 messages, then 5 a second (`answer` does not count, since the server asks for it). A message
over the limit is dropped with `error { code: rate_limited }`. Past 20 such refusals (then 1 a second) the socket closes with 1008.

Presence goes to every socket whose user can see the room (`Hub::broadcast_room`), in voice or not, so the room list shows who
is talking. `peer_joined` and `peer_state` skip the session's own socket. `peer_left` also reaches the session's own socket: that
is how a tab learns its call ended (kick, another tab took over, lost access, SFU). The socket stays open for the rest.
A restricted room's presence reaches only sockets that can see it. When a room's member list or a role changes, everyone in
voice who can no longer see their room is taken out (`Voice::refresh`, called from `api_rooms::rooms_changed`). A socket that
just lost sight of a room gets no `peer_left` for the people in it: the client drops the peers of every room missing from
`rooms_changed`.

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

JSON over HTTP under `/api` (`api_accounts.rs`, `api_admin.rs`, `api_rooms.rs`, `api_messages.rs`, `uploads.rs`). Bodies are the `vesper-protocol` types in `api.rs`. A failed call returns
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
| `GET /api/rooms/{id}/messages?before=\|after=\|around=&limit=` | sees the room | `MessagePage`, oldest first; at most one cursor (else 400), `limit` 1 to 100 (default 50) |
| `GET /api/rooms/{id}/pins` | sees the room | `Message[]`, newest pin first |
| `POST /api/rooms/{id}/messages` (`SendMessage`) | sees the room, Message limit by user | 201 `Message`; pushes `message_new` |
| `PATCH /api/messages/{id}` (`EditMessage`) | sees the room, the author | the new `Message`; pushes `message_updated` |
| `DELETE /api/messages/{id}` | sees the room, the author, or a moderator or the owner | 204; pushes `message_updated` with `deleted: true` |
| `PUT /api/messages/{id}/reaction` (`ReactionReq`) | sees the room | the new `Message`; pushes `message_updated` |
| `PUT /api/messages/{id}/pin` (`PinReq`) | sees the room | the new `Message`; pushes `message_updated` |
| `PUT /api/rooms/{id}/read` (`ReadReq`) | sees the room | 204; sends `read` to every socket of the user |
| `GET /api/search?q=&room_id=&author_id=&from=&to=&before=` (`SearchQuery`) | a session, Search limit by user | `SearchPage`, 20 hits, newest first, only rooms you can see; a `room_id` you cannot see is 404 |
| `POST /api/rooms/{id}/uploads?name=` (the raw file as the body) | sees the room, Upload limit by user | 201 `UploadDone`; 413 `too_large` past 25 MiB, 507 `quota_full` |
| `GET /api/uploads/{id}` | sees the room; an upload not attached yet needs the uploader | the file (Range works), else 404 |
| `GET /api/admin/disk` | owner | `DiskUse` |

Rooms (`api_rooms.rs`, store `rooms.rs`): every room has text and voice. A fresh database has `general` and `hangout`
(migration `0002_rooms.sql`). Room names follow the display name rule (NFC, 1 to 32 characters, no control or hidden format
characters, else 400) and are unique ignoring case (409). A restricted room is seen only by the owner, the moderators and the
accounts on its member list (`can_see`); `members` is filled only while the room is restricted. Opening a room clears its member list, so restricting it again
starts empty. A banned account can not be put on a list (400). Every change pushes `rooms_changed` to open sockets.

Chat (`api_messages.rs`, store `messages.rs`): a room you cannot see, and a message in one, is 404 on every route, the same as
an unknown id. Changes are pushed with `Hub::broadcast_room`, so only sockets that can see the room get them. Rules:

- **Body.** NFC, trimmed, 1 to 4000 characters (not bytes), no control characters except newline and tab, else 400. The same
  rule holds for an edit. Bodies are never logged.
- **Who.** Anyone who can see the room posts, reacts and pins. Only the author edits (403 for anyone else, the owner too). The author
  deletes, and a moderator or the owner deletes anyone's (403 for a member on someone else's). A delete is soft: the body is
  emptied, and reactions, mentions and the pin go. A deleted message is 404 for edit, delete, reaction and pin.
- **Send limit.** 10 messages, then 1 a second, per user (429). Edits, reactions and pins are not limited.
- **Mentions.** `@name` at a word start, the name being `[a-z0-9_]{3,24}` after an ASCII lowercase, at most 20 per message, read again
  on each edit. A name that is unknown, banned or cannot see the room is dropped, so it never counts as a mention.
- **Reactions.** 1 to 32 bytes with no ASCII letter or digit, whitespace or control character (400). At most 20 different ones per message (409).
- **Pins.** At most 50 per room (409).
- **Read marker.** `last_read_id` is 0 or a message of that room (else 400), so it cannot be set past future messages. It never
  moves back; `read` carries where it really is. Unread counts skip your own and deleted messages.
- **Typing.** `typing { room_id }` on the socket goes to every other socket that can see the room (not back to the sender's
  socket). Per user and room at most one every 3 s; more, an unknown room or a hidden one is dropped without an answer. It is never
  stored and has no stop event: the client shows it for 5 s after the last one.
- A banned user's messages stay. They cannot post, since their sessions are gone.

Search (`GET /api/search`, store `search.rs`): FTS5 over `messages_fts`, tokenizer `unicode61 remove_diacritics 2`, so `cafe`
finds `Café`. Rules:

- **Query.** `fts_query` splits `q` on whitespace and control characters, cuts each term to 64 characters, drops terms with no
  letter or digit and keeps 16. Each term is quoted (inner `"` doubled) and the last one is a prefix (`"dog"*`). So `OR`, `NEAR(`,
  `body:x`, `^`, `-` and `*` are plain words and can not change the query or make it fail. A `q` with nothing left is 200 with no hits.
- **Who sees what.** The server lists the rooms the user can see (`can_see`) and the store puts that list inside the SQL
  (`m.room_id IN (SELECT value FROM json_each(?))`), so a page is never cut short or filled by rows from hidden rooms.
  Deleted messages are not in the index (the update trigger removes them); an edit is found by its new text only. A banned
  author's messages stay searchable.
- **Filters.** `room_id`, `author_id`, and `from` and `to` in unix ms, both inclusive. The server keeps no time zone: the client
  turns a local day into ms.
- **Order and pages.** Newest first; `next_before` is the last hit's id, sent back as `before`. Ids only grow, so a page never
  repeats or skips a hit when messages arrive meanwhile. Not bm25: ranking scores every match before the limit and took 57 to
  80 ms at 100k messages; newest first stops after one page.
- **Snippet.** `snippet(messages_fts, 0, char(2), char(3), '...', 12)`: `\u0002` before and `\u0003` after each match, plain text,
  never HTML. The client renders the marked parts as text in a `<mark>`. If a body itself holds either character, its snippet
  comes back with none, so a message can not fake a highlight (the body rule refuses them anyway).
- **Context.** The client opens a hit with `GET /api/rooms/{id}/messages?around=<id>`.
- **Limit.** 10 searches, then 1 a second, per user (429).
- `messages_fts` `integrity-check` reports a mismatch once any message is deleted, since deleted bodies are left out on purpose.
  Do not use it as a health check.

Uploads (`uploads.rs`, store `uploads.rs`): one file per request, sent as the raw body (no multipart). Rules:

- **Size.** At most 25 MiB (`MAX_UPLOAD`). A larger `Content-Length` is 413 before the body is read; a body with no length is
  counted while it streams to `uploads/tmp/<id>` and is cut off with 413 once it passes the cap. The partial file is deleted, also
  when the client goes away mid-body. The Upload limit is 10, then 1 every 30 s, per user.
- **Quota.** Uploads together stay under `VESPER_UPLOAD_QUOTA_MB` ([env.md](env.md)). The check, the move into place and the row
  insert run under one lock, so uploads racing each other cannot pass it (507 `quota_full`).
- **On disk.** The file name is the id (32 random hex), under `uploads/<first 2>/`, dirs 0700 and files 0600. A request id that is
  not 32 lowercase hex is 404 before any path is built, so no client text ever reaches a path.
- **Type.** Decided from the first bytes, never from the name or the request's `Content-Type`. Only png, jpeg, gif and webp count
  as images (`Attachment.image`) and are served inline with their type. Everything else, SVG and HTML included, is
  `application/octet-stream` with `Content-Disposition: attachment`.
- **Name.** Display text only: control, bidi and hidden format characters dropped, `/` and `\` turned into `_`, at most 200
  characters, `file` when empty. `Content-Disposition` carries an ASCII `filename` and the exact name as `filename*` (RFC 6266 and
  5987), so a name can not add a header.
- **Served headers.** `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox`,
  `Cross-Origin-Resource-Policy: same-origin`, `Cache-Control: private, max-age=31536000, immutable` (the file behind an id never changes).
- **Who.** Attaching (`SendMessage.attachments`) needs your own upload, not attached yet, in the same room (else 409). An attached
  file is served to anyone who can see the room; one not attached yet only to its uploader. Everything else is the same 404 as an
  unknown id.
- **Removal.** Deleting a message deletes its files. Uploads still not attached after 24 h are swept every hour (and at start). A
  file whose delete fails stays on disk with no row; nothing sweeps it.
- **Disk use.** `DiskUse` holds the database plus WAL size, the sum of upload sizes, the quota, and the filesystem total and free
  space of the data dir (`statvfs`). `warn` is set when uploads pass 80% of the quota or the filesystem is over 80% full.

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
- **Caching.** Every `/api` response has `Cache-Control: no-store`, except a served upload.
- Handlers never log request bodies, and the request types with a password or invite code print `***` in `Debug`. A body that does
  not parse gets a fixed "invalid request body" message. The WebSocket upgrade takes the same session cookie and Origin check.

Tests: `crates/vesper-server/tests/accounts.rs`, `tests/admin.rs`, `tests/rooms.rs` (every route for every caller and target role) `tests/chat.rs` (chat routes, pushes, typing, unread, search) and `tests/uploads.rs` (limits, quota race, served headers, the access matrix, sweep), through
`tests/common/mod.rs`; the voice actions in `tests/signaling.rs` (`voice_actions_check_roles`).

## Performance

Measured on 2026-11-01 on bawn (Intel i7-12700, 20 threads), release build, by
`scripts/capped.sh cargo test --release -p vesper-store --test perf -- --ignored --nocapture`: 100 000 messages in 10 rooms by
10 authors, 10 to 300 characters from a 22-word list, so every word matches most messages (a hard case for search). Search runs
over all 10 rooms with a page of 20; p50 and p95 of 50 runs. The database sat in `/tmp`, which is tmpfs (RAM) on bawn, so disk
speed is not in these numbers. The box was not idle, so the numbers move a few ms between runs.

| What | Time |
|---|---|
| Insert 100 000 messages in one transaction | 2.6 s |
| `history` latest 50 / before the middle 50 | 0.38 ms / 0.31 ms |
| `unread` for 10 rooms | 7.7 ms |
| Search one word (`hello`) | p50 1.7 ms, p95 1.9 ms |
| Search three words (`voice game tonight`) | p50 1.9 ms, p95 2.2 ms |
| Search a prefix (`mu`) | p50 2.8 ms, p95 4.3 ms |
| Search one word where only the oldest 1000 messages pass the `to` filter (the whole index is walked) | p50 19.5 ms, p95 20.1 ms |
| Database size after a checkpoint | 37.5 MiB |

The test fails if any of these is 50 ms or more. Not measured: the 1 GB VM (expect it slower), a database with years of real chat.
A one-letter prefix is still the slow case; the client should not search on one character.

## Static serving

`guard` in `vesper-server/src/lib.rs`: dotfiles and `%2e` are 404; an extensionless 404 gets `index.html` (SPA routes); a missing file with
an extension or under `/api/` stays 404; `/assets/*` is `immutable`, `/api/*` `no-store` (unless the handler set its own, as uploads do), everything else `no-cache`; `nosniff` on all. Tests: `crates/vesper-server/tests/static_files.rs`.

## Not verified

Read from the code on 2026-10-30, not run:

- Behaviour in real Chrome and Firefox, including the `ontrack` stream id and `replaceTrack` during a mode swap.
- Everything in the SFU loop beyond its doc comments (`room.rs` forwarding, `client.rs` negotiation).
- Text chat and rooms beyond a name: not built (spec "Build order").
- The accounts and admin REST API is built and tested in process only (`tests/accounts.rs`, `tests/admin.rs`). No real browser run, and no https run of the
  `Secure` `__Host-` cookie beyond the in-process test.
