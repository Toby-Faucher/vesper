# 0007. Accounts, invites and session cookies

Status: accepted, 2026-11-01. Supersedes [0003](0003-shared-join-key.md).

Context: The voice foundation used one shared join key and named people by the display name they typed ([0003](0003-shared-join-key.md)).
Anyone with the key could take any name and kick that name's session, and lookalike names counted as different people. Text
chat needs stable identities, roles and history per person.

Decision:

- **Accounts come from invites.** An invite is a link `<origin>/invite#<code>` with 32 random bytes. The database keeps only the
  SHA-256 of the code. Default: 1 use, 7 days, role member. The owner account comes only from `vesper-server create-owner`.
- **Passwords** are 8 to 128 characters, hashed with argon2id (m = 19456 KiB, t = 2, p = 1), at most two at a time. Usernames are
  `[a-z0-9_]{3,24}`, lowercased; display names are NFC and refuse control, format and bidi characters.
- **Sessions are cookies.** The cookie holds 32 random bytes as hex; the database holds their SHA-256, so a database leak gives no
  usable cookie. `HttpOnly; SameSite=Strict; Path=/`, and `Secure` with the `__Host-` name on https. A session ends 90 days after
  its last use, on logout, on revoke, on a password change (the other sessions) and on a ban.
- **Origin check.** Every `/api` request that changes something, and the `/ws` upgrade, need `Origin` equal to `VESPER_ORIGIN`,
  else 403. Together with `SameSite=Strict` this stops another site from using a signed-in browser.
- **REST plus one WebSocket.** Every change, history and search go over REST JSON under `/api`: status codes, one CSRF rule and
  simple tests. The WebSocket carries what must be pushed: presence, voice signaling, room and user changes, later chat events and
  typing. It is authenticated by the same cookie on the upgrade (401 without a live session), and the server closes it with code
  4001 when its session ends.
- **`user_id` is `accounts.id`**: 16 random bytes as 32 lowercase hex characters, so it stays a valid MSID stream id with no
  derivation.

Why: Invites keep the server private to a friend group without a public signup page. Hashed tokens and codes keep a stolen
database from being a key ring. One cookie for REST and the socket means one way to be signed in and one way to be signed out.

Removed from 0003: `VESPER_DEV_KEY`, the key in the join message and its check, the user id from a hash of the display name,
the `shortcut:` comment in `config.rs`, the join deadline (`VESPER_JOIN_DEADLINE_SECS`) and the cap of five failed messages before
a join. Sockets are now authenticated before the upgrade, so those two limits are gone. A signed-in user can still flood
their socket, so each socket has its own message rate limit instead (`ws.rs`, see `docs/architecture.md`).

Consequences: A lost owner password needed a manual database edit (superseded in v1.3, see the update below). Clients that are not browsers
(`curl`, `vesper-loadtest`) must send the cookie and `Origin` themselves. Behind Vite dev, `VESPER_ORIGIN` must be
`http://localhost:5173`. Each socket's outgoing queue is bounded (256 messages) and a full queue closes that socket, so a slow
client reconnects instead of growing memory.

Update, v1.3: one-use reset links (Admin > Members) and `vesper-server reset-password <username>` replace the manual database
edit for a lost password ([deploy.md](../deploy.md#password-recovery)). The code is 32 random bytes kept as SHA-256, like an
invite, and using it ends every session of the account. Making one needs the maker's own password and never works on yourself
in the app. Every reset row is kept in the database; the owner's Members page lists the latest link and the resets used in
the last 30 days.

Update, v1.6: passkeys (WebAuthn, issue #151) are a second way to sign in, not a new kind of session. A passkey sign in ends in the
same code as a password one: a random cookie token, only its SHA-256 stored, the same cookie flags, and the same drop of a presented
cookie. The Origin check covers its routes like every other change, and WebAuthn checks the origin and rp id again inside the
signed data. Sign in finds the account from the credential (no username), so the answer does not tell who has a passkey. Passwords
stay, and are the recovery path. A banned account is refused at passkey sign in. A password reset does not remove passkeys
(see [known issues](../known-issues.md)). Challenges are single use and held in memory for 5 minutes.
