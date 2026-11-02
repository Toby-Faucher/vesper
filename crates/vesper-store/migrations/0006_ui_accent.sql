-- The UI accent each person chose in Settings > Appearance: a preset key or #rrggbb, checked by the
-- server. NULL means amber, the default. Not accounts.accent, which is the avatar colour.
ALTER TABLE accounts ADD COLUMN ui_accent TEXT NULL CHECK (length(ui_accent) <= 7);
