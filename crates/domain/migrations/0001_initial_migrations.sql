CREATE TABLE diary (
  id          INTEGER PRIMARY KEY,
  title       TEXT NOT NULL,
  description TEXT,
  share_token TEXT NOT NULL UNIQUE,
  timezone    TEXT NOT NULL DEFAULT 'Europe/Berlin',
  created_at  INTEGER NOT NULL
) STRICT;

CREATE TABLE entry (
  id          INTEGER PRIMARY KEY,
  diary_id    INTEGER NOT NULL REFERENCES diary(id) ON DELETE CASCADE,
  local_date  TEXT    NOT NULL,   -- 'YYYY-MM-DD', computed in the diary's tz at write time
  occurred_at INTEGER NOT NULL,   -- unix seconds, UTC
  created_at  INTEGER NOT NULL,
  text        TEXT
) STRICT;

CREATE INDEX entry_by_day ON entry(diary_id, local_date, occurred_at);

CREATE TABLE image (
  id       INTEGER PRIMARY KEY,
  entry_id INTEGER NOT NULL REFERENCES entry(id) ON DELETE CASCADE,
  hash     TEXT    NOT NULL,
  width    INTEGER NOT NULL,
  height   INTEGER NOT NULL,
  position INTEGER NOT NULL,
  alt      TEXT,
  UNIQUE (entry_id, position)
) STRICT;
