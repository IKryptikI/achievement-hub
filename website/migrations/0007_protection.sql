-- Email verification, bans, password reset by email, and reports.
ALTER TABLE users ADD COLUMN email_verified INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN email_code_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN email_code_expires INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN banned INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN ban_reason TEXT NOT NULL DEFAULT '';

CREATE TABLE password_resets (
  token_hash  TEXT PRIMARY KEY,
  user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires     INTEGER NOT NULL
);

-- Things players flagged for the admin to look at.
CREATE TABLE reports (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL,            -- post, thread, tip, user
  target      TEXT NOT NULL,            -- post/thread/tip id, or a gamertag
  reason      TEXT NOT NULL,
  reporter_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
  created     INTEGER NOT NULL,
  status      TEXT NOT NULL DEFAULT 'open'  -- open, done, dismissed
);
CREATE UNIQUE INDEX reports_once ON reports(kind, target, reporter_id);
