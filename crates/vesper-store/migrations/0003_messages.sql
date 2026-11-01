-- Messages are never hard-deleted: a delete empties the body and sets deleted_at (a tombstone), so a
-- reply keeps its anchor. Ids only grow, which keeps history cursors stable.
CREATE TABLE messages (
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
    CHECK ((deleted_at IS NULL) = (length(body) > 0))
) STRICT;

-- (room_id, id) for history; author_id and deleted_at make the unread count an index-only scan.
CREATE INDEX messages_room ON messages (room_id, id, author_id, deleted_at);
CREATE INDEX messages_author ON messages (author_id, id);
CREATE INDEX messages_pinned ON messages (room_id) WHERE pinned_at IS NOT NULL;

CREATE TABLE reactions (
    message_id INTEGER NOT NULL REFERENCES messages (id),
    account_id TEXT NOT NULL REFERENCES accounts (id),
    emoji TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, emoji, account_id)
) STRICT, WITHOUT ROWID;

-- Parsed from @username on the server at send and edit.
CREATE TABLE mentions (
    account_id TEXT NOT NULL REFERENCES accounts (id),
    message_id INTEGER NOT NULL REFERENCES messages (id),
    PRIMARY KEY (account_id, message_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX mentions_message ON mentions (message_id);

-- Per user, per room: the newest message id the user has read. Only moves forward.
CREATE TABLE reads (
    account_id TEXT NOT NULL REFERENCES accounts (id),
    room_id INTEGER NOT NULL REFERENCES rooms (id),
    last_read_id INTEGER NOT NULL,
    PRIMARY KEY (account_id, room_id)
) STRICT, WITHOUT ROWID;

-- Uploaded files (Task 14 writes them). message_id stays NULL until a message uses the upload.
CREATE TABLE uploads (
    id TEXT PRIMARY KEY CHECK (length(id) = 32),
    room_id INTEGER NOT NULL REFERENCES rooms (id),
    uploader_id TEXT NOT NULL REFERENCES accounts (id),
    message_id INTEGER NULL REFERENCES messages (id),
    name TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX uploads_message ON uploads (message_id);

-- Full-text index over live bodies, kept by triggers. A soft delete drops the row from the index.
CREATE VIRTUAL TABLE messages_fts USING fts5 (
    body, content = 'messages', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, body) VALUES (new.id, new.body);
END;

-- 'delete' needs the old values exactly as indexed; a tombstone (empty body) indexes no token.
CREATE TRIGGER messages_au AFTER UPDATE OF body, deleted_at ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, body) VALUES ('delete', old.id, old.body);
    INSERT INTO messages_fts (rowid, body) SELECT new.id, new.body WHERE new.deleted_at IS NULL;
END;
