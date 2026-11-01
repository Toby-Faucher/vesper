-- accounts.id is the user_id: 16 random bytes as 32 lowercase hex.
CREATE TABLE accounts (
    id TEXT PRIMARY KEY CHECK (length(id) = 32),
    username TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_phc TEXT NULL,
    role TEXT NOT NULL CHECK (role IN ('owner', 'moderator', 'member')),
    accent TEXT NOT NULL DEFAULT '#7c8cff',
    banned_at INTEGER NULL,
    created_at INTEGER NOT NULL
) STRICT;

-- token_hash is SHA-256 of the cookie token; the token itself is never stored.
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    token_hash BLOB NOT NULL UNIQUE CHECK (length(token_hash) = 32),
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_account ON sessions (account_id);

-- code_hash is SHA-256 of the invite code.
CREATE TABLE invites (
    id INTEGER PRIMARY KEY,
    code_hash BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    role TEXT NOT NULL CHECK (role IN ('owner', 'moderator', 'member')),
    max_uses INTEGER NOT NULL CHECK (max_uses >= 1),
    uses INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER NULL,
    created_by TEXT NULL REFERENCES accounts (id),
    created_at INTEGER NOT NULL
) STRICT;
