//! What the readers did, out of OpenObserve.
//!
//! The public site's `rum.js` sends page views, web vitals and link clicks to
//! OpenObserve on this same host (stream `_rumdata`, org `default`), for the readers who
//! pressed Allow on the consent bar. This module asks that store a handful of questions
//! over its SQL search API and hands the rows to the page. The questions are the ones in
//! `scripts/rum-queries.sql` in the site repo, minus the per-session paths, which name
//! individual visits and have no business on a dashboard.
//!
//! Aggregation happens in the store — that is what a column store is for — and nothing
//! is interpreted here. `admin.js` decides what a number means.
//!
//! **This is optional.** Leave `O2_URL` unset and the section does not exist: the
//! newsletter half of the page never depended on OpenObserve and should not start to.
//! When it is set, the other three variables are required, and every query failing
//! independently reports into `errors` like the JMAP and DAV reads do.
//!
//! The credential is a dedicated service account in the read-only `agent_readonly`
//! role, not the root user that also ingests: a credential that can read the telemetry
//! must not be one that can alter it. Basic auth of `user:token`, assembled here, the
//! same way the Stalwart credential is.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct RumConfig {
    /// `http://127.0.0.1:5080`, plaintext on loopback.
    pub base_url: String,
    pub org: String,
    pub user: String,
    pub token: String,
}

/// How far back the tables look. Web traffic on a personal blog is thin, and a month
/// is long enough for a table of pages to say something.
pub const TABLE_DAYS: i64 = 30;
/// How far back the daily series goes: twelve weeks, matching the newsletter chart.
pub const SERIES_DAYS: i64 = 84;

const STREAM: &str = "\"_rumdata\"";

/// Everything the page draws, as rows straight out of the store.
#[derive(Serialize, Default)]
pub struct RumOverview {
    /// ISO date the tables start at.
    pub since: String,
    /// Daily `{day, views, sessions}` over `SERIES_DAYS`.
    pub daily: Vec<Value>,
    /// `{url, views, sessions}`.
    pub pages: Vec<Value>,
    /// `{referrer, sessions}` for sessions' first page loads.
    pub referrers: Vec<Value>,
    /// `{href, internal, clicks}` from the site's own `link` action.
    pub links: Vec<Value>,
    /// `{country, sessions}`.
    pub countries: Vec<Value>,
    /// `{issue, views, sessions}`: landing views whose URL carried `?issue=<slug>`,
    /// which every link in a newsletter issue does. Per issue, never per reader.
    pub issues: Vec<Value>,
    /// One row: `{lcp_p75, fcp_p75, ttfb_p75, views}`, in nanoseconds as the SDK sends.
    pub vitals: Vec<Value>,
}

#[derive(Serialize)]
struct SearchRequest<'a> {
    query: Query<'a>,
}

#[derive(Serialize)]
struct Query<'a> {
    sql: &'a str,
    start_time: i64,
    end_time: i64,
    size: usize,
}

#[derive(Deserialize)]
struct SearchResponse {
    hits: Vec<Value>,
}

fn basic_auth(user: &str, token: &str) -> String {
    let pair = format!("{user}:{token}");
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(pair)
    )
}

/// The search endpoint for an org, tolerating a trailing slash on the base URL.
pub fn search_url(base_url: &str, org: &str) -> String {
    format!("{}/api/{}/_search", base_url.trim_end_matches('/'), org)
}

/// Microseconds since the epoch, which is what OpenObserve's `start_time` and
/// `end_time` are in. `_timestamp` in the rows is the same unit.
fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

const DAY_MICROS: i64 = 86_400_000_000;

async fn search(
    client: &reqwest::Client,
    config: &RumConfig,
    sql: &str,
    days: i64,
    size: usize,
) -> Result<Vec<Value>, String> {
    let end_time = now_micros();
    let request = SearchRequest {
        query: Query {
            sql,
            start_time: end_time - days * DAY_MICROS,
            end_time,
            size,
        },
    };
    let response = client
        .post(search_url(&config.base_url, &config.org))
        .header("Authorization", basic_auth(&config.user, &config.token))
        .header("Accept", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("OpenObserve unreachable: {e}"))?;

    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        return Err("OpenObserve refused the service account".to_string());
    }
    if status != 200 {
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(200).collect();
        return Err(format!("OpenObserve answered {status}: {body}"));
    }
    response
        .json::<SearchResponse>()
        .await
        .map(|r| r.hits)
        .map_err(|e| format!("OpenObserve returned unexpected JSON: {e}"))
}

