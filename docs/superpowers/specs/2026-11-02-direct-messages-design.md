# Direct messages (v1.5)

Status: draft for review. Nothing here is built. Written 2026-11-02 against `main` at v1.4.2 (`f0dd39d`).
"Verified" below means read in the code on that commit. Nothing was run in a browser.

## Why

Everything said in Vesper is said in a room, and everyone sees a room. A small thing like a sign-up link
for one friend has nowhere to go. The case that started this: a friend needed a fresh invite link on their
PC, and it had been sent to their phone. Posting it in a room would show it to everyone.

## What the user asked for (Toby, 2026-11-02)

- A DM is exactly two people. No private groups.
- History is kept forever. A person can delete their own messages.
- An unread badge, plus an optional browser notification that shows the sender's profile picture.

## Goals

1. Open a private text chat with any other member from their person card.
2. Everything a room message already does works in a DM: replies, reactions, uploads, edits, deletes,
   markdown, typing, read markers, unread counts, search.
3. Only the two people can see a DM. The owner and moderators cannot see it in the app.
4. A new DM message can reach a person who is in a voice room, in another tab, or has the tab closed.

## Non-goals (v1.5)

- Private groups of 3 or more.
- 1:1 voice calls. A DM has no voice. A join request for a DM room is refused.
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
- `push_subscriptions.dms INTEGER NOT NULL DEFAULT 1 CHECK (dms IN (0, 1))`.

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
| `Voice::join` | refuse a DM room (`forbidden`) |

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
- A DM opens at `/r/<id>`, the same route as a room, with the chat panel and no voice controls.
- The person card (`PersonCard.svelte`) gets a "Message" button that calls `POST /api/dms` and navigates.
- Rooms keep working the same. Any code that assumes every `RoomInfo` has voice checks `kind` first.

### Notifications

- Unread badge: from `Hello.unread` and live events, as for rooms. Every unread DM message counts as a
  mention (it is addressed to you), so the badge uses the strong style.
- Web Push: a third kind beside `mentions` and `voice`. "alice sent you a message", with alice's profile
  picture as the notification icon. Settings > Notifications gets a "Direct messages" switch per device,
  on by default.
- The push does not carry the message text. This matches the existing mention push (my call).
- One push per sender per person per 30 seconds, reusing the dedupe used for "started voice", so a burst
  of messages is one notification (my call).

### What "private" means

Plain storage in `vesper.db`. The app never shows a DM to anyone but its two members. The person who runs
the server can still read the database file and the backups. This design does not change that, and the
UI should say so in one line on the Direct messages empty state.

End-to-end encryption would break server-side search and history sync, and is a project of its own.

## Order of work

1. Migration, store functions, `can_see` and every row in the visibility table, with tests. (about half a day)
2. `POST /api/dms`, `RoomInfo.kind`, regenerated types, voice refusal. (about 2 hours)
3. Sidebar section, person card button, hide voice controls. (about half a day)
4. DM push kind and settings switch. (about 2 hours)
5. Docs: `architecture.md`, `env.md` if a variable changes (none planned), `known-issues.md`.

About two days in total. Step 1 can ship alone behind no UI.

## Open questions for Toby

1. Should a moderator ever be able to read a DM, for abuse reports? This design says no. Say if that is wrong.
2. When an account is banned, should its DMs stay readable to the other person? This design says yes.
3. Unread DM messages as strong badges, or the quieter room style?

## Not verified

- Anything in a browser, including how the sidebar and the notification icon look.
- Whether every call site of `Store::rooms()` is covered by the table above. A grep for callers is the
  first task of step 1.
- That push icons work on every browser Vesper supports.
- The estimate. It assumes the existing chat panel needs no change beyond hiding voice.
