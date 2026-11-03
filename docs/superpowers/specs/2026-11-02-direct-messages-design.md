# Direct messages (v1.5)

Status: built on this branch. The sections below are the design; "Built, and where it differs" at the end lists the changes made while building. Written 2026-11-02 against `main` at v1.4.2 (`f0dd39d`).
"Verified" below means read in the code on that commit. Nothing was run in a browser.

## Why

Everything said in Vesper is said in a room, and everyone sees a room. A small thing like a sign-up link
for one friend has nowhere to go. The case that started this: a friend needed a fresh invite link on their
PC, and it had been sent to their phone. Posting it in a room would show it to everyone.

## What the user asked for (Toby, 2026-11-02)

- A DM is exactly two people. No private groups.
- History is kept forever. A person can delete their own messages.
- An unread badge, plus an optional browser notification that shows the sender's profile picture.
- Voice in a DM, the same as in a room.
- A moderator can never read a DM. A banned account's DMs stay readable to the other person.
- The unread badge in a DM is the quiet room style, not the strong mention style.

## Goals

1. Open a private text chat with any other member from their person card.
2. Everything a room message already does works in a DM: replies, reactions, uploads, edits, deletes,
   markdown, typing, read markers, unread counts, search.
3. Only the two people can see a DM. The owner and moderators cannot see it in the app.
4. A new DM message can reach a person who is in a voice room, in another tab, or has the tab closed.
5. Two people can talk in a DM's voice, with the same mute, deafen, volume and noise suppression as a room.

## Non-goals (v1.5)

- Private groups of 3 or more.
- Ringing. Nobody gets a ring or an accept and decline prompt. The other person sees who is in the DM's voice
  and gets a push (see Voice).
- Video and screen share.
- End-to-end encryption. See "What 'private' means" below.
- Message requests, blocking, or per-person mute.
- Pins in a DM. They stay in the schema but the UI does not offer them.

## Anti-goal

A DM must never appear in Admin > Rooms, in the owner's room list, in another person's search, or in a
moderator's `can_see`. A leak there is worse than a missing feature.

## Design

### A DM is a room

The smallest change is to store a DM as a row in `rooms` with two members. Messages, reactions, mentions,
reads, uploads and the search index all key on `room_id` already (`0003_messages.sql`), so they work
unchanged. No second message path to keep in step.

Migration `0011_direct_messages.sql` (additive):

- `rooms.kind TEXT NOT NULL DEFAULT 'room' CHECK (kind IN ('room', 'dm'))`.
- A DM row has `restricted = 1`, exactly two `room_members` rows, `position = 0`, and a generated name
  `dm-<idA>-<idB>` with the two account ids sorted. `rooms.name` is already `UNIQUE`, so a pair can only
  have one DM, and "find or create" is one insert that cannot race into two rooms.

### Visibility (the part to review hardest)

`can_see` in `api_rooms.rs` lets the owner and moderators see every restricted room. For `kind = 'dm'`
that rule is replaced: only the two listed members can see it, whatever their role.

Every place that lists or edits rooms must skip DMs:

| Place | Change |
|---|---|
| `can_see`, `listed` | DM: members only, no role override, never archived |
| `Store::rooms()` / `GET /api/rooms` | return DMs only to their two members |
| `PATCH /api/rooms/:id`, reorder, `PUT members` | refuse a DM (`forbidden`) |
| Admin > Rooms | filter out `kind = 'dm'` |
| Search | already takes `visible_rooms` from `can_see`, so it follows the fix above |
| `Voice::join`, `Voice::refresh`, `Hub::broadcast_room` | no change: all three already gate on `can_see` (verified in `voice.rs` and `hub.rs`), so DM voice and its presence reach only the two members |

A test per row. The `can_see` test already loops over every role, so a DM case goes in the same loop.

### Starting a DM

`POST /api/dms { user_id }` returns the `RoomInfo` of the DM with that person, creating it if needed.
It refuses yourself, a banned account, and an unknown id. It is rate limited like room creation.
Calling it twice returns the same room.

### Wire types

`RoomInfo` gains `kind: RoomKind` (`room` | `dm`). The other person is the member who is not you, so no new
field is needed. `Unread`, `message_new`, `typing` and `read` are unchanged, because they carry `room_id`.
`hello.rooms` includes the user's DMs, so the existing unread counts cover them. Regenerate `gen/*.ts`
with `cargo test -p vesper-protocol`.

### Web app

