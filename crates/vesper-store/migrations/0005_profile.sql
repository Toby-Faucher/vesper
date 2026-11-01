-- A short bio, and a profile picture. accounts.avatar is the picture's version (hex of its hash,
-- set by the server) or NULL for none; the bytes live in avatars, one row per account, and go with it.
ALTER TABLE accounts ADD COLUMN bio TEXT NOT NULL DEFAULT '' CHECK (length(bio) <= 160);
ALTER TABLE accounts ADD COLUMN avatar TEXT NULL;

-- mime is what the server sniffed from the bytes, never the client's claim. 512 KiB at most.
CREATE TABLE avatars (
    account_id TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    mime TEXT NOT NULL CHECK (mime IN ('image/png', 'image/jpeg', 'image/webp')),
    data BLOB NOT NULL CHECK (length(data) <= 524288)
) STRICT;
