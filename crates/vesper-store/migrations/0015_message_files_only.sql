-- no-transaction
-- A message of files with no text has an empty body (clients used to send the file names, because the body could not be
-- empty). Only the row check changes: a deleted message must still be empty, a live one may now be. The server allows an
-- empty body only with files. Every row is copied as it is, ids included.
-- SQLite cannot change a CHECK in place, so the table is rebuilt. Other tables reference messages, and dropping it with
-- foreign keys on would fail, so they are off for the rebuild. That pragma does nothing inside a transaction, so this file
-- runs outside sqlx's transaction and opens its own.
PRAGMA foreign_keys = OFF;
BEGIN IMMEDIATE;

CREATE TABLE messages_new (
    id INTEGER PRIMARY KEY,
    room_id INTEGER NOT NULL REFERENCES rooms (id),
    author_id TEXT NOT NULL REFERENCES accounts (id),
    body TEXT NOT NULL CHECK (length(body) <= 4000),
    reply_to INTEGER NULL REFERENCES messages (id),
    created_at INTEGER NOT NULL,
    edited_at INTEGER NULL,
    deleted_at INTEGER NULL,
    deleted_by TEXT NULL REFERENCES accounts (id),
    pinned_at INTEGER NULL,
    pinned_by TEXT NULL REFERENCES accounts (id),
    updated_at INTEGER NOT NULL DEFAULT 0,
    CHECK (deleted_at IS NULL OR length(body) = 0)
) STRICT;

INSERT INTO messages_new (id, room_id, author_id, body, reply_to, created_at, edited_at, deleted_at, deleted_by,
                          pinned_at, pinned_by, updated_at)
SELECT id, room_id, author_id, body, reply_to, created_at, edited_at, deleted_at, deleted_by, pinned_at, pinned_by,
       updated_at
FROM messages;

DROP TABLE messages;
ALTER TABLE messages_new RENAME TO messages;

CREATE INDEX messages_room ON messages (room_id, id, author_id, deleted_at);
CREATE INDEX messages_author ON messages (author_id, id);
CREATE INDEX messages_pinned ON messages (room_id) WHERE pinned_at IS NOT NULL;

-- The full-text index keeps its rows (same ids, same bodies); only its triggers went with the old table.
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, body) VALUES (new.id, new.body);
END;

CREATE TRIGGER messages_au AFTER UPDATE OF body, deleted_at ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.id, old.body);
    INSERT INTO messages_fts (rowid, body) SELECT new.id, new.body WHERE new.deleted_at IS NULL;
END;

COMMIT;
PRAGMA foreign_keys = ON;
