-- Direct messages (v1.5). Additive only.
-- A DM is a restricted room of exactly two members, named dm-<idA>-<idB> with the ids sorted.
-- rooms.name is UNIQUE, so a pair has at most one DM.
ALTER TABLE rooms ADD COLUMN kind TEXT NOT NULL DEFAULT 'room' CHECK (kind IN ('room', 'dm'));
