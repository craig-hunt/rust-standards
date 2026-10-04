-- The shape the stores expect, applied at startup by the migrator.
--
-- Written so a second run changes nothing. A reference service that refused to
-- start twice would make every local experiment begin with dropping a database,
-- and the first thing anyone learns would be a workaround.
--
-- The inventory rows are NOT seeded here. They live in the domain, and seeding
-- them from SQL would put the same six names in two places with nothing to keep
-- them equal. persistence::apply_schema inserts them from the domain's own seed.

CREATE TABLE IF NOT EXISTS tasks (
  id        BIGSERIAL PRIMARY KEY,
  title     TEXT    NOT NULL,
  completed BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS signups (
  id         BIGSERIAL PRIMARY KEY,
  full_name  TEXT        NOT NULL,
  email      TEXT        NOT NULL,
  plan       TEXT        NOT NULL,
  seats      INTEGER     NOT NULL,
  notes      TEXT        NOT NULL,
  created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS inventory (
  name     TEXT    PRIMARY KEY,
  quantity INTEGER NOT NULL,
  status   TEXT    NOT NULL
);

-- event_id is the primary key, so a relay that retries an insert cannot create a
-- second copy of the same event.
CREATE TABLE IF NOT EXISTS outbox (
  event_id     UUID        PRIMARY KEY,
  type         TEXT        NOT NULL,
  payload      JSONB       NOT NULL,
  occurred_at  TIMESTAMPTZ NOT NULL,
  published_at TIMESTAMPTZ
);

-- The relay reads only undelivered rows, and a partial index keeps that scan
-- proportional to the backlog rather than to every event the service ever
-- published.
CREATE INDEX IF NOT EXISTS outbox_pending
  ON outbox (occurred_at)
  WHERE published_at IS NULL;
