//! The newsletter's state: three tables in the Postgres on this box. `schema.sql` is
//! the authority on their shape; this file is every statement the service runs.
//!
//! Connected over loopback in plaintext with scram-sha-256, as the `newsletter` role,
//! which owns the tables and nothing else. The pool reconnects on its own, so a Postgres
//! restart costs one failed request rather than a service restart.
//!
//! Timestamps come back as ISO 8601 strings rendered by Postgres itself, so no date
//! crate is needed and the JSON the dashboard reads is the shape it always read:
//! `2026-08-29T12:34:56Z`.

use deadpool_postgres::{Config, Pool, Runtime};
use serde::Serialize;
use tokio_postgres::NoTls;

pub struct Db {
    pool: Pool,
}

/// One subscriber-lifecycle event. `subject` is the pseudonym, never an address.
#[derive(Serialize, Debug, PartialEq)]
pub struct EventRecord {
    pub at: String,
    pub event: String,
    pub subject: String,
}

/// Outcome of trying to claim a slug for sending.
pub enum Claim {
    /// Nothing had sent this issue. The claim is now ours.
    Won,
    /// A row already existed, so someone sent it (or is sending it).
    AlreadySent,
}

const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"";

impl Db {
    pub fn connect(url: &str) -> Result<Self, String> {
        let cfg = Config {
            url: Some(url.to_string()),
            ..Default::default()
        };
        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| format!("DATABASE_URL: {e}"))?;
        Ok(Self { pool })
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, String> {
        self.pool
            .get()
            .await
            .map_err(|e| format!("Postgres unavailable: {e}"))
    }

    /// Prove the connection and the schema at startup: a wrong URL or a missing table
    /// is a refusal to boot with the reason, not a 500 on the first subscribe.
    pub async fn check(&self) -> Result<(), String> {
        let c = self.client().await?;
        for table in ["subscribers", "events", "sends"] {
            c.query_one(&format!("SELECT count(*) FROM {table}"), &[])
                .await
                .map_err(|e| format!("table {table}: {e} -- has schema.sql been applied?"))?;
        }
        Ok(())
    }

    /// Add an address. `false` if it was already there, which is a no-op by design: a
    /// second click on a confirmation link, or a resubscribe, changes nothing.
    pub async fn subscribe(&self, email: &str, source: &str) -> Result<bool, String> {
        let c = self.client().await?;
        let n = c
            .execute(
                "INSERT INTO subscribers (email, source) VALUES ($1, $2) ON CONFLICT (email) DO NOTHING",
                &[&email, &source],
            )
            .await
            .map_err(|e| format!("subscribe: {e}"))?;
        Ok(n == 1)
    }

    /// Remove an address. `false` if it was not there, which callers treat as success:
    /// the outcome the person asked for is the state of the table either way.
    pub async fn unsubscribe(&self, email: &str) -> Result<bool, String> {
        let c = self.client().await?;
        let n = c
            .execute("DELETE FROM subscribers WHERE email = $1", &[&email])
            .await
            .map_err(|e| format!("unsubscribe: {e}"))?;
        Ok(n == 1)
    }

    pub async fn recipients(&self) -> Result<Vec<String>, String> {
        let c = self.client().await?;
        let rows = c
            .query("SELECT email FROM subscribers ORDER BY email", &[])
            .await
            .map_err(|e| format!("recipients: {e}"))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }

    pub async fn subscriber_count(&self) -> Result<i64, String> {
        let c = self.client().await?;
        let row = c
            .query_one("SELECT count(*) FROM subscribers", &[])
            .await
            .map_err(|e| format!("count: {e}"))?;
        Ok(row.get(0))
    }

    /// Record an event. Callers never fail the operation that triggered it: refusing
    /// someone's unsubscribe because an audit row could not be written is indefensible.
    pub async fn log_event(&self, kind: &str, subject: &str) -> Result<(), String> {
        let c = self.client().await?;
        c.execute(
            "INSERT INTO events (kind, subject) VALUES ($1, $2)",
            &[&kind, &subject],
        )
        .await
        .map_err(|e| format!("event: {e}"))?;
        Ok(())
    }

    /// Every event, oldest first, in the shape the dashboard has always read.
    pub async fn events(&self) -> Result<Vec<EventRecord>, String> {
        let c = self.client().await?;
        let rows = c
            .query(
                &format!(
                    "SELECT to_char(at AT TIME ZONE 'UTC', '{ISO}'), kind, subject \
                     FROM events ORDER BY at, id"
                ),
                &[],
            )
            .await
            .map_err(|e| format!("events: {e}"))?;
        Ok(rows
            .iter()
            .map(|r| EventRecord {
                at: r.get(0),
                event: r.get(1),
                subject: r.get(2),
            })
            .collect())
    }

    /// Claim `slug`. The primary key is the lock: the insert either happens or
    /// conflicts, atomically, before a single message goes out.
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

    /// Turn the claim into a report. Unconditional: the claim is the gate, this is what
    /// tells a later reader whether the issue actually went out.
    pub async fn record_send(
        &self,
        slug: &str,
        sent: i32,
        failed: &[String],
        ok: bool,
    ) -> Result<(), String> {
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

    /// Slugs of every issue claimed, oldest first.
    pub async fn sends(&self) -> Result<Vec<String>, String> {
        let c = self.client().await?;
        let rows = c
            .query("SELECT slug FROM sends ORDER BY claimed_at", &[])
            .await
            .map_err(|e| format!("sends: {e}"))?;
        Ok(rows.iter().map(|r| r.get(0)).collect())
    }
}
