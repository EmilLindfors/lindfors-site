//! The newsletter's state: four tables in the Postgres on this box. `schema.sql` is
//! the authority on their shape; this file is every statement the service runs.
//!
//! Connected over loopback in plaintext with scram-sha-256, as the `newsletter` role,
//! which owns the tables and nothing else. The pool reconnects on its own, so a Postgres
//! restart costs one failed request rather than a service restart.
//!
//! Addresses go in sealed (`crypto.rs`) and are keyed by their pseudonym
//! (`tokens::event_subject`), so every lookup is by pseudonym and nothing is decrypted
//! until a message is about to be sent or the dashboard asks who got what.
//!
//! Timestamps come back as ISO 8601 strings rendered by Postgres itself, so no date
//! crate is needed: `2026-08-29T12:34:56Z`.

use std::collections::HashSet;

use deadpool_postgres::{Config, Pool, Runtime};
use serde::Serialize;
use tokio_postgres::NoTls;

use crate::crypto::Vault;
use crate::tokens;

pub struct Db {
    pool: Pool,
    vault: Vault,
    subject_secret: String,
}

/// One subscriber-lifecycle event. `subject` is the pseudonym, never an address.
#[derive(Serialize, Debug, PartialEq)]
pub struct EventRecord {
    pub at: String,
    pub event: String,
    pub subject: String,
}

/// A current subscriber, decrypted. Only ever handed to the sender and the dashboard.
#[derive(Serialize, Debug)]
pub struct Subscriber {
    pub subject: String,
    pub email: String,
    pub subscribed_at: String,
    pub source: String,
}

/// One issue's record.
#[derive(Serialize, Debug)]
pub struct SendRecord {
    pub slug: String,
    pub claimed_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub recipients: i32,
    pub sent: i32,
    pub failed: Vec<String>,
}

/// Which issue went to whom, decrypted.
#[derive(Serialize, Debug)]
pub struct Delivery {
    pub slug: String,
    pub subject: String,
    pub email: String,
    pub at: String,
    pub status: String,
}

/// Outcome of trying to claim a slug for sending.
pub enum Claim {
    /// Nothing had sent this issue. The claim is now ours.
    Won,
    /// A row already existed, so someone sent it (or is sending it).
    AlreadySent,
}

const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"";

fn iso(col: &str) -> String {
    format!("to_char({col} AT TIME ZONE 'UTC', '{ISO}')")
}

impl Db {
    pub fn connect(url: &str, vault: Vault, subject_secret: String) -> Result<Self, String> {
        let cfg = Config {
            url: Some(url.to_string()),
            ..Default::default()
        };
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("DATABASE_URL: {e}"))?;
        Ok(Self { pool, vault, subject_secret })
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, String> {
        self.pool.get().await.map_err(|e| format!("Postgres unavailable: {e}"))
    }

    /// The pseudonym for an address: the same one the event log uses.
    pub fn subject(&self, email: &str) -> String {
        tokens::event_subject(&self.subject_secret, email)
    }

    /// Prove the connection and the schema at startup: a wrong URL, a missing table or
    /// the pre-encryption shape is a refusal to boot with the reason.
    pub async fn check(&self) -> Result<(), String> {
        let c = self.client().await?;
        for (table, col) in [
            ("subscribers", "email_enc"),
            ("events", "subject"),
            ("sends", "slug"),
            ("deliveries", "email_enc"),
        ] {
            c.query_one(&format!("SELECT count({col}) FROM {table}"), &[])
                .await
                .map_err(|e| format!("table {table}: {e} -- run `lindfors-newsletter migrate`"))?;
        }
        Ok(())
    }

    // -- subscribers -----------------------------------------------------------

    /// Add an address. `false` if it was already there, which is a no-op by design.
    pub async fn subscribe(&self, email: &str, source: &str) -> Result<bool, String> {
        let c = self.client().await?;
        let n = c
            .execute(
                "INSERT INTO subscribers (subject, email_enc, source) VALUES ($1, $2, $3) \
                 ON CONFLICT (subject) DO NOTHING",
                &[&self.subject(email), &self.vault.seal(email), &source],
            )
            .await
            .map_err(|e| format!("subscribe: {e}"))?;
        Ok(n == 1)
    }

    /// Remove an address. `false` if it was not there, which callers treat as success.
    pub async fn unsubscribe(&self, email: &str) -> Result<bool, String> {
        let c = self.client().await?;
        let n = c
            .execute("DELETE FROM subscribers WHERE subject = $1", &[&self.subject(email)])
            .await
            .map_err(|e| format!("unsubscribe: {e}"))?;
        Ok(n == 1)
    }

