-- A version stamp per message: unix ms, raised by every edit, delete, reaction and pin change, and
-- always above the previous value (max(old + 1, now)), so a client can drop an older copy of a
-- message that arrives after a newer one. Not covered by the FTS trigger (body, deleted_at only).
ALTER TABLE messages ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;

UPDATE messages SET updated_at = max(
    created_at, coalesce(edited_at, 0), coalesce(deleted_at, 0), coalesce(pinned_at, 0),
    coalesce((SELECT max(r.created_at) FROM reactions r WHERE r.message_id = messages.id), 0)
);
