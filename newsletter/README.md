# lindfors-newsletter

The newsletter, as one service on `mail.lindfors.no`: the endpoints a reader reaches
(subscribe, confirm, unsubscribe), the send an operator triggers, and the dashboard
that shows what happened. State is in the Postgres on the same box. One binary, one
environment file, one OpenRC service.

It replaced two things on 2026-09-03: a Cloudflare Worker (`api/`, now deleted) that
kept the list in a Stalwart mailing list, the history in WebDAV filenames and the send
lock in an HTTP precondition; and a separate `admin/` service that read those back.
Those were ways of not having a database. This host has one.

## Why self-hosted, why Postgres

- **The failure domain did not get worse.** Every subscribe already ended in a mail
  from Stalwart on this box, so a Worker that was up while the box was down only
  failed later and less clearly. What was lost is the form staying up during a box
  outage, which is a form on a static site.
- **The lock is a real one.** A send starts with an `INSERT` of the slug into `sends`;
  the primary key makes a second send of the same slug fail before a message goes out.
  The Worker's 45-recipient subrequest cap went with the Worker.
- **No secret leaves the host.** Postgres is on loopback, Stalwart on loopback, and
  nothing reachable from outside holds a credential. The Worker held the list password
  in Cloudflare.
- **The public half and the dashboard are one domain model,** so they are one binary:
  the dashboard reads counts from the tables the public half writes, and the JMAP and
  DAV reads it used to do, along with the credential they needed, are gone.

## What is where

| Piece | File |
|---|---|
| Public routes, send, mail templates, HTML pages | `src/public.rs` |
| Signed links (confirm, unsubscribe) and the event pseudonym | `src/tokens.rs` |
| Address and slug validation | `src/validate.rs` |
| Per-key limits in the process | `src/ratelimit.rs` |
| The four tables and every statement run against them | `src/db.rs`, `schema.sql` |
| Addresses sealed at rest | `src/crypto.rs` |
| Sending through Stalwart's JMAP on loopback | `src/mail.rs` |
| Config, router, dashboard handlers | `src/main.rs` |
| Dashboard sign-in: OIDC discovery, userinfo gate | `src/oidc.rs`, `src/auth.rs` |
| Reader analytics out of OpenObserve | `src/rum.rs` |
| The dashboard page, compiled in | `static/` |

Routes, and which nginx name may reach them:

| Route | Name | Guard |
|---|---|---|
| `POST /api/subscribe`, `GET`/`POST /api/confirm`, `GET`/`POST /api/unsubscribe` | `newsletter.lindfors.no` | rate limits; signed tokens where a token is involved |
| `POST /api/send-newsletter` | `admin.lindfors.no` | `ADMIN_KEY` bearer, for `site-tools newsletter send` |
| `/`, `/admin.js`, `/admin-auth.js`, `/api/config`, `/api/overview` | `admin.lindfors.no` | Kanidm, on `/api/overview` |

The public vhost routes exactly three paths and 404s the rest, so the operator routes
are unreachable from the public name however the service is misconfigured.

## The tables

`schema.sql`, applied by `lindfors-newsletter migrate`, which runs as the `newsletter`
role that owns them. **No address is stored in the clear.** Every table that needs one
holds it sealed with XChaCha20-Poly1305 under `DATA_KEY` (`src/crypto.rs`) and keyed by
its pseudonym, the HMAC under `EVENT_LOG_SECRET` that the event log already used. So a
copy of the database, or of a nightly dump, names nobody, and the service decrypts only
when it is about to send or when the dashboard asks who got what. The key exists only
in the environment file; losing it loses the list, so it belongs in the password manager
too.

- **`subscribers`** — who gets the newsletter *now*. An unsubscribe `DELETE`s the row.
  `source` is `confirmed` for a double opt-in, `migrated` for the three addresses
  carried over from the Stalwart list.
- **`deliveries`** — which issue went to whom, one row per recipient per issue, written
  as each message is accepted or refused. This is what a **catch-up send** reads: an
  issue goes to every current subscriber with no `sent` or `assumed` row for it, so an
  old post or a whole series can reach a newcomer without everyone else getting it
  again. `assumed` rows were written by `assume-delivered` for the two issues sent
  before this table existed; delete one to put that address back in the catch-up.
- **`events`** — requested, confirmed, unsubscribed, with a `subject` that is an HMAC
  of the address under `EVENT_LOG_SECRET`, truncated to 16 hex characters. The same
  pseudonym the WebDAV log used, so old rows import unchanged and a future row for the
  same address matches. The table answers "how many confirmed in March" and "did the
  holder of this address consent, and when" (recompute the hash and look) without
  naming anyone. Writing it fails open: refusing someone's unsubscribe because an audit
  row could not be written would be indefensible.
- **`sends`** — one row per issue, the primary key being the lock. `status` says
  `sending` (claimed, then the process died), `sent`, or `partial` with the addresses
  that missed it in `failed`. Re-send deliberately by deleting the row.

## Flows

**Subscribe** mails a signed link and touches no table but `events`. The link is
`confirm:v1:<exp>:<email>` under `CONFIRM_SECRET`, so the pending state lives in the
link and nothing expires in a table. The response is the same whether the address is
new, pending or already subscribed, because anything else is a membership oracle.

**Confirm** is a GET that renders a button and a POST that inserts. Mail scanners fetch
every link they find; a GET that subscribed would let Outlook Safe Links confirm on the
reader's behalf. After the insert, the welcome mail goes out with the recent posts from
`recent.json`, fetched from the published site.