    /// Every current subscriber, decrypted, oldest first.
    pub async fn subscribers(&self) -> Result<Vec<Subscriber>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                &format!(
                    "SELECT subject, email_enc, {}, source FROM subscribers ORDER BY subscribed_at, subject",
                    iso("subscribed_at")
                ),
                &[],
            )
            .await
            .map_err(|e| format!("subscribers: {e}"))?;
        rows.iter()
            .map(|r| {
                let blob: Vec<u8> = r.get(1);
                Ok(Subscriber {
                    subject: r.get(0),
                    email: self.vault.open(&blob)?,
                    subscribed_at: r.get(2),
                    source: r.get(3),
                })
            })
            .collect()
    }

    pub async fn subscriber_count(&self) -> Result<i64, String> {
        let c = self.client().await?;
        let row = c
            .query_one("SELECT count(*) FROM subscribers", &[])
            .await
            .map_err(|e| format!("count: {e}"))?;
        Ok(row.get(0))
    }

    // -- events ----------------------------------------------------------------

    pub async fn log_event(&self, kind: &str, subject: &str) -> Result<(), String> {
        let c = self.client().await?;
        c.execute("INSERT INTO events (kind, subject) VALUES ($1, $2)", &[&kind, &subject])
            .await
            .map_err(|e| format!("event: {e}"))?;
        Ok(())
    }

    /// Every event, oldest first, in the shape the dashboard has always read.
    pub async fn events(&self) -> Result<Vec<EventRecord>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                &format!("SELECT {}, kind, subject FROM events ORDER BY at, id", iso("at")),
                &[],
            )
            .await
            .map_err(|e| format!("events: {e}"))?;
        Ok(rows
            .iter()
            .map(|r| EventRecord { at: r.get(0), event: r.get(1), subject: r.get(2) })
            .collect())
    }

    // -- sends and deliveries --------------------------------------------------

    /// Claim `slug` for a full send. The primary key is the lock.
    pub async fn claim_send(&self, slug: &str, recipients: i32) -> Result<Claim, String> {
        let c = self.client().await?;
        let n = c
            .execute(
                "INSERT INTO sends (slug, recipients) VALUES ($1, $2) ON CONFLICT (slug) DO NOTHING",
                &[&slug, &recipients],
            )
            .await
            .map_err(|e| format!("claim: {e}"))?;
        Ok(if n == 1 { Claim::Won } else { Claim::AlreadySent })
    }

    /// Turn a full send's claim into its report.
    pub async fn record_send(&self, slug: &str, sent: i32, failed: &[String], ok: bool) -> Result<(), String> {
        let c = self.client().await?;
        let status = if ok { "sent" } else { "partial" };
        c.execute(
            "UPDATE sends SET status = $2, sent = $3, failed = $4, finished_at = now() WHERE slug = $1",
            &[&slug, &status, &sent, &failed],
        )
        .await
        .map_err(|e| format!("record send: {e}"))?;
        Ok(())
    }

    /// Add a catch-up send's outcome to an issue's report. The failures are appended;
    /// the status becomes `partial` if any remain, `sent` otherwise.
    pub async fn record_catch_up(&self, slug: &str, sent_delta: i32, failed: &[String]) -> Result<(), String> {
        let c = self.client().await?;
        c.execute(
            "UPDATE sends SET sent = sent + $2, failed = failed || $3, \
             status = CASE WHEN cardinality(failed || $3) = 0 THEN 'sent' ELSE 'partial' END, \
             finished_at = now() WHERE slug = $1",
            &[&slug, &sent_delta, &failed],
        )
        .await
        .map_err(|e| format!("record catch-up: {e}"))?;
        Ok(())
    }

    /// Record one recipient's outcome for one issue.
    pub async fn record_delivery(&self, slug: &str, email: &str, status: &str) -> Result<(), String> {
        let c = self.client().await?;
        c.execute(
            "INSERT INTO deliveries (slug, subject, email_enc, status) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (slug, subject) DO UPDATE SET status = EXCLUDED.status, at = now()",
            &[&slug, &self.subject(email), &self.vault.seal(email), &status],
        )
        .await
        .map_err(|e| format!("record delivery: {e}"))?;
        Ok(())
    }

    /// The pseudonyms an issue has already reached (`sent` or `assumed`; a failed row
    /// is retried by a catch-up).
    pub async fn delivered_subjects(&self, slug: &str) -> Result<HashSet<String>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                "SELECT subject FROM deliveries WHERE slug = $1 AND status IN ('sent', 'assumed')",
                &[&slug],
            )
            .await
            .map_err(|e| format!("deliveries: {e}"))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn send_exists(&self, slug: &str) -> Result<bool, String> {
        let c = self.client().await?;
        let row = c
            .query_one("SELECT count(*) FROM sends WHERE slug = $1", &[&slug])
            .await
            .map_err(|e| format!("sends: {e}"))?;
        Ok(row.get::<_, i64>(0) > 0)
    }

    /// Every issue, oldest first.
    pub async fn sends(&self) -> Result<Vec<SendRecord>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                &format!(
                    "SELECT slug, {}, {}, status, recipients, sent, failed FROM sends ORDER BY claimed_at",
                    iso("claimed_at"),
                    "CASE WHEN finished_at IS NULL THEN NULL ELSE to_char(finished_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') END"
                ),
                &[],
            )
            .await
            .map_err(|e| format!("sends: {e}"))?;
        Ok(rows
            .iter()
            .map(|r| SendRecord {
                slug: r.get(0),
                claimed_at: r.get(1),
                finished_at: r.get(2),
                status: r.get(3),
                recipients: r.get(4),
                sent: r.get(5),
                failed: r.get(6),
            })
            .collect())
    }

    /// Every delivery, decrypted, oldest first.
    pub async fn deliveries(&self) -> Result<Vec<Delivery>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                &format!("SELECT slug, subject, email_enc, {}, status FROM deliveries ORDER BY at, slug, subject", iso("at")),
                &[],
            )
            .await
            .map_err(|e| format!("deliveries: {e}"))?;
        rows.iter()
            .map(|r| {
                let blob: Vec<u8> = r.get(2);
                Ok(Delivery {
                    slug: r.get(0),
                    subject: r.get(1),
                    email: self.vault.open(&blob)?,
                    at: r.get(3),
                    status: r.get(4),
                })
            })
            .collect()
    }

    // -- operator commands -----------------------------------------------------

    /// Apply `schema.sql`, then the one transition it cannot express: rows in
    /// `subscribers` that still carry a plaintext `email` column become sealed rows
    /// keyed by pseudonym. Safe to run again; it finds nothing to do.
    pub async fn migrate(&self, schema: &str) -> Result<String, String> {
        let mut c = self.client().await?;
        c.batch_execute(schema).await.map_err(|e| format!("schema.sql: {e}"))?;

        let has_plain = c
            .query_one(
                "SELECT count(*) FROM information_schema.columns \
                 WHERE table_name = 'subscribers' AND column_name = 'email'",
                &[],
            )
            .await
            .map_err(|e| e.to_string())?
            .get::<_, i64>(0)
            > 0;
        if !has_plain {
            return Ok("schema applied; addresses already sealed".into());
        }

        let tx = c.transaction().await.map_err(|e| e.to_string())?;
        tx.batch_execute(
            "ALTER TABLE subscribers ADD COLUMN IF NOT EXISTS subject text; \
             ALTER TABLE subscribers ADD COLUMN IF NOT EXISTS email_enc bytea",
        )
        .await
        .map_err(|e| format!("add columns: {e}"))?;
        let rows = tx
            .query("SELECT email FROM subscribers WHERE subject IS NULL", &[])
            .await
            .map_err(|e| e.to_string())?;
        let mut n = 0;
        for row in &rows {
            let email: String = row.get(0);
            tx.execute(
                "UPDATE subscribers SET subject = $1, email_enc = $2 WHERE email = $3",
                &[&self.subject(&email), &self.vault.seal(&email), &email],
            )
            .await
            .map_err(|e| format!("seal row: {e}"))?;
            n += 1;
        }
        tx.batch_execute(
            "ALTER TABLE subscribers DROP CONSTRAINT subscribers_pkey; \
             ALTER TABLE subscribers ADD PRIMARY KEY (subject); \
             ALTER TABLE subscribers ALTER COLUMN email_enc SET NOT NULL; \
             ALTER TABLE subscribers DROP COLUMN email",
        )
        .await
        .map_err(|e| format!("re-key: {e}"))?;
        tx.commit().await.map_err(|e| e.to_string())?;
        Ok(format!("schema applied; {n} address(es) sealed and the table re-keyed"))
    }

    /// Mark an issue as already delivered to every current subscriber (or one address),
    /// for issues sent before `deliveries` existed. Creates the `sends` row if the
    /// issue has none. Returns how many rows were written.
    pub async fn assume_delivered(&self, slug: &str, only: Option<&str>) -> Result<usize, String> {
        let subscribers = self.subscribers().await?;
        let targets: Vec<&Subscriber> = subscribers
            .iter()
            .filter(|s| only.is_none_or(|e| e == s.email))
            .collect();
        let c = self.client().await?;
        c.execute(
            "INSERT INTO sends (slug, recipients, sent, status, finished_at) \
             VALUES ($1, $2, $2, 'sent', now()) ON CONFLICT (slug) DO NOTHING",
            &[&slug, &(targets.len() as i32)],
        )
        .await
        .map_err(|e| format!("sends: {e}"))?;
        let mut n = 0;
        for s in targets {
            n += c
                .execute(
                    "INSERT INTO deliveries (slug, subject, email_enc, status) VALUES ($1, $2, $3, 'assumed') \
                     ON CONFLICT (slug, subject) DO NOTHING",
                    &[&slug, &s.subject, &self.vault.seal(&s.email)],
                )
                .await
                .map_err(|e| format!("deliveries: {e}"))? as usize;
        }
        Ok(n)
    }
}
