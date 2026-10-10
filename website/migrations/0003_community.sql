-- Admins (the site owner), profile banner and showcase.
ALTER TABLE users ADD COLUMN is_admin INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN banner TEXT NOT NULL DEFAULT '';     -- a game id whose art is the profile banner
ALTER TABLE users ADD COLUMN showcase TEXT NOT NULL DEFAULT '[]'; -- up to 5 "game/achievement" picks

-- Per-game progress for leaderboards and rarity (from the app's progress).
CREATE TABLE player_games (
  user_id   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  game      TEXT NOT NULL,
  points    INTEGER NOT NULL,
  unlocked  INTEGER NOT NULL,
  total     INTEGER NOT NULL,
  PRIMARY KEY (user_id, game)
);
CREATE INDEX player_games_game ON player_games(game, points DESC);

-- People who want to help map achievements (opt-in, contact details).
CREATE TABLE mapper_applications (
  id          INTEGER PRIMARY KEY,
  user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
  name        TEXT NOT NULL,
  email       TEXT NOT NULL,
  discord     TEXT NOT NULL DEFAULT '',
  platform    TEXT NOT NULL DEFAULT '',
  games       TEXT NOT NULL DEFAULT '',
  experience  TEXT NOT NULL DEFAULT '',
  hours       TEXT NOT NULL DEFAULT '',
  notes       TEXT NOT NULL DEFAULT '',
  created     INTEGER NOT NULL,
  status      TEXT NOT NULL DEFAULT 'new'  -- new, contacted, accepted, declined
);

-- Game requests and votes.
CREATE TABLE game_requests (
  id        INTEGER PRIMARY KEY,
  title     TEXT NOT NULL,
  store     TEXT NOT NULL DEFAULT '',
  notes     TEXT NOT NULL DEFAULT '',
  user_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created   INTEGER NOT NULL,
  status    TEXT NOT NULL DEFAULT 'open',  -- open, planned, added, declined
  votes     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE request_votes (
  request_id INTEGER NOT NULL REFERENCES game_requests(id) ON DELETE CASCADE,
  user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY (request_id, user_id)
);

-- Player-written tips for achievements, with votes.
CREATE TABLE guides (
  id          INTEGER PRIMARY KEY,
  game        TEXT NOT NULL,
  achievement TEXT NOT NULL,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  body        TEXT NOT NULL,
  created     INTEGER NOT NULL,
  votes       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX guides_ach ON guides(game, achievement);
CREATE TABLE guide_votes (
  guide_id INTEGER NOT NULL REFERENCES guides(id) ON DELETE CASCADE,
  user_id  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY (guide_id, user_id)
);

-- News / changelog posts (written by admins).
CREATE TABLE news (
  id       INTEGER PRIMARY KEY,
  title    TEXT NOT NULL,
  body     TEXT NOT NULL,
  created  INTEGER NOT NULL
);

-- Simple abuse limits per connection.
CREATE TABLE rate_limits (
  key      TEXT PRIMARY KEY,
  count    INTEGER NOT NULL,
  window   INTEGER NOT NULL
);
