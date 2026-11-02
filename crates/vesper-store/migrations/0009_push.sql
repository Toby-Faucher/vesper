-- Web Push (issue #58). Additive only.

-- Secrets the server makes for itself, such as the VAPID signing key (a PKCS#8 document). In the
-- database so that a copy of vesper.db (the backup) holds it too. Never sent to a client.
CREATE TABLE server_secrets (
    name TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

-- One browser push subscription per session (signed-in browser). Deleting the session (logout,
-- revoke, ban, password change) deletes it. endpoint is the push service URL the server POSTs to;
-- p256dh (65-byte P-256 point) and auth (16 bytes) are the browser's encryption keys. mentions and
-- voice are this device's switches for the two kinds of push.
CREATE TABLE push_subscriptions (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL UNIQUE REFERENCES sessions (id) ON DELETE CASCADE,
    endpoint TEXT NOT NULL UNIQUE CHECK (length(endpoint) <= 2048),
    p256dh BLOB NOT NULL CHECK (length(p256dh) = 65),
    auth BLOB NOT NULL CHECK (length(auth) = 16),
    mentions INTEGER NOT NULL CHECK (mentions IN (0, 1)),
    voice INTEGER NOT NULL CHECK (voice IN (0, 1)),
    created_at INTEGER NOT NULL,
    last_ok_at INTEGER NULL,
    user_agent_hint TEXT NOT NULL CHECK (length(user_agent_hint) <= 64)
) STRICT;

CREATE INDEX push_subscriptions_account ON push_subscriptions (account_id);
