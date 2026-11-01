-- One-use password reset codes (owner or moderator issued, or `vesper-server reset-password`).
-- code_hash is SHA-256 of the code; the code itself is never stored. account_id is UNIQUE, so an
-- account has at most one live code: a new one replaces the old, and using it deletes the row.
CREATE TABLE password_resets (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL UNIQUE REFERENCES accounts (id) ON DELETE CASCADE,
    code_hash BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    expires_at INTEGER NOT NULL,
    created_by TEXT NULL REFERENCES accounts (id),
    created_at INTEGER NOT NULL
) STRICT;
