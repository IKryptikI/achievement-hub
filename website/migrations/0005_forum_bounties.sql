-- The forum: threads with a tag, posts (the first is the thread's text),
-- emoji reactions, and an accepted answer.
CREATE TABLE forum_threads (
  id          INTEGER PRIMARY KEY,
  tag         TEXT NOT NULL,
  title       TEXT NOT NULL,
  user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created     INTEGER NOT NULL,
  last_post   INTEGER NOT NULL,
  replies     INTEGER NOT NULL DEFAULT 0,
  pinned      INTEGER NOT NULL DEFAULT 0,
  locked      INTEGER NOT NULL DEFAULT 0,
  answer      INTEGER                       -- the post marked as the answer
);
CREATE INDEX forum_threads_order ON forum_threads(pinned DESC, last_post DESC);

CREATE TABLE forum_posts (
  id          INTEGER PRIMARY KEY,
  thread_id   INTEGER NOT NULL REFERENCES forum_threads(id) ON DELETE CASCADE,
  user_id     INTEGER REFERENCES users(id) ON DELETE SET NULL,
  body        TEXT NOT NULL,
  created     INTEGER NOT NULL,
  edited      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX forum_posts_thread ON forum_posts(thread_id, id);

CREATE TABLE forum_reactions (
  post_id     INTEGER NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  emoji       TEXT NOT NULL,
  PRIMARY KEY (post_id, user_id, emoji)
);

-- The mapping bounty board: a mapper claims an achievement to work on.
CREATE TABLE bounties (
  game        TEXT NOT NULL,
  achievement TEXT NOT NULL,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  claimed     INTEGER NOT NULL,
  status      TEXT NOT NULL DEFAULT 'claimed',  -- claimed, submitted, done
  PRIMARY KEY (game, achievement)
);

-- Gamercard images (PNG), drawn by the owner's browser from /card/<tag>.svg
-- (rendering on the server costs more CPU than the free plan allows).
CREATE TABLE gamercards (
  user_id     INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  png         BLOB NOT NULL,
  updated     INTEGER NOT NULL
);
