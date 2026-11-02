# Theme plan: Inside Jokes

- **Status:** Planned, not started.
- **Date:** 2026-11-02 (written against v1.4.0).
- **Releases:** follow the release notes convention in [versions.md](../versions.md#how-a-release-is-cut).

This is a plan, not a promise. Nothing here is built. Sizes are S, M or L and say nothing about time.

## Goal and the story

The group's own emoji, polls and spoilers, and rooms that stay quiet when you want.

For the people who chat between calls: "Someone adds the `:sambruh:` emoji, everyone reacts with it, and the 'which game tonight' poll settles it in a minute."

## Why now

The evidence is thin, and this plan says so. Reaction counts are from the GitHub search API on 2026-11-01:

- Rocket.Chat [#1723](https://github.com/RocketChat/Rocket.Chat/issues/1723) "Custom emoji": 124 (closed).
- Element Web [#2950](https://github.com/element-hq/element-web/issues/2950) "Implement sticker packs": 74 (closed).
- Element Web [#5891](https://github.com/element-hq/element-web/issues/5891) "Customize ringing/notification sound": 150, open.
- Stoat [#124](https://github.com/stoatchat/stoatchat/issues/124) "Spoiler embeds": 10. Stoat [#190](https://github.com/stoatchat/stoatchat/issues/190) "built-in poll support": 7.
- Zulip [#6027](https://github.com/zulip/zulip/issues/6027), per-topic push subscriptions: 45.

Element's tracker has almost no custom-emoji demand (the top title match has 4 reactions). So this rests more on Discord habit than on open-source trackers.

Why it is reasonable now: the work is low risk, and v1.4 made room for it. The all-JS budget went from 150 to 170 kB, and a backup is taken before every migration, so the new tables here deploy with a way back.

The design spec says "No threads, custom emoji, GIF search or link previews in v1" ([spec](../superpowers/specs/2026-10-30-vesper-design.md)). Custom emoji needs a yes from Toby against that line.

## Features

| # | Feature | Size |
|---|---|---|
| 1 | **Custom emoji, static only.** Moderators add them, 128 px, at most 256 KiB, png, jpeg or webp. Animated files are refused. Used as `:name:` in messages. | M |
| 2 | **Custom emoji as reactions,** stored as the `:name:` token in the existing reactions table. A deleted emoji shows its name. | S |
| 3 | **Polls.** One question, 2 to 6 options, one vote each, the vote can change, results live. | M |
| 4 | **Spoilers** `\|\|text\|\|`: hidden until clicked, and hidden in notifications, reply previews and search snippets. | S |
| 5 | **Per-room notification rule:** all, mentions only, nothing, or quiet for 1 hour. Applies to tab notifications, unread counts and Web Push. | M |
| 6 | **Drafts survive a reload,** per room, in `localStorage`, cleared on send and at sign out, pruned after 7 days. | S |
| 7 | **Per-person volumes follow you to every device.** | S |

## What each needs

Checked against the code on 2026-11-02. Migrations on `main` end at 0010, so new ones start at 0011. The numbers below are in order of the slices; the real numbers are given when each lands.

| # | Migration | Dependency | Server | CSP | Size (gzip, lazy unless said) |
|---|---|---|---|---|---|
| 1 | `custom_emoji` (0011) | None | Reuse the avatar checks, add animation checks | None (`img-src 'self'`) | Markdown +0.4, Chat +2.5, Admin +1.5 kB |
| 2 | None | None | Change `valid_reaction` | None | Small, Chat |
| 3 | `polls`, `poll_votes` | None | New routes, additive `poll_updated` message | None | Chat +2.5 kB |
| 4 | None | None | Mask spoilers in search snippets | None | Markdown +0.3 kB |
| 5 | `room_prefs` | None | Push recipients and unread counts follow the rule | None | Settings +1 kB, first load: the muted-room mark |
| 6 | None | None | None | None | Chat +0.3 kB |
| 7 | A prefs table or column | None | One route | None | Settings +0.3 kB |

What the code says today:

- **Image checks.** `avatar_mime` in `crates/vesper-server/src/avatars.rs` sniffs png, jpeg and webp, refuses gif and svg, checks that a png ends at IEND and a webp at its RIFF size, refuses EXIF and png text chunks, and caps each side at 1024 px. It does **not** check for animation. Emoji need two new checks: a png `acTL` chunk, and the webp VP8X animation flag or an `ANIM` chunk. The avatar bytes live in the database, not with the uploads; emoji at 256 KiB or less can do the same. The server never decodes an image.
- **Reactions** are `reactions (message_id, account_id, emoji)` with a cap of 20 distinct emoji per message (`set_reaction` in `crates/vesper-store/src/messages.rs`). No migration is needed for #2. But `valid_reaction` in `api_messages.rs` refuses any ASCII letter or digit, so a `:name:` token is refused today. It needs a second rule for the token form.
- **Spoilers are more than a markdown change.** Pushes never carry message text (the body is "X mentioned you in Room", `push.rs`). But tab notifications show up to 120 characters of the message (`web/src/lib/ui/notify.ts`), reply previews quote it, and search snippets are cut by SQLite FTS5 on the server (`crates/vesper-store/src/search.rs`), so the `||` marks can fall outside the 12-token snippet. Each of these must hide spoiler text.
- **Drafts** are kept in memory (`chat.drafts` in `web/src/lib/chat.svelte.ts`) and cleared at sign out so the next account sees none (`session.svelte.ts`). Drafts in `localStorage` must be cleared at sign out too.
- **Volumes** are saved per device in `localStorage` (`saveVolumes` in `web/src/lib/voice.svelte.ts`) and never pruned ([known-issues.md](../known-issues.md)). Moving them to the server can fix the pruning too.
- **Notifications.** Web Push switches are per device (`push_subscriptions.mentions` and `voice`, migration 0009). Tab notification choices live in `localStorage` (`vesper.notify`). A per-room rule is new server state.

### Size, with v1.4.0 numbers

Measured on `main` at v1.4.0 (`npm run build`, `scripts/size-check.mjs`, gzip): first load 56.6 kB against a ratchet of 56.9 (hard cap 70), Settings 14.7 (the largest lazy chunk, cap 35), Chat 14.4, Admin 11.3, all JS 149.9 against 170.

- **All JS:** about 8 to 9 kB of lazy code. It fits in the 20 kB of headroom, but Game Night wants about half of that headroom too.
- **Lazy chunks:** Chat grows to about 20 kB and Settings to about 16. Both stay under 35.
- **First load:** only 0.3 kB is left under the ratchet. The rule menu (#5) can be lazy. The "muted" mark on a room in the sidebar is first load: keep it to an icon, or raise the ratchet on purpose (a decision below).

## Risks

- **Security.** User images go to every browser. Static only, size and side caps, magic-byte sniffing and no svg keep it small. CVE-2023-4863 (a libwebp heap overflow reached through a crafted image, fixed in Chrome 116.0.5845.187) is why decoding stays in the browser and files stay small and static.
- **Product.** Emoji spam. Moderators curate, with a cap of 200 emoji.
- **Privacy.** A spoiler must never leak through a notification, a reply preview or a search snippet.
- **Notifications.** Rules that go wrong are a common complaint (Element [#24392](https://github.com/element-hq/element-web/issues/24392) "Stuck notifications", 111). Each rule needs a test across tab, unread count and push.

## Decisions for Toby

The all-JS budget is already decided: it was raised from 150 to 170 kB on 2026-11-02.

- [ ] Allow custom emoji, against the spec's "no custom emoji in v1"? (yes/no)
- [ ] May members (not only moderators) add custom emoji? (yes/no)
- [ ] Polls in scope (a new message kind)? (yes/no)
- [ ] Do room notification rules also mute "started voice" pushes? (yes/no)
- [ ] Raise the first-load ratchet by up to 0.5 kB for the muted-room mark, if it does not fit in 0.3 kB? (yes/no)

## Definition of done

- Server tests refuse animated webp, APNG, gif, svg, EXIF, oversize files and too-large sides (fixture files).
- A `valid_reaction` test accepts a known `:name:` and still refuses plain text.
- Markdown unit tests: `:name:` and spoilers, and no HTML injection.
- Tests that a spoiler is hidden in a tab notification, a reply preview and a search snippet.
- e2e: add an emoji, use it, react with it, delete it.
- A test that drafts are gone after sign out.
- Each new migration is additive, and the size check passes with all five numbers in the PR.

## Not included

Animated emoji, GIF avatars, stickers and GIF search: decoder risk, and autoplay CPU during a call (UI principle 5 in [ui-design.md](../ui-design.md)). Stoat [#664](https://github.com/stoatchat/for-web/issues/664) asks to turn GIF autoplay off (6). Also out:

- Link previews: a server-side fetch is an SSRF risk next to other services on the same host. The push sender's guard is narrow (a short list of push hosts) and does not generalise.
- Threads and forums: a work-chat shape, against "nothing on the right".
- Per-room nicknames, themes beyond the accent, a soundboard.
- Voice messages and webhooks: thin evidence.
- Offline mode and OPFS caches: drafts fit in `localStorage`.

## First slice that ships alone

Custom static emoji in messages (#1), with drafts (#6) riding along. One migration (0011), no protocol change beyond the emoji list.

## Sources

- Rocket.Chat [#1723](https://github.com/RocketChat/Rocket.Chat/issues/1723), Element Web [#2950](https://github.com/element-hq/element-web/issues/2950), [#5891](https://github.com/element-hq/element-web/issues/5891) and [#24392](https://github.com/element-hq/element-web/issues/24392), Stoat [#124](https://github.com/stoatchat/stoatchat/issues/124), [#190](https://github.com/stoatchat/stoatchat/issues/190) and for-web [#664](https://github.com/stoatchat/for-web/issues/664), Zulip [#6027](https://github.com/zulip/zulip/issues/6027). Seen 2026-11-01.
- Element Call [#3594](https://github.com/element-hq/element-call/issues/3594) "Keep user volume persistent between calls" (14), for #7.
- [NVD CVE-2023-4863](https://nvd.nist.gov/vuln/detail/CVE-2023-4863).
- In this repo: `crates/vesper-server/src/avatars.rs`, `crates/vesper-server/src/api_messages.rs`, `crates/vesper-store/src/messages.rs`, `crates/vesper-store/src/search.rs`, `crates/vesper-server/src/push.rs`, `crates/vesper-store/migrations/`, `web/src/lib/ui/notify.ts`, `web/src/lib/chat.svelte.ts`, `web/src/lib/voice.svelte.ts`, `web/scripts/size-check.mjs`.
