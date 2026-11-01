-- Every room has both text and voice. restricted = 1 means only listed members (plus the owner and
-- moderators) can see it; the list is kept when a room is opened again.
CREATE TABLE rooms (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    position INTEGER NOT NULL,
    restricted INTEGER NOT NULL DEFAULT 0 CHECK (restricted IN (0, 1)),
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE room_members (
    room_id INTEGER NOT NULL REFERENCES rooms (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    PRIMARY KEY (room_id, account_id)
) STRICT, WITHOUT ROWID;

-- The day-one rooms.
INSERT INTO rooms (name, position, created_at) VALUES ('general', 0, 0), ('hangout', 1, 0);