/// Views of a local build are not readers. `zola serve` on the workstation runs the same
/// rum.js with the same keys, so its rows land in the same stream, and the only thing
/// that tells them apart is the page's own origin. Every question below leaves them out.
const NOT_LOCAL: &str =
    "view_url NOT LIKE '%://localhost%' AND view_url NOT LIKE '%://127.0.0.1%'";

/// The SQL, in one place so the page's columns and the store's agree by reading.
pub mod sql {
    use super::{NOT_LOCAL, STREAM};

    pub fn daily() -> String {
        format!(
            "SELECT histogram(_timestamp, '1 day') AS day, \
             count(DISTINCT view_id) AS views, count(DISTINCT session_id) AS sessions \
             FROM {STREAM} WHERE type = 'view' AND {NOT_LOCAL} \
             GROUP BY day ORDER BY day"
        )
    }

    /// One row per view event; a long read that flushes several times produces
    /// several rows for the same `view_id`, so views are counted distinct. The query
    /// string is dropped first, so a landing from a newsletter issue (`?issue=...`)
    /// counts with the page it landed on rather than as a page of its own.
    pub fn pages() -> String {
        format!(
            "SELECT regexp_replace(view_url, '\\?.*$', '') AS url, \
             count(DISTINCT view_id) AS views, count(DISTINCT session_id) AS sessions \
             FROM {STREAM} WHERE type = 'view' AND {NOT_LOCAL} \
             GROUP BY url ORDER BY views DESC LIMIT 20"
        )
    }

    /// Visits that started from a newsletter issue: the `?issue=<slug>` that
    /// `site-tools newsletter gen` puts on every link in one. The same parameter for
    /// every recipient, so this counts readers an issue brought, and nothing about who.
    pub fn issues() -> String {
        format!(
            "SELECT regexp_replace(view_url, '^.*[?&]issue=([^&#]+).*$', '\\1') AS issue, \
             count(DISTINCT view_id) AS views, count(DISTINCT session_id) AS sessions \
             FROM {STREAM} WHERE type = 'view' AND view_url LIKE '%issue=%' AND {NOT_LOCAL} \
             GROUP BY issue ORDER BY views DESC LIMIT 20"
        )
    }

    /// Where sessions come from: the referrer on a session's first page load only,
    /// since every later view's referrer is this site. A local build linking here is
    /// not a source either, so the referrer gets the same exclusion as the page.
    pub fn referrers() -> String {
        format!(
            "SELECT view_referrer AS referrer, count(DISTINCT session_id) AS sessions \
             FROM {STREAM} WHERE type = 'view' AND view_loading_type = 'initial_load' \
             AND {NOT_LOCAL} \
             AND view_referrer NOT LIKE '%://localhost%' \
             AND view_referrer NOT LIKE '%://127.0.0.1%' \
             GROUP BY view_referrer ORDER BY sessions DESC LIMIT 20"
        )
    }

    /// The site's own `link` action, recorded by rum.js because the SDK loses clicks
    /// that navigate. Its context flattens to `context_href` and `context_internal`.
    pub fn links() -> String {
        format!(
            "SELECT context_href AS href, context_internal AS internal, count(*) AS clicks \
             FROM {STREAM} WHERE type = 'action' AND action_target_name = 'link' \
             AND {NOT_LOCAL} \
             GROUP BY context_href, context_internal ORDER BY clicks DESC LIMIT 20"
        )
    }

    pub fn countries() -> String {
        format!(
            "SELECT geo_info_country AS country, count(DISTINCT session_id) AS sessions \
             FROM {STREAM} WHERE type = 'view' AND {NOT_LOCAL} \
             GROUP BY geo_info_country ORDER BY sessions DESC LIMIT 10"
        )
    }

    /// 75th percentiles, which is how web vitals are reported everywhere else, over
    /// views that measured a paint at all (a view abandoned before first paint sends
    /// zero, and would drag the figure toward a number no reader saw).
    pub fn vitals() -> String {
        format!(
            "SELECT approx_percentile_cont(view_largest_contentful_paint, 0.75) AS lcp_p75, \
             approx_percentile_cont(view_first_contentful_paint, 0.75) AS fcp_p75, \
             approx_percentile_cont(view_first_byte, 0.75) AS ttfb_p75, \
             count(DISTINCT view_id) AS views \
             FROM {STREAM} WHERE type = 'view' AND view_largest_contentful_paint > 0 \
             AND {NOT_LOCAL}"
        )
    }
}

