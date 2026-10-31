# 0003. One shared join key until accounts exist

Status: accepted, temporary, 2026-10-30

Context: The voice foundation ships before accounts and invites. The server still must not accept anonymous sockets.

Decision: `VESPER_DEV_KEY` is a single shared secret. It has no default: the server refuses to start without it. A user id is the first 32 hex characters of the SHA-256 of the display name. The key is redacted from `Config`'s Debug output.

Why: It is the smallest gate that works for about 10 friends. Accounts, invites (passkeys with a password fallback) and sessions come in a later plan.

Consequences: Any key holder can join under any name and kick that name's session. Names are not normalised (NFC, zero-width). The key travels in the join message, so the site must be served over TLS. Remove the key, the name hash and the `shortcut:` comment in `config.rs` when accounts land.
