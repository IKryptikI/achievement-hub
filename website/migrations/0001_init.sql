-- Accounts, sessions, linked apps, progress and friends.
CREATE TABLE users (
  id          INTEGER PRIMARY KEY,
  gamertag    TEXT NOT NULL UNIQUE COLLATE NOCASE,
  email       TEXT UNIQUE COLLATE NOCASE,
  pass_hash   TEXT NOT NULL,
  pass_salt   TEXT NOT NULL,
  created     INTEGER NOT NULL,
  -- profile customization
  bio         TEXT NOT NULL DEFAULT '',
  avatar      TEXT NOT NULL DEFAULT '',       -- one emoji/letter shown in the gamer picture
  color       TEXT NOT NULL DEFAULT '#3fae2a', -- profile accent
  motto       TEXT NOT NULL DEFAULT '',
  favorite    TEXT NOT NULL DEFAULT '',       -- a game id
  private     INTEGER NOT NULL DEFAULT 0      -- 1: only friends see progress
);

CREATE TABLE sessions (
  token_hash  TEXT PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires     INTEGER NOT NULL
);

-- The desktop app/overlay, linked to an account with a short code.
CREATE TABLE link_codes (
  code        TEXT PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires     INTEGER NOT NULL
);
CREATE TABLE app_tokens (
  token_hash  TEXT PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created     INTEGER NOT NULL,
  last_used   INTEGER NOT NULL DEFAULT 0
);

-- The app's latest snapshot: Gamerscore, games, recent unlocks, playing.
CREATE TABLE progress (
  user_id     INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  snapshot    TEXT NOT NULL,
  gamerscore  INTEGER NOT NULL DEFAULT 0,
  sent        INTEGER NOT NULL
);

-- requester -> addressee; accepted friendships are mutual.
CREATE TABLE friends (
  requester   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  addressee   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  status      TEXT NOT NULL CHECK (status IN ('pending', 'accepted')),
  created     INTEGER NOT NULL,
  PRIMARY KEY (requester, addressee)
);
CREATE INDEX friends_addressee ON friends(addressee);
