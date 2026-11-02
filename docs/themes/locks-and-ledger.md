# Theme plan: Locks and Ledger

- **Status:** Planned, not started.
- **Date:** 2026-11-02 (written against v1.4.0).
- **Releases:** follow the release notes convention in [versions.md](../versions.md#how-a-release-is-cut).

This is a plan, not a promise. Nothing here is built. Sizes are S, M or L and say nothing about time.

## Goal and the story

Who can get in, and what happened. Passkeys for the first, a log and reasons for the second.

For the owner, and for anyone who forgets passwords: "Jo signs in with a fingerprint and no password. When the owner bans someone, the person sees why, and the owner's log shows who did it and when."

## Why now

- The design spec already chose "Username with passkey (WebAuthn) and password fallback" ([spec](../superpowers/specs/2026-10-30-vesper-design.md)). [known-issues.md](../known-issues.md) says "No passkeys yet".
- known-issues.md: "A moderator can take over a member's account through a reset link", and the member is told only afterwards.
- Element Web [#2630](https://github.com/element-hq/element-web/issues/2630) "Feature request: export chat logs": 188 reactions (closed, GitHub search API, 2026-11-01).
- The safety net exists now. v1.4 takes a backup before every migration ([versions.md](../versions.md)), and this theme adds up to four migrations.

It has the lowest felt value of the planned themes for a group that trusts each other. Its first slice, the audit log, can ride along with any release.

## Features

| # | Feature | Size |
|---|---|---|
| 1 | **Passkeys:** add and remove in Settings > Account, a "Sign in with a passkey" button, the password stays as a fallback. | M |
| 2 | **Admin audit log:** every owner and moderator action with actor, target, time and "LAN or internet". Admin > Log, owner only. | S |
| 3 | **A reason with a ban, a disconnect or a force-mute,** shown to the person in one plain line. | S |
| 4 | **Tell the member when a reset link is made for them,** by Web Push and the sign-in notice, when it is made, not after it is used. | S |
| 5 | **Sign out every other device in one click,** and show "LAN or internet" per device. | S |
| 6 | **Export:** `vesper-server export <dir>` (rooms as Markdown and JSON, plus files) and "Download my messages" per person. | M |
| 7 | **(Decision) Restore window for deletes:** a deleted message keeps its body hidden for 7 days, the owner can restore it, then it is purged. | M |

## What each needs

Checked against the code on 2026-11-02. Migrations on `main` end at 0010, so new ones start at 0011. The real numbers are given when each lands.

| # | Migration | Dependency | Server | CSP | Size (gzip) |
|---|---|---|---|---|---|
| 1 | `passkeys`, plus ceremony state on the server | A WebAuthn crate (see below) | New routes, RP ID from the `VESPER_ORIGIN` host | None | Login button on first load (small), ceremony code lazy, Settings +1.2 kB |
| 2 | `audit_log` | None | A write in every admin route and the CLI | None | Admin +1.2 kB |
| 3 | None (kept in the audit row) | None | Additive field on ban, disconnect and force-mute | None | Small |
| 4 | None | None | Reuse `push.rs` and the reset notices | None | Small |
| 5 | `sessions.last_from` | None | One route | None | Settings +0.4 kB |
| 6 | None | Maybe a zip or tar crate for the web download; a plain directory for the CLI | New CLI command and route | None | Settings, small |
| 7 | A side table for deleted bodies | None | A sweep | None | Admin, small |

What the code says today:

- **WebAuthn crate and its license.** `webauthn-rs` 0.6.2 is MPL-2.0. `deny.toml` allows only Apache-2.0, BSD-3-Clause, CDLA-Permissive-2.0, ISC, MIT, Unicode-3.0 and Zlib, so `cargo deny` refuses it as is. Its core has no OpenSSL in its normal dependencies (crates.io, 2026-11-02), which matters next to aws-lc-rs. One option is `webauthn_rp` 0.3.0 (MIT OR Apache-2.0, RustCrypto `p256`, `p384`, `ed25519-dalek` and `rsa`). The `rsa` crate has an open advisory (RUSTSEC-2023-0071, a timing side channel in private-key operations); a relying party only verifies signatures, but `cargo deny` may still flag it. Not tried.
- **Ceremony state** must stay on the server ([webauthn-rs docs](https://docs.rs/webauthn-rs/latest/webauthn_rs/)): a cookie holding it lets a client replay an old state.
- **Browser side.** `PublicKeyCredential.parseCreationOptionsFromJSON` and `parseRequestOptionsFromJSON` exist in Chrome 129, Firefox 119 and Safari 18.4 (MDN browser-compat-data), so no base64 helper library.
- **Admin routes** (`crates/vesper-server/src/api_admin.rs`): role, reset link, ban, unban, kick, force-mute, disconnect and invites made or revoked. Room routes (`api_rooms.rs`): create, reorder, update (archive) and members. CLI (`cli.rs`): `create-owner`, `invite`, `reset-password` and `backup`. Only reset links leave a record today: their rows are never deleted while the account exists (migration 0007), plus a log line.
- **Ban and sign in.** Ban is owner only. It deletes every session and closes the sockets. A banned person who signs in gets the same "wrong login" error as a wrong password (`login` in `api_accounts.rs`). So a ban reason can only be shown after a correct password, and showing it tells that person the password was right. That is a choice, below.
- **Kick, force-mute and disconnect** are for moderators and above, on a lower role. A disconnect closes sockets with code 4003 and the sessions stay valid, so the reason can travel in a socket message.
- **Devices.** `GET /api/sessions` and `DELETE /api/sessions/{id}` exist, and Settings > Account lists devices (`Device { id, label, created_at, last_seen_at, current }`). There is no "all others" route, and `sessions` has no column for where it was last used. Reset rows already store `used_from` as `lan` or `internet`, never the address.
- **Deleted messages.** `messages` has `CHECK ((deleted_at IS NULL) = (length(body) > 0))` (migration 0003): a deleted row cannot keep its body. A restore window needs the body in a separate table, and its search index rows handled. SQLite cannot change that check without rebuilding the table, which an additive migration must not do.
- **Backup** exists: `vesper-server backup <dir>` writes a database copy and the uploads. Export is a readable copy for people, not a restore copy.

### Size, with v1.4.0 numbers

Measured on `main` at v1.4.0 (`npm run build`, `scripts/size-check.mjs`, gzip): first load 56.6 kB against a ratchet of 56.9 (hard cap 70), largest lazy chunk 14.7 (Settings, cap 35), Admin 11.3, all JS 149.9 against 170.

- **All JS:** about 3 kB of lazy code. Small next to Game Night and Inside Jokes.
- **First load:** only 0.3 kB is left under the ratchet, and sign in is in the first load. The research estimate was +0.8 kB for passkeys there. Keep only the button on first load and load the ceremony code on click, or raise the ratchet on purpose (a decision below).

## Risks

- **Security.** Passkeys bind accounts to the hostname. If `VESPER_ORIGIN` changes, every passkey is orphaned, so the password stays.
- **Auth code.** A new dependency in the sign-in path. Keep it behind the same limits as password login.
- **Privacy.** The audit log and "LAN or internet" are records about people. Keep the address class, never the IP.
- **Restore window.** A delete stops being immediate. That is a privacy step back unless the UI says so.
- **Export.** A person's export must never include rooms they cannot see, and the owner's must say which rooms are archived.
- **Devices.** passkeys.dev (updated 2026-09-21) says Linux has synced passkeys only through browser extensions, and Firefox is not listed for cross-device sign in on desktop. Which systems the group uses is not known.

## Decisions for Toby

- [ ] Add passkeys as a sign-in option (the password stays)? (yes/no)
- [ ] If yes, use an MPL-2.0 crate (`webauthn-rs`) and add MPL-2.0 to `deny.toml`? (yes/no; no means trying `webauthn_rp`)
- [ ] Show the ban reason at sign in, after a correct password (it tells that person the password was right)? (yes/no)
- [ ] May moderators still make reset links for members? (yes/no)
- [ ] Keep deleted messages restorable for 7 days? (yes/no)
- [ ] Let members export their own messages? (yes/no)
- [ ] Raise the first-load ratchet by up to 0.5 kB for the passkey button, if it does not fit in 0.3 kB? (yes/no)

## Definition of done

- WebAuthn ceremonies tested with a software authenticator in headless Chromium (CDP `WebAuthn.addVirtualAuthenticator`).
- A replayed or expired ceremony state is refused (test).
- Every admin route and CLI command that changes something writes one audit row (a test that walks the route table).
- The reason text reaches a disconnected or force-muted person (e2e), and a banned login if Toby says yes.
- An export test proves hidden rooms are absent.
- `cargo deny` passes, each migration is additive, and the size check passes with all five numbers in the PR.

## Not included

TOTP two-factor (passkeys cover it), GeoIP location (a third-party database), IP bans, end-to-end encryption, content scanning, email. A sessions page: it shipped already (Settings > Account devices); only "sign out every other device" is left, as #5.

## First slice that ships alone

The audit log with reasons (#2 and #3): S plus S, server first, one migration (0011).

## Sources

- Element Web [#2630](https://github.com/element-hq/element-web/issues/2630). Seen 2026-11-01.
- [webauthn-rs docs](https://docs.rs/webauthn-rs/latest/webauthn_rs/) (server-side state, "conditional-ui" not recommended), [webauthn-rs on crates.io](https://crates.io/crates/webauthn-rs), [webauthn_rp on crates.io](https://crates.io/crates/webauthn_rp), [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html).
- [passkeys.dev device support](https://passkeys.dev/device-support/).
- [MDN browser-compat-data](https://github.com/mdn/browser-compat-data) for `PublicKeyCredential.parseCreationOptionsFromJSON`.
- In this repo: `deny.toml`, `crates/vesper-server/src/api_admin.rs`, `api_accounts.rs`, `api_rooms.rs`, `cli.rs`, `crates/vesper-protocol/src/api.rs` (`Device`), `crates/vesper-store/migrations/0001_accounts.sql`, `0003_messages.sql`, `0007_password_resets.sql`, [known-issues.md](../known-issues.md), `web/scripts/size-check.mjs`.
