-- Players upload their own gamer picture (no presets).
CREATE TABLE gamerpics (
  user_id     INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  img         BLOB NOT NULL,
  type        TEXT NOT NULL,
  updated     INTEGER NOT NULL
);
-- The preset pictures are gone: back to the gamertag's initial.
UPDATE users SET avatar = upper(substr(gamertag, 1, 1)) WHERE avatar LIKE 'gp:%';
