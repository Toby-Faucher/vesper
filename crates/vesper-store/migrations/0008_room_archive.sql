-- An archived room is hidden from everyone but the owner's Admin > Rooms; its messages stay.
-- NULL means not archived; otherwise when it was archived (Unix ms).
ALTER TABLE rooms ADD COLUMN archived_at INTEGER NULL;
