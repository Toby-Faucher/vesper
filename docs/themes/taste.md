# Theme plan: Taste

Status: Planned, not started. Written 2026-11-02. No code exists for it yet.

## Goal

Each member gets a Taste page that tracks what they watch, play, read and listen to: anime, TV shows, movies, games, books and music. Each entry has a status (Watching, Completed, Planned, Paused, Dropped), a score from 1 to 10, progress, notes, start and finish dates, a rewatch count, and the page has a favourites row. The model follows AniList and Kitsu. The group can then see what they have in common: shared titles with both scores, a taste match, and what everyone has Planned for game night.

## Why

The group already talks about what they are watching and playing. Today that lives in chat and scrolls away. A small tracker on the server they already use keeps it, and the overlap features give people a reason to look at each other's pages. It stays inside the group: members only, nothing public, nothing posted into chat.

## Non-goals

- No third-party list import in the first slices (see Sources: AniList's terms forbid tracker apps).
- No covers from other sites and no hot-linked images.
- No seasons or per-episode tracking, no diary row per watch, no reviews, no custom lists or tags.
- No per-user score formats yet. One fixed 1 to 10 scale for everyone (Toby's ruling), stored so a format can be added later.
- No public profiles, no chat posts or bots for updates, no scrobbling, no scraping.
- Nothing on the call path and nothing heavy in the first load.

## Data model

One shared catalogue that members type into, and one row per person per item. Migration `0011_taste.sql`, additive (take the next free number when the code is written).

**`media_items`**: `id`, `kind` (anime, tv, movie, game, book, music), `title` (1 to 200 characters), `title_norm` (set by the server), `original_title`, `year` (nullable), `units_total` (episodes or chapters, nullable), external id columns reserved for later (`anilist_id`, `mal_id`, `tmdb_id`, `igdb_id`, `openlibrary_id`, `musicbrainz_id`), `created_by` (set to NULL when the account goes), `created_at`, `merged_into`. A unique index on `(kind, title_norm, ifnull(year, 0))` for items that are not merged. An FTS5 index on the titles, kept by triggers like `messages_fts`.

**`media_entries`**: primary key `(account_id, item_id)`; `status`; `score` (nullable, 1 to 100); `progress` and `progress_total`; `notes` (up to 1000 characters); `started_at` and `finished_at` as fuzzy dates (`YYYY`, `YYYY-MM` or `YYYY-MM-DD`); `rewatches`; `favourite_rank` (1 to 10, unique per person); `private`; `created_at`; `updated_at`. Entries are deleted with the account.

**Score.** The database keeps 1 to 100. Today the server accepts only 10, 20 and so on up to 100, and the page shows them as 1 to 10. A later format (decimal, 100 points, 5 stars) is then a display setting and a relaxed server check, with no migration. Kitsu stores a 2 to 20 rating for the same reason. No score is NULL.

**Progress unit** follows the kind: episodes for anime and TV, chapters or pages for books, hours for games, watched or not for movies, listened or not for music albums. Rewatching is Watching with a rewatch count, as in Kitsu.

**Dedupe.** The server normalises titles (Unicode NFKC, lower case, `&` to "and", punctuation removed, a leading "the", "a" or "an" dropped, spaces collapsed). Before creating, the page shows "Is it one of these?" with up to five matches of the same kind. An exact duplicate is refused with the existing item, so the page offers "Use that one". Moderators and the owner can merge two items: entries move across, a person with both keeps the newer entry plus the other's rewatches, and the old id points at the new one.

**Privacy.** Members only, like every `/api` route. Each entry can be private: then only its owner sees it, and it is left out of every comparison and the recent strip. Moderators do not see private entries.

**Limits.** 5000 entries per person, notes 1000 characters, 10 favourites, 50 new catalogue items per person per day, and a new rate limit kind for Taste writes. Search uses the existing search limit.

## API

Wire types live in `vesper-protocol` with generated TypeScript.

| Route | What it does |
|---|---|
| `GET /api/media/search?q=&kind=` | Catalogue search for the picker and the near-match check. At most 20 results. |
| `POST /api/media` | Create an item. Answers 409 with the existing item on an exact duplicate. |
| `PATCH /api/media/{id}` | Fix the title, year or total. Its creator within 24 hours, or a moderator. |
| `POST /api/media/{id}/merge` | Moderators: merge into another item. |
| `GET /api/users/{id}/taste` | A person's entries (private ones only for yourself), favourites and stats. Filters by kind and status. |
| `PUT` and `DELETE /api/me/taste/{item_id}` | Add, change or remove your entry. |
| `PUT /api/me/taste/favourites` | Your favourites, in order, up to 10. |
| `GET /api/taste/compare/{user_id}` | Shared items with both scores, and the taste match. |
| `GET /api/taste/planned?kind=` | Items that two or more members have Planned. |
| `GET /api/taste/recent` | The last 20 completions and scores that are not private. |

No WebSocket event in the first slice: the page loads when it opens. A later additive event can refresh an open page. Updates are never posted into a room.

## Overlap and taste match

- **Shared items**: both people have an entry that is not private. Both scores side by side, biggest difference first.
- **Taste match**: over shared items that both scored, take the mean absolute difference `d` on the 1 to 10 scale; the match is `100 * (1 - d / 9)` percent, rounded. It shows only with at least 5 shared scored items, and the count is always next to it ("78%, 12 shared"). Below that it says "Not enough in common yet (3 of 5)".
- **Planned by everyone**: items Planned by at least two members, most first, filtered by kind, with names. Games for game night.
- **Recent strip**: a quiet list on the Taste page of what people finished or scored lately. No chat messages, no notifications.

The comparison logic is pure Rust with unit tests; SQL only collects rows.

## UI

- Routes `/taste` (yours) and `/taste/{user_id}`. The page is one lazy chunk, opened from the person card and from your own row.
- Page layout: a header with avatar and name; a stats strip (completed per kind, mean score, entries this year, in tabular figures); the favourites row (title, kind and year tiles with the kind icon); then the lists, one tab per kind, with a status filter and a sort (updated, score, title). A row shows title, year, status, score or a dash, progress such as "7 / 24", and dates. Editing is a native dialog with plain fields. Someone else's page has a "Compare with me" section.
- Person card: one line, "Taste: 42 completed", and a link to the page. With 5 or more shared scored items it adds the match.
- Settings: nothing in the first slice. "New entries start private" can come later if people ask.
- The Rulings hold: dark only, amber only for the primary button and focus, no colour per kind (the kind is a word and an icon), every row reachable with the keyboard, and "Saved" as `role="status"`.

## Covers

1. **None** in the first slice. Text tiles with a kind icon.
2. **Member upload** later, through the avatar pipeline: the same type sniffing, EXIF refusal and size caps, served from Vesper itself, so the CSP does not change. Any member can set one; a moderator can remove it.
3. **Cached from an outside source**: not planned. It needs API keys, outbound fetches through the push sender's SSRF guard, a credit line and refresh rules (TMDB allows caching for at most 6 months).
4. **Hot-linking**: rejected. Every page view would send each viewer's address to the image host, and the CSP would have to allow other hosts in `img-src`.

Recommendation: 1 now, 2 later.

## Import (later, if at all)

AniList's terms forbid using its API in list or tracker services, for user data and media data, and MyAnimeList's agreement forbids storing its users' personal information. Letterboxd has no public API for personal projects. So there is no import by username in the plan. Two options for a later slice, each needing Toby's yes:

- Read a member's own export file (an upload, parsed on the server, no outbound fetch). Which formats are worth it is not checked yet.
- AniList by username only with written permission from AniList. Then: one fixed host, the push sender's SSRF pattern (https on 443, public addresses only, no redirects, a time limit), one import at a time, under AniList's current limit of 30 requests a minute, statuses and scores mapped onto Vesper's.

## Size

Estimates, not measured:

- First load: the route, a lazy import and the person card line, about 0.2 to 0.4 kB gzip. It must fit under the 56.9 kB first-load ratchet. If it does not, the person card line moves into the chunk.
- Taste chunk: 12 to 18 kB gzip, under the 35 kB lazy budget. The compare view can be a second chunk if it grows.
- All JS: about 15 to 20 kB more, against 170.

## Security

- Titles and notes are shown as text only, never as HTML or markdown.
- Search input is quoted before it reaches FTS5, as message search does.
- Merge and edit rights are checked on the server.
- Covers, when they come, reuse the avatar checks and the sandbox CSP.
- Any outbound fetch, when it comes, reuses the push sender's SSRF guard with a fixed host list.

## Slices

| Slice | Size | What |
|---|---|---|
| 1 | L | Migration, catalogue with dedupe and search, your entries with every field, favourites, the Taste page with lists by kind and the stats strip, the person card link. **Ships alone.** |
| 2 | M | Compare with a person, taste match, Planned by everyone, the recent strip, the match on the person card. |
| 3 | S | Merge tool for moderators in Admin, the item edit rules. |
| 4 | M | Member-uploaded covers through the avatar pipeline. |
| 5 | M | Import from a member's own export file, or from AniList with written permission. Needs a decision first. |

## Decisions for Toby

1. Build Taste at all, as a lazy page away from the voice path? (yes or no)
2. Store the score as 1 to 100 inside and show 1 to 10 today? (yes or no)
3. A private flag per entry, and no whole-page private switch? (yes or no)
4. Any member may add catalogue items, and moderators merge duplicates? (yes or no)
5. No AniList import unless AniList gives written permission? (yes or no)
6. Allow import from a member's own export file in a later slice? (yes or no)
7. No outside metadata lookups (TMDB, IGDB, Open Library, MusicBrainz) for now? (yes or no)
8. No covers in slice 1, member uploads later, never hot-linked? (yes or no)
9. Show the taste match on the person card? (yes or no)
10. Track books in pages rather than chapters? (yes or no)
11. Track music as albums only at first? (yes or no)

## Definition of done

- **Slice 1**: the migration runs on a fresh and an upgraded database; store tests for title normalisation, the duplicate refusal, the entry checks, unique favourite ranks and private entries hidden from others; server tests for sign-in, roles and limits on every route; the normalisation test is mutation checked; an e2e step adds an item and an entry, edits it, makes it a favourite, filters by kind, and checks another member sees it but not a private one; screenshots looked at; size check passes under the ratchet; wording check passes; `docs/architecture.md` updated.
- **Slice 2**: taste match unit tests (minimum overlap, private entries left out, rounding), mutation checked; a two-browser e2e step shows the shared list and the match; size check passes.
- **Slice 3**: merge tests (both people had an entry, rewatches added, the old id follows, search updated) and role tests.
- **Slice 4**: fixture tests for the avatar refusals (svg, gif, animated webp, EXIF, too large) on covers, and covers served with nosniff and the sandbox CSP.
- **Slice 5**: parser tests on sample export files; if AniList is used, SSRF tests copied from the push sender.

## Risks

- **Terms.** The obvious import sources forbid it. Hand entry comes first, and slices 1 to 4 make no outbound calls.
- **Duplicates.** Typed titles will clash (translated names, "Season 2"). Normalisation, suggestions, the unique index and the merge tool keep it small, not zero.
- **Scope.** Trackers grow without end (seasons, diaries, reviews, lists). The non-goals above hold the line.
- **Fit.** Vesper is voice first. Taste must not touch the call path or the first load.
- **Size.** The first-load ratchet has about 1 kB of room. Everything but a route and a link is lazy.
- **Privacy.** Scores and notes are personal. Members only, a private flag per entry, nothing in chat.

## Sources

Read on 2026-11-02.

- AniList terms of use (tracker services clause changed 2026-08-18): https://github.com/AniList/docs/blob/master/docs/guide/terms-of-use.md
- AniList rate limits (90 a minute, currently 30): https://github.com/AniList/docs/blob/master/docs/guide/rate-limiting.md
- AniList `MediaList`, `MediaListStatus` and `ScoreFormat`: https://github.com/AniList/docs/tree/master/docs/reference
- MyAnimeList API agreement: https://myanimelist.net/static/apiagreement.html
- Kitsu library entry model (2 to 20 rating, private flag): https://github.com/hummingbird-me/kitsu-server/blob/the-future/app/models/library_entry.rb
- Letterboxd API access page: https://letterboxd.com/api-beta/
- TMDB API terms (attribution, 6 month cache): https://www.themoviedb.org/api-terms-of-use
- IGDB API docs (Twitch account, 4 requests a second): https://api-docs.igdb.com/
- Open Library API and Covers API: https://openlibrary.org/developers/api and https://openlibrary.org/dev/docs/api/covers
- MusicBrainz rate limiting and data licence: https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting and https://musicbrainz.org/doc/About/Data_License
- Last.fm API terms: https://www.last.fm/api/tos
- Spotify developer policy: https://developer.spotify.com/policy
