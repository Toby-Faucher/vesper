# Vesper Accounts, Rooms and Text Chat (Backend) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the shared join key with real accounts (invites, passwords, sessions, roles), add stored rooms with live voice presence for everyone, and add text chat (history, edits, replies, reactions, mentions, pins, unread, uploads, search), with just enough web code that the existing voice screen works behind a login.

**Architecture:** A new crate `vesper-store` owns SQLite (sqlx, WAL, FTS5, migrations) and is the only crate that imports sqlx (ADR 0005). `vesper-server` gains REST routes under `/api` for every mutation and for history and search, and keeps one authenticated WebSocket per tab for pushed events, voice signaling and typing. The session cookie authenticates both; an Origin check guards every state-changing request and the WebSocket upgrade.

**Tech Stack:** Rust 1.99.1 (pinned), tokio, axum 0.8, sqlx 0.9.0 (sqlite-bundled), argon2 0.6.0, unicode-normalization 0.1.26, getrandom 0.3, rustix 1 (`fs`), sha2 0.11, ts-rs 10; Svelte 5, TypeScript, vitest, playwright-core 1.64.1. Stretch: webauthn_rp 0.3.0.

**Spec:** `docs/superpowers/specs/2026-10-30-vesper-design.md` (sections "Text chat", "Rooms and voice presence", "Accounts and access", "Security", "Calls added after builder review"). Decisions: `docs/decisions/0003` (removed here), `0004` (revisited here), `0005` (built here).

**Precondition (ADR 0006):** the quality checkpoint (`docs/testing/checkpoint.md` section 11.5) is not signed off as of 2026-11-01. ADR 0006 says accounts and text plans do not start before it. This plan is written so it is ready; the controller must get Toby's sign-off or an explicit waiver before Task 1 runs.

## Global Constraints

- `scripts/check.sh` passes at the end of every milestone; `scripts/check.sh fast` at the end of every task. Builds run through `scripts/capped.sh` (the scripts already do).
- `unsafe` is forbidden in every crate, including `vesper-store`. Add `[lints] workspace = true` and `#![forbid(unsafe_code)]`.
- No blanket `cargo update`. Add each crate at the version below; if the lock needs a move, `cargo update -p <crate> --precise <ver>`.
- Wire types (WebSocket messages and every REST request and response body) live in `crates/vesper-protocol` with `#[derive(TS)] #[ts(export)]`. Run `cargo test -p vesper-protocol` and commit `web/src/lib/gen/*.ts`. Never edit `gen/` by hand.
- Every `i64` id or time in a wire type carries `#[ts(type = "number")]` (ts-rs maps `i64` to `bigint` otherwise). Times are unix milliseconds.
- `user_id` is 32 lowercase hex characters. From M2 on it is `accounts.id` itself (random, see Task 1).
- No secrets in files: no real passwords, tokens or invite codes in tests, docs, logs or commits. Test passwords look like `test-password-1`. Never log a token, invite code, password or cookie value; `Config`'s `Debug` stays redacted.
- Wording rules apply to every doc and web string (`scripts/check-wording.sh`): short plain sentences, no em dashes.
- Docs live once: variables in `docs/env.md`, routes and messages in `docs/architecture.md`. Change a variable, route or message in the same commit as its doc.
- Limits from the spec: uploads 25 MB per file (25 × 1024 × 1024 bytes) plus a total quota; disk warning at 80%; sessions 90 days; voice soft cap 12; history never auto-deleted.
- Roles: `owner`, `moderator`, `member`. Moderators delete others' messages, kick from a voice room, force-mute, disconnect. Bans and role changes are owner-only.
- No new npm dependency. New Rust crates only from the table below; each must pass `scripts/check.sh deny`.

### Crates

| Crate | Version | Where | License (cargo-deny) | Risk |
|---|---|---|---|---|
| `sqlx` | `0.9.0`, `default-features = false`, features `runtime-tokio`, `sqlite-bundled`, `migrate`, `macros` | vesper-store | MIT OR Apache-2.0 | Pulls `foldhash 0.2` (Zlib) through `hashlink`: add `"Zlib"` to `deny.toml` `allow` with a comment. Checked with `cargo deny check bans licenses sources` in a throwaway project on 2026-11-01: passes with Zlib added, no openssl. `libsqlite3-sys 0.37` bundled (MIT, SQLite is public domain). Needs rustc 1.94, we pin 1.99.1. |
| `argon2` | `0.6.0` (default features) | vesper-server | MIT OR Apache-2.0 | Pulls `password-hash 0.6.1`, `blake2`, `base64ct`: all MIT OR Apache-2.0. |
| `unicode-normalization` | `0.1.26` | vesper-server | (MIT OR Apache-2.0) AND Unicode-3.0 | Already in the lock and allowed. |
| `getrandom` | `0.3` (lock has 0.3.4) | vesper-store | MIT OR Apache-2.0 | Already in the lock. |
| `rustix` | `1` (lock has 1.1.6), feature `fs` | vesper-server | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | Already in the lock; only `statvfs` is used. |
| `tempfile` | `3` | dev-dependency of vesper-store, vesper-server | MIT OR Apache-2.0 | New, small. |
| `tower` | `0.5` (lock 0.5.3), feature `util` | dev-dependency of vesper-server | MIT | Already in the lock (axum). |
| `http-body-util` | `0.1` (lock 0.1.5) | dev-dependency of vesper-server | MIT | Already in the lock. |
| `webauthn_rp` | `0.3.0` | vesper-server, stretch Task 6 only | MIT OR Apache-2.0 | Passes deny in the same throwaway check, no openssl. Smaller project than webauthn-rs; read its source before use. `webauthn-rs 0.6.2` is out: MPL-2.0 (not allowed) and openssl (banned). |

