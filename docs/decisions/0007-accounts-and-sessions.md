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
a join. Sockets are now authenticated before the upgrade, so neither limit is needed.

Consequences: A lost owner password needs a manual database edit; there is no reset command. Clients that are not browsers
(`curl`, `vesper-loadtest`) must send the cookie and `Origin` themselves. Behind Vite dev, `VESPER_ORIGIN` must be
`http://localhost:5173`. Each socket's outgoing queue is bounded (256 messages) and a full queue closes that socket, so a slow
client reconnects instead of growing memory.
