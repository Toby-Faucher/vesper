-- A separate push switch for direct messages (issue #150). Additive only. Existing devices keep
-- getting DM pushes (they used the mentions switch before), so the column defaults to on.
ALTER TABLE push_subscriptions ADD COLUMN dms INTEGER NOT NULL DEFAULT 1 CHECK (dms IN (0, 1));
