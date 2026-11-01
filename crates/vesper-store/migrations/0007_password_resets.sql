-- Password reset links (owner or moderator issued, or `vesper-server reset-password`), kept as an
-- audit: a row is never deleted with its account still there. code_hash is SHA-256 of the code;
-- the code itself is never stored. created_by is NULL for the command line.
-- A row is active while used_at and cancelled_at are NULL and expires_at is ahead; the partial
-- index allows one such row per account (a new link, a role change, a ban or a password change
-- cancels it). used_from is 'lan' or 'internet', never the address. seen_at: the person was shown
-- the "your password was reset" notice.
CREATE TABLE password_resets (
    id INTEGER PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    code_hash BLOB NOT NULL UNIQUE CHECK (length(code_hash) = 32),
    expires_at INTEGER NOT NULL,
    created_by TEXT NULL REFERENCES accounts (id),
    created_at INTEGER NOT NULL,
    used_at INTEGER NULL,
    used_from TEXT NULL CHECK (used_from IN ('lan', 'internet')),
    cancelled_at INTEGER NULL,
    seen_at INTEGER NULL
) STRICT;

CREATE UNIQUE INDEX password_resets_open ON password_resets (account_id)
    WHERE used_at IS NULL AND cancelled_at IS NULL;
