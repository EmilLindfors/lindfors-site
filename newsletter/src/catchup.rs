//! The weekly catch-up: when no issue went out this week, send the oldest one that
//! some current subscribers have not had.
//!
//! Emil's rule (TODO.md P15): a job runs weekly and mails one thing at most. New posts
//! are the publisher's business (`site-tools publish` on this box mails an issue the
//! moment its post is pushed), so what is left for this job is the archive: an old post
//! or a series reaching the people who joined after it went out, one issue a week,
//! nobody getting anything twice. `deliveries` is what makes that possible; the
//! `sends` table's own order is the order things were published, which is the order a
//! series wants.
//!
//! Invariant: one mail per subscriber per week, welcome and confirmation aside. This
//! job keeps it by doing nothing in a week that already has a row in `sends`, whoever
//! put it there.

use std::collections::HashSet;

use crate::public::{self, SendRefused};
use crate::App;

/// The subject line of an archive issue says that it is one.
pub const ARCHIVE_PREFIX: &str = "From the archive: ";

/// One issue as this job weighs it.
#[derive(Debug, PartialEq)]
pub struct Candidate {
    pub slug: String,
    pub status: String,
    /// Current subscribers with no `sent` or `assumed` delivery of it.
    pub missing: usize,
}

/// Which issue goes: the first, in send order, that some current subscriber lacks. A
/// `partial` from an earlier send is exactly such an issue, so a failed send retries
/// itself here without a special case. A `sending` row is a send in progress, or one
/// that died mid-way and has not been recorded; left alone either way.
pub fn pick(candidates: &[Candidate]) -> Option<&Candidate> {
    candidates
        .iter()
        .find(|c| matches!(c.status.as_str(), "sent" | "partial") && c.missing > 0)
}

/// How many of the current subscribers an issue has not reached.
pub fn missing(subscribers: &[String], delivered: &HashSet<String>) -> usize {
    subscribers.iter().filter(|s| !delivered.contains(*s)).count()
}

/// Decide, and unless `dry_run`, send. The returned line is the whole report; the
/// caller prints it, and cron's redirect puts it in the log.
pub async fn run(app: &App, dry_run: bool) -> Result<String, String> {
    let this_week = app.db.sends_this_week().await?;
    if !this_week.is_empty() {
        return Ok(format!("catch-up: this week already has {}; nothing to send", this_week.join(", ")));
    }

    let subscribers: Vec<String> = app.db.subscribers().await?.into_iter().map(|s| s.subject).collect();
    if subscribers.is_empty() {
        return Ok("catch-up: no subscribers; nothing to send".into());
    }

    let mut candidates = Vec::new();
    for send in app.db.sends().await? {
        let delivered = app.db.delivered_subjects(&send.slug).await?;
        candidates.push(Candidate {
            slug: send.slug,
            status: send.status,
            missing: missing(&subscribers, &delivered),
        });
    }

    let Some(choice) = pick(&candidates) else {
        return Ok(format!(
            "catch-up: every one of {} issue(s) has reached all {} subscriber(s); nothing to send",
            candidates.len(),
            subscribers.len()
        ));
    };

    // The issue must still be on the site, and its title is the subject.
    let title = match public::issue_title(app, &choice.slug).await {
        Ok(title) => title,
        Err(e) => return Err(format!("catch-up: {} is the pick but its issue file is not readable: {e}", choice.slug)),
    };
    let subject = format!("{ARCHIVE_PREFIX}{title}");

    if dry_run {
        return Ok(format!(
            "catch-up (dry run): would send {} as \"{subject}\" to {} of {} subscriber(s)",
            choice.slug,
            choice.missing,
            subscribers.len()
        ));
    }

    match public::send_issue(app, &choice.slug, Some(subject.clone()), true).await {
        Ok(outcome) => Ok(format!(
            "catch-up: sent {} as \"{subject}\": {} sent, {} skipped, {} failed{}",
            choice.slug,
            outcome.sent,
            outcome.skipped,
            outcome.failed.len(),
            if outcome.failed.is_empty() { String::new() } else { format!(" ({})", outcome.failed.join(", ")) }
        )),
        Err(SendRefused { status, message, .. }) => Err(format!("catch-up: send of {} refused ({status}): {message}", choice.slug)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(slug: &str, status: &str, missing: usize) -> Candidate {
        Candidate { slug: slug.into(), status: status.into(), missing }
    }

    #[test]
    fn the_oldest_issue_someone_lacks_goes_first() {
        let cs = vec![c("first", "sent", 0), c("second", "sent", 2), c("third", "sent", 5)];
        assert_eq!(pick(&cs).unwrap().slug, "second");
    }

    #[test]
    fn a_partial_send_retries_itself_and_an_in_flight_one_is_left_alone() {
        let cs = vec![c("stuck", "sending", 3), c("half", "partial", 1), c("new", "sent", 3)];
        assert_eq!(pick(&cs).unwrap().slug, "half");
    }

    #[test]
    fn nothing_to_send_when_everyone_has_everything() {
        let cs = vec![c("a", "sent", 0), c("b", "partial", 0)];
        assert_eq!(pick(&cs), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn missing_counts_current_subscribers_without_a_delivery() {
        let subs: Vec<String> = ["s1", "s2", "s3"].iter().map(|s| s.to_string()).collect();
        let delivered: HashSet<String> = ["s1", "gone-subscriber"].iter().map(|s| s.to_string()).collect();
        assert_eq!(missing(&subs, &delivered), 2);
        assert_eq!(missing(&[], &delivered), 0);
    }
}
