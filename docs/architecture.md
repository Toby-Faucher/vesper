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
  `/health` answers `{"status":"ok","version":"<release>","build":"<id>"}` (503 with `"status":"sfu down"`). The build id is the git commit compiled in by
  `build.rs` (the Dockerfile's `VESPER_BUILD_ID` arg, else `build-id.txt` as `git archive` fills it in, else the version), or `VESPER_BUILD_ID` at run time.
  At start, before the migrations run, it backs the database up into `<data>/backups` when migrations are pending, and does not start if that fails (`backup_before_migration`, `lib.rs`).
  It does no TLS; Caddy does. Media never goes through Caddy. (`crates/vesper-server/src/lib.rs`, `deploy/Caddyfile.example`)
- **The SFU** runs on its own OS thread (`vesper-sfu` thread, `crates/vesper-sfu/src/lib.rs`). The server talks to it through a cloneable
  `SfuHandle`. If the thread dies, `/health` returns 503 and the process exits with code 1 so Docker or systemd restarts it
  (`watch_sfu` in `lib.rs`).
- **The browser** captures the mic, cleans it with DeepFilterNet in WASM, and sends Opus over WebRTC (`web/src/lib/audio/`, `signaling.ts`).
- **The web app** (`web/src/App.svelte`, paths in `web/src/lib/route.ts`) has `/login`, `/reset` (a new password from a reset link; the code is read and removed like an invite's), `/invite` (signup; the code is read from the part after `#`, then removed from the URL; a second link opened in the
  same tab is read too), `/welcome` (after signup, and from Settings > Account: name and colour, a mic check, mention notifications; every step can be skipped; it renders inside the Shell, so a call keeps running)
  and the app shell (`web/src/routes/Shell.svelte`) for every other path, which needs a session (`GET /api/me`, else `/login`): `/r/<id>` (a room),
  `/settings/<section>`, `/admin/<section>`, and `/`, which opens the last room (`localStorage['vesper.lastRoom']`) or the first. Any other path
  shows a plain not-found page. The shell starts the call store once, so a call stays up while you move between pages, and shows the call bar (`CallBar.svelte`) at the bottom of the main pane while you are in voice. A person in the sidebar or the call bar, and a name, avatar or mention in a message or in Admin > Members, opens their person card (`PersonCard.svelte`: bio, role, volume, connection quality, and the moderator voice actions; names use `PersonButton.svelte`). Connection quality per person comes from the call's one `getStats()` every 2 s: each remote track's `inbound-rtp` (loss over the window, jitter, concealed samples) and, for you, the round trip and loss. `web/src/lib/ui/link.ts` grades it and holds a grade until two windows in a row agree on a new one (the math is in `link-stats.ts`, in the call chunk with the rest of the call code, see [dev.md](dev.md)). Three windows in a row without a packet from someone who is not muted turn their icon back to measuring; your own row counts as all lost while ICE is disconnected. No server change. "Even out voices" (Settings > Voice, off by default, saved in `vesper.level`) adds a gain per remote person in `playback.ts`, under the manual volume: `web/src/lib/level.ts` averages the speech level over about 3 s from the speaking analyser (frames under -40 dBFS are skipped, so the gain holds in silence and room noise is never lifted) and moves the gain towards a common level, at most 10 dB either way, cutting in about 1 s and lifting in about 4 s. Each join starts at 0 dB. It is on the playback side only; the mic path is not touched. The manual volume on the person card goes from 0 to 400 % (+20 dB at the top, `web/src/lib/volume.ts`), and a limiter after it (a `DynamicsCompressorNode` at -3 dBFS) keeps a loud word at a high volume from clipping. `web/src/lib/session.svelte.ts` keeps one socket per
  page and the state it pushes (`state.ts`). After a close it reconnects after a random wait that doubles from 1 to 2 s up to 15 to 30 s, asking `/api/me` first; the wait
  starts over only after a socket stayed up for 10 s.
  It does not reconnect after 4001 (it goes to `/login`) or 4003 (it says a moderator disconnected you). Nor after a server build change: the page keeps the build id (`Me.server.build`) of its first `hello`; when a later `hello` or a reconnect's `/api/me` names another, it closes the socket for good without applying that `hello`, so it never acts on messages of a newer protocol, the composer stays blocked like offline, and `ConnectionBanner.svelte` shows "Vesper was updated. Reload to continue." with a Reload button. It compares build ids, not versions, and never reloads by itself, so it can not loop. A `peer_left` with your
  own `user_id` ends the call in that tab; the socket stays open.
- **The chat panel** (`web/src/routes/Chat.svelte`, store in `web/src/lib/chat.svelte.ts`) fills the open room's page (`RoomView.svelte`) under its
  header. History, sends, edits, deletes, reactions, pins, read markers, uploads and search go over REST; new and changed messages and
  typing come over the socket. A `message_updated` copy with a smaller `updated_at` than the one held is dropped, and after
  every `hello` the shown room's newest page is fetched again and merged, so missed or reordered events heal. A message pushed while that
  page is in flight (also on a room's first open) is held and kept. Re-entering a room left on an older slice after a search jump shows its newest page. Unread counts come from `Hello.unread` and live events (`state.ts`); a room is
  marked read when its newest message is on screen in a visible tab. Typing is sent at most every 3 s and shown for 5 s. The shown line has two dots that pulse and, for one typer, their picture; both enter and leave with the `row` preset inside the fixed 16 px line, so the list never moves, and under reduced motion only the text stays. The text is the live region. Drafts are kept in memory
  only. Search is a dialog (`SearchDialog.svelte`, Ctrl/Cmd+Shift+F or the header's Search button) with `from:`, `in:`, `after:` and
  `before:` in the text (`lib/ui/searchq.ts`; dates are whole local days, both included; the box takes 200 characters and sends at most 12
  words). Ctrl/Cmd+Shift+F works while typing in the composer too. On macOS, Cmd+Shift+F may also be a browser's full screen key (not
  verified). The dialog waits 300 ms after the last key, does not send a lone one-letter word, aborts a request the next query replaces,
  retries a 429 once after a second and then says search is busy until Enter, drops the hits of a room you can no longer see and asks again,
  and a result jumps to the message with `around=`. New messages from others are read out by a polite live region; paste
  takes only PNG, JPEG, GIF and WebP images. A pasted image named `image.png` gets a name like `Pasted image 2026-11-03 14.05.png` (local time, ` (2)` for a repeat); other names stay.
- **Markdown** (`web/src/lib/markdown.ts`) is a hand-written parser into a tree that `Markdown.svelte` renders with text nodes and fixed
  elements. The server stores and searches the raw text; only the client formats it. This is the whole subset:

  | Written | Shown |
  |---|---|
  | `**bold**`, `*italic*` or `_italic_`, `~~strike~~` | bold, italic, struck out; nesting stops at 4 levels; `_` inside a word (`snake_case`) is text |
  | `` `code` `` | inline code |
  | a line of ```` ``` ````, lines, a line of ```` ``` ```` | a code block. One word after the opening fence (`` ```rust ``) is the language, shown as the block's label. Other text on a fence line is shown as text |
  | `# `, `## `, `### ` at the start of a line | a heading at 20, 16 or 14 px; `####` to `######` show as `###` |
  | `> ` at the start of lines | one level of quote |
  | `- `, `* ` or `+ `, and `1. ` or `1) ` at the start of lines | a bullet or numbered list; a numbered list starts at its first number. An item indented with more spaces than the one above nests in it, up to 4 levels |
  | `---`, `***` or `___` alone on a line | a rule |
  | a bare `http://` or `https://` URL | a link |
  | `@name` | a mention |

  Not supported, shown as typed: raw HTML, images (`![x](y)`), `[text](url)` links (the text could hide where a link goes),
  tables, task lists, backslash escapes, setext (underlined) headings and indented code. An unclosed marker is text.
  A link is kept only when `new URL` parses it as http or https with no user name or password, and its text is the full parsed
  `href` (ASCII: other characters percent-encoded, hosts punycoded) so it cannot read differently from where it goes. Links get
  `rel="noopener noreferrer nofollow"`, `target="_blank"` and `dir="ltr"`. No `{@html}`, `innerHTML` or similar sink exists in `web/src`
  (`no-raw-html.test.ts` fails on one). The parser is one pass per line with a delimiter stack; a test parses 100k-character hostile inputs under 250 ms.
  Code blocks with a known language (rust, js and ts, json, shell, python, html and xml, css: `langOf`) are coloured by `lib/highlight.ts`,
  a small tokenizer in its own chunk (about 1.3 kB gzip), loaded the first time such a block is shown. Each block has a Copy button.
  The colours are the `--syn-*` tokens (no amber). Reply quotes, the reply bar, pins, search results, notifications and announcements show
  `plain(parse(body))`: the text without the syntax, on one line. The reply excerpt is the server's first 100 characters, so a marker cut
  in half shows as typed.
  The composer is a live preview: the textarea's text is transparent and a layer under it draws the same characters from `decorate()`,
  which cuts the source into styled runs by the same rules, markers dimmed. The layer changes only colour, background, stroke and
  decoration, so its glyphs keep the textarea's widths; italic is therefore not slanted there.

## Crates

| Crate | Job |
|---|---|
| `vesper-protocol` | Wire types only: `ClientMsg`, `ServerMsg`, `Peer`, `ErrorCode`, the limit `MAX_MSG_BYTES` (64 KiB) and the close codes `CLOSE_SESSION_ENDED` (4001) and `CLOSE_DISCONNECTED` (4003). `api.rs` holds REST bodies and shared structs (`Role`, `UserInfo`, `Device`, `InviteInfo`, `RoomInfo`, `Unread`). `ts-rs` generates `web/src/lib/gen/*.ts` from them. `lib.rs` |
| `vesper-store` | SQLite through sqlx (ADR 0005), the only crate that imports sqlx. WAL, foreign keys on, a pool of 4, migrations in `migrations/`. Accounts, sessions, invites, rooms, messages, search and upload rows (`accounts.rs`, `sessions.rs`, `invites.rs`, `rooms.rs`, `messages.rs`, `search.rs`, `uploads.rs`). Messages are never hard-deleted: a delete empties the body and keeps a tombstone. Triggers keep the FTS5 table `messages_fts` in step with live bodies. Reactions, mentions (user ids, parsed by the server), pins and per-room read markers have their own tables; the `uploads` table is in the same migration (`0003_messages.sql`). `messages.updated_at` (`0004_updated_at.sql`) is unix ms, raised to `max(old + 1, now)` inside every edit, delete, reaction and pin change, so of two copies of one message the larger is the newer, even when the pushes arrive out of order. Bios and profile pictures (`0005_profile.sql`): `accounts.bio`, `accounts.avatar` (the picture version) and the `avatars` table. The UI accent from Settings > Appearance (`0006_ui_accent.sql`): `accounts.ui_accent`, a preset key or `#rrggbb`, NULL for amber. Password resets (`0007_password_resets.sql`, `resets.rs`): every reset row is kept as an audit (who made it, used when and from the LAN or the internet, shown to the person); at most one live code per account (a partial unique index), cancelled by a new one, a role change, a ban or unban and a password change; a role change or ban of the maker cancels the live links they made. Archived rooms (`0008_room_archive.sql`): `rooms.archived_at`, NULL while not archived. `Store::backup_into` writes a consistent copy with `VACUUM INTO` (for `vesper-server backup` and the automatic backups). `Store::schema_versions` reads the newest applied and the newest embedded migration without changing the file, so the server can back up before `Store::open` migrates. Passkeys (`0013_passkeys.sql`, `passkeys.rs`): one row per WebAuthn credential, the credential id unique and the `webauthn-rs` credential kept as JSON (public key and counter), deleted with the account. It stores only hashes of session tokens, invite codes and reset codes. |
| `vesper-sfu` | SFU on str0m. One UDP socket, rooms keyed by name, Opus voice and one VP8 screen per client forwarded as is (no decode, mix or transcode). `room.rs` is the loop, `client.rs` one session, `lib.rs` the public handle. No signaling inside. |
| `vesper-server` | `config.rs` (env), `lib.rs` (router, static serving, `/health`, SFU watcher, the Origin check), `api_accounts.rs` (account routes, `ApiError`, the `Authed` extractor), `api_passkeys.rs` (passkeys, WebAuthn through `webauthn-rs`), `api_admin.rs` (users, invites, reset links, roles, bans, `require`), `api_rooms.rs` (rooms, `can_see`, `listed`, `visible_room`), `api_messages.rs` (chat routes, body, mention and reaction rules), `uploads.rs` (uploads, the sweep, disk use), `avatars.rs` (profile pictures), `auth.rs` (passwords, names, cookies, client address), `limits.rs` (rate limits), `cli.rs` (admin subcommands, `backup`, the automatic `pre-*` and `auto-*` backups and their keep rule), `build.rs` (the build id), `ws.rs` (one socket), `hub.rs` (open sockets, pushed events, closing sockets), `voice.rs` (voice presence, session numbers, force-mute, the connection check), `public_addr.rs` (advertised address). |
| `vesper-dsp` | The capture `Pipeline` around DeepFilterNet3 on the `tract` runtime, 480-sample (10 ms, 48 kHz) frames. Native for tests (feature `tract`), WASM for the browser (feature `wasm`). `scripts/build-dsp.sh` builds the WASM. |
| `vesper-loadtest` | Synthetic speakers over WebSocket and WebRTC, for load and loss checks. See `docs/testing/latency.md`. |

## Direct messages

A DM is a row in `rooms` with `kind = 'dm'` (migration `0011_direct_messages.sql`): `restricted`, exactly two `room_members`, and the name
`dm-<idA>-<idB>` with the ids sorted. `rooms.name` is unique, so a pair has one DM. `Store::dm_with` finds or makes it in one `BEGIN IMMEDIATE`
transaction. Messages, reactions, reads, uploads, search, typing and voice use the room paths unchanged, because they key on `room_id`.

- **Who sees it.** `can_see` (`api_rooms.rs`) lets only the two members in, whatever their role, so the owner and moderators get 404 on its
  history, send and search, and it is not in their `GET /api/rooms` or `hello`. `patch_room`, `reorder_rooms` and `set_room_members` in the store
  match `kind = 'room'` only, so a DM id is `NotFound` to them, and the "last room cannot be archived" count ignores DMs.
- **Bans.** A banned account cannot be messaged and cannot see anything, but its DMs stay readable to the other person.
- **Voice.** `Voice::join`, presence and `Voice::refresh` all use `can_see`, so a DM has voice for its two members with no extra code.
- **Push.** A DM message pushes `kind: "dm"` ("alice sent you a message", no text) to the other member when they have no socket open, at most
  once per 30 s per DM and person, under the device's own direct messages switch (`push_subscriptions.dms`, migration `0012_push_dms.sql`, default on so an older device keeps getting them). A mention push is not sent for a DM. A voice start in a DM reads "alice is in
  voice with you". Both carry `icon`, the sender's picture path; the service worker fetches it with the session cookie and falls back to the default icon.
- **Web.** `AppState.dms` holds them apart from `rooms`, each renamed to the other person's display name (`titled` in `state.ts`), so every
  room name on screen shows the person. The sidebar has a "Direct messages" section; the person card has a "Message" button.
- **Ring.** When someone joins voice in a DM and the other member is not in that call, `Voice::ring` sends `ring_start { room_id, user_id }`
  (the caller) to the other member's open sockets only; a room never rings. One ring per call, kept in `Voice` by room. It ends with
  `ring_stop { room_id }` when the caller leaves (or is removed), the other member joins from any tab, the other member sends
  `decline_ring { room_id }` (only the called member's decline counts; the caller stays in the call), or after `ring_timeout`
  (30 s, `Config::ring_timeout`, not an environment variable). It is not repeated: the caller leaving and joining again is a new call.
  A person with no socket open gets only the push. The web app shows `RingPrompt.svelte` (picture and name, Accept joins the DM's
  voice, Decline, Esc declines) and repeats a ring tone while it shows; the tone has its own switch in Settings > Notifications. A new
  `hello` clears rings held by the client. Tests: the `ring_*` tests in `crates/vesper-server/tests/signaling.rs`, `web/src/lib/ring.test.ts`.
- **Not private from the server's owner.** Plain storage. Whoever can read `vesper.db` or a backup can read DMs.

## Message flow

All messages are JSON on one WebSocket per tab, tagged by `type` (`vesper-protocol/src/lib.rs`).

Client to server: `join { room_id, offer }`, `answer { sdp }`, `set_voice_state { muted, deafened }`, `leave`, `check { offer }`,
`typing { room_id }`, `decline_ring { room_id }`.
Server to client: `hello { me, users, rooms, voice, unread }` (`me.server` is the server's `version` and `build` id), `joined { user_id, answer, peers }`, `offer { sdp }`, `peer_joined`,
`peer_left`, `peer_state { user_id, muted, deafened, force_muted }`, `rooms_changed { rooms }`, `user_updated { user }`,
`user_created { user }`,
`force_muted { muted }`, `message_new { message }`, `message_updated { message }`, `typing { room_id, user_id }`,
`read { room_id, last_read_id }`, `ring_start { room_id, user_id }`, `ring_stop { room_id }`, `error { code, message }`. `unread` in `hello` has one `Unread` per room the user can see.
Error codes: `room_full`, `bad_message`, `forbidden`, `rate_limited`, `internal`.

1. The client opens `/ws` with its session cookie. The upgrade gets 403 unless `Origin` equals `VESPER_ORIGIN`, 401 unless the
   cookie names a live session, and 503 past `VESPER_MAX_SOCKETS` (`ws.rs` `upgrade`).
2. The server writes `hello` first: the user and session, every user, the rooms this user can see, and who is in voice in them.
   The socket joins the hub before `hello` is read, so no change in between is lost; changes queued meanwhile follow `hello`.
   Right after it joins the hub, the server checks the session again: a logout, ban or expiry that landed after the upgrade
   closes the socket with 4001 before `hello`. If `hello` cannot be built (a store error), it is logged and the socket closes with 1011.
3. The client builds an SDP offer, then sends `join` with a room id. The server reads the user and the room again, so a role,
   ban or member list change counts (an unknown or hidden room is `forbidden`; a store error is `internal`, logged). `Voice::join` (`voice.rs`) refuses a banned user or
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
A restricted room's presence reaches only sockets that can see it. When a room's member list or a role changes, or the room is archived, everyone in
voice who can no longer see their room is taken out (`Voice::refresh`, called from `api_rooms::rooms_changed`). A socket that
just lost sight of a room gets no `peer_left` for the people in it: the client drops the peers of every room missing from
`rooms_changed`.

The connection check: `check { offer }` starts an SFU session under a random 32 hex id in an SFU room of its own
(`check-<id>`) and answers `joined` with that id and no peers. It is not in presence and does not touch the user's call, since
the SFU keys sessions by that id, not the account. A new `check` ends the last one; so do `leave` and a closed socket.

Force-mute (`Voice::force_mute`): sets `force_muted` on the account, sends `force_muted { muted }` to all of that user's sockets
and `peer_state` to the room. While it holds, `set_voice_state` cannot clear `muted`. It holds across leave and rejoin until a
moderator lifts it, and lifting does not unmute. It lives in memory, so a restart lifts it. The SFU enforces it too:
`force_mute` calls `SfuHandle::set_muted`, and the SFU drops every audio packet from that user at the forward point, so a
client that ignores the mute is not heard. The SFU keys it by user id, not by session, so it holds across a rejoin as well.

Pushed from REST (`hub.rs`): a room change sends each socket `rooms_changed` with only the rooms in its user's list (`listed`: what it can see, plus archived rooms for the owner); a role
change also does, since the role decides that. A role, ban, profile or picture change sends everyone `user_updated`; a signup sends everyone `user_created`
(every member sees the user list, so nobody is left out).

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
  (`join`, `answer`, `leave`, `set_muted`, `set_sharing`, `watch`, `set_advertised`) over a channel and gets `SfuEvent::Offer`,
  `SfuEvent::ShareCut` and `SfuEvent::Closed` back per session.
- Video ([ADR 0009](decisions/0009-video-in-the-sfu.md)): a join offer has one audio m-line and at most one VP8 video m-line. A client with
  video is offered every other video track at join. Packets flow only while `set_sharing(user, true)` holds for the sender and
  `watch(viewer, sharer, true)` for the receiver. Both are per session, so a rejoin starts with neither.
- Keyframes: `watch(viewer, sharer, true)` sends the sharer a PLI, so the new viewer gets a picture right away. A viewer's own PLI or FIR on a
  forwarded video goes to the sharer as the same kind. Covered by `tests/forwarding.rs` with str0m test clients, not a browser.
- Share ceiling: the SFU measures each sharer's incoming video in windows of at least a second. When it stays above `share_ceiling_bps`
  (35 Mbps) for `share_ceiling_secs` (5) windows in a row, the SFU stops forwarding it, sets the sender to not sharing, and sends
  `SfuEvent::ShareCut` once to that session. Video from a client that is not sharing is dropped before it is measured.
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
  (30 ms) and holds at most 8 (80 ms); if it stays above 5 for 2 s it drops one frame a second down to 3. When it runs dry (an underrun, or empty exactly at a block end) the samples it played in that block fade to 0 (at most 2.5 ms, 120 samples; as little as 32 when it runs dry early in a 128-sample block), and the next audio fades in over 2.5 ms, so a starved output dips instead of clicking (unit-tested with a max sample step check; not listened). A queue that is empty exactly at a block end and refilled in time dips the same way but is not counted in `underruns`. Every start of playback (the first, and each re-prime) fades in. They absorb jitter in the worklet to worker messages. The delay they add is estimated, not measured.
- Two steps run inside the worklet (`dynamics.ts`), so the order stays mic, echo cancellation, gain, DeepFilterNet, Opus:
  - A limiter on the boosted input, before the denoiser, on only while the input boost is above 0 dB. A peak envelope (20 ms hold,
    80 ms release) feeds a soft curve: gain 1 below -6 dBFS, and the denoiser input never passes -1 dBFS. It needs no lookahead, so
    it adds no delay. At 0 dB boost (the default) it is off and the path is bit-identical to before; a boost change crossfades over
    10 ms. The worklet reports the input peak before the limiter (`inputPeak`), and the Settings clipping hint reads that, since
    the sent track can no longer reach full scale.
  - The noise gate (`vad.gate`, off by default) on the denoised output. It reads each frame's peak when it arrives from the worker and
    shapes the samples as they play, so the output queue (about 20 to 80 ms) is its lookahead. It opens on a peak above the level marker,
    closes after 200 ms of frames under the marker minus 4 dB, and ramps the gain (2 ms up, 120 ms down) to 0. The hold counts
    from frame arrival, so in played time the release starts 200 ms minus the queue depth after the sound drops. Turning it off
    ramps it open. The switch is disabled outside DeepFilterNet mode. The passthrough before the model is
    ready, and after a failure, is not gated.
- In `browser` and `off` modes the raw mic track is sent, so the fallback does not depend on the AudioContext or the worklet. Browser noise
  suppression is on in `browser` mode, in `deepfilter` mode while the model loads, and in `deepfilter` mode if the user turned it on.
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
- A mic that ends mid-call (or that the system mutes for over 3 s) leaves the call up with `off (microphone lost: ...)`, and the call bar offers
  Retry microphone: `capture.retry()` asks for the mic again (a saved mic that is gone falls back to the default, as at the start) and builds
  the graph anew, and the sender gets the new track with `replaceTrack`. A failed retry names the error and keeps the button.
- A load failure, a missing model or a worker that stalls also switches to `browser`, with a reason string the UI shows
  (`capture.ts` `fail` and `switchTo`). The call keeps going. The track is swapped in place and `replaceTrack` is used on the sender.
- The worker gets 20 s to report `ready` (`READY_TIMEOUT_MS` in `capture.ts`). Until then the mic passes through with the browser's noise suppression on
  (the mic is asked for with `noiseSuppression: true`), and the status line says `loading, browser filter until then`. At `ready` (unless "Browser noise suppression on top"
  is on, when the line says only `loading`) the capture calls `applyConstraints({ noiseSuppression: { exact: false } })` on the track. Where that works
  (`getSettings()` then says false; Firefox is expected to, not verified) nothing else happens. Chromium rejects it (OverconstrainedError; without
  `exact` it resolves and keeps the filter), and a second track asked for with a plain `false` while the first runs gets the first one's filter.
  Asked for with `{ exact: false }` it gets its own unfiltered source (all seen in headless Chromium 156, 2026-11-02). So the capture opens that
  second track while the first keeps running and feeding the graph (make before break), and listens to it with an analyser for up to 1.5 s
  (`CHECK_MS`). Only when it shows any level above digital silence does the graph's source move to it. The filtered track is kept, unconnected,
  until a worklet report shows sound, then ended. Nothing depends on the second `getUserMedia` working: if it fails, does not answer within 3 s
  (`REOPEN_TIMEOUT_MS`; a hidden tab can hold the request), is silent at the check, or goes silent for 4 reports right after the move (Edge on
  Windows was seen to hand back a live track of zeros), DeepFilterNet goes on over the filtered track (the two filters stack) and the line says
  `swap skipped: ...`. A silent second track also turns the swap off for the rest of the page session. No further `getUserMedia` call is made on
  that path. Diagnostics and its copied report show how many mic requests answered (`mic opened Nx`) and the `Filter swap` outcome
  (`applied-constraints`, `reopened`, `skipped: <why>` or `none`). The sent track does not change. Retry from the "no sound" banner ends the old
  track before it asks again, since a running track would hand the new request its own (silent) source. The e2e checks both tracks in headless
  Chromium, and a second track of zeros (the filtered one stays); whether the switch is heard, and real Edge on Windows, are not verified.
- The page starts fetching the denoiser files (`web/src/lib/audio/preload.ts`, called from `main.ts`) when it loads, before Join. The server answers `/dsp/*` with `no-cache`, so the worker's later fetch is a revalidation. `scripts/build-dsp.sh` writes `vesper_dsp_bg.wasm.gz` next to the wasm and the server sends it to browsers that accept gzip (`ServeDir` `precompressed_gzip`): 2.2 MB instead of 10.9 MB. `npm run build` also writes a `.br` of it in `dist/dsp` (1.3 MB, measured 2026-11-02), sent to browsers that accept brotli. With the 8.0 MB model the first visit downloads about 9 to 10 MB. Skipped when the browser asks to save data. Not verified in a real browser.
- The server side has no fallback. A dead SFU is not restarted in process; the process exits and the supervisor restarts it.

## HTTP API

JSON over HTTP under `/api` (`api_accounts.rs`, `api_admin.rs`, `api_rooms.rs`, `api_messages.rs`, `api_push.rs`, `api_passkeys.rs`, `uploads.rs`, `avatars.rs`). Bodies are the `vesper-protocol` types in `api.rs`. A failed call returns
`ApiError { code, message }`, and the status follows the code: `bad_request` 400, `unauthorized` 401, `forbidden` 403,
`not_found`, `invite_invalid` and `reset_invalid` 404, `conflict` 409, `too_large` 413, `rate_limited` 429 (with `Retry-After` in seconds),
`internal` 500, `quota_full` 507.

| Route | Needs | Returns |
|---|---|---|
| `GET /api/invites/preview?code=` | Invite limit by address | `InvitePreview` or 404 `invite_invalid` |
| `POST /api/signup` (`SignupReq`) | Invite limit by address | 201 `Me` and the session cookie |
| `POST /api/login` (`LoginReq`) | Login limits by address, then by address and username | `Me` and a new session cookie |
| `POST /api/logout` | a session | 204, the session is deleted and the cookie cleared |
| `GET /api/me` | a session | `Me`, with `server { version, build }` |
| `PATCH /api/me` (`ProfilePatch`) | a session | the new `UserInfo`; `bio` follows the display name rule, 0 to 160 characters; `ui_accent` is an `AccentPreset` key or `#rrggbb` (the client checks a custom colour's contrast) |
| `PUT /api/me/avatar` (the raw picture as the body) | a session, Upload limit by user | the new `UserInfo`; 400 unless png, jpeg or webp of at most 1024 px a side (see Profile pictures), 413 `too_large` past 512 KiB |
| `DELETE /api/me/avatar` | a session, Change limit by user | the new `UserInfo` with `avatar: null` |
| `GET /api/users/{id}/avatar?v=` | a session | the picture, or 404 when there is none; `ETag` is the version, and `v` equal to it makes the answer `immutable` |
| `POST /api/me/password` (`PasswordChange`) | a session, the old password, Login limit | 204, every other session is deleted |
| `POST /api/reset` (`ResetReq`) | a reset code, Reset limit by address (10, then 1 every 6 minutes) | 204, the password is set, the code is marked used and every session of that account is deleted; 404 `reset_invalid` for an unknown, used, cancelled or expired code (all the same) |
| `POST /api/me/reset-notices` | a session | `ResetNotice[]`: resets of your password not shown yet, now marked shown (the sign-in page shows them once) |
| `POST /api/me/calls` (`CallSummary`) | a session, Calls limit by user (10, then 1 a minute) | 204; stored for your account only (an account field in the body is ignored, like any unknown field). 400 when `duration_s` is over 24 h, `fallback_reason` is over 120 characters or has a control character, a percentage is outside 0 to 100, `jitter_ms` outside 0 to 10000 or `rtt_ms` outside 0 to 60000. The newest 20 per account are kept |
| `GET /api/me/calls` | a session | `CallRecord[]`: your own last 20 calls, newest first. Nobody else's, the owner included |
| `GET /api/sessions` | a session | `Device[]`, `current` marks this one |
| `DELETE /api/sessions/{id}` | a session | 204 (the cookie is cleared if it is this session), or 404 if the id is not one of yours |
| `GET /api/passkeys` | a session | `PasskeyList`: `enabled` (false when `VESPER_ORIGIN` has an IP address host) and your `PasskeyInfo[]` |
| `POST /api/passkeys/register/start` | a session, Change limit by user | `PasskeyChallenge`: `navigator.credentials.create` options and a one-use `challenge_id`. Passkeys you have are in `excludeCredentials` |
| `POST /api/passkeys/register/finish` (`PasskeyFinish`, `name` optional) | a session, Change limit | 201 `PasskeyInfo`. 400 for a wrong, expired, reused or foreign challenge, a wrong origin or a key you already added; 409 past 10 passkeys |
| `PATCH /api/passkeys/{id}` (`PasskeyRename`), `DELETE /api/passkeys/{id}` | a session, Change limit | 204, or 404 if the id is not one of yours |
| `POST /api/passkeys/login/start` | none, LoginIp limit by address | `PasskeyChallenge` for a discoverable credential: no username is sent, so the answer says nothing about who has a passkey. 404 when passkeys are off |
| `POST /api/passkeys/login/finish` (`PasskeyFinish`) | none, LoginIp limit | `Me` and a new session cookie, exactly like `/api/login`. Every failure is the same 401 `passkey sign in failed`: unknown or removed passkey, a banned account, a wrong origin or signature, a challenge that is expired or used |
| `GET /api/push` | a session | `PushInfo`: the VAPID public key (null when push is off) and `PushDevice[]`, `current` marks this session's |
| `PUT /api/push/subscription` (`PushSubscribe`) | a session, Push limit by user (20, then 1 per 10 s, shared with the switches and the test) | 204; replaces this session's subscription. 400 for an endpoint off the push service list or bad keys, 404 when push is off, 409 past 5 devices or for an endpoint another session holds with other keys |
| `DELETE /api/push/subscription` | a session | 204, this session's subscription is gone |
| `DELETE /api/push/devices/{id}` | a session | 204, or 404 if the id is not one of yours |
| `POST /api/push/test` | a session, Push limit | 204 once the push service took it; 404 with no subscription or when the service says it is gone or refuses the key (then deleted), 500 for any other answer |
| `GET /api/users` | a session | `UserInfo[]`, every account (role and banned included) to any signed-in member by design: the WebSocket hello sends the same list |
| `GET /api/invites` | moderator or owner | `InviteInfo[]`, newest first |
| `POST /api/invites` (`InviteCreate`) | moderator or owner; a moderator invite needs the owner | 201 `InviteCreated` with the link `<origin>/invite#<code>` |
| `DELETE /api/invites/{id}` | moderator or owner | 204, or 404 for an unknown id |
| `POST /api/users/{id}/reset` (`ResetCreate`: your own password) | owner for anyone else; a moderator for a member only; 400 on yourself; ResetCreate limit by user (10, then 1 every 6 minutes), then the Login limit | 201 `ResetCreated` with the link `<origin>/reset#<code>`, one use, 24 h; it cancels the account's older code. 403 `wrong password` |
| `GET /api/resets` | owner | `ResetInfo[]`, each account's latest reset plus every reset used in the last 30 days, newest first (no code) |
| `PATCH /api/users/{id}/role` (`RolePatch`) | owner | the new `UserInfo` |
| `POST /api/users/{id}/ban` | owner | the new `UserInfo`; every session of that user is deleted |
| `POST /api/users/{id}/unban` | owner | the new `UserInfo` |
| `POST /api/users/{id}/kick` | moderator or owner, on a lower role | 204, the user leaves voice; 404 if not in voice |
| `POST /api/users/{id}/force-mute` (`ForceMute`) | moderator or owner, on a lower role | 204; `muted: false` lifts it; works when not in voice too |
| `POST /api/users/{id}/disconnect` | moderator or owner, on a lower role | 204, every socket of the user closes with 4003 |
| `GET /api/rooms` | a session | `RoomInfo[]` by position, only the rooms you can see. A DM (`kind: "dm"`) is listed to its two members only |
| `POST /api/dms` (`DmCreate`) | a session, Change limit by user | `RoomInfo` of the DM with that person, made if needed; yourself and a banned account are 400, an unknown one 404 |
| `POST /api/rooms` (`RoomCreate`) | owner | 201 `RoomInfo`, open, placed last |
| `PATCH /api/rooms/{id}` (`RoomPatch`: `name`, `restricted`, `archived`) | owner | the new `RoomInfo`; archiving the last room that is not archived is 409 |
| `PUT /api/rooms/order` (`RoomOrder`) | owner | 204; `ids` must hold every room id once, else 409 |
| `PUT /api/rooms/{id}/members` (`RoomMembers`) | owner | the new `RoomInfo`; the list replaces the old one, an unknown account is 404, a banned one 400 |
| `GET /api/rooms/{id}/messages?before=\|after=\|around=&limit=` | sees the room | `MessagePage`, oldest first; at most one cursor (else 400), `limit` 1 to 100 (default 50) |
| `GET /api/rooms/{id}/pins` | sees the room | `Message[]`, newest pin first |
| `POST /api/rooms/{id}/messages` (`SendMessage`) | sees the room, Message limit by user | 201 `Message`; pushes `message_new` |
| `PATCH /api/messages/{id}` (`EditMessage`) | sees the room, the author, Change limit by user | the new `Message`; pushes `message_updated` |
| `DELETE /api/messages/{id}` | sees the room, the author, or a moderator or the owner | 204; pushes `message_updated` with `deleted: true` |
| `PUT /api/messages/{id}/reaction` (`ReactionReq`) | sees the room, Change limit by user | the new `Message`; pushes `message_updated` |
| `PUT /api/messages/{id}/pin` (`PinReq`) | sees the room, Change limit by user | the new `Message`; pushes `message_updated` |
| `PUT /api/rooms/{id}/read` (`ReadReq`) | sees the room | 204; sends `read` to every socket of the user |
| `GET /api/search?q=&room_id=&author_id=&from=&to=&before=` (`SearchQuery`) | a session, Search limit by user | `SearchPage`, 20 hits, newest first, only rooms you can see; a `room_id` you cannot see is 404 |
| `POST /api/rooms/{id}/uploads?name=` (the raw file as the body) | sees the room, Upload limit by user | 201 `UploadDone`; 413 `too_large` past 25 MiB, 507 `quota_full` |
| `GET /api/uploads/{id}` | sees the room; an upload not attached yet needs the uploader | the file (Range works), else 404 |
| `GET /api/admin/disk` | owner | `DiskUse` |
| `GET /api/admin/server` | owner | `ServerStatus`: version and build, uptime, the `DiskUse` numbers, people in voice, failed pushes in the last 24 h (in memory, since start if shorter), the newest `*.db` in `<data>/backups` (name and time). No secrets and no one's address. The advertised public address is not in it: the server does not record when it changed |

Your recent calls (`api_calls.rs`, store `calls.rs`, table `call_stats` from migration `0010_call_stats.sql`): when you leave a call your
client posts one summary: how long it lasted, the noise suppression mode it ended in and its fallback reason, incoming loss, jitter,
concealed share and round trip (averages over the call's 2 s stats polls), and whether the browser filter ran while DeepFilterNet
loaded. No room and no other people are stored. Each person reads only their own (Settings > Account, "Your recent calls"); there is
no owner view (a ruling for v1.4). Rows older than 30 days are deleted at start and every 24 h, and each account keeps its newest 20.

Rooms (`api_rooms.rs`, store `rooms.rs`): every room has text and voice. A fresh database has `general` and `hangout`
(migration `0002_rooms.sql`). Room names follow the display name rule (NFC, 1 to 32 characters, no control or hidden format
characters, else 400) and are unique ignoring case (409). A restricted room is seen only by the owner, the moderators and the
accounts on its member list (`can_see`); `members` is filled only while the room is restricted. Opening a room clears its member list, so restricting it again
starts empty. A banned account can not be put on a list (400). Every change pushes `rooms_changed` to open sockets.
Archive (`PATCH` with `archived`, owner only like every room change): `can_see` is false for an archived room for everyone, so
nobody reads its history, posts, types, searches it (its hits are left out) or joins its voice, and the people in its call are taken
out like on any loss of access. Its messages and files stay. The owner still gets it in `GET /api/rooms`, `hello` and
`rooms_changed` with `archived: true`, so Admin > Rooms can list it, dimmed, with Unarchive; the web client keeps it out of every
other list (`AppState.archived`). Archiving the last room that is not archived is 409, checked inside the write transaction.
Any room can be archived, the default ones too. There is still no hard delete.

Chat (`api_messages.rs`, store `messages.rs`): a room you cannot see, and a message in one, is 404 on every route, the same as
an unknown id. Changes are pushed with `Hub::broadcast_room`, so only sockets that can see the room get them. Rules:

- **Body.** NFC, trimmed, 1 to 4000 characters (not bytes), no control characters except newline and tab, else 400. The same
  rule holds for an edit. Bodies are never logged.
- **Who.** Anyone who can see the room posts, reacts and pins. Only the author edits (403 for anyone else, the owner too). The author
  deletes, and a moderator or the owner deletes anyone's (403 for a member on someone else's). A delete is soft: the body is
  emptied, and reactions, mentions and the pin go. A deleted message is 404 for edit, delete, reaction and pin.
- **Send limit.** 10 messages, then 1 a second, per user (429). Edits, reactions and pins share the Change limit: 20, then 1 a
  second, per user (429). Deletes are not limited: each message is deleted once, so sends already bound them.
- **Replies.** `reply_to` may name a deleted message of the room; the reply shows it as deleted. Allowed on purpose.
- **Mentions.** `@name` at a word start, the name being `[a-z0-9_]{3,24}` after an ASCII lowercase, at most 20 per message, read again
  on each edit. A name that is unknown, banned or cannot see the room is dropped, so it never counts as a mention.
- **Reactions.** 1 to 32 bytes with no ASCII letter or digit, whitespace or control character (400). At most 20 different ones per message (409).
- **Pins.** At most 50 per room (409). A DM has no pins: the pin route answers 403 for a DM message.
- **Read marker.** `last_read_id` is 0 or a message of that room (else 400), so it cannot be set past future messages. It never
  moves back; `read` carries where it really is. Unread counts skip your own and deleted messages.
- **Typing.** `typing { room_id }` on the socket goes to every other socket that can see the room (not back to the sender's
  socket). Per user and room at most one every 3 s; more, an unknown room or a hidden one is dropped without an answer (the room is
  checked before the limit, so made-up room ids take no limiter entry; a store error is logged). It is never
  stored and has no stop event: the client shows it for 5 s after the last one.
- A banned user's messages stay. They cannot post, since their sessions are gone.

Search (`GET /api/search`, store `search.rs`): FTS5 over `messages_fts`, tokenizer `unicode61 remove_diacritics 2`, so `cafe`
finds `Café`. Rules:

- **Query.** `fts_query` splits `q` on whitespace and control characters, cuts each term to 64 characters, drops terms with no
  letter or digit and keeps 16. Each term is quoted (inner `"` doubled) and the last one is a prefix (`"dog"*`), unless it is one
  character: `a` matches the word `a` only, because the prefix `a*` is the slow case. The web client waits for 2 characters anyway. So `OR`, `NEAR(`,
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

Profile pictures (`avatars.rs`, store `accounts.rs`, migration `0005_profile.sql`): the web app crops and scales the file to
a 256 px square and sends it as WebP (PNG where the browser cannot). The server does not trust the client's type: it sniffs png,
jpeg or webp from the bytes (no svg, no gif). It reads the width and height from the header (png: IHDR must come first; jpeg:
the segments up to the first frame header; webp: `VP8X`, `VP8 ` or `VP8L`) and refuses a header it cannot read or a side over
1024 px, so a small file cannot unpack to gigabytes in every member's browser. A png must end at `IEND` and a webp at its RIFF
size; a jpeg is only checked to end with `FFD9`. EXIF (jpeg APP1, png `eXIf`, the webp EXIF flag) and png text chunks are refused,
as is a file holding `<script`, `<html`, `<svg` and similar markup. That list is easy to get round: the real guard is how the
picture is served (below). The Upload limit is checked before the body is read. The bytes live in the `avatars` table (at most 512 KiB, one per account, deleted with it), not with the
uploads, and are not counted in the upload quota. `UserInfo.avatar` is the first 16 hex of the SHA-256, so a new picture gets a
new URL. It is served with the uploads' headers: the sniffed type, `nosniff`, `default-src 'none'; sandbox` and
`Cross-Origin-Resource-Policy: same-origin`. Tests: `tests/avatars.rs`, `avatars.rs` `avatar_mime_rules`.

Uploads (`uploads.rs`, store `uploads.rs`): one file per request, sent as the raw body (no multipart). Rules:

- **Size.** At most 25 MiB (`MAX_UPLOAD`). A larger `Content-Length` is 413 before the body is read; a body with no length is
  counted while it streams to `uploads/tmp/<id>` and is cut off with 413 once it passes the cap. The partial file is deleted, also
  when the client goes away mid-body, sends nothing for 30 s (`READ_IDLE`) or is not done after 10 minutes (`READ_TOTAL`): 400
  `upload too slow`. So a 25 MiB file needs about 44 KB/s (350 kbit/s) on average. The Upload limit is 10, then 1 every 30 s, per user.
- **Quota.** Uploads together stay under `VESPER_UPLOAD_QUOTA_MB` ([env.md](env.md)). The check, the move into place and the row
  insert run under one lock, so uploads racing each other cannot pass it (507 `quota_full`). A `Content-Length` that cannot fit
  what is left is 507 before the body is read.
- **On disk.** The file name is the id (32 random hex), under `uploads/<first 2>/`, dirs 0700 and files 0600. A request id that is
  not 32 lowercase hex is 404 before any path is built, so no client text ever reaches a path.
- **Type.** Decided from the first bytes, never from the name or the request's `Content-Type`. Only png, jpeg, gif and webp count
  as images (`Attachment.image`) and are served inline with their type. Everything else, SVG and HTML included, is
  `application/octet-stream` with `Content-Disposition: attachment`.
- **Name.** Display text only: control, bidi and hidden format characters dropped, `/` and `\` turned into `_`, at most 200
  characters, `file` when empty. `Content-Disposition` carries an ASCII `filename` and the exact name as `filename*` (RFC 6266 and
  5987), so a name can not add a header.
- **Served headers.** `X-Content-Type-Options: nosniff`, `Content-Security-Policy: default-src 'none'; sandbox` (kept in place of the page's policy),
  `Cross-Origin-Resource-Policy: same-origin`, `Cache-Control: private, max-age=31536000, immutable` (the file behind an id never changes).
- **Who.** Attaching (`SendMessage.attachments`) needs your own upload, not attached yet, in the same room (else 409). An attached
  file is served to anyone who can see the room; one not attached yet only to its uploader. Everything else is the same 404 as an
  unknown id.
- **Removal.** Deleting a message deletes its files. Uploads still not attached after 24 h are swept every hour (and at start).
  The same hourly pass deletes files under `uploads/<xx>/` with no row that were last written over 24 h ago (a delete that
  failed, or a crash between the move and the row insert), checking under the upload lock (`sweep_orphans`).
- **Disk use.** `DiskUse` holds the database plus WAL size, the sum of upload sizes, the quota, and the filesystem total and free
  space of the data dir (`statvfs`). `warn` is set when uploads pass 80% of the quota or the filesystem is over 80% full.

A ban ends the user's sessions but does not revoke invites they already created; the owner revokes those by hand. `InviteCreate` needs all three fields (`role`, `max_uses`, `expires_hours`), so the web client must send them.

Roles (`require`, `api_admin.rs`; owner > moderator > member):

| Action | Member | Moderator | Owner |
|---|---|---|---|
| List users | yes | yes | yes |
| List invites, revoke a member invite | no | yes | yes |
| Revoke a moderator invite | no | no | yes |
| Mint a member invite | no | yes | yes |
| Mint a moderator invite | no | no | yes |
| Change a role, ban, unban | no | no | yes, not on themselves or an owner |
| Mint or grant the owner role | no | no | no, only `vesper-server create-owner` |
| See a restricted room | only when listed | yes | yes |
| Create, rename, reorder, restrict, archive a room or set its members | no | no | yes |
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
- **Caching.** Every `/api` response has `Cache-Control: no-store`, except a served upload or profile picture.
- Handlers never log request bodies, and the request types with a password or invite code print `***` in `Debug`. A body that does
  not parse gets a fixed "invalid request body" message. The WebSocket upgrade takes the same session cookie and Origin check.

Tests: `crates/vesper-server/tests/accounts.rs`, `tests/admin.rs`, `tests/resets.rs`, `tests/rooms.rs` (every route for every caller and target role) `tests/chat.rs` (chat routes, pushes, typing, unread, search) and `tests/uploads.rs` (limits, quota race, served headers, the access matrix, sweep), through
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
A one-letter prefix (`a*`) was the slow case; the server now matches a last term ending in a one-letter or one-digit token (`a`, `a.`, `x-a`) as a whole word, and the web client waits for 2 characters.

## Web Push

Notifications while Vesper is closed (issue #58): "alice mentioned you in Games" and "alice started voice in Games". Server
`push.rs` and `api_push.rs`, store `push.rs` (migration `0009_push.sql`), client `web/src/lib/push.ts`,
`routes/settings/PushSettings.svelte` and `web/public/sw.js`.

- **Opt-in per device.** Settings > Notifications > While Vesper is closed. The switch asks for permission (from the click), registers
  `/sw.js` and subscribes with the server's VAPID public key. Each device has a mentions switch, a direct messages switch and a voice switch, all off
  until the person turns them on (a device saved before `dms` existed keeps direct messages on). The in-page notification has its own
  mentions and direct messages switches in `vesper.notify`; `dms` follows `mentions` until it is set. Turning it off, or logging out, unregisters the worker; the server deletes the subscription with the session.
- **Who gets one.** A new message pushes to the accounts it mentions (already limited to people who can see the room and are not
  banned). The first join of an empty voice room pushes to everyone who can see the room. Never to the sender, never to an account with a
  WebSocket open at that moment (its tab shows the event), never to a device with that switch off or a session unused for 90 days,
  and "started voice" at most once per room and person per 10 minutes (in memory, so a restart forgets it). Edits do not push.
- **What the push service sees.** The payload `PushPayload { kind, room_id, title, body }` is encrypted for the browser (RFC 8291,
  `aes128gcm`, tested against the RFC's Appendix A vector), so Google, Mozilla, Apple or Microsoft see only that a push went to that
  browser, its size and time. The body names the sender and the room, never the message text. TTL 24 h for mentions, 10 minutes for
  voice; `Topic` per room, so a waiting push is replaced by a newer one.
- **VAPID.** An ES256 key made on first start and kept in the database (`server_secrets`), so a copy of `vesper.db` keeps it and
  existing subscriptions keep working after a restore. If the key changes (a restore without that row), push services refuse the old
  subscriptions with 401 or 403 and the server deletes them; Settings compares the browser's subscription with the current key and
  asks to turn it on again. JWTs last 12 hours. The contact (`sub`) is `VESPER_PUSH_SUBJECT`, else an https
  `VESPER_ORIGIN`; with neither, push is off ([env.md](env.md)).
- **SSRF.** The server POSTs to a URL the browser gave it, so `parse_endpoint` allows only https on port 443 to `fcm.googleapis.com`,
  `updates.push.services.mozilla.com`, `*.push.apple.com` and `*.notify.windows.com`, with no user part and at most 2048 bytes. The
  name is resolved once; if any address is not public (loopback, private, link-local such as 169.254.169.254, CGNAT, documentation,
  multicast, IPv6 unique-local, or IPv4 wrapped in IPv6) the push is refused, and the connection goes to the checked address. TLS with
  the Mozilla root list (`webpki-roots`), HTTP/1.1, no redirects followed, no cookies, 10 s for the lookup and the exchange together,
  only the status line of the answer read. 404 or 410 (gone) and 401 or 403 (made for another VAPID key) delete the subscription. At
  most 8 sends run at once and 256 wait or run; past that a push is dropped (a debug log line), so a mention storm cannot grow a
  backlog. A port other than 443 is refused from the raw text (`http::Uri` reads `:65979` as no port), and so is a path that does not
  start with `/`. A second session may take over an endpoint already stored only when its keys match too (the same browser, signed in
  as someone else); otherwise 409, so knowing an endpoint is not enough to take another person's pushes. The `test-push` cargo feature (tests and the push e2e) also allows
  `http://127.0.0.1:<port>`; it is a compile error in a release build.
- **App badge** (`web/src/lib/ui/badge.ts`): the page sets the installed app's badge to the sum of unread mentions (`Unread.mentions`) and clears it at 0, sending only changes; the worker sets a plain mark (no number) on a mention push. Only where `navigator.setAppBadge` exists (Chromium on Windows and macOS, Safari 17 installed apps); Firefox has none and nothing happens. Unit tests with a stub (`badge.test.ts`, `sw.test.ts`); not seen on a real installed app.
- **The worker** shows the notification (tag `<kind>-<room id>`, so a voice push never replaces a mention), focuses an open Vesper tab and asks it to open the room (the app navigates
  in place, so a call goes on) or opens `/r/<id>`, and resubscribes on `pushsubscriptionchange` with the device's switches. It has
  no fetch handler and caches nothing.
- **Install.** `web/public/manifest.webmanifest` (standalone, `#100e0c`, 192 and 512 px icons) is linked from `index.html`. Under the
  CSP the manifest falls under `default-src 'self'` and the worker under `worker-src 'self'`; nothing changed in the policy.

Verified: unit tests in `push.rs` (RFC 8291 vector, VAPID JWT signature, `aud`, `exp` at most 24 h, `sub`, the SSRF matrix, the
recipients rule, the 10 minute rule), `crates/vesper-server/tests/push.rs` (a fake push service on 127.0.0.1: signature, decrypted
payload, no text in the clear, 410 deletes, switches, open socket, hidden room, rate limit, cap), `crates/vesper-store/tests/push.rs`,
`web/src/lib/push.test.ts` and `web/e2e/push.mjs` (headless Chromium, with a stand-in `PushManager`). Not verified: a real push
service, a notification on screen, iOS, Firefox and Safari.

## Connection timeouts

`accept_loop` in `lib.rs` serves HTTP/1 only, with hyper's `header_read_timeout`; the body timeouts are tower-http layers in
`router`. Values are constants in `config.rs` (no variables); tests set the `Config` fields to short timers.

- **Request head:** 10 s (`HEADER_TIMEOUT`). hyper runs the same timer while a kept-alive connection waits for its next request,
  so an idle connection is also closed after 10 s.
- **JSON `/api` bodies:** cut after 15 s with no bytes (`BODY_IDLE`) or 30 s in all (`BODY_TOTAL`). The handler sees a body error
  (400, tested for the stalled login body) and the connection closes, since the body was not read to the end. The bodies are at most 32 KiB.
- **Uploads and profile pictures:** 30 s idle and 10 minutes in all (`uploads::READ_IDLE`, `READ_TOTAL`): in `read_body` for
  uploads, as layers for `/api/me/avatar`. The partial upload file is deleted when the body stops, errors or the client goes away (one `Drop` guard; tested for the client going away).
- **WebSocket:** once upgraded the socket leaves hyper, so none of these apply. It has its own ping (25 s, dropped after two silent periods).
- **No response timeout.** A client that reads a download slowly keeps its connection ([known-issues.md](known-issues.md#security-and-abuse)).

Tests: `crates/vesper-server/tests/timeouts.rs` over raw TCP (half a head, a kept-alive connection answered twice and then closed when
idle, a stalled and a trickled JSON body, a slow upload past the JSON total, an aborted upload's partial file, a WebSocket past every
timer). Checked once by hand when written: each test failed with its piece removed (`header_read_timeout`, the idle layer, the deadline
layer, keep-alive turned off, the JSON deadline also on uploads, the file delete in `Cleanup`).

## Static serving

`web/scripts/precompress.mjs` (run by `npm run build`) writes a `.br` and a `.gz` next to every html, js, css, json, webmanifest, svg, txt and wasm file in `dist/` when that is smaller and no such copy is there yet (build-dsp.sh's wasm `.gz` is kept; the wasm also gets a `.br`); images, fonts and archives get none. ServeDir (`precompressed_br`, `precompressed_gzip`) sends one by `Accept-Encoding` with the original's `Content-Type`, `Content-Encoding` and `Vary: accept-encoding`.
`guard` in `vesper-server/src/lib.rs`: dotfiles and `%2e` are 404; an extensionless 404 gets `index.html` (SPA routes); a missing file with
an extension or under `/api/` stays 404; `/assets/*` is `immutable`, `/api/*` `no-store` (unless the handler set its own, as uploads do), everything else `no-cache`. Every response gets the security headers and the CSP of [decision 0008](decisions/0008-csp.md); a CSP the handler set (uploads' sandbox) is kept. Tests: `crates/vesper-server/tests/static_files.rs`, `tests/uploads.rs` `upload_keeps_sandbox_csp`.

## Not verified

Last checked 2026-11-01. Run only in headless Chromium (`npm --prefix web run e2e`, fake microphones, plain http on 127.0.0.1); everything
else here was read from the code, not run:

- Behaviour in real (headed) Chrome, Firefox and Safari, including the `ontrack` stream id and `replaceTrack` during a mode swap.
- Everything in the SFU loop beyond its doc comments (`room.rs` forwarding, `client.rs` negotiation).
- Chat, uploads and search in the browser: verified only in headless Chromium by the e2e (send with Markdown and raw HTML,
  reply, edit, a PNG shown inline, search that skips a hidden room). Scroll anchoring, paste, typing, unread
  badges and the reconnect refetch are unit-tested through the pure helpers only.
- Sign up and sign in run in headless Chromium over http only. The `Secure` `__Host-` cookie and HSTS on https are tested in process only.
- The CSP ([decision 0008](decisions/0008-csp.md)): no violation in headless Chromium through the e2e. The denoiser under the CSP runs
  only when `web/dist/dsp/` is built (CI copies it from the dsp-wasm job). Not checked through Caddy and Cloudflare, which may add or change headers.
- Timeouts are tested over loopback TCP with short timers (`tests/timeouts.rs`, `read_body_times_out_idle_and_total`), not over a real slow
  network or through Caddy and the Cloudflare tunnel.