**Unsubscribe** accepts four callers and answers each in kind: the site's JSON form, the
button on the signed page, the typed-address form (a plain form POST, no script), and an
RFC 8058 one-click POST whose only identifying content is the token in the URL. Signed
tokens never expire, because an unsubscribe link sits in a mailbox for as long as the
message does and one that stops working is how a reader who wanted to leave reports the
mail as spam instead. The typed form is the fallback if `CONFIRM_SECRET` is ever rotated.

**Send** fetches `static/newsletter/<slug>.md` from the published site, renders and
sanitises it, claims the slug, then sends one message per recipient with that
recipient's own unsubscribe URL in `List-Unsubscribe` and the footer, recording each
delivery as it happens and the report at the end. A partial send answers 502 with the
addresses to retry, and the claim stands. `site-tools newsletter send <slug>
--catch-up` (`"mode": "catch-up"`) skips the claim when the issue has gone out and
mails only those without a delivery; on an issue never sent it is a full send.

## Rate limits, in two layers

nginx, on the public vhost, keyed on the Cloudflare-supplied client address: 30 a
minute with a burst of 20. The process, per key, in sixty-second windows: five
subscribes per address, two per typed email, fifteen confirm or unsubscribe per
address. The second layer sees the request body, which nginx does not; the first drops
a flood before it becomes a connection.

## Everything is on loopback, and nothing speaks TLS

The binary carries no TLS stack and no C, which is what makes cross-compiling it for
the host a plain `cargo build --target aarch64-unknown-linux-musl` with `rust-lld`
(`.cargo/config.toml`). Four things it talks to, all ports on the same machine:

| Upstream | Port | Note |
|---|---|---|
| Postgres | `127.0.0.1:5432` | scram-sha-256 as `newsletter`; the pg_hba line for it sits above the older blanket `trust` line |
| Stalwart JMAP | `127.0.0.1:8080` | postmaster's app password; `Email/set` + `EmailSubmission/set` |
| Kanidm, through nginx | `127.0.0.1:8447` | kanidmd only speaks TLS; `admin.lindfors.no.conf` fronts it in plain HTTP |
| The site, through nginx | `127.0.0.1:8448` | Cloudflare Pages only speaks TLS; `newsletter.lindfors.no.conf` fronts it the same way, for the issue bodies and `recent.json` |

`plaintext()` in `main.rs` refuses an `https://` value for any of these at startup,
because reqwest would otherwise reject it when the request is built, far from the
line that caused it.

## Deployment

```bash
./build.sh                          # aarch64-unknown-linux-musl, ~7 MB static ELF
scp target/aarch64-unknown-linux-musl/release/lindfors-newsletter hetzner:/tmp/
ssh hetzner 'sudo install -m 755 /tmp/lindfors-newsletter /opt/lindfors-newsletter/ && sudo rc-service lindfors-newsletter restart'
```

First install, on the host as root: `addgroup -S lindfors-newsletter`, `adduser -S -D -H
-s /sbin/nologin -G lindfors-newsletter lindfors-newsletter`, the binary to
`/opt/lindfors-newsletter/`, `lindfors-newsletter.openrc` to `/etc/init.d/`, and
`env.example` filled in at `/etc/lindfors-newsletter.env`, root-owned and 0600. The init
script refuses any other mode, sources the file as root and drops to the service
account. **Values in that file are single-quoted**: it is sourced by a shell, and a
secret containing a space or a semicolon executes half of itself otherwise.

`depend()` needs `postgresql`, and `db.check()` proves the connection and the four
tables at startup, so a missing schema is a refusal to boot with the reason.

Two operator commands run the binary with the same environment sourced and exit:

```bash
sudo sh -c '. /etc/lindfors-newsletter.env; export $(grep -o "^[A-Z0-9_]*" /etc/lindfors-newsletter.env | tr "
" " ");   /opt/lindfors-newsletter/lindfors-newsletter migrate'                       # schema.sql, then seal any plaintext rows
sudo sh -c '...; /opt/lindfors-newsletter/lindfors-newsletter assume-delivered <slug> [email]'   # mark an old issue as delivered

### The Stalwart side

`JMAP_SENDER_USER` is `postmaster@lindfors.no` and `JMAP_SENDER_PASSWORD` is an app
password for it, created in Stalwart's WebUI. `JMAP_ACCOUNT_ID` and `JMAP_IDENTITY_ID`
are that account's ids as `/jmap/session` reports them.

### The Kanidm side

Unchanged from the dashboard's own deployment: public client `lindfors-admin`, PKCE
S256, redirect `https://admin.lindfors.no/`, scope map `openid` for `newsletter_admins`,
`ADMIN_SUBJECT` the account UUID. The service reads the client's discovery document
and never parses a token: it hands the access token back to `userinfo` and the issuer
says. That is also the only check that works against Kanidm's encrypted tokens.

### The site side

`newsletter_endpoint` in `zola.toml` is `https://newsletter.lindfors.no/api/subscribe`,
which appears in `connect-src` and `form-action` in `static/_headers`. `static/_redirects`
sends the old `lindfors.no/api/*` links, which sit in delivered mail, to the new name
with a 308 so a one-click POST stays a POST.

### Backups

`/etc/periodic/daily/postgres-backup` dumps every database into `/var/backups/postgres`,
root-only, fourteen days kept. That is an on-box copy: it survives a bad migration, not
a lost disk. Copying it off the host is the open item in the host's `TODO.md`, and the
subscriber list is the one thing here that cannot be rebuilt.

## Development

```bash
cargo test                                          # 39 tests: tokens, validation, limits, templates, SQL shapes
cargo clippy --all-targets
node --test tests/admin-auth.test.mjs tests/admin-readers.test.mjs
```

The network paths are thin wrappers so the pure halves cover most of the logic. There
is no test against a live Postgres; the smoke test that stood this up ran the binary on
a side port against the real one and is the shape to repeat after a schema change.
