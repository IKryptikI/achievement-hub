-- Record-mode findings players chose to share from the app.
CREATE TABLE shared_mappings (
  id          INTEGER PRIMARY KEY,
  user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
  gamertag    TEXT NOT NULL DEFAULT '',
  game        TEXT NOT NULL,
  achievement TEXT NOT NULL,
  data        TEXT NOT NULL,     -- the mapping file's JSON
  created     INTEGER NOT NULL,
  status      TEXT NOT NULL DEFAULT 'new'  -- new, merged, rejected
);
