-- lindfors-newsletter: the newsletter's state, in the Postgres that already runs on
-- mail.lindfors.no. Applied by hand as the `newsletter` role, which then owns every
-- table (the service connects as that role and needs no DDL):
--
--   sudo -u postgres psql -U newsletter -d newsletter -f schema.sql
--
-- Idempotent: every statement is IF NOT EXISTS, so re-running it after a change adds
-- what is new and touches nothing else.

-- Who gets the newsletter, now. Only current subscribers are here: an unsubscribe
-- DELETEs the row rather than flagging it, so this table is never a list of people
-- who used to be on the list. The history lives in `events`, under a pseudonym.
CREATE TABLE IF NOT EXISTS subscribers (
    email         text        PRIMARY KEY,
    subscribed_at timestamptz NOT NULL DEFAULT now(),
    -- 'confirmed' for a double opt-in through the site, 'migrated' for the addresses
    -- carried over from the Stalwart mailing list on 2026-09-03, whose consent was
    -- recorded there under the old system.
    source        text        NOT NULL DEFAULT 'confirmed'
);

-- What happened, when. `subject` is an HMAC of the address under EVENT_LOG_SECRET,
-- truncated to 16 hex characters -- the same pseudonym the old WebDAV log used, so its
-- rows import unchanged and a future row for the same address matches an old one.
-- The table answers "how many confirmed in March" and "did the holder of this address
-- consent, and when" (recompute the hash and look), without naming anyone.
CREATE TABLE IF NOT EXISTS events (
    id      bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at      timestamptz NOT NULL DEFAULT now(),
    kind    text        NOT NULL CHECK (kind IN ('requested', 'confirmed', 'unsubscribed')),
    subject text        NOT NULL
);
CREATE INDEX IF NOT EXISTS events_at_idx ON events (at);

-- One row per issue sent. The primary key is the lock: a send starts with an INSERT
-- of the slug, and a second send of the same slug fails on the key before a single
-- message goes out. `status` tells a later reader whether the issue went out
-- ('sent'), half went out ('partial', with the addresses in `failed`), or the process
-- died between the claim and the report ('sending'). Re-send deliberately by deleting
-- the row.
CREATE TABLE IF NOT EXISTS sends (
    slug        text        PRIMARY KEY,
    claimed_at  timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    status      text        NOT NULL DEFAULT 'sending'
                            CHECK (status IN ('sending', 'sent', 'partial')),
    recipients  integer     NOT NULL,
    sent        integer     NOT NULL DEFAULT 0,
    failed      text[]      NOT NULL DEFAULT '{}'
);
