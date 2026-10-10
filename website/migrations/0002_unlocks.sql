-- Which achievements each player has unlocked (from the app's progress), so
-- game pages can show which ones are confirmed to work.
CREATE TABLE player_unlocks (
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  game        TEXT NOT NULL,
  achievement TEXT NOT NULL,
  PRIMARY KEY (user_id, game, achievement)
);
CREATE INDEX player_unlocks_game ON player_unlocks(game, achievement);

-- Account recovery without email: a one-time code shown at sign-up.
ALTER TABLE users ADD COLUMN recovery_hash TEXT NOT NULL DEFAULT '';