- Sidebar: a "Direct messages" section above the rooms, one row per DM, newest activity first. Each row
  shows the other person's profile picture, their name and the unread badge.
- A DM opens at `/r/<id>`, the same route as a room, with the chat panel and the join voice button.
- The sidebar row shows a small speaking indicator while the other person is in the DM's voice, as a room row does.
- The person card (`PersonCard.svelte`) gets a "Message" button that calls `POST /api/dms` and navigates.
- Anywhere the UI shows a room name shows the other person's name for a DM, since the stored name is generated.

### Notifications

- Unread badge: from `Hello.unread` and live events, as for rooms, in the quiet room style (Toby's call).
  A DM message does not count as a mention.
- Web Push: "alice sent you a message", with alice's profile picture as the notification icon. It uses the
  device's own direct messages switch (`push_subscriptions.dms`, issue #150; first built under the mentions switch).
- The push does not carry the message text. This matches the existing mention push (my call).
- A burst of messages is one push: at most one per sender and person per 30 seconds. The existing
  "started voice" dedupe is once per 10 minutes (`VOICE_EVERY`), so this needs its own small map (my call).

### Voice

- A DM uses the same join, SFU room (`room-<id>`) and presence as a room. The room cap of 12 does not matter.
- The existing "started voice" push already goes to people who can see the room, so in a DM it goes to the
  other person. In a DM the text reads "alice is in voice with you" and it uses the existing `voice` switch.
  The 10 minute dedupe applies.
- No ring. The other person finds out by push, or by the speaking indicator in the sidebar if the app is open.
- If a call is up and the other person is banned, `Voice::refresh` removes them, as for any room.

### What "private" means

Plain storage in `vesper.db`. The app never shows a DM to anyone but its two members. The person who runs
the server can still read the database file and the backups. This design does not change that, and the
UI should say so in one line on the Direct messages empty state.

End-to-end encryption would break server-side search and history sync, and is a project of its own.

## Order of work

1. Migration, store functions, `can_see` and every row in the visibility table, with tests. (about half a day)
2. `POST /api/dms`, `RoomInfo.kind`, regenerated types, voice refusal. (about 2 hours)
3. Sidebar section, person card button, hide voice controls. (about half a day)
4. DM push kind and settings switch, and the DM wording of the voice push. (about 3 hours)
5. DM voice in the web app: join from a DM, sidebar indicator, DM name in the call bar. (about half a day)
6. Docs: `architecture.md`, `env.md` if a variable changes (none planned), `known-issues.md`.

About two and a half days in total. Step 1 can ship alone behind no UI.

## Decided (Toby, 2026-11-02)

1. A moderator can never read a DM, for any reason.
2. When an account is banned, the other person still sees the DMs.
3. The unread badge in a DM is the quiet room style.
4. DMs have voice.

## Open questions for Toby

1. Is a push and a sidebar indicator enough for voice, or do you want a ring (a sound and an accept prompt) in v1.5?
   This design says no ring, because a ring is its own piece of work.

## Not verified

- Anything in a browser, including how the sidebar and the notification icon look.
- Whether every call site of `Store::rooms()` is covered by the table above. A grep for callers is the
  first task of step 1.
- That push icons work on every browser Vesper supports.
- Voice in a DM. The server side is read from `voice.rs` and `hub.rs` only. No call was made, and the web call
  code was not read for assumptions that a call belongs to a named room.
- The estimate. It assumes the existing chat panel needs no change beyond hiding voice.

## Built, and where it differs

Built in `6e03f31` and the commits after it on this branch. The differences, all my calls:

- **A separate DM push switch** (issue #150, v1.6.0; v1.5 first reused the mentions switch). Migration 0012 adds
  `push_subscriptions.dms`, default on so existing devices keep their DM pushes. Both Settings panels have a
  "Mentions" and a "Direct messages" switch.
- **The in-page notification also covers DMs.** `notify.ts` already shows a browser notification for a mention
  when the tab is hidden. It now does the same for any message in a DM, under the direct messages switch (#150).
- **The picture in a push** is a `PushPayload.icon` path. The service worker fetches it with the cookie and
  makes a data URL, because the notification system fetches icons without the cookie. Not checked in a browser.
- **The privacy line** is the tooltip of the "Direct message" label in the DM header, not an empty-state text.
- **DMs are sorted by name** in the sidebar. The state has no activity time to sort on.
- **Pins are hidden in the DM header.** The server does not refuse a pin; a moderator who is a member could still pin through the API.
- **No ring**, as the design said. The question is still open.
