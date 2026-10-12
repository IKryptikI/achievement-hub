-- When each achievement was unlocked (for profile timelines).
ALTER TABLE player_unlocks ADD COLUMN time INTEGER NOT NULL DEFAULT 0;
CREATE INDEX player_unlocks_user_time ON player_unlocks(user_id, time DESC);