/// Ask every question at once. Each answer is independent; a failed one is an entry in
/// `errors` and an empty table, never a missing section.
pub async fn overview(
    client: &reqwest::Client,
    config: &RumConfig,
    errors: &mut Vec<String>,
) -> RumOverview {
    let queries = (
        sql::daily(),
        sql::pages(),
        sql::referrers(),
        sql::links(),
        sql::countries(),
        sql::vitals(),
        sql::issues(),
    );
    let (daily, pages, referrers, links, countries, vitals, issues) = tokio::join!(
        search(client, config, &queries.0, SERIES_DAYS, 200),
        search(client, config, &queries.1, TABLE_DAYS, 20),
        search(client, config, &queries.2, TABLE_DAYS, 20),
        search(client, config, &queries.3, TABLE_DAYS, 20),
        search(client, config, &queries.4, TABLE_DAYS, 10),
        search(client, config, &queries.5, TABLE_DAYS, 1),
        search(client, config, &queries.6, TABLE_DAYS, 20),
    );

    let mut take = |name: &str, result: Result<Vec<Value>, String>| match result {
        Ok(rows) => rows,
        Err(e) => {
            errors.push(format!("readers ({name}): {e}"));
            Vec::new()
        }
    };

    let since_micros = now_micros() - TABLE_DAYS * DAY_MICROS;
    RumOverview {
        since: iso_date(since_micros),
        daily: take("daily", daily),
        pages: take("pages", pages),
        referrers: take("referrers", referrers),
        links: take("links", links),
        countries: take("countries", countries),
        vitals: take("vitals", vitals),
        issues: take("issues", issues),
    }
}

/// `YYYY-MM-DD` for microseconds since the epoch, UTC. Civil-from-days as in Howard
/// Hinnant's algorithm; no date crate for one date.
pub fn iso_date(micros: i64) -> String {
    let days = micros.div_euclid(DAY_MICROS);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SQL reaches OpenObserve as text, so the escapes have to come out right in
    /// the string, not just in the Rust source: one backslash before `?` and `1`.
    #[test]
    fn pages_group_on_the_url_without_its_query_and_issues_extract_the_slug() {
        let pages = sql::pages();
        assert!(pages.contains("regexp_replace(view_url, '\\?.*$', '') AS url"), "{pages}");
        assert!(pages.contains("GROUP BY url"));
        let issues = sql::issues();
        assert!(issues.contains("issue=([^&#]+).*$', '\\1') AS issue"), "{issues}");
        assert!(issues.contains("view_url LIKE '%issue=%'"));
        assert!(issues.contains("GROUP BY issue"));
    }

    #[test]
    fn the_search_url_is_the_org_search_endpoint() {
        assert_eq!(
            search_url("http://127.0.0.1:5080", "default"),
            "http://127.0.0.1:5080/api/default/_search"
        );
        assert_eq!(
            search_url("http://127.0.0.1:5080/", "default"),
            "http://127.0.0.1:5080/api/default/_search"
        );
    }

    /// The same shape `oo` builds: base64 of `user:token`, never a pre-encoded blob.
    #[test]
    fn basic_auth_is_user_colon_token() {
        assert_eq!(basic_auth("a@b", "t"), "Basic YUBiOnQ=");
    }

    /// The epoch, a leap day, and the day the RUM stream first had rows in it.
    #[test]
    fn dates_come_out_of_microseconds() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(951_782_400 * 1_000_000), "2000-02-29");
        assert_eq!(iso_date(1_788_379_140_639_123), "2026-09-02");
    }

    /// Every query names the stream and only the stream; a typo here is a 400 from
    /// the store on every page load, which is the sort of thing a test can catch.
    #[test]
    fn every_query_reads_the_rum_stream() {
        for q in [
            sql::daily(),
            sql::pages(),
            sql::referrers(),
            sql::links(),
            sql::countries(),
            sql::vitals(),
        ] {
            assert!(q.contains("FROM \"_rumdata\""), "{q}");
            assert!(!q.contains("session_id,"), "no per-session listing: {q}");
            // A local build sends the same rows; every question leaves them out.
            assert!(q.contains("view_url NOT LIKE '%://localhost%'"), "{q}");
            assert!(q.contains("view_url NOT LIKE '%://127.0.0.1%'"), "{q}");
        }
    }

    /// The request body is what OpenObserve's `_search` expects: a `query` object with
    /// the SQL and a microsecond window.
    #[test]
    fn the_request_body_has_the_search_shape() {
        let body = serde_json::to_value(SearchRequest {
            query: Query {
                sql: "SELECT 1",
                start_time: 1,
                end_time: 2,
                size: 3,
            },
        })
        .unwrap();
        assert_eq!(body["query"]["sql"], "SELECT 1");
        assert_eq!(body["query"]["start_time"], 1);
        assert_eq!(body["query"]["end_time"], 2);
        assert_eq!(body["query"]["size"], 3);
    }
}
