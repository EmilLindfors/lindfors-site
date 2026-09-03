-- lindfors-newsletter: the newsletter's state, in the Postgres that already runs on
-- mail.lindfors.no. Applied by the service itself, as the `newsletter` role that owns
-- every table:
--
--   sudo sh -c '. /etc/lindfors-newsletter.env; export $(cut -d= -f1 /etc/lindfors-newsletter.env | grep -v "^#"); /opt/lindfors-newsletter/lindfors-newsletter migrate'
--
-- Idempotent: every statement is IF NOT EXISTS, so re-running it after a change adds
-- what is new and touches nothing else. The one transition that is not expressible
-- this way -- plaintext addresses becoming sealed ones, 2026-09-03 -- lives in
-- `db::migrate` and runs once.
--
-- Addresses are never stored in the clear. Every table that needs one holds it sealed
-- under DATA_KEY (`crypto.rs`) and keyed by its pseudonym: the HMAC of the address
-- under EVENT_LOG_SECRET, 16 hex characters (`tokens::event_subject`). One pseudonym
-- per address everywhere, so the tables join on it and nothing is decrypted to look
-- something up.

-- Who gets the newsletter, now. Only current subscribers are here: an unsubscribe
-- DELETEs the row rather than flagging it. What went to whom stays in `deliveries`.
CREATE TABLE IF NOT EXISTS subscribers (
    subject       text        PRIMARY KEY,
    email_enc     bytea       NOT NULL,
    subscribed_at timestamptz NOT NULL DEFAULT now(),
    -- 'confirmed' for a double opt-in through the site, 'migrated' for the addresses
    -- carried over from the Stalwart mailing list on 2026-09-03, whose consent was
    -- recorded there under the old system.
    source        text        NOT NULL DEFAULT 'confirmed'
);

-- What happened, when. Same pseudonym as the old WebDAV log used, so its rows import
-- unchanged. The table answers "how many confirmed in March" without naming anyone.
CREATE TABLE IF NOT EXISTS events (
    id      bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at      timestamptz NOT NULL DEFAULT now(),
    kind    text        NOT NULL CHECK (kind IN ('requested', 'confirmed', 'unsubscribed')),
    subject text        NOT NULL
);
CREATE INDEX IF NOT EXISTS events_at_idx ON events (at);

-- One row per issue sent. The primary key is the lock: a full send starts with an
-- INSERT of the slug, and a second full send of the same slug fails on the key before
-- a single message goes out. `status` tells a later reader whether the issue went out
-- ('sent'), half went out ('partial', with the addresses in `failed`), or the process
-- died between the claim and the report ('sending'). Deleting the row deletes its
-- deliveries too; that is how a full re-send is done deliberately.
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

-- Which issue went to whom. One row per recipient per issue, the address sealed. This
-- is what a catch-up send reads: an issue goes to every current subscriber without a
-- row here for it. 'assumed' marks rows written by hand for issues sent before this
-- table existed; delete one to make the catch-up include that address.
CREATE TABLE IF NOT EXISTS deliveries (
    slug      text        NOT NULL REFERENCES sends (slug) ON DELETE CASCADE,
    subject   text        NOT NULL,
    email_enc bytea       NOT NULL,
    at        timestamptz NOT NULL DEFAULT now(),
    status    text        NOT NULL CHECK (status IN ('sent', 'failed', 'assumed')),
    PRIMARY KEY (slug, subject)
);
CREATE INDEX IF NOT EXISTS deliveries_subject_idx ON deliveries (subject);