Not added, on purpose: `axum-extra` (cookies are two small functions), `governor` (Task 2's limiter is about 60 lines), `multer` (uploads are a raw body), `reqwest` (tests use `tower::ServiceExt::oneshot`), any Markdown or sanitiser npm package (Task 16).

## Rulings

Toby was not available; each call below is the planner's. Format: `Ruling: <what> | why | cost if wrong`.

- Ruling: execution waits for the ADR 0006 sign-off or a waiver | ADR 0006 is accepted and binding | none if obeyed.
- Ruling: `vesper-store` returns `vesper-protocol` wire structs where the shape matches (`UserInfo`, `RoomInfo`, `Device`, `InviteInfo`, `Message`, `SearchHit`); private rows (password hash, token hash) never leave the crate | no mirror type layer | a storage-only field could leak into TS; split that type then.
- Ruling: runtime `sqlx::query` / `query_as` with `#[derive(FromRow)]`, no `query!` macros; `sqlx::migrate!` embeds `crates/vesper-store/migrations` | no `.sqlx` cache or `DATABASE_URL` in CI (ADR 0005 consequence) | SQL typos are caught by tests, not the compiler.
- Ruling: sqlx 0.9.0, and `Zlib` joins the deny allow list | checked on 2026-11-01 | if 0.9 misbehaves, 0.8.6 has the same API for what we use.
- Ruling: store tests use a temp-file database (`tempfile::TempDir`), never `:memory:` | WAL and a multi-connection pool behave as in production | tests a few ms slower.
- Ruling: `accounts.id` is 16 random bytes as 32 lowercase hex and is the `user_id` | keeps the MSID rule with no derivation, ids are not guessable | none known.
- Ruling: usernames are ASCII `[a-z0-9_]{3,24}`, lowercased on input; display names are Unicode, NFC-normalised, 1 to 32 chars, with control, format, zero-width and bidi-override characters refused | login names cannot be lookalikes | friends with non-ASCII handles put them in the display name.
- Ruling: passwords 8 to 128 chars; argon2id m = 19456 KiB, t = 2, p = 1; at most 2 hashes run at once (a semaphore) in `spawn_blocking` | OWASP baseline on a 1 GB box | logins queue under a burst.
- Ruling: session token is 32 random bytes as hex in the cookie, SHA-256 of it in the DB; expiry 90 days after last use, `last_seen_at` written at most hourly | spec "90 days"; a DB leak gives no usable cookie | none.
- Ruling: cookie `__Host-vesper_session` (Secure) when the origin is https, `vesper_session` when http; always `HttpOnly; SameSite=Strict; Path=/; Max-Age=7776000` | `__Host-` blocks subdomain cookie tossing; dev and e2e run on http | none.
- Ruling: one variable `VESPER_ORIGIN` (default `http://localhost:8080`) drives the Origin check, the Secure flag and every printed link | one source of truth | Vite dev must set it to `http://localhost:5173` (docs/dev.md says so).
- Ruling: invites are 32 random bytes, stored hashed, default 1 use, 7 days, role `member`; owner and moderators mint member invites, only the owner mints moderator invites, the owner role only comes from the CLI; the link is `<origin>/invite#<code>` | the fragment never reaches Caddy logs or a Referer | a link pasted without the fragment fails with a clear page.
- Ruling: `vesper-server create-owner` writes a one-use owner invite valid 24 h and prints its link; it refuses when an owner exists | reuses the signup path, as the spec says invites work the same way | a lost owner password needs a manual DB edit (no reset command in this plan).
- Ruling: every mutation and all history and search go over REST JSON under `/api`; the WebSocket carries pushed events, voice signaling and typing | status codes, simple tests, one CSRF rule | a send costs one HTTP round trip.
- Ruling: room management (create, rename, reorder, restrict, member list) is owner-only | the spec lists moderator powers in full and rooms are not among them | one role check to change.
- Ruling: owner and moderators see restricted rooms; members see them only when listed | moderators must be able to moderate | moderators can read private rooms.
- Ruling: restricted rooms' messages, typing and voice presence go only to sockets that can see the room | privacy follows the member list | none.
- Ruling: the voice cap of 12 stays a constant for everyone | "soft" is read as a policy number, not a protocol rule | the 13th person waits.
- Ruling: force-mute is presence plus client enforcement (the SFU cannot drop one sender's track today) | no SFU change in this plan | a modified client can ignore it; the moderator then disconnects them. Goes in known-issues.
- Ruling: moderator voice actions (kick from voice, force-mute, disconnect) are REST endpoints | same rule as all mutations | none.
- Ruling: the per-socket outbound queue becomes bounded (256); a full queue closes that socket | chat makes the known unbounded-queue issue real | a very slow client reconnects.
- Ruling: no join deadline and no pre-join error cap; an unauthenticated upgrade gets 401 before the upgrade | sockets are authenticated up front | `VESPER_JOIN_DEADLINE_SECS` is removed.
- Ruling: `rooms.rs` is renamed `voice.rs` | "rooms" now means stored rooms | one commit of churn.
- Ruling: rate limits are an in-process token bucket, no crate | about 60 lines | per process only, fine for one binary.
- Ruling: client IP is the rightmost `X-Forwarded-For` entry when the TCP peer is loopback or a private address, else the peer | Caddy sits in front | spoofable if the HTTP port is exposed to the LAN without Caddy (deploy.md warns).
- Ruling: message body 1 to 4000 chars after trimming, control characters refused except `\n` and `\t`; JSON request bodies capped at 32 KiB | a cap well above chat use | long pastes go as a file.
- Ruling: edit own only; delete own, or any as moderator or owner; delete is soft (body emptied, tombstone kept, its uploads removed from disk) | replies keep their anchor | deleted text is gone for good (intended).
- Ruling: anyone who can see a room can pin; 50 pins per room | friend group | pin fights; change to moderators.
- Ruling: a reaction is any string of 1 to 32 bytes with no ASCII letter, digit, whitespace or control character; 20 distinct per message | no emoji table dependency | odd symbols pass.
- Ruling: mentions are parsed on the server from `@username` at send and edit, kept in a table | unread mention counts need them | none.
- Ruling: edit, delete, reaction and pin changes all push one `MessageUpdated` with the full message | fewer wire types | slightly larger frames.
- Ruling: Markdown is parsed by a hand-written TS tokenizer into a tree rendered with Svelte text nodes; no `{@html}` or `innerHTML` anywhere (a test enforces it); links are bare `http`/`https` autolinks only, no `[text](url)` | no HTML string exists, so there is nothing to sanitise | fewer Markdown features.
- Ruling: uploads are a raw-body POST, one file per request, no multipart | streaming to disk under a byte cap is simpler | none.
- Ruling: images are shown inline only for png, jpeg, gif and webp whose magic bytes match; everything else is served `application/octet-stream` as an attachment; SVG is never inline | blocks script in uploads | no SVG previews.
- Ruling: uploads not attached to a message within 24 h are swept | the quota must not fill with abandoned uploads | a draft upload left a day is lost.
- Ruling: `VESPER_UPLOAD_QUOTA_MB` defaults to 10240 | the VM has 20 GB | set it on deploy.
- Ruling: the disk figure is DB plus WAL size, the sum of upload sizes against the quota, and filesystem total and free from `rustix::fs::statvfs` | spec "admin page shows disk use and warns at 80%" | none.
- Ruling: FTS5 external-content table kept by triggers, tokenizer `unicode61 remove_diacritics 2`; user input becomes quoted terms, the last one a prefix | no FTS syntax errors and no operator injection | no boolean search syntax.
- Ruling: search returns a snippet with `\u0002` / `\u0003` around matches plus the message; context comes from history `around=<id>` | the client renders highlights as text | none.
- Ruling: REST tests drive the router in process with `tower::ServiceExt::oneshot`; WebSocket tests use a real socket as today | no HTTP client dependency | none.
- Ruling: the load test authenticates with tokens from `vesper-server loadtest-accounts --count N` | no HTTP client in the load test | must run on a throwaway data dir (docs say so).
- Ruling: M2 web is login, invite signup, a session store, a room picker and logout; account and admin screens wait for the redesign plan; invites come from the CLI and the API until then | scope given | Toby uses the CLI for invites for a while.
- Ruling: M3 web is the Markdown module, a chat store and a bare chat panel, enough for the e2e test | scope given | it looks rough until the redesign.
- Ruling: one e2e script (`web/e2e/two-browser.mjs`) grows the account and chat steps, helpers move to `web/e2e/lib.mjs` | one server start, one CI job | a chat failure fails the voice job too.
- Ruling: a migration `general` room exists on a fresh install | the voice screen needs a room | none.
- Ruling: `VESPER_DATA_DIR` defaults to `data` (relative, dev); Docker mounts `/data`; systemd uses `StateDirectory=vesper` (`/var/lib/vesper`) | spec: one directory | the host directory must be owned by uid 10001 for Docker.
- Ruling: the 100k-message performance test is `#[ignore]` and run by hand in release | debug-built SQLite timings are noise in CI | regressions show only when someone runs it.
- Ruling: the server sends the CSP on every response, not Caddy; upload responses get their own `sandbox` policy | one place, tested | the Caddyfile CSP comment is deleted.
- Ruling: the connectivity check joins a hidden SFU room under a random SFU user id | the SFU keys sessions by user, so the real id would end your own call | none.
- Ruling: WebSocket close code 4001 means "session ended" (logout, revoke, ban) and 4003 "disconnected by a moderator" | the client must tell a lost session from a network drop | none.
- Ruling: passkeys are Task 6, run only after M1 to M3 are green | stretch per brief | password fallback is the only login until then.

## Review Focus

1. **Lookalike names.** A username or display name using fullwidth letters, Cyrillic lookalikes, zero-width or bidi-override characters (`ad\u{200B}min`, `\u{202E}nimda`) must not pass as someone else. Expected: usernames outside `[a-z0-9_]` refused, `Toby` and `toby` are one account, display names with format characters refused. Test: Task 2 `username_rules`, `display_name_refuses_format_chars`.
2. **Replayed or raced invite links.** The same single-use link used twice, two signups racing on it, an expired or revoked link, a code with a changed letter. Expected: exactly one account, the rest get a clear "invite not valid" with no hint which case. Test: Task 1 `invite_is_single_use_under_race`, Task 4 `signup_with_used_invite_is_rejected`.
3. **Huge or hostile message bodies.** 4001 chars, 1 MB JSON, `<script>`, `javascript:` links, 10 000 asterisks, NUL bytes, a deep `> > > >` quote. Expected: oversize refused with 413 or 400 and no partial write; hostile text shown as text; the renderer stays linear. Test: Task 13 `send_refuses_oversize_and_control_chars`, Task 16 `markdown_is_inert` and `markdown_is_linear`.
4. **Upload path and type tricks.** Ids like `../../db.sqlite`, `%2e%2e`, uppercase hex, names with `"`, CR LF or `/`, an HTML file named `cat.png`, an SVG, a 25 MB plus 1 byte body, a body with no Content-Length. Expected: 404 for bad ids, octet-stream attachment for anything not a verified raster image, 413 past the cap with the temp file removed, header injection impossible. Test: Task 14 `upload_id_traversal_is_404`, `html_disguised_as_png_is_attachment`, `oversize_body_is_413_and_cleaned`, `content_disposition_escapes_name`.
5. **Sessions that should be dead.** A revoked device, a banned user, a changed password, a cookie set before login (fixation), a cookie from another origin's page. Expected: the live WebSocket closes with 4001 within one second, voice is left, the next request is 401, login always mints a new token, cross-origin POST and upgrade get 403. Test: Task 8 `revoked_session_closes_socket`, `ban_closes_socket_and_voice`, `wrong_origin_upgrade_is_403`; Task 4 `login_mints_new_token_and_drops_presented_one`.

---

## File Structure

```
crates/vesper-store/                    new: the only crate that imports sqlx
  Cargo.toml
  migrations/0001_accounts.sql          accounts, sessions, invites
  migrations/0002_rooms.sql             rooms, room_members, the general room
  migrations/0003_messages.sql          messages, reactions, mentions, reads, messages_fts + triggers
  migrations/0004_uploads.sql           uploads
  src/lib.rs                            Store, StoreError, open, random_hex, now_ms
  src/accounts.rs  src/sessions.rs  src/invites.rs
  src/rooms.rs  src/messages.rs  src/uploads.rs  src/search.rs
  tests/accounts.rs  tests/rooms.rs  tests/messages.rs  tests/search.rs  tests/perf.rs
crates/vesper-protocol/src/lib.rs       voice and WS messages (existing file)
crates/vesper-protocol/src/api.rs       new: REST bodies and shared structs
crates/vesper-server/src/
  config.rs     data dir, origin, quota (edit)
  main.rs       subcommand dispatch (edit)
  cli.rs        new: create-owner, invite, loadtest-accounts
  lib.rs        AppState, router, headers (edit)
  auth.rs       new: passwords, names, cookies, Authed extractor, origin check, client ip
  limits.rs     new: token-bucket limiter
  api_accounts.rs  api_admin.rs  api_rooms.rs  api_messages.rs  uploads.rs   new
  hub.rs        new: connected sockets, visibility-filtered broadcast
  voice.rs      renamed from rooms.rs: voice presence by room id
  ws.rs         authenticated socket (edit)
crates/vesper-server/tests/
  common/mod.rs  new: test server, login helper, cookie and origin headers
  signaling.rs   (edit)  static_files.rs (edit)  accounts.rs  rooms.rs  chat.rs  uploads.rs  (new)
web/src/lib/api.ts  state.ts  session.svelte.ts  chat.svelte.ts  markdown.ts   new (+ .test.ts each)
web/src/routes/Login.svelte  Signup.svelte  Chat.svelte  Markdown.svelte     new
web/src/App.svelte                       new: tiny path router
web/src/lib/signaling.ts  peers.ts  diag.ts  web/src/routes/Room.svelte  Diagnostics.svelte  web/src/main.ts   edit
web/e2e/lib.mjs (new)  web/e2e/two-browser.mjs (edit)
crates/vesper-loadtest/src/main.rs (edit)
deploy/compose.yaml  deploy/vesper.service  deploy/Dockerfile  deploy/Caddyfile.example   edit
docs/decisions/0007-accounts-and-sessions.md  0008-csp.md   new
docs/env.md  deploy.md  runbook.md  architecture.md  known-issues.md  dev.md  testing/latency.md  CLAUDE.md   edit
deny.toml  scripts/check-secrets.sh  scripts/secrets-allow.txt   edit
```

---

# Milestone 1: store and accounts

### Task 1: `vesper-store` crate, FTS5 spike, account tables

**Files:**
- Create: `crates/vesper-store/{Cargo.toml,src/lib.rs,src/accounts.rs,src/sessions.rs,src/invites.rs,migrations/0001_accounts.sql,tests/accounts.rs}`
- Create: `crates/vesper-protocol/src/api.rs` (types below), re-exported from `lib.rs`
- Modify: root `Cargo.toml` (member, `vesper-store` workspace dep), `deny.toml` (`"Zlib", # foldhash, via sqlx-core's hashlink`), `docs/decisions/0005-sqlite-sqlx.md` status line to "accepted, built in 2026-11-01 plan" and the README table, `docs/architecture.md` crate table row.

**Interfaces:**
- Produces (protocol, `api.rs`):
  - `enum Role { Owner, Moderator, Member }` (`snake_case`)
  - `struct UserInfo { user_id: String, username: String, display_name: String, accent: String, role: Role, banned: bool }`
  - `struct Device { id: i64, label: String, created_at: i64, last_seen_at: i64, current: bool }`
  - `struct InviteInfo { id: i64, role: Role, uses: u32, max_uses: u32, expires_at: i64, created_by: Option<String>, revoked: bool }`
- Produces (store):
  - `pub struct Store` (`Clone`, wraps `SqlitePool`); `pub async fn Store::open(path: &Path) -> Result<Store, StoreError>`
  - `pub enum StoreError { NotFound, Conflict(&'static str), InviteInvalid, Db(sqlx::Error) }` with `Display` and `From<sqlx::Error>`
  - `pub fn random_hex(bytes: usize) -> String` (getrandom, lowercase); `pub fn now_ms() -> i64`
  - `pub struct NewAccount<'a> { pub username: &'a str, pub display_name: &'a str, pub password_phc: Option<&'a str>, pub role: Role }`
  - `pub struct NewSession<'a> { pub token_hash: [u8; 32], pub label: &'a str, pub now: i64 }`
  - accounts: `create_account(&self, a: NewAccount<'_>) -> Result<UserInfo>`; `signup(&self, invite_hash: &[u8; 32], a: NewAccount<'_>, s: NewSession<'_>) -> Result<(UserInfo, i64)>` (one transaction: use invite, take its role, insert account, insert session; returns session id; `a.role` is ignored here); `login_row(&self, username: &str) -> Result<Option<LoginRow>>` with `pub struct LoginRow { pub user_id: String, pub password_phc: Option<String>, pub banned: bool }`; `user(&self, id: &str) -> Result<UserInfo>`; `users(&self) -> Result<Vec<UserInfo>>`; `update_profile(&self, id: &str, display_name: Option<&str>, accent: Option<&str>) -> Result<UserInfo>`; `set_password(&self, id: &str, phc: &str, keep_session: i64) -> Result<()>` (deletes the user's other sessions); `set_role(&self, id: &str, role: Role) -> Result<UserInfo>`; `set_banned(&self, id: &str, banned: bool, now: i64) -> Result<UserInfo>` (ban deletes all sessions); `owner_exists(&self) -> Result<bool>`
  - sessions: `create_session(&self, user_id: &str, s: NewSession<'_>) -> Result<i64>`; `session_user(&self, token_hash: &[u8; 32], now: i64) -> Result<Option<(i64, UserInfo)>>` (None when missing, older than 90 days since `last_seen_at`, or banned; updates `last_seen_at` when it is over an hour old); `sessions(&self, user_id: &str, current: i64) -> Result<Vec<Device>>`; `delete_session(&self, user_id: &str, id: i64) -> Result<()>` (NotFound if not theirs)
  - invites: `create_invite(&self, code_hash: &[u8; 32], role: Role, max_uses: u32, expires_at: i64, created_by: Option<&str>) -> Result<InviteInfo>`; `invites(&self) -> Result<Vec<InviteInfo>>`; `revoke_invite(&self, id: i64, now: i64) -> Result<()>`; `invite_role(&self, code_hash: &[u8; 32], now: i64) -> Result<Option<Role>>`
  - `pub const SESSION_TTL_MS: i64 = 90 * 24 * 3600 * 1000;`

Schema notes for `0001_accounts.sql`: all tables `STRICT`; `accounts(id TEXT PRIMARY KEY CHECK (length(id) = 32), username TEXT NOT NULL UNIQUE, display_name, password_phc TEXT NULL, role TEXT CHECK (role IN ('owner','moderator','member')), accent TEXT NOT NULL DEFAULT '#7c8cff', banned_at INTEGER, created_at INTEGER)`; `sessions(id INTEGER PRIMARY KEY, account_id REFERENCES accounts ON DELETE CASCADE, token_hash BLOB UNIQUE, label, created_at, last_seen_at)`; `invites(id INTEGER PRIMARY KEY, code_hash BLOB UNIQUE, role, max_uses, uses DEFAULT 0, expires_at, revoked_at, created_by REFERENCES accounts, created_at)`. Using an invite is one statement inside the signup transaction:

```sql
UPDATE invites SET uses = uses + 1
 WHERE code_hash = ?1 AND uses < max_uses AND expires_at > ?2 AND revoked_at IS NULL
RETURNING role;   -- no row: StoreError::InviteInvalid
```

`open` sets, per connection: `journal_mode=WAL`, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000`; `create_if_missing`; pool of 4 connections; runs `sqlx::migrate!()`. Creates the parent directory.

- [ ] **Step 1: Write the failing tests** in `crates/vesper-store/tests/accounts.rs` (each opens `Store::open(tmp.path().join("v.db"))`):
  - `fts5_is_compiled_in`: `SELECT sqlite_compileoption_used('ENABLE_FTS5')` is 1, and `CREATE VIRTUAL TABLE t USING fts5(x)` then a `MATCH` query returns the inserted row. This is the spike: if it fails, stop and report; the fix is a sqlx feature, not a workaround.
  - `wal_mode_is_on`: `PRAGMA journal_mode` returns `wal`; `PRAGMA foreign_keys` returns 1.
  - `random_hex_is_lowercase_and_sized`: `random_hex(16)` has len 32, all `0-9a-f`, and two calls differ.
  - `create_account_rejects_duplicate_username`: second `create_account` with the same username is `Conflict(_)`; ids are 32 hex.
  - `invite_is_single_use_under_race`: one invite with `max_uses = 1`; 8 concurrent `signup` calls with different usernames; exactly 1 `Ok`, 7 `InviteInvalid`; `users().len() == 1`.
  - `expired_and_revoked_invites_are_invalid`: expires_at in the past gives `InviteInvalid`; revoked gives `InviteInvalid`; `invite_role` returns None for both.
  - `signup_takes_role_from_invite`: a moderator invite yields `Role::Moderator` even with `a.role = Member`.
  - `session_expires_after_ttl_and_touch_is_hourly`: `session_user` at `now + SESSION_TTL_MS + 1` after the last touch is None; at `now + 30 min` it does not change `last_seen_at`; at `now + 61 min` it does.
  - `ban_and_password_change_drop_sessions`: `set_banned(true)` leaves 0 sessions and `session_user` None; `set_password(keep)` leaves only `keep`.
  - `delete_session_of_someone_else_is_not_found`.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-store` and see them fail to compile.
- [ ] **Step 3: Implement** the crate as in Interfaces. Hash and token generation live in the server; the store only sees hashes.
- [ ] **Step 4: Run** the same command: all pass. Then `scripts/check.sh fast bindings deny`.
- [ ] **Step 5: Commit** `feat(store): sqlite store with accounts, sessions and invites`.

### Task 2: Auth primitives and the rate limiter

**Files:**
- Create: `crates/vesper-server/src/auth.rs`, `crates/vesper-server/src/limits.rs` (unit tests inline)
- Modify: `crates/vesper-server/Cargo.toml` (argon2, unicode-normalization, vesper-store), `lib.rs` (`pub mod`)

**Interfaces:**
- Produces (`auth.rs`):
  - `pub fn valid_username(s: &str) -> Option<String>` (trim, lowercase ASCII, `[a-z0-9_]{3,24}`)
  - `pub fn valid_display_name(s: &str) -> Option<String>` (trim, NFC, collapse inner whitespace runs to one space, 1 to 32 chars, refuse `char::is_control` and the format ranges U+00AD, U+061C, U+180E, U+200B to U+200F, U+202A to U+202E, U+2060 to U+2064, U+2066 to U+2069, U+FEFF). Room names use the same rule.
  - `pub fn valid_accent(s: &str) -> Option<String>` (`#rrggbb`, lowercased)
  - `pub fn valid_password(s: &str) -> bool` (8 to 128 chars)
  - `pub async fn hash_password(p: String) -> String` and `pub async fn verify_password(p: String, phc: Option<String>) -> bool` (argon2id with the Rulings' params, `spawn_blocking`, a `static` `Semaphore` of 2; `None` verifies against a fixed dummy PHC so unknown users take the same time)
  - `pub fn new_token() -> (String, [u8; 32])` (64-hex token, its SHA-256)
  - `pub fn hash_token(hex: &str) -> Option<[u8; 32]>` (None unless 64 lowercase hex)
  - `pub fn session_cookie(token: &str, secure: bool) -> HeaderValue` and `pub fn clear_cookie(secure: bool) -> HeaderValue`
  - `pub fn cookie_token(headers: &HeaderMap, secure: bool) -> Option<String>` (reads only the cookie name for `secure`)
  - `pub fn origin_ok(headers: &HeaderMap, origin: &str) -> bool` (exact match of the `Origin` header; missing is false)
  - `pub fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr` (Ruling on X-Forwarded-For)
  - `pub fn device_label(user_agent: Option<&str>) -> String` ("Firefox on Linux", "Chrome on Windows", fallback "Unknown browser"; at most 40 chars)
- Produces (`limits.rs`):
  - `pub enum Kind { Login, LoginIp, Invite, Message, Upload, Typing }`
  - `pub struct Limiter` (`Default`, `Clone` via `Arc`); `pub fn check(&self, kind: Kind, key: &str, now: Instant) -> Result<(), Duration>` (Err carries retry-after)
  - Budgets (burst / refill): Login per `ip+username` 5 / 1 per 60 s; LoginIp per ip 20 / 1 per 15 s; Invite per ip 10 / 1 per 6 min; Message per user 10 / 1 per s; Upload per user 10 / 1 per 30 s; Typing per user+room 1 / 1 per 3 s. Entries idle 10 min are pruned when the map passes 10 000 keys.

- [ ] **Step 1: Write failing unit tests:**
  - `username_rules`: `"Toby"` gives `"toby"`; `"ab"`, `"a b"`, `"ｔｏｂｙ"` (fullwidth), `"tоby"` (Cyrillic о), `"to\u{200B}by"` all None; 24 chars ok, 25 None.
  - `display_name_refuses_format_chars`: `"Al\u{200B}ice"`, `"\u{202E}ecila"`, `"a\u{0007}"` are None; `"Ame\u{0301}lie"` returns the NFC form `"Amélie"`; `"  a   b "` gives `"a b"`.
  - `password_roundtrip_and_dummy`: `verify_password(p, Some(hash_password(p)))` true; a wrong password false; `verify_password(p, None)` false.
  - `token_hash_rejects_bad_hex`: `hash_token` of 63 chars, uppercase, or non-hex is None; `new_token().1 == hash_token(&new_token().0)` for the same token.
  - `cookie_flags`: secure cookie string starts `__Host-vesper_session=` and contains `Secure; HttpOnly; SameSite=Strict; Path=/; Max-Age=7776000`; the http one has no `Secure` and the plain name.
  - `cookie_token_ignores_other_cookies`: header `a=1; vesper_session=<64 hex>; b=2` returns the token; with `secure = true` it returns None.
  - `origin_check`: exact origin true; `http://localhost:8080.evil.com`, `null`, a missing header and a different port are false.
  - `client_ip_trusts_proxy_only_from_private_peer`: peer 127.0.0.1 with `X-Forwarded-For: 1.2.3.4, 5.6.7.8` gives 5.6.7.8; peer 8.8.8.8 with the same header gives 8.8.8.8.
  - `limiter_burst_then_refill`: 5 Login checks pass, the 6th is Err with retry-after near 60 s, passes again after `now + 60 s`; a different key is unaffected.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server auth limits`: fails.
- [ ] **Step 3: Implement** the functions as listed.
- [ ] **Step 4: Run** again: passes. `scripts/check.sh fast deny`.
- [ ] **Step 5: Commit** `feat(server): password, name, cookie and origin helpers, rate limiter`.

### Task 3: Config, data dir and the CLI

**Files:**
- Create: `crates/vesper-server/src/cli.rs`
- Modify: `crates/vesper-server/src/config.rs`, `src/main.rs`, `docs/env.md`, `docs/dev.md` (run steps: data dir, `create-owner`)

**Interfaces:**
- Consumes: Task 1 `Store`, Task 2 `new_token`.
- Produces: `Config` gains `data_dir: PathBuf` (`VESPER_DATA_DIR`, default `data`), `origin: String` (`VESPER_ORIGIN`, default `http://localhost:8080`, must parse as `http://` or `https://` with no path), `secure_cookies: bool` (true when origin is https), `upload_quota_bytes: u64` (`VESPER_UPLOAD_QUOTA_MB`, default 10240). `dev_key` and `join_deadline` stay until Task 8 removes them.
- Produces: `pub async fn cli::run(args: &[String], cfg: &Config) -> Result<Option<String>, String>`; `Ok(None)` means "no subcommand, serve". Subcommands:
  - `create-owner` prints `<origin>/invite#<code>` (one use, 24 h, role owner); Err "an owner already exists" otherwise.
  - `invite [--role member|moderator] [--uses N] [--hours H]` prints a link (defaults 1 use, 168 h).
  - `loadtest-accounts --count N` creates N member accounts `load_<i>_<4 hex>` with no password and one session each, prints one token per line. The help text says: throwaway data dir only.
- `main.rs`: parse config, open nothing for serving yet, call `cli::run` first; print its output to stdout and exit 0, or the error to stderr and exit 1.

- [ ] **Step 1: Write failing tests** (config inline, cli in `crates/vesper-server/tests/cli.rs` with a temp data dir):
  - `origin_must_be_scheme_host_port`: `https://vesper.tfaucher.com` ok with `secure_cookies`; `ftp://x`, `https://x/path`, `x` are Err; default is `http://localhost:8080` with `secure_cookies == false`.
  - `create_owner_prints_link_once`: output starts with `http://localhost:8080/invite#` and has a 64-hex fragment; the invite's role is Owner; a second run after an owner signs up is Err. Before anyone signs up, a second run still works (the first link may be lost).
  - `invite_cli_defaults`: one use, role member, expires in 168 h.
  - `loadtest_accounts_tokens_authenticate`: each printed token `hash_token` maps through `session_user` to a distinct member.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server config cli`: fails.
- [ ] **Step 3: Implement.** Hand-parse args like `vesper-loadtest` does; no clap.
- [ ] **Step 4: Run**: passes. Update `docs/env.md` (three new rows) in this commit. `scripts/check.sh fast wording`.
- [ ] **Step 5: Commit** `feat(server): data dir, origin config and owner, invite and loadtest CLI`.

### Task 4: Account REST API

**Files:**
- Create: `crates/vesper-server/src/api_accounts.rs`, `crates/vesper-server/tests/common/mod.rs`, `crates/vesper-server/tests/accounts.rs`
- Modify: `crates/vesper-server/src/lib.rs` (`AppState`, `pub fn router(state: AppState) -> Router`, Origin middleware, 32 KiB `DefaultBodyLimit` on `/api`, `into_make_service_with_connect_info::<SocketAddr>()`), `crates/vesper-protocol/src/api.rs`, `docs/architecture.md` (new "HTTP API" section listing the routes)

**Interfaces:**
- Consumes: Tasks 1 to 3.
- Produces:
  - `#[derive(Clone)] pub struct AppState { pub store: Store, pub cfg: Arc<Config>, pub limits: Limiter, /* hub, voice added in Task 8 */ }`
  - `pub async fn start_with_sfu(cfg, sfu)` now opens the store at `cfg.data_dir.join("vesper.db")`.
  - `pub struct Authed { pub user: UserInfo, pub session_id: i64 }` implementing `FromRequestParts<AppState>`: 401 `ApiError` when the cookie is missing, malformed, unknown, expired or banned.
  - Origin middleware `require_origin`: every `/api` request that is not GET or HEAD needs `origin_ok`, else 403.
  - Protocol: `struct ApiError { code: ApiErrorCode, message: String }`; `enum ApiErrorCode { BadRequest, Unauthorized, Forbidden, NotFound, Conflict, RateLimited, TooLarge, QuotaFull, InviteInvalid }`; `struct SignupReq { invite: String, username: String, password: String, display_name: String }`; `struct LoginReq { username: String, password: String }`; `struct ProfilePatch { display_name: Option<String>, accent: Option<String> }`; `struct PasswordChange { old: String, new: String }`; `struct Me { user: UserInfo, session_id: i64 }`; `struct InvitePreview { role: Role }`.
  - Routes: `GET /api/invites/preview?code=` (Invite limiter; 200 `InvitePreview` or 404 `InviteInvalid`), `POST /api/signup` (Invite limiter by IP; 201 `Me` + Set-Cookie), `POST /api/login` (Login and LoginIp limiters; 200 `Me` + Set-Cookie; any failure is 401 "wrong username or password"), `POST /api/logout` (204, deletes the session, clears the cookie), `GET /api/me`, `PATCH /api/me`, `POST /api/me/password` (verifies old, drops other sessions), `GET /api/sessions` (`Vec<Device>`), `DELETE /api/sessions/{id}`. 429 responses carry `Retry-After`.
  - Test helper `common::TestServer { addr, state, data: TempDir }` with `async fn new() -> TestServer`, `async fn call(&self, req: Request<Body>) -> (StatusCode, HeaderMap, serde_json::Value)`, `async fn owner(&self) -> Session` (CLI invite plus signup), `async fn member(&self, name: &str) -> Session`, and `struct Session { cookie: String, user: UserInfo }`; requests built with `common::post(path, json, &Session)` that set `Origin` to the configured origin.

- [ ] **Step 1: Write failing tests** in `tests/accounts.rs`:
  - `signup_with_invite_sets_strict_cookie`: 201, `Set-Cookie` has `HttpOnly`, `SameSite=Strict`, no `Secure` on http; `GET /api/me` with it returns the username lowercased.
  - `signup_with_used_invite_is_rejected`: second signup on a 1-use invite is 404 `invite_invalid`; a changed character is the same response body.
  - `login_wrong_password_and_unknown_user_look_the_same`: same status and body for both.
  - `login_is_rate_limited`: the 6th failed login for one username from one IP is 429 with `Retry-After`.
  - `login_mints_new_token_and_drops_presented_one`: log in while sending an existing valid cookie; the new cookie differs, the old one now gives 401 (session fixation).
  - `post_without_or_with_wrong_origin_is_403`: login with no `Origin`, and with `http://evil.example`, is 403 and nothing is written.
  - `password_change_drops_other_sessions`: two logins; change password on one; the other gets 401; the changer still works.
  - `revoke_device`: `GET /api/sessions` lists 2 with one `current`; `DELETE` the other; it gets 401; deleting an id of another user is 404.
  - `profile_patch_validates`: bad accent `red` is 400; display name with U+200B is 400; valid patch returns the new `UserInfo`.
  - `body_over_32k_is_413`.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server --test accounts`: fails.
- [ ] **Step 3: Implement** the routes in `api_accounts.rs` and the state, router and middleware in `lib.rs`. `ApiError` implements `IntoResponse`. Handlers never log request bodies.
- [ ] **Step 4: Run**: passes; `tests/static_files.rs` and `tests/signaling.rs` still pass (they build `Config` by hand: add the new fields with a temp data dir). `scripts/check.sh fast bindings`.
- [ ] **Step 5: Commit** `feat(server): signup, login, sessions and profile API`.

### Task 5: Admin and moderation API (accounts side)

**Files:**
- Create: `crates/vesper-server/src/api_admin.rs`, `crates/vesper-server/tests/admin.rs`
- Modify: `crates/vesper-protocol/src/api.rs`, `docs/architecture.md` (routes, the roles table)

**Interfaces:**
- Consumes: `Authed`, `Store` invites and roles.
- Produces:
  - Protocol: `struct InviteCreate { role: Role, max_uses: u32, expires_hours: u32 }` (max_uses 1 to 50, hours 1 to 720, role not Owner); `struct InviteCreated { info: InviteInfo, url: String }`; `struct RolePatch { role: Role }` (not Owner).
  - `fn require(user: &UserInfo, min: Role) -> Result<(), ApiError>` in `api_admin.rs` (Owner > Moderator > Member).
  - Routes: `GET /api/users` (any signed-in user; `Vec<UserInfo>`), `GET /api/invites` and `POST /api/invites` (moderator or owner; moderators only `member` invites), `DELETE /api/invites/{id}` (moderator or owner), `PATCH /api/users/{id}/role`, `POST /api/users/{id}/ban`, `POST /api/users/{id}/unban` (owner only; the owner cannot target themselves).
  - Produces for Task 8: `ban` must later also close sockets; leave a `// Task 8: hub.disconnect_user` call site as a function `fn after_ban(state: &AppState, user_id: &str)` that is empty until Task 8 fills it.

- [ ] **Step 1: Write failing tests** in `tests/admin.rs`:
  - `member_cannot_create_invites`: 403.
  - `moderator_creates_member_invite_not_moderator_invite`: member 201 with a `url` starting with the origin and `/invite#`; moderator role 403.
  - `owner_role_cannot_be_minted_or_assigned`: `InviteCreate { role: Owner }` and `RolePatch { role: Owner }` are 400.
  - `role_change_and_ban_are_owner_only`: moderator gets 403 on both; owner sets a member to moderator; ban makes that user's cookie 401 and login 401; unban lets them log in again.
  - `owner_cannot_ban_self`: 400.
  - `revoked_invite_preview_is_404`.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server --test admin`: fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run**: passes. `scripts/check.sh fast bindings wording`.
- [ ] **Step 5: Commit** `feat(server): invites, roles and bans API`.

### Task 6 (OPTIONAL, STRETCH): Passkeys

> **Run only after Tasks 1 to 17 are merged and `scripts/check.sh` plus the e2e job are green.** It is numbered here because it belongs to accounts. Skipping it leaves password login, which the spec names as the fallback.

**Files:**
- Create: `crates/vesper-store/migrations/0005_passkeys.sql` (`passkeys(id BLOB PRIMARY KEY, account_id REFERENCES accounts ON DELETE CASCADE, public_key BLOB, sign_count INTEGER, label, created_at, last_used_at)`), `crates/vesper-store/src/passkeys.rs`, `crates/vesper-server/src/api_passkeys.rs`, `crates/vesper-server/tests/passkeys.rs`, `web/src/lib/passkey.ts` (+ test)
- Modify: `Cargo.toml` of vesper-server (`webauthn_rp = "0.3.0"`), `docs/env.md` (`VESPER_RP_ID`, default the host of `VESPER_ORIGIN`), `web/src/routes/Login.svelte`, docs/decisions (new 0009 if the crate is kept)

**Interfaces:**
- Routes: `POST /api/passkeys/register/start` and `/finish` (signed in), `POST /api/passkeys/login/start` and `/finish` (Login limiter), `GET /api/passkeys`, `DELETE /api/passkeys/{id}`. Challenges live in memory for 2 minutes keyed by a random id.
- RP id: `vesper.tfaucher.com` in production (spec), the origin host in dev.
- [ ] **Step 0:** `scripts/check.sh deny` with the crate added. If it fails, or the crate needs `unsafe` in our code, stop and write the finding to known-issues instead.
- [ ] **Step 1: Failing tests:** `register_then_login_with_soft_authenticator` (use the crate's test helpers or a fixed recorded attestation; if neither exists, this task stops at Step 0 with a note), `challenge_is_single_use`, `wrong_origin_assertion_is_rejected`, `sign_count_regression_is_rejected`.
- [ ] **Steps 2 to 4:** run, implement, run.
- [ ] **Step 5: Commit** `feat: passkey sign-in (optional)`.

**M1 gate:** `scripts/check.sh` passes. The app still runs on the shared key; nothing user-visible changed yet.

---

# Milestone 2: rooms, voice presence, key removal

### Task 7: Rooms in the store and the rooms API

**Files:**
- Create: `crates/vesper-store/migrations/0002_rooms.sql`, `crates/vesper-store/src/rooms.rs`, `crates/vesper-store/tests/rooms.rs`, `crates/vesper-server/src/api_rooms.rs`, `crates/vesper-server/tests/rooms.rs`
- Modify: `crates/vesper-protocol/src/api.rs`, `docs/architecture.md`

**Interfaces:**
- Protocol: `struct RoomInfo { id: i64, name: String, position: i64, restricted: bool, members: Vec<String> }` (`members` filled only when `restricted`); `struct RoomCreate { name: String }`; `struct RoomPatch { name: Option<String>, restricted: Option<bool> }`; `struct RoomOrder { ids: Vec<i64> }`; `struct RoomMembers { user_ids: Vec<String> }`.
- Store: `rooms(&self) -> Result<Vec<RoomInfo>>` (by position); `create_room(&self, name: &str, now: i64) -> Result<RoomInfo>` (position = max + 1; duplicate name, case-insensitive, is Conflict); `patch_room(&self, id: i64, name: Option<&str>, restricted: Option<bool>) -> Result<RoomInfo>`; `reorder_rooms(&self, ids: &[i64]) -> Result<()>` (must be a permutation of all ids, else Conflict); `set_room_members(&self, id: i64, user_ids: &[String]) -> Result<RoomInfo>`.
- Server: `pub fn can_see(user: &UserInfo, room: &RoomInfo) -> bool` in `api_rooms.rs` (owner and moderator always; others when not restricted or listed). Routes: `GET /api/rooms` (visible only), `POST /api/rooms`, `PATCH /api/rooms/{id}`, `PUT /api/rooms/order`, `PUT /api/rooms/{id}/members` (all owner-only). Every change calls `fn rooms_changed(state: &AppState)`, a no-op until Task 8 broadcasts it.
- Migration: `rooms(id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, position INTEGER NOT NULL, restricted INTEGER NOT NULL DEFAULT 0 CHECK (restricted IN (0,1)), created_at INTEGER NOT NULL) STRICT`; `room_members(room_id, account_id, PRIMARY KEY (room_id, account_id)) STRICT, WITHOUT ROWID` with cascades; `INSERT INTO rooms (name, position, created_at) VALUES ('general', 0, 0)`.

- [ ] **Step 1: Failing tests.** Store: `fresh_db_has_general`, `room_names_unique_ignoring_case`, `reorder_requires_permutation`, `members_only_for_restricted`. Server: `members_do_not_see_restricted_rooms`, `moderator_sees_restricted_rooms`, `room_admin_is_owner_only` (moderator 403 on all four writes), `room_name_rules` (empty, 33 chars and U+202E are 400).
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-store -p vesper-server rooms`: fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run**: passes. `scripts/check.sh fast bindings`.
- [ ] **Step 5: Commit** `feat: rooms with order and member lists`.

### Task 8: Authenticated WebSocket, the hub, removal of the join key

**Files:**
- Create: `crates/vesper-server/src/hub.rs`, `docs/decisions/0007-accounts-and-sessions.md`
- Modify: `crates/vesper-server/src/ws.rs`, `lib.rs`, `config.rs` (remove `dev_key`, `join_deadline`), `rooms.rs` (key check, `user_id(name)` and the name hash removed; see Task 9 for the rename), `api_admin.rs` (`after_ban`), `api_accounts.rs` (logout, revoke and password change close sockets), `api_rooms.rs` (`rooms_changed`), `crates/vesper-protocol/src/lib.rs`, `crates/vesper-server/tests/signaling.rs` (connect with a cookie and an Origin), `docs/decisions/0003-shared-join-key.md` (status "superseded by 0007"), `docs/decisions/README.md`, `docs/env.md` (remove the two variables), `docs/known-issues.md` (delete: kick by name, names not normalised, no Origin check, per-socket join limits, pipelined pre-join messages, unbounded queue), `CLAUDE.md` (the join-key bullet becomes a sessions bullet; the user_id bullet says it is `accounts.id`), `scripts/check-secrets.sh` (swap the `VESPER_DEV_KEY` pattern for `vesper_session=[0-9a-f]{64}`) and `scripts/secrets-allow.txt` (drop the dev-key lines), `scripts/test-check-secrets.sh`.

**Interfaces:**
- Consumes: `Authed` logic (`cookie_token`, `session_user`), `origin_ok`, `can_see`.
- Produces (protocol, `lib.rs`):
  - `ClientMsg::Join { room_id: i64, offer: String }` (the `name` and `key` fields are gone), `Answer`, `SetVoiceState { muted: bool, deafened: bool }` (replaces `SetMuted`), `Leave`, `Check { offer: String }`, `Typing { room_id: i64 }`.
  - `ServerMsg::Hello { me: Me, users: Vec<UserInfo>, rooms: Vec<RoomInfo>, voice: Vec<Peer>, unread: Vec<Unread> }` (first message on every socket; `unread` empty until Task 13), `Joined { user_id, answer, peers }`, `Offer`, `PeerJoined { peer }`, `PeerLeft { user_id }`, `PeerState { user_id, muted, deafened, force_muted }` (replaces `PeerMuted`), `RoomsChanged { rooms }`, `UserUpdated { user }`, `ForceMuted { muted }`, `Error { code, message }`. `Peer` becomes `{ user_id, name, room_id: i64, muted, deafened, force_muted }`. `ErrorCode` loses `BadKey`, gains `Forbidden`, `RateLimited`.
  - `pub const CLOSE_SESSION_ENDED: u16 = 4001; pub const CLOSE_DISCONNECTED: u16 = 4003;` in vesper-protocol.
- Produces (`hub.rs`):
  - `pub struct Hub` (`Clone`); `pub fn add(&self, user: UserInfo, session_id: i64, tx: mpsc::Sender<Out>) -> ConnId`; `pub fn remove(&self, id: ConnId)`; `pub fn send_user(&self, user_id: &str, msg: ServerMsg)`; `pub fn broadcast(&self, msg: ServerMsg)`; `pub fn broadcast_room(&self, room: &RoomInfo, msg: ServerMsg)` (only sockets whose user `can_see` it); `pub fn close_session(&self, session_id: i64, code: u16)`; `pub fn close_user(&self, user_id: &str, code: u16)`; `pub fn update_user(&self, user: UserInfo)` (role or ban change, also broadcasts `UserUpdated`).
  - `Out` gains `Close(u16)`; queues are `mpsc::channel(256)` and every send is `try_send`; a full queue closes that socket.
- `ws::upgrade`: 403 unless `origin_ok`; 401 unless the cookie maps to a live session; 503 on the socket cap as today; then `Hello`, then the loop. `rooms_changed` broadcasts `RoomsChanged` per user with only the rooms they can see. `after_ban`, logout, device revoke and password change call `close_user` or `close_session` with 4001.

- [ ] **Step 1: Failing tests** in `tests/signaling.rs` (rewrite `Conn::open` to take a `common::Session`):
  - `upgrade_without_cookie_is_401`, `wrong_origin_upgrade_is_403`.
  - `hello_is_first_message`: has `me.user.user_id` equal to the account id, `rooms` contains `general`.
  - `revoked_session_closes_socket`: delete the device via REST; the socket gets close code 4001 within 1 s.
  - `ban_closes_socket_and_voice`: the banned user is in voice; after ban, close 4001 and the other socket sees `PeerLeft`.
  - `restricted_room_changes_reach_only_members`: a non-member socket gets `RoomsChanged` without the restricted room.
  - `full_queue_closes_only_that_socket`: a socket that never reads gets closed after 256 queued messages; another socket keeps working.
  - Keep and adapt the existing tests: replace-on-rejoin (now the same account from a second tab), stale session offers, room full at 12, bad JSON, oversized message, SFU death.
  - Delete the tests for the key and the join deadline.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server --test signaling`: fails.
- [ ] **Step 3: Implement.** Write ADR 0007 (accounts with invites, argon2id, hashed session cookies, Origin check, why REST plus WebSocket, what was removed from 0003) and update the docs listed above in the same commit. `grep -rn "VESPER_DEV_KEY\|dev_key\|shortcut: one shared" crates docs deploy web/src` returns only ADR 0003 history.
- [ ] **Step 4: Run** `scripts/check.sh fast bindings secrets wording scripts`: passes. The web build breaks on the generated types here; Task 10 fixes it, so run `scripts/check.sh web` only after Task 10.
- [ ] **Step 5: Commit** `feat!: session auth on the WebSocket, remove the shared join key`.

### Task 9: Voice presence by room for everyone, moderator voice actions

**Files:**
- Modify: `git mv crates/vesper-server/src/rooms.rs crates/vesper-server/src/voice.rs`; `ws.rs`, `lib.rs`, `api_admin.rs` (three routes), `crates/vesper-server/tests/signaling.rs`, `docs/architecture.md` (message flow and identity sections rewritten), `docs/known-issues.md` (force-mute is client-enforced; one join lock remains)

**Interfaces:**
- Consumes: `Hub::broadcast_room`, `can_see`, room list from the store (cached in `Voice` and refreshed by `rooms_changed`).
- Produces (`voice.rs`):
  - `pub struct Voice` (was `Rooms`); `pub fn new(sfu: SfuHandle, hub: Hub, max_room_size: usize) -> Voice`
  - `pub async fn join(&self, user: &UserInfo, room: &RoomInfo, offer: String, tx: mpsc::Sender<Out>) -> Result<JoinResult, ErrorCode>` (Forbidden if `!can_see`; RoomFull at 12; replaces the user's earlier voice session from any socket; SFU room name `room-<id>`)
  - `pub fn set_state(&self, user_id: &str, session: u64, muted: bool, deafened: bool)` (a force-muted user stays `muted = true`)
  - `pub fn peers(&self, visible: impl Fn(i64) -> bool) -> Vec<Peer>` for `Hello`
  - `pub fn kick(&self, user_id: &str) -> bool`; `pub fn force_mute(&self, user_id: &str, muted: bool) -> bool`
  - `pub async fn check(&self, offer: String, tx: mpsc::Sender<Out>) -> Result<JoinResult, ErrorCode>` (SFU user id `random_hex(16)`, room `check-<that id>`, no presence)
  - All presence messages go through `hub.broadcast_room` instead of the old per-room member broadcast.
- Routes (moderator or owner): `POST /api/users/{id}/kick` (leave voice), `POST /api/users/{id}/force-mute` with `struct ForceMute { muted: bool }` (sets `force_muted`, sends `ForceMuted` to that user, broadcasts `PeerState`), `POST /api/users/{id}/disconnect` (`close_user` with 4003). Moderators cannot act on the owner (403).

- [ ] **Step 1: Failing tests** in `tests/signaling.rs`:
  - `presence_reaches_sockets_outside_the_room`: carol is connected but not in voice; alice joins `general`; carol gets `PeerJoined` with `room_id` of general.
  - `restricted_room_presence_is_private`: a non-member never sees `PeerJoined` for a restricted room, and `Join` to it is `Forbidden`.
  - `voice_state_broadcasts_deafened`.
  - `second_tab_join_replaces_first_voice_session`: the first socket stays open (chat) but gets `PeerLeft` for itself; presence shows one entry.
  - `force_mute_holds_until_lifted`: after force-mute, `SetVoiceState { muted: false }` keeps `muted: true` in presence.
  - `moderator_cannot_kick_owner`: 403.
  - `check_does_not_touch_presence_or_own_call`: a user in voice runs `Check` from a second socket; the call stays, nobody sees a peer event, `Joined` has no peers.
  - `twelfth_joins_thirteenth_is_full` (adapted).
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server --test signaling`: fails.
- [ ] **Step 3: Implement.** Keep the join lock and the session-number rules from `rooms.rs` as they are; only identity and broadcast change.
- [ ] **Step 4: Run**: passes. `scripts/check.sh fast bindings wording`.
- [ ] **Step 5: Commit** `feat(server): voice presence for everyone, kick, force-mute, disconnect`.

### Task 10: Minimal web plumbing: login, signup, session store

**Files:**
- Create: `web/src/lib/api.ts` (+ `api.test.ts`), `web/src/lib/state.ts` (+ `state.test.ts`), `web/src/lib/session.svelte.ts`, `web/src/App.svelte`, `web/src/routes/Login.svelte`, `web/src/routes/Signup.svelte`
- Modify: `web/src/main.ts` (mount `App`), `web/src/lib/signaling.ts` (+ test), `web/src/lib/peers.ts` (+ test), `web/src/lib/diag.ts` (+ test), `web/src/routes/Room.svelte`, `web/src/routes/Diagnostics.svelte`, `web/src/lib/join.test.ts`, `web/vite.config.ts` (proxy `/api` too), `docs/dev.md` (`VESPER_ORIGIN=http://localhost:5173` for Vite)

**Interfaces:**
- Consumes: generated `ClientMsg`, `ServerMsg`, `Me`, `RoomInfo`, `UserInfo`, `Peer`, `ApiError` types.
- Produces:
  - `api.ts`: `export class ApiFailure extends Error { status: number; code: ApiErrorCode }`; `export async function api<T>(method: 'GET' | 'POST' | 'PATCH' | 'PUT' | 'DELETE', path: string, body?: unknown): Promise<T>` (JSON, `credentials: 'same-origin'`, 204 resolves `undefined`).
  - `state.ts`: `export type AppState = { me: Me | null; users: Record<string, UserInfo>; rooms: RoomInfo[]; voice: Peer[] }`; `export function applyServerMsg(s: AppState, m: ServerMsg): AppState` (pure; handles `hello`, `rooms_changed`, `user_updated`, and voice messages via `applyPeerMsg`).
  - `peers.ts`: `applyPeerMsg` handles `peer_joined`, `peer_left`, `peer_state`; `export function inRoom(peers: Peer[], roomId: number): Peer[]`.
  - `session.svelte.ts`: `export const session: { state: AppState; sig: Signaling | null; connect(): void; logout(): Promise<void> }`; `connect` opens one `Signaling` per page after login and reconnects with backoff 1 s, 2 s, 5 s, 10 s unless the close code is 4001 or 4003 (4001 sends the user to `/login`).
  - `signaling.ts`: `onMessage` and `onClose` return an unsubscribe function; `leave()` sends `leave` without closing the socket; `close(): void` unchanged; `joinRoom(sig, { roomId, stream, bitrateKbps, dtx?, check?, ... })` sends `join { room_id, offer }`, or `check { offer }` when `check` is true. `name` and `key` options are gone.
  - `App.svelte`: path router on `location.pathname` plus `history.pushState`: `/login`, `/invite` (reads the code from `location.hash`), `/` (needs a session: `GET /api/me`, else `/login`).
  - `Room.svelte`: the Name and Key fields go; a `<select aria-label="Room">` lists `session.state.rooms`; the Join button joins the chosen room on the shared socket; the people list shows `inRoom(voice, roomId)`; a Log out button; deafen sends `set_voice_state`. A `force_muted` message mutes and disables the mute button.
  - `Diagnostics.svelte` and `diag.ts`: the connection test uses `check: true` on a new `Signaling`; the key prop is gone.
  - Copy (exact strings): Login heading "Sign in", fields "Username", "Password", button "Sign in", error "Wrong username or password." Signup heading "Create your account", fields "Username", "Display name", "Password", button "Create account", bad link "This invite link is not valid. Ask for a new one."

- [ ] **Step 1: Failing vitest tests:**
  - `api.test.ts` `api_throws_ApiFailure_with_code`: a stubbed `fetch` returning 429 `{code:'rate_limited'}` rejects with `status 429`, `code 'rate_limited'`.
  - `state.test.ts` `hello_replaces_state`, `rooms_changed_keeps_voice`, `user_updated_merges`.
  - `peers.test.ts` `peer_state_updates_muted_and_deafened`, `inRoom_filters_by_room_id`.
  - `signaling.test.ts` `join_sends_room_id_and_no_key`, `check_sends_check_message`, `unsubscribe_stops_callbacks`, `leave_keeps_socket_open`.
  - `diag.test.ts` adapted: no key argument.
- [ ] **Step 2: Run** `npm --prefix web test`: fails.
- [ ] **Step 3: Implement.** Ugly is fine: plain Tailwind classes already in use, no new components.
- [ ] **Step 4: Run** `scripts/check.sh web wording`: passes (svelte-check, vitest, build, size budget).
- [ ] **Step 5: Commit** `feat(web): sign in, invite signup and a session store for the voice screen`.

### Task 11: e2e, load test, deploy and migration docs

**Files:**
- Create: `web/e2e/lib.mjs` (server start with a temp data dir, `cli(args)`, `browser(name, args)`, `signup(page, link, name)`, `sleep`)
- Modify: `web/e2e/two-browser.mjs`, `crates/vesper-loadtest/src/main.rs`, `docs/testing/latency.md`, `deploy/compose.yaml`, `deploy/vesper.service`, `deploy/Dockerfile`, `docs/deploy.md`, `docs/runbook.md`, `docs/env.md` (deploy rows), `docs/known-issues.md`, `.github/workflows/ci.yml` (only if the e2e job needs a change; see Step 3)

**Interfaces:**
- e2e flow: spawn the server with `VESPER_DATA_DIR=<mkdtemp>`, `VESPER_ORIGIN=http://127.0.0.1:18090`; `cli(['create-owner'])` gives alice's link; alice signs up in the browser; alice's page runs `fetch('/api/invites', { method: 'POST', ... })` to get bob's link (Origin is sent by the browser); bob signs up; both pick `general` and join; the existing audio and speaking-ring checks stay as they are; then add `presence_seen_by_both` (each page lists the other in the people list).
- Load test: flags become `--url ws://HOST:PORT/ws --origin http://HOST:PORT --tokens FILE --room-id N --speakers N --seconds S [--pid PID]`; each speaker sends `Cookie: vesper_session=<token>` and `Origin` on the upgrade (tokio-tungstenite `IntoClientRequest` with headers); `Join { room_id, offer }`. The one-room cap of 12 still applies, so `--speakers` above 12 fails as before.
- Deploy: compose adds `volumes: ["./data:/data"]`, `VESPER_DATA_DIR: /data`, `VESPER_ORIGIN: ${VESPER_ORIGIN:?set it in .env}`, drops `VESPER_DEV_KEY`; the Dockerfile creates `/data` owned by uid 10001; `vesper.service` adds `StateDirectory=vesper`, `Environment=VESPER_DATA_DIR=/var/lib/vesper` and drops the key from the env file note.

- [ ] **Step 1: Update the e2e test first** and run `npm --prefix web run e2e` against the Task 10 build: it must pass. Record the output line `e2e two-browser: PASS`.
- [ ] **Step 2: Update the load test** and run `docs/testing/latency.md` section 1 locally with 12 speakers: `vesper-server loadtest-accounts --count 12 > <tmp>/tokens` on a throwaway data dir, then the load test. Expected: exit 0. Update the doc commands.
- [ ] **Step 3: CI.** The e2e job builds the release server and runs `npm run e2e`; the test now owns its data dir, so the workflow needs no new variable. Change `ci.yml` only if the run shows otherwise.
- [ ] **Step 4: Migration notes** in `docs/deploy.md` "Upgrade from the shared key" (and the same steps in `docs/runbook.md`):
  1. Tag the running image (`docker tag vesper:latest vesper:previous`).
  2. On the host, create the data dir next to `compose.yaml` and give it to uid 10001 (`mkdir data && sudo chown 10001:10001 data`).
  3. In `.env`: remove the old key line, add `VESPER_ORIGIN=https://vesper.tfaucher.com`.
  4. `docker compose up -d --build`, check `healthy`.
  5. `docker compose exec vesper vesper-server create-owner`; open the printed link on your own device and create the owner account.
  6. Make invites with `docker compose exec vesper vesper-server invite` (or the API) and send each friend their link.
  7. Backups: the Proxmox snapshot now has to include the `data` directory. Rollback to the shared-key image still works; the data dir is ignored by it.
  Also: runbook "Where things are" gains the data dir and the CLI; deploy.md warns that the HTTP port must only be reachable through Caddy (X-Forwarded-For ruling). Mark all of it "not verified" (no Docker on the dev machine).
- [ ] **Step 5: Run** `scripts/check.sh`: passes. **Commit** `test: e2e and load test sign in with accounts; deploy notes for the data dir`.

**M2 gate:** `scripts/check.sh` and `npm --prefix web run e2e` pass. The voice screen works behind login with stored rooms.

---

# Milestone 3: text chat

### Task 12: Messages in the store

**Files:**
- Create: `crates/vesper-store/migrations/0003_messages.sql`, `crates/vesper-store/src/messages.rs`, `crates/vesper-store/tests/messages.rs`, `crates/vesper-store/tests/perf.rs`
- Modify: `crates/vesper-protocol/src/api.rs`

**Interfaces:**
- Protocol: `struct Message { id: i64, room_id: i64, author_id: String, body: String, created_at: i64, edited_at: Option<i64>, deleted: bool, reply_to: Option<ReplyPreview>, reactions: Vec<Reaction>, attachments: Vec<Attachment>, mentions: Vec<String>, pinned: bool }`; `struct ReplyPreview { id: i64, author_id: String, excerpt: String }` (first 100 chars, empty when deleted); `struct Reaction { emoji: String, user_ids: Vec<String> }`; `struct Attachment { id: String, name: String, mime: String, size: i64, image: bool }`; `struct MessagePage { messages: Vec<Message>, more_before: bool, more_after: bool }`; `struct Unread { room_id: i64, last_read_id: i64, unread: u32, mentions: u32 }`.
- Store:
  - `pub struct NewMessage<'a> { pub room_id: i64, pub author_id: &'a str, pub body: &'a str, pub reply_to: Option<i64>, pub mentions: &'a [String], pub attachments: &'a [String], pub now: i64 }`
  - `post_message(&self, m: NewMessage<'_>) -> Result<Message>` (reply_to must be in the same room, else Conflict; attachments must be the author's, unattached, same room)
  - `edit_message(&self, id: i64, author_id: &str, body: &str, mentions: &[String], now: i64) -> Result<Message>` (NotFound when not the author or deleted)
  - `delete_message(&self, id: i64, now: i64, by: &str) -> Result<(Message, Vec<String>)>` (soft; returns removed upload ids so the caller deletes files)
  - `set_reaction(&self, id: i64, user_id: &str, emoji: &str, on: bool, now: i64) -> Result<Message>` (Conflict past 20 distinct)
  - `set_pinned(&self, id: i64, by: &str, on: bool, now: i64) -> Result<Message>` (Conflict past 50 in the room)
  - `message(&self, id: i64) -> Result<Message>`
  - `pub enum Cursor { Latest, Before(i64), After(i64), Around(i64) }`; `history(&self, room_id: i64, cursor: Cursor, limit: u32) -> Result<MessagePage>` (limit clamped 1 to 100; `Around` gives limit/2 each side plus the target)
  - `pins(&self, room_id: i64) -> Result<Vec<Message>>`
  - `mark_read(&self, user_id: &str, room_id: i64, last_read_id: i64) -> Result<()>` (never moves backwards)
  - `unread(&self, user_id: &str, room_ids: &[i64]) -> Result<Vec<Unread>>`
  - `account_ids_by_username(&self, names: &[String]) -> Result<Vec<String>>` (for mentions)
- Migration: tables and indexes as listed; FTS kept by triggers:

```sql
CREATE VIRTUAL TABLE messages_fts USING fts5(body, content='messages', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
  INSERT INTO messages_fts(rowid, body) VALUES (new.id, new.body); END;
CREATE TRIGGER messages_au AFTER UPDATE OF body, deleted_at ON messages BEGIN
  INSERT INTO messages_fts(messages_fts, rowid, body) VALUES ('delete', old.id, old.body);
  INSERT INTO messages_fts(rowid, body) SELECT new.id, new.body WHERE new.deleted_at IS NULL; END;
```

  Tables: `messages(id INTEGER PRIMARY KEY, room_id REFERENCES rooms, author_id REFERENCES accounts, body TEXT NOT NULL, reply_to INTEGER REFERENCES messages(id), created_at, edited_at, deleted_at, deleted_by, pinned_at, pinned_by) STRICT`; indexes `(room_id, id)`, `(author_id, id)`, partial `(room_id) WHERE pinned_at IS NOT NULL`; `reactions(message_id, account_id, emoji, created_at, PRIMARY KEY (message_id, emoji, account_id)) WITHOUT ROWID`; `mentions(account_id, message_id, PRIMARY KEY (account_id, message_id)) WITHOUT ROWID`; `reads(account_id, room_id, last_read_id, PRIMARY KEY (account_id, room_id)) WITHOUT ROWID`. Messages are never hard-deleted, so no delete trigger. A page's reactions, mentions and attachments load in one query each with `WHERE message_id IN (SELECT value FROM json_each(?1))`.

- [ ] **Step 1: Failing tests** in `tests/messages.rs`:
  - `history_pages_backwards_without_gaps`: 250 messages; `Latest` limit 100 then `Before(oldest)` twice yields all 250 ids once each, ascending within a page, `more_before` false at the end.
  - `around_centres_on_target`.
  - `edit_only_by_author_and_not_after_delete`.
  - `delete_is_soft_and_removes_from_fts`: body becomes empty, `deleted` true, a reply to it keeps `reply_to.id` with an empty excerpt, `SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'word'` is 0.
  - `edit_updates_fts`: old word no longer matches, new one does.
  - `reply_must_be_same_room`.
  - `reactions_toggle_and_cap`: on/on is idempotent; the 21st distinct emoji is Conflict.
  - `pins_cap_at_50`.
  - `unread_counts_and_mentions`: 5 messages, mark read at the 2nd, unread 3; one mentions the user, mentions 1; `mark_read` backwards is ignored.
- [ ] **Step 2: Write** `tests/perf.rs` `perf_100k_messages` with `#[ignore]`: insert 100 000 messages (10 rooms, 10 authors, 10 to 300 chars, one transaction), then time `history(Latest, 50)`, `history(Before(mid), 50)`, `unread` for 10 rooms and a 2-term search (Task 15 adds that call; until then skip it). Print the timings and assert each is under 50 ms. Run by hand: `scripts/capped.sh cargo test --release -p vesper-store --test perf -- --ignored --nocapture`.
- [ ] **Step 3: Run** `scripts/capped.sh cargo nextest run -p vesper-store messages`: fails. **Implement.** Run again: passes.
- [ ] **Step 4:** `scripts/check.sh fast bindings`.
- [ ] **Step 5: Commit** `feat(store): messages, reactions, pins, mentions and read markers`.

### Task 13: Chat API and live events

**Files:**
- Create: `crates/vesper-server/src/api_messages.rs`, `crates/vesper-server/tests/chat.rs`
- Modify: `crates/vesper-protocol/src/lib.rs` and `api.rs`, `ws.rs` (typing), `lib.rs` (`Hello.unread`), `docs/architecture.md`

**Interfaces:**
- Protocol: `struct SendMessage { body: String, reply_to: Option<i64>, attachments: Vec<String> }`; `struct EditMessage { body: String }`; `struct ReactionReq { emoji: String, on: bool }`; `struct PinReq { on: bool }`; `struct ReadReq { last_read_id: i64 }`. `ServerMsg` gains `MessageNew { message }`, `MessageUpdated { message }`, `Typing { room_id, user_id }`, `Read { room_id, last_read_id }` (to the same user's other sockets).
- Server: `pub fn valid_body(s: &str) -> Option<String>` (trim, 1 to 4000 chars, no control characters except `\n` and `\t`, NFC); `pub fn mentioned_usernames(body: &str) -> Vec<String>` (`@` then `[a-z0-9_]{3,24}` after lowercasing, at a word start, deduplicated, at most 20); `pub fn valid_reaction(s: &str) -> bool`.
- Routes (every one checks `can_see`; a room the user cannot see is 404, not 403):
  - `GET /api/rooms/{id}/messages?before=|after=|around=&limit=` returns `MessagePage`
  - `GET /api/rooms/{id}/pins`
  - `POST /api/rooms/{id}/messages` (Message limiter) returns 201 `Message`, broadcasts `MessageNew` to the room
  - `PATCH /api/messages/{id}` (author), `DELETE /api/messages/{id}` (author, or moderator and owner), `PUT /api/messages/{id}/reaction`, `PUT /api/messages/{id}/pin`: each broadcasts `MessageUpdated`
  - `PUT /api/rooms/{id}/read`
- WS: `ClientMsg::Typing { room_id }` through the Typing limiter (excess dropped without an error), broadcast to the room except the sender. Typing is never stored.

- [ ] **Step 1: Failing tests** in `tests/chat.rs`:
  - `send_and_receive_live`: bob's socket gets `MessageNew` with alice's message; `GET` history returns it.
  - `send_refuses_oversize_and_control_chars`: 4001 chars is 400, 4000 is 201; `"a\u{0000}b"` is 400; a 40 KiB body is 413; nothing is stored for the refusals.
  - `message_rate_limit`: the 11th message in one second is 429.
  - `moderator_deletes_any_member_only_own`: member deleting another's message is 403; moderator 204; both sockets get `MessageUpdated` with `deleted: true`.
  - `restricted_room_is_404_and_silent`: a non-member gets 404 on history and send, and never receives that room's `MessageNew` or `Typing`.
  - `mentions_count_in_unread`: alice writes `hi @bob`; bob's next `Hello` has `mentions: 1` for that room; `PUT read` makes it 0 and bob's other socket gets `Read`.
  - `typing_is_throttled_and_ephemeral`: three typing messages in one second reach carol once.
  - `reaction_validation`: `"ab"` and `" "` are 400; `"👍"` is ok.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server --test chat`: fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run**: passes. `scripts/check.sh fast bindings wording`.
- [ ] **Step 5: Commit** `feat(server): chat API with live events, typing and unread`.

### Task 14: Uploads, quota and disk use

**Files:**
- Create: `crates/vesper-store/migrations/0004_uploads.sql`, `crates/vesper-store/src/uploads.rs`, `crates/vesper-server/src/uploads.rs`, `crates/vesper-server/tests/uploads.rs`
- Modify: `crates/vesper-server/Cargo.toml` (`rustix` with `fs`), `lib.rs` (routes, hourly sweep task, clean `uploads/tmp` at start), `api_messages.rs` (delete removes files), `crates/vesper-protocol/src/api.rs`, `docs/architecture.md`, `docs/env.md` (quota row already there, add the layout note)

**Interfaces:**
- Migration: `uploads(id TEXT PRIMARY KEY CHECK (length(id) = 32), room_id REFERENCES rooms, uploader_id REFERENCES accounts, message_id INTEGER REFERENCES messages(id), name TEXT NOT NULL, mime TEXT NOT NULL, size INTEGER NOT NULL, created_at INTEGER NOT NULL) STRICT`, index on `message_id`.
- Store: `insert_upload(&self, u: &UploadRow) -> Result<()>`; `upload(&self, id: &str) -> Result<UploadRow>`; `upload_bytes(&self) -> Result<i64>`; `stale_uploads(&self, before: i64) -> Result<Vec<String>>` and `delete_uploads(&self, ids: &[String]) -> Result<()>`; `pub struct UploadRow { pub id: String, pub room_id: i64, pub uploader_id: String, pub message_id: Option<i64>, pub name: String, pub mime: String, pub size: i64, pub created_at: i64 }`.
- Protocol: `struct UploadDone { attachment: Attachment }`; `struct DiskUse { db_bytes: i64, upload_bytes: i64, quota_bytes: i64, fs_total: i64, fs_free: i64, warn: bool }` (`warn` when uploads pass 80% of quota or the filesystem is 80% full).
- Server (`uploads.rs`):
  - `pub const MAX_UPLOAD: u64 = 25 * 1024 * 1024;`
  - `pub fn upload_path(data_dir: &Path, id: &str) -> Option<PathBuf>` (None unless 32 lowercase hex; `uploads/<first 2>/<id>`)
  - `pub fn sniff_image(head: &[u8]) -> Option<&'static str>` (PNG `89 50 4E 47 0D 0A 1A 0A`, JPEG `FF D8 FF`, GIF `GIF87a`/`GIF89a`, WebP `RIFF....WEBP`)
  - `pub fn clean_name(s: &str) -> String` (control chars dropped, `/` and `\` become `_`, trimmed to 200 chars, empty becomes `file`)
  - `pub fn content_disposition(name: &str, inline: bool) -> HeaderValue` (`inline` or `attachment`, an ASCII fallback `filename="..."` with `"` and `\` replaced, plus `filename*=UTF-8''<percent-encoded>`)
  - Routes: `POST /api/rooms/{id}/uploads?name=` (Upload limiter; raw body; Content-Length above `MAX_UPLOAD` is 413 before reading; streams to `uploads/tmp/<id>` counting bytes and aborts past the cap; a global `tokio::sync::Mutex` around the quota check and the rename; quota full is 507 `quota_full`) returns 201 `UploadDone`; `GET /api/uploads/{id}` (signed in and `can_see` the upload's room, else 404); `GET /api/admin/disk` (owner) returns `DiskUse`.
  - Serving headers: `Content-Type` is the sniffed image type when `image`, else `application/octet-stream`; `Content-Disposition` inline for images, attachment otherwise; `X-Content-Type-Options: nosniff`; `Content-Security-Policy: default-src 'none'; sandbox`; `Cross-Origin-Resource-Policy: same-origin`; `Cache-Control: private, max-age=31536000, immutable`. Range requests through `tower_http::services::ServeFile`.
  - Sweep: every hour, uploads with no message older than 24 h lose their row and file.

- [ ] **Step 1: Failing tests** (unit tests inline for the pure functions; HTTP in `tests/uploads.rs`):
  - `upload_path_rejects_traversal`: `../../vesper.db`, `%2e%2e`, 31 or 33 chars, uppercase hex, `/etc/passwd` all None; a valid id gives a path under `data/uploads/`.
  - `upload_id_traversal_is_404`: `GET /api/uploads/..%2F..%2Fvesper.db` and `/api/uploads/AAAA...` are 404.
  - `content_disposition_escapes_name`: `a"b\r\nSet-Cookie: x.txt` yields a valid `HeaderValue` with no CR or LF and the `filename*` holds `%22` and `%0D%0A`.
  - `png_is_inline_image`: a real 1x1 PNG round trips with `image/png` and `inline`.
  - `html_disguised_as_png_is_attachment`: body `<html><script>` with `Content-Type: image/png` and name `cat.png` serves `application/octet-stream`, `attachment`, `image: false`.
  - `svg_is_attachment`.
  - `oversize_body_is_413_and_cleaned`: `MAX_UPLOAD + 1` bytes, sent chunked with no Content-Length, is 413 and `uploads/tmp` is empty afterwards.
  - `quota_full_is_507`: quota set to 1 MiB in the test config; the second 600 KiB upload is 507.
  - `other_room_upload_is_404`: a non-member of a restricted room gets 404 for its upload.
  - `deleting_message_removes_file`.
  - `stale_unattached_upload_is_swept`: call the sweep with `now + 25 h`; row and file gone.
  - `disk_use_is_owner_only`: moderator 403; owner sees `upload_bytes` equal to the sum.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-server uploads`: fails.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run**: passes. `scripts/check.sh fast bindings deny`.
- [ ] **Step 5: Commit** `feat(server): uploads with quota, safe serving and disk use`.

### Task 15: Search

**Files:**
- Create: `crates/vesper-store/src/search.rs`, `crates/vesper-store/tests/search.rs`
- Modify: `api_messages.rs` (route), `crates/vesper-protocol/src/api.rs`, `crates/vesper-store/tests/perf.rs` (enable the search timing), `docs/architecture.md`

**Interfaces:**
- Protocol: `struct SearchQuery { q: String, room_id: Option<i64>, author_id: Option<String>, from: Option<i64>, to: Option<i64>, before: Option<i64> }` (query string); `struct SearchHit { message: Message, snippet: String }`; `struct SearchPage { hits: Vec<SearchHit>, next_before: Option<i64> }`.
- Store: `pub fn fts_query(input: &str) -> Option<String>`; `search(&self, q: &SearchQuery, visible_rooms: &[i64], limit: u32) -> Result<SearchPage>` (newest first, limit 1 to 50; `snippet(messages_fts, 0, char(2), char(3), '...', 12)`; rooms restricted with `m.room_id IN (SELECT value FROM json_each(?))`; deleted messages never match because the trigger removed them).
- `fts_query` rule: split on Unicode whitespace; keep at most 16 terms of at most 64 chars; drop terms with no letter or digit; wrap each in double quotes with inner `"` doubled; add `*` after the last term's closing quote; join with spaces; None when no term is left.
- Route: `GET /api/search?...` returns `SearchPage`. Context: the client calls history with `around=<hit id>`.

- [ ] **Step 1: Failing tests** in `tests/search.rs`:
  - `fts_query_quotes_operators`: `fts_query("cat OR dog")` is `"cat" "OR" "dog"*`; `fts_query("a\"b")` is `"a""b"*`; `fts_query("  ")` and `fts_query("***")` are None; 20 terms give 16.
  - `search_finds_prefix_and_diacritics`: `cafe` finds `Café au lait`; `lai` finds it as a prefix.
  - `search_filters_room_author_date`: each filter alone and combined narrows the hit set as expected.
  - `search_skips_invisible_rooms_and_deleted`.
  - `search_pages_with_next_before`.
  - `hostile_query_never_errors`: `NEAR(`, `"`, `col:x`, `^`, a 10 KiB string all return Ok.
- [ ] **Step 2: Run** `scripts/capped.sh cargo nextest run -p vesper-store search`: fails. **Implement.** Run: passes.
- [ ] **Step 3: Run the perf test** by hand (Task 12 Step 2 command) and write the measured numbers, with machine and date, into `docs/architecture.md` "Performance" (see the note at the end of this plan). If any number is over 50 ms, stop and report.
- [ ] **Step 4:** `scripts/check.sh fast bindings`.
- [ ] **Step 5: Commit** `feat: full-text search with room, author and date filters`.

### Task 16: Minimal chat web plumbing and safe Markdown

**Files:**
- Create: `web/src/lib/markdown.ts` (+ `markdown.test.ts`), `web/src/routes/Markdown.svelte`, `web/src/lib/chat.svelte.ts` (+ `chat.test.ts` for its pure reducer), `web/src/routes/Chat.svelte`, `web/src/lib/no-raw-html.test.ts`
- Modify: `web/src/lib/state.ts` (unread), `web/src/routes/Room.svelte` (chat panel beside the voice controls, unread count in the room select), `web/e2e/two-browser.mjs`, `web/e2e/fixtures/` (a 1x1 PNG made in the test, nothing committed)

**Interfaces:**
- `markdown.ts`: `export type Md = { t: 'text'; v: string } | { t: 'br' } | { t: 'b' | 'i' | 's' | 'quote'; c: Md[] } | { t: 'code'; v: string } | { t: 'pre'; v: string } | { t: 'link'; href: string } | { t: 'mention'; username: string }`; `export function parse(src: string): Md[]`.
  - Supported: `**bold**`, `*italic*` and `_italic_`, `~~strike~~`, `` `code` ``, fenced ``` blocks, lines starting `> ` (one level; deeper `>` stays text), bare `http://` and `https://` URLs (trailing `.,;:!?)` excluded), `@username`, newlines. Anything else is text. Unclosed markers are text. Nesting depth at most 4.
  - Links: `href` kept only if `new URL(s)` succeeds and the protocol is `http:` or `https:`.
  - Must be linear: one left-to-right pass with a delimiter stack, no backtracking regex over the whole input.
- `Markdown.svelte`: renders `Md[]` recursively with `{v}` text interpolation only; links as `<a href={href} rel="noopener noreferrer nofollow" target="_blank">{href}</a>`.
- `chat.svelte.ts`: `export function applyChatMsg(s: ChatState, m: ServerMsg): ChatState` (pure: `message_new`, `message_updated`, `typing`, `read`); `loadOlder(roomId)`, `send(roomId, body, replyTo?, attachments?)`, `upload(roomId, file): Promise<Attachment>`, `search(q: SearchQuery)`.
- `Chat.svelte` (rough on purpose): message list with author display name, `Markdown`, edit and delete for own messages (delete for moderators too), reply, three fixed reaction buttons (👍 😂 ❤️), pin toggle, "load older" button, "X is typing" line for 5 s, file input that uploads then sends, `<img>` for `image` attachments and a download link for others, a search box listing snippets (marker chars rendered as `<mark>` via the same token approach) with a jump that loads `around`.
- Copy: placeholder "Message #<room>", button "Send", "Load older messages", "<name> is typing".

- [ ] **Step 1: Failing vitest tests:**
  - `markdown_is_inert`: `parse('<script>alert(1)</script>')` is one text node with that exact string; `parse('javascript:alert(1)')` has no `link`; `parse('https://x.test/a?b=<c>')` gives a link whose `href` is the URL-parsed form.
  - `markdown_basic`: bold, italic, strike, code, a fenced block, a quote, a mention and an autolink each give the expected tree.
  - `markdown_unclosed_is_text`: `**a` is text `**a`.
  - `markdown_is_linear`: `parse('*'.repeat(10000))` and `parse('**a'.repeat(3000))` each finish in under 50 ms.
  - `no_raw_html.test.ts`: reads every `web/src/**/*.svelte` and `*.ts` (not tests, not `gen/`) and fails on `{@html`, `innerHTML`, `outerHTML`, `insertAdjacentHTML` or `document.write`.
  - `chat.test.ts`: `message_updated_replaces_in_place`, `typing_expires_after_5s` (fake timers).
- [ ] **Step 2: Run** `npm --prefix web test`: fails. **Implement.** Run: passes.
- [ ] **Step 3: Extend the e2e test** (same server and browsers as the voice part, after the voice checks): `chat_roundtrip`: alice sends `hello **bob** <b>x</b>`; bob's page shows a `strong` with text `bob` and the literal text `<b>x</b>`; bob replies; alice sees the reply. `upload_roundtrip`: alice uploads a PNG built in the test; bob's page shows an `img` whose `naturalWidth` is 1. `search_roundtrip`: bob searches `hello` and sees one hit. Run `npm --prefix web run e2e`: PASS.
- [ ] **Step 4:** `scripts/check.sh web wording`.
- [ ] **Step 5: Commit** `feat(web): bare chat panel with safe Markdown, uploads and search`.

### Task 17: Security checklist and CSP (ADR 0004 revisited)

**Files:**
- Create: `docs/decisions/0008-csp.md` (supersedes 0004)
- Modify: `crates/vesper-server/src/lib.rs` (`guard` adds headers), `crates/vesper-server/tests/static_files.rs`, `deploy/Caddyfile.example` (drop the commented CSP, keep HSTS), `docs/deploy.md` "Security headers", `docs/decisions/0004-no-csp-yet.md` (status), `docs/decisions/README.md`, `docs/known-issues.md`, `web/e2e/two-browser.mjs`, `web/vite.config.ts` (`build.assetsInlineLimit: 0` if the worklet or any asset would become a `data:` URL)

**Interfaces:**
- `pub const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; img-src 'self' blob:; media-src 'self' blob:; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";` set on every response from `guard` except `/api/uploads/*`, which keeps its own sandbox policy. Also `Referrer-Policy: no-referrer`, `Permissions-Policy: microphone=(self), camera=(), geolocation=()`, `Cross-Origin-Opener-Policy: same-origin`. `'unsafe-inline'` for styles is there because Svelte writes `style` attributes; scripts stay strict.

- [ ] **Step 1: Failing tests:** `static_files.rs` `csp_on_html_and_api`, `upload_keeps_sandbox_csp`. In the e2e test: `page.on('console')` and an init script that records `securitypolicyviolation` events into `window.__csp`; assert both pages end with `__csp.length === 0` after voice, chat and upload ran.
- [ ] **Step 2: Run** them: fail. **Implement** the headers. Run: pass. If the e2e shows a violation, fix the cause (asset inlining, a blob worker) rather than loosening the policy; a needed loosening goes in ADR 0008 with the reason.
- [ ] **Step 3: Walk the checklist** and fix or record each item (fix: a test in the owning file; record: a line in `docs/known-issues.md`):
  1. Every non-GET `/api` route and `/ws` refuses a missing or foreign `Origin` (a test that iterates the router's route list, or one test per route group).
  2. No handler logs a body, a cookie, a token or an invite code (`grep -rn "tracing::\|info!\|warn!\|error!" crates/vesper-server/src` read by hand).
  3. Every route that takes a room id or message id checks `can_see` and answers 404 for hidden rooms.
  4. Role checks: one test per moderator power and per owner-only route, positive and negative (Tasks 5, 7, 9, 13 already have them; list the gaps).
  5. Rate limits: login, invite preview and signup, message send, upload, typing are each covered by a test.
  6. Cookies: `HttpOnly`, `SameSite=Strict`, `Secure` and `__Host-` under an https origin (unit test from Task 2 plus a config test).
  7. Uploads: the Task 14 tests pass; no route serves a file from a path built from user input other than `upload_path`.
  8. No `{@html}` (Task 16 test); CSP has no `unsafe-eval` and no `unsafe-inline` for scripts.
  9. `scripts/check.sh deny` and `npm audit --audit-level=high` pass.
  10. Known-issues updated: force-mute client-enforced, X-Forwarded-For trust, no password reset, no backup command, uploads not virus-scanned, passkeys pending (if Task 6 not run).
- [ ] **Step 4: Run** `scripts/check.sh` and `npm --prefix web run e2e`: both pass.
- [ ] **Step 5: Commit** `feat(server): strict CSP and security headers; ADR 0008`.

**M3 gate:** `scripts/check.sh` and the e2e test pass; the perf numbers are recorded. Then, only then, Task 6.

---

## Test strategy summary

| Task | Unit | Integration | e2e |
|---|---|---|---|
| 1 | random_hex | store on a temp-file DB: FTS5 spike, WAL, invites race, sessions | none |
| 2 | names, passwords, tokens, cookies, origin, ip, limiter | none | none |
| 3 | origin parsing | CLI against a temp data dir | none |
| 4, 5 | none | router via `oneshot`, `common::TestServer` | none |
| 6 | none | soft authenticator if the crate provides one | none |
| 7 | none | store and router | none |
| 8, 9 | none | real WebSocket plus `FakeClient` (as today) | none |
| 10 | vitest: api, state, peers, signaling, diag | none | two-browser sign-up and voice (Task 11) |
| 11 | none | load test 12 speakers by hand | two-browser: owner link, API invite, voice, presence |
| 12, 15 | fts_query | store; perf test `#[ignore]` | none |
| 13, 14 | body, mention, reaction, path, sniff, disposition | router plus WebSocket | none |
| 16 | vitest: markdown, chat reducer, no raw HTML | none | chat, upload, search |
| 17 | none | header tests | no CSP violations across the whole run |

## Performance note (10 users, 100k messages)

Expected, not measured; Task 15 Step 3 replaces this with numbers. 100k messages of about 100 bytes is roughly 10 MB of table plus a similar FTS index. History reads walk the `(room_id, id)` index for 50 rows, so they cost the same at 1k or 1M messages. Unread is an indexed range count per room, about ten rooms per `Hello`. FTS5 queries on an index this size are typically a few milliseconds; a one-letter prefix (`a*`) is the slow case, so `fts_query` drops terms with no letter or digit and the client should not search on one character. Writes are one at a time (WAL, `busy_timeout` 5 s); ten people typing is far below SQLite's write rate. argon2id at 19 MiB, two at a time, is the largest memory use (about 40 MB) on the 1 GB VM. The bounded socket queue caps per-socket memory at 256 messages.

## Self-review notes

- Spec coverage: invites, username and password with argon2id, three roles, 90-day sessions with a device list, display name, accent, colour and initials avatar (client-side from `user_id`), first owner by CLI, CSRF, rate limits, moderator powers, room create, rename, reorder and restrict, live presence, soft cap 12, edit, delete, reply, reactions, mentions, Markdown, uploads with 25 MB and a quota, typing, unread, pins, jump-to-reply (`around`), infinite scroll (`before`), search, disk use with an 80% warning, strict CSP. Passkeys are Task 6 (stretch).
- Not in this plan, on purpose: notifications, the redesign, avatar upload, password reset, a backup command, account deletion, room deletion, TURN.
