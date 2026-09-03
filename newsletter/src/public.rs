//! The endpoints a reader reaches, and the send an operator triggers.
//!
//! Ported from the Cloudflare Worker in `api/` with the mechanics kept and the storage
//! replaced: the list is a table, the send claim is a primary key, the event log is a
//! table, and Stalwart is reached over loopback. What did not change, and why, is in
//! the comments that came along.
//!
//! Three kinds of response leave here. JSON, for the site's form, with CORS headers for
//! lindfors.no. HTML pages, for the links in mail -- confirm, unsubscribe, and their
//! outcomes -- which are the first thing a new subscriber sees after the message itself
//! and should not look like they belong to a different site. And the send report.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{RawQuery, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::db::Claim;
use crate::mail;
use crate::ratelimit::{API_PER_IP, SUBSCRIBE_PER_EMAIL, SUBSCRIBE_PER_IP};
use crate::tokens::{self, TokenState};
use crate::validate;
use crate::App;

#[derive(Deserialize)]
struct EmailRequest {
    email: String,
}

#[derive(Deserialize)]
struct SendRequest {
    slug: String,
    subject: Option<String>,
    /// `all` (the default): a full send, refused if the issue has gone out before.
    /// `catch-up`: only to current subscribers with no delivery of this issue, which
    /// is how a newcomer gets an old post or a series without everyone else getting
    /// it twice. A catch-up of an issue never sent is a full send.
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// The result of a per-recipient send. `failed` exists because the send is not one
/// atomic message: it can half-succeed, and the operator needs to know whom to retry.
#[derive(Serialize)]
struct SendResponse {
    success: bool,
    sent: usize,
    /// Subscribers a catch-up left alone because they already had the issue.
    #[serde(skip_serializing_if = "is_zero")]
    skipped: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn send_failure(status: u16, error: String) -> Response {
    json(status, &SendResponse { success: false, sent: 0, skipped: 0, failed: vec![], error: Some(error) })
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// The origin the site's form may call this from. The request's own `Origin` is echoed
/// only when it is one of ours; anything else gets the canonical one, which the browser
/// will then refuse to read -- the right outcome for a foreign page.
fn cors_origin(app: &App, headers: &HeaderMap) -> String {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if app.config.cors_origins.iter().any(|o| o == origin) {
        origin.to_string()
    } else {
        app.config.cors_origins[0].clone()
    }
}

fn with_cors(mut response: Response, origin: &str) -> Response {
    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(origin) {
        h.insert(HeaderName::from_static("access-control-allow-origin"), v);
    }
    h.insert(
        HeaderName::from_static("access-control-allow-methods"),
        HeaderValue::from_static("POST, GET, OPTIONS"),
    );
    h.insert(
        HeaderName::from_static("access-control-allow-headers"),
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    h.insert(header::VARY, HeaderValue::from_static("Origin"));
    response
}

fn json<T: Serialize>(status: u16, body: &T) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), Json(body)).into_response()
}

fn api_error(status: u16, message: &str) -> Response {
    json(status, &ApiResponse { success: false, error: Some(message.into()) })
}

fn api_ok() -> Response {
    json(200, &ApiResponse { success: true, error: None })
}

/// An HTML page, with the headers every page here carries: no indexing, no framing.
fn page(status: u16, title: &str, body: &str) -> Response {
    let mut response = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Html(page_shell(title, body)),
    )
        .into_response();
    let h = response.headers_mut();
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(HeaderName::from_static("x-robots-tag"), HeaderValue::from_static("noindex, nofollow"));
    // The pages carry no script and one inline style block.
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    response
}

/// The client's address, as nginx passes it. Behind the Cloudflare proxy nginx sets
/// this from `CF-Connecting-IP`, which the edge sets and a client cannot; on loopback
/// it is absent and every header-less request shares one bucket, the conservative
/// direction.
fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn pairs(s: &str) -> impl Iterator<Item = (String, String)> + '_ {
    form_urlencoded::parse(s.as_bytes()).into_owned()
}

/// Record an event. Never fails the operation that triggered it: refusing someone's
/// unsubscribe because an audit row could not be written is indefensible.
async fn log_event(app: &App, kind: &str, email: &str) {
    let subject = tokens::event_subject(&app.config.event_log_secret, email);
    if let Err(e) = app.db.log_event(kind, &subject).await {
        eprintln!("event not recorded ({kind}): {e}");
    }
}

// ---------------------------------------------------------------------------
// Subscribe
// ---------------------------------------------------------------------------

pub async fn preflight(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let origin = cors_origin(&app, &headers);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        HeaderName::from_static("access-control-max-age"),
        HeaderValue::from_static("86400"),
    );
    with_cors(response, &origin)
}

/// POST /api/subscribe -- mail a signed confirmation link. Does **not** touch the list.
///
/// The response is identical whether the address is already subscribed, newly pending,
/// or nonexistent. Reporting "already subscribed" would turn this into a membership
/// oracle for any address a stranger cares to type, and the uniform answer costs
/// nothing: confirming an address that is already on the list is a no-op.
pub async fn subscribe(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let origin = cors_origin(&app, &headers);
    let response = subscribe_inner(&app, &headers, &body).await;
    with_cors(response, &origin)
}

async fn subscribe_inner(app: &App, headers: &HeaderMap, body: &[u8]) -> Response {
    let Ok(parsed) = serde_json::from_slice::<EmailRequest>(body) else {
        return api_error(400, "Invalid request body");
    };
    let email = validate::normalise(&parsed.email);
    if !validate::is_valid_email(&email) {
        return api_error(400, "Invalid email address");
    }

    // Two limiters, because they bound different attacks and neither covers the other.
    // The per-address check runs second so a spraying client is rejected before its
    // target's own budget is spent.
    let now = tokens::now_secs();
    if !app.limiter.allow(&format!("sub-ip:{}", client_ip(headers)), SUBSCRIBE_PER_IP, now)
        || !app.limiter.allow(&format!("sub-email:{email}"), SUBSCRIBE_PER_EMAIL, now)
    {
        return api_error(429, "Too many requests — please wait a minute and try again.");
    }

    let exp = now + tokens::CONFIRM_TTL_SECS;
    let sig = tokens::confirm_signature(&app.config.confirm_secret, &email, exp);
    let link = tokens::confirm_link(&app.config.public_url, &email, exp, &sig);

    match mail::send(
        &app.client,
        &app.config.sender,
        &email,
        "Confirm your subscription to lindfors.no",
        &confirmation_email_template(&link, &app.config.site_url),
        None,
    )
    .await
    {
        Ok(()) => {
            // The address is deliberately absent from this line. The log records that a
            // confirmation went out, not who is mid-signup.
            println!("confirmation mail sent");
            log_event(app, "requested", &email).await;
            api_ok()
        }
        Err(e) => {
            eprintln!("confirmation send failed: {e}");
            api_error(502, "Could not send the confirmation email. Please try again.")
        }
    }
}

// ---------------------------------------------------------------------------
// Confirm
// ---------------------------------------------------------------------------

/// GET /api/confirm -- render the confirmation button. **Performs nothing.**
///
/// The subscription happens on POST, because links in mail get fetched by things that
/// are not the recipient: Outlook Safe Links, corporate scanners, any client that
/// prefetches. A GET that subscribed would let those confirm on the reader's behalf,
/// which is precisely the human act this whole mechanism exists to require.
pub async fn confirm_page(State(app): State<Arc<App>>, RawQuery(query): RawQuery) -> Response {
    let Some(token) = tokens::parse_confirm_token(pairs(query.as_deref().unwrap_or(""))) else {
        return confirm_error_page(TokenState::Invalid);
    };
    match tokens::check_confirm_token(&app.config.confirm_secret, &token, tokens::now_secs()) {
        TokenState::Valid => page(
            200,
            "Confirm your subscription",
            &format!(
                r#"<h1>Confirm your subscription</h1>
<p>One click and <strong>{email}</strong> is on the list for new posts from lindfors.no.</p>
<form method="POST" action="/api/confirm">
    <input type="hidden" name="email" value="{email}">
    <input type="hidden" name="exp" value="{exp}">
    <input type="hidden" name="sig" value="{sig}">
    <button type="submit">Confirm subscription</button>
</form>
<p class="note">Didn&rsquo;t sign up? Close this page. Nothing has been added, and the link expires on its own.</p>"#,
                email = html_escape(&token.email),
                exp = token.exp,
                sig = html_escape(&token.sig),
            ),
        ),
        state => confirm_error_page(state),
    }
}

/// POST /api/confirm -- verify the signed link and add the address to the list.
///
/// This is the step that makes the consent demonstrable: reaching it requires having
/// read a message delivered to the address, and having clicked. A second click is
/// harmless: the insert is a no-op on conflict.
pub async fn confirm(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: String,
) -> Response {
    let now = tokens::now_secs();
    if !app.limiter.allow(&format!("confirm-ip:{}", client_ip(&headers)), API_PER_IP, now) {
        return page(429, "Too many requests", "<h1>Too many requests</h1><p>Please wait a minute and click the link again.</p>");
    }

    // The form posts the token in the body; the query string is accepted too, so the
    // endpoint stays usable from curl without a form encoding.
    let token = tokens::parse_confirm_token(pairs(&body))
        .or_else(|| tokens::parse_confirm_token(pairs(query.as_deref().unwrap_or(""))));
    let Some(token) = token else {
        return confirm_error_page(TokenState::Invalid);
    };
    match tokens::check_confirm_token(&app.config.confirm_secret, &token, now) {
        TokenState::Valid => {}
        state => return confirm_error_page(state),
    }
    let email = validate::normalise(&token.email);

    match app.db.subscribe(&email, "confirmed").await {
        Ok(_) => {
            println!("subscription confirmed");
            log_event(&app, "confirmed", &email).await;
            send_welcome(&app, &email).await;
            page(
                200,
                "You are subscribed",
                r#"<h1>You&rsquo;re subscribed</h1>
<p>New posts will arrive by email. Every issue carries an unsubscribe link, and you can also <a href="/api/unsubscribe">unsubscribe here</a> at any time.</p>"#,
            )
        }
        Err(e) => {
            eprintln!("confirm failed: {e}");
            page(
                502,
                "Something went wrong",
                r#"<h1>Something went wrong</h1>
<p>The link was valid, but the subscription could not be saved. Please try again in a few minutes &mdash; the link keeps working until it expires.</p>"#,
            )
        }
    }
}

fn confirm_error_page(state: TokenState) -> Response {
    match state {
        TokenState::Expired => page(
            410,
            "Link expired",
            r#"<h1>Link expired</h1>
<p>Confirmation links are good for 48 hours, and this one is past that. Nothing was added.</p>
<p><a href="https://lindfors.no">Sign up again on lindfors.no</a> and a fresh link will be on its way.</p>"#,
        ),
        // An expired link is told so above; everything else gets one answer, so a
        // forged link learns nothing about which part of it was wrong.
        _ => page(
            400,
            "Invalid link",
            r#"<h1>Invalid link</h1>
<p>This confirmation link isn&rsquo;t valid. It may have been altered in transit, or truncated by a mail client that wrapped the line.</p>
<p><a href="https://lindfors.no">Sign up again on lindfors.no</a> to get a fresh one.</p>"#,
        ),
    }
}

// ---------------------------------------------------------------------------
// Unsubscribe
// ---------------------------------------------------------------------------

/// GET /api/unsubscribe -- with a signed token, a one-button confirmation; without one,
/// the typed-address form. **Performs nothing either way**: this URL is printed in the
/// footer of every newsletter, and mail scanners fetch every link they find.
pub async fn unsubscribe_page(State(app): State<Arc<App>>, RawQuery(query): RawQuery) -> Response {
    let token = tokens::parse_unsubscribe_token(pairs(query.as_deref().unwrap_or("")))
        .filter(|(email, sig)| tokens::unsubscribe_signature_matches(&app.config.confirm_secret, email, sig));

    // An absent or bad token is not an error worth a page of its own: fall through to
    // the form, which needs no token and is the permanent way back for anyone whose
    // link has been mangled or whose secret was rotated.
    let Some((email, sig)) = token else {
        return page(200, "Unsubscribe", UNSUBSCRIBE_FORM);
    };
    page(
        200,
        "Unsubscribe",
        &format!(
            r#"<h1>Unsubscribe</h1>
<p>Stop sending new posts to <strong>{email}</strong>?</p>
<form method="POST" action="/api/unsubscribe">
    <input type="hidden" name="email" value="{email}">
    <input type="hidden" name="sig" value="{sig}">
    <button type="submit">Unsubscribe</button>
</form>
<p class="note">You can resubscribe any time from the site.</p>"#,
            email = html_escape(&email),
            sig = html_escape(&sig),
        ),
    )
}

/// The typed-address form. A plain form POST rather than a script, so the page can be
/// served under a policy that allows no script at all.
const UNSUBSCRIBE_FORM: &str = r#"<h1>Unsubscribe</h1>
<p>Enter your email to unsubscribe from the lindfors.no newsletter.</p>
<form method="POST" action="/api/unsubscribe">
    <input type="email" name="email" placeholder="your@email.com" required aria-label="Email address">
    <button type="submit">Unsubscribe</button>
</form>"#;

/// POST /api/unsubscribe -- remove an address from the list.
///
/// Four kinds of caller reach this and disagree about everything except the outcome:
///   - the site's form, sending JSON `{"email": "..."}` and expecting JSON back
///   - the button on the signed page above, posting `email` and `sig` as form fields
///   - the typed-address form, posting `email` alone as a form field
///   - an RFC 8058 one-click unsubscribe, posting the fixed body
///     `List-Unsubscribe=One-Click` with the token in the *URL*
///
/// Deliberately unauthenticated in the untokened cases. Subscribing needs proof of
/// control because it creates an obligation; unsubscribing removes one, and the worst
/// an unauthenticated caller achieves is stopping mail to an address that is not
/// theirs. A subscriber who cannot leave easily reports the mail as spam instead.
pub async fn unsubscribe(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    body: String,
) -> Response {
    let origin = cors_origin(&app, &headers);
    let now = tokens::now_secs();
    if !app.limiter.allow(&format!("unsub-ip:{}", client_ip(&headers)), API_PER_IP, now) {
        return with_cors(api_error(429, "Too many requests — please wait a minute and try again."), &origin);
    }

    // A signed token beats the body: it identifies the recipient without trusting the
    // caller to name themselves, and it is the only thing a one-click POST provides.
    let signed = tokens::parse_unsubscribe_token(pairs(&body))
        .or_else(|| tokens::parse_unsubscribe_token(pairs(query.as_deref().unwrap_or(""))))
        .filter(|(email, sig)| tokens::unsubscribe_signature_matches(&app.config.confirm_secret, email, sig));

    if let Some((email, _)) = signed {
        let email = validate::normalise(&email);
        return match app.db.unsubscribe(&email).await {
            Ok(_) => {
                println!("unsubscribed via signed link");
                log_event(&app, "unsubscribed", &email).await;
                // HTML, even for the one-click POST. RFC 8058 says the response body is
                // never shown to the user, so a page costs the machine nothing and is
                // what the human pressing the button needs.
                unsubscribed_page()
            }
            Err(e) => {
                eprintln!("signed unsubscribe failed: {e}");
                page(502, "Something went wrong", "<h1>Something went wrong</h1><p>The link was valid, but the change could not be saved. Please try again in a few minutes.</p>")
            }
        };
    }

    // The typed-address form: `email` alone, form-encoded, answered with a page.
    let is_form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/x-www-form-urlencoded"));
    if is_form {
        let typed = pairs(&body).find(|(k, _)| k == "email").map(|(_, v)| validate::normalise(&v));
        let Some(email) = typed.filter(|e| validate::is_valid_email(e)) else {
            return page(400, "Unsubscribe", &format!("{UNSUBSCRIBE_FORM}<p class=\"msg err\">That address does not look right. Check it and try again.</p>"));
        };
        return match app.db.unsubscribe(&email).await {
            Ok(_) => {
                log_event(&app, "unsubscribed", &email).await;
                unsubscribed_page()
            }
            Err(e) => {
                eprintln!("form unsubscribe failed: {e}");
                page(502, "Something went wrong", "<h1>Something went wrong</h1><p>The change could not be saved. Please try again in a few minutes.</p>")
            }
        };
    }

    // The site's JSON form, which answers in JSON.
    let Ok(parsed) = serde_json::from_str::<EmailRequest>(&body) else {
        return with_cors(api_error(400, "Invalid request body"), &origin);
    };
    let email = validate::normalise(&parsed.email);
    if !validate::is_valid_email(&email) {
        return with_cors(api_error(400, "Invalid email address"), &origin);
    }
    match app.db.unsubscribe(&email).await {
        Ok(_) => {
            log_event(&app, "unsubscribed", &email).await;
            with_cors(api_ok(), &origin)
        }
        Err(e) => {
            eprintln!("unsubscribe failed: {e}");
            with_cors(api_error(502, "Subscription service unavailable"), &origin)
        }
    }
}

fn unsubscribed_page() -> Response {
    page(
        200,
        "Unsubscribed",
        r#"<h1>Unsubscribed</h1>
<p>That address has been removed and will not receive further newsletters.</p>
<p class="note">Changed your mind? You can sign up again from the site.</p>"#,
    )
}

// ---------------------------------------------------------------------------
// Send
// ---------------------------------------------------------------------------

/// POST /api/send-newsletter -- send an issue to everyone on the list.
///
/// Behind `ADMIN_KEY`, because `site-tools newsletter send` drives it and a CLI has
/// nowhere to put a browser redirect. Reached through the admin vhost only; the public
/// vhost does not route this path.
pub async fn send_newsletter(State(app): State<Arc<App>>, headers: HeaderMap, body: Bytes) -> Response {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !tokens::key_matches(presented, &app.config.admin_key) {
        return api_error(401, "Unauthorized");
    }

    let Ok(request) = serde_json::from_slice::<SendRequest>(&body) else {
        return api_error(400, "Invalid request body — expected {\"slug\": \"...\"}");
    };
    if !validate::is_valid_slug(&request.slug) {
        return api_error(400, "Invalid slug — only lowercase letters, digits, and hyphens allowed");
    }
    let catch_up = request.mode.as_deref() == Some("catch-up");

    match send_issue(&app, &request.slug, request.subject, catch_up).await {
        Ok(outcome) => {
            let all_ok = outcome.failed.is_empty();
            json(
                if all_ok { 200 } else { 502 },
                &SendResponse {
                    success: all_ok,
                    sent: outcome.sent,
                    skipped: outcome.skipped,
                    failed: outcome.failed,
                    error: if all_ok { None } else { Some("Some recipients could not be reached; see `failed`.".into()) },
                },
            )
        }
        Err(refused) if refused.report => send_failure(refused.status, refused.message),
        Err(refused) => api_error(refused.status, &refused.message),
    }
}

/// The title of a published issue, read from its file on the site. Also the cheapest
/// proof that the issue still exists there, which the catch-up wants before it picks.
pub async fn issue_title(app: &App, slug: &str) -> Result<String, String> {
    let url = format!("{}/newsletter/{slug}.md", app.config.site_internal_url.trim_end_matches('/'));
    let text = match app.client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        Ok(r) => return Err(format!("{url} answered {}", r.status().as_u16())),
        Err(e) => return Err(format!("could not fetch {url}: {e}")),
    };
    let (meta, _) = parse_frontmatter(&text);
    Ok(meta.get("title").cloned().unwrap_or_else(|| slug.to_string()))
}

/// What a send did: every recipient is one of these three.
pub struct SendOutcome {
    pub sent: usize,
    /// Subscribers a catch-up left alone because they already had the issue.
    pub skipped: usize,
    pub failed: Vec<String>,
}

/// Why a send did not start, with the status the HTTP face reports it as. `report`
/// says whether that face answers with the send report shape or the plain API error;
/// the CLI prints the message either way.
pub struct SendRefused {
    pub status: u16,
    pub message: String,
    report: bool,
}

impl SendRefused {
    fn api(status: u16, message: String) -> SendRefused {
        SendRefused { status, message, report: false }
    }
    fn report(status: u16, message: String) -> SendRefused {
        SendRefused { status, message, report: true }
    }
}

/// Send one issue to the list. The HTTP route above and the `send` operator command
/// in `main.rs` both come here, so the publisher on this box mails an issue over
/// loopback with no key in the request, and the workstation's `site-tools newsletter
/// send` still works through `ADMIN_KEY` for a send by hand.
pub async fn send_issue(app: &App, slug: &str, subject: Option<String>, catch_up: bool) -> Result<SendOutcome, SendRefused> {
    // The issue body comes from the site, through the loopback front for it, so the
    // published file is the one true copy and a draft never goes out by accident.
    let url = format!("{}/newsletter/{slug}.md", app.config.site_internal_url.trim_end_matches('/'));
    let md_source = match app.client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
        Ok(r) => return Err(SendRefused::api(404, format!("Newsletter not found at {url} (status {})", r.status().as_u16()))),
        Err(e) => return Err(SendRefused::api(502, format!("Could not fetch {url}: {e}"))),
    };
    let (meta, md_body) = parse_frontmatter(&md_source);
    let title = meta.get("title").cloned().unwrap_or_else(|| slug.to_string());
    let description = meta.get("description").cloned().unwrap_or_default();
    let date = meta.get("date").cloned().unwrap_or_default();
    let post_url = meta
        .get("url")
        .cloned()
        .unwrap_or_else(|| format!("{}/blog/{slug}/", app.config.site_url));
    let rendered_body = render_markdown(md_body);
    let subject = subject.unwrap_or_else(|| title.clone());

    let recipients = match app.db.subscribers().await {
        Ok(r) => r,
        Err(e) => return Err(SendRefused::api(503, format!("Could not read the list: {e}"))),
    };

    // A full send claims the slug before a single message goes out: the primary key is
    // the lock, and a send whose response was lost, followed by a retry, meets it
    // here. A catch-up of an issue that has gone out skips the claim and the delivery
    // table decides who is left; a catch-up of one that never went out is a full send.
    let already_sent = match app.db.send_exists(slug).await {
        Ok(b) => b,
        Err(e) => return Err(SendRefused::report(503, format!("Send log unavailable, refusing to send unguarded: {e}"))),
    };
    let topping_up = catch_up && already_sent;
    if !topping_up {
        match app.db.claim_send(slug, recipients.len() as i32).await {
            Ok(Claim::Won) => {}
            Ok(Claim::AlreadySent) => {
                println!("refusing to send {slug}: already claimed");
                return Err(SendRefused::report(409, format!(
                    "{slug} has already been sent. Use --catch-up to reach only those who have not had it, \
                     or delete its row in `sends` to send it to everyone again."
                )));
            }
            Err(e) => {
                eprintln!("refusing to send {slug}: claim unavailable: {e}");
                return Err(SendRefused::report(503, format!("Send log unavailable, refusing to send unguarded: {e}")));
            }
        }
    }
    let delivered = if topping_up {
        match app.db.delivered_subjects(slug).await {
            Ok(d) => d,
            Err(e) => return Err(SendRefused::report(503, format!("Could not read deliveries: {e}"))),
        }
    } else {
        Default::default()
    };

    // One message per recipient, so each carries its own signed unsubscribe URL in the
    // header and the footer, which is what makes one-click unsubscribe possible at all.
    // Each outcome is written to `deliveries` as it happens, so a process that dies
    // mid-send leaves a record a catch-up can finish from.
    let mut sent = 0usize;
    let mut skipped = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for subscriber in &recipients {
        if delivered.contains(&subscriber.subject) {
            skipped += 1;
            continue;
        }
        let email = &subscriber.email;
        let sig = tokens::unsubscribe_signature(&app.config.confirm_secret, email);
        let unsubscribe_url = tokens::unsubscribe_link(&app.config.public_url, email, &sig);
        let html = email_template(&title, &description, &date, &post_url, &rendered_body, &app.config.site_url, &unsubscribe_url);
        let outcome = mail::send(&app.client, &app.config.sender, email, &subject, &html, Some(&unsubscribe_url)).await;
        let status = match &outcome {
            Ok(()) => {
                sent += 1;
                "sent"
            }
            Err(e) => {
                // Named, because this endpoint is admin-only and the operator needs to
                // know exactly whom to retry.
                eprintln!("newsletter send to {email} failed: {e}");
                failed.push(email.clone());
                "failed"
            }
        };
        if let Err(e) = app.db.record_delivery(slug, email, status).await {
            eprintln!("could not record the delivery of {slug}: {e}");
        }
    }
    println!("newsletter {slug}: {sent} sent, {skipped} skipped, {} failed", failed.len());

    let all_ok = failed.is_empty();
    let recorded = if topping_up {
        app.db.record_catch_up(slug, sent as i32, &failed).await
    } else {
        app.db.record_send(slug, sent as i32, &failed, all_ok).await
    };
    if let Err(e) = recorded {
        eprintln!("could not record the send of {slug}: {e}");
    }

    Ok(SendOutcome { sent, skipped, failed })
}

// ---------------------------------------------------------------------------
// The welcome mail
// ---------------------------------------------------------------------------

/// A post as `static/newsletter/recent.json` lists it.
#[derive(Deserialize)]
pub struct RecentPost {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
}

/// The most recent posts, from a file the build writes. Empty on any failure: a welcome
/// mail without a post list is a great deal better than no welcome mail.
async fn recent_posts(app: &App) -> Vec<RecentPost> {
    let url = format!("{}/newsletter/recent.json", app.config.site_internal_url.trim_end_matches('/'));
    match app.client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json::<Vec<RecentPost>>().await.unwrap_or_default(),
        Ok(r) => {
            eprintln!("recent.json answered {}; welcome mail will list no posts", r.status().as_u16());
            Vec::new()
        }
        Err(e) => {
            eprintln!("could not read recent.json ({e}); welcome mail will list no posts");
            Vec::new()
        }
    }
}

/// Mail the welcome message. Never fails the confirmation that triggered it: the
/// subscription is already saved, and someone who then saw an error would reasonably
/// conclude they are not subscribed and try again.
async fn send_welcome(app: &App, email: &str) {
    let sig = tokens::unsubscribe_signature(&app.config.confirm_secret, email);
    let unsubscribe_url = tokens::unsubscribe_link(&app.config.public_url, email, &sig);
    let posts = recent_posts(app).await;
    let html = welcome_email_template(&app.config.site_url, &unsubscribe_url, &recent_posts_html(&posts));
    match mail::send(&app.client, &app.config.sender, email, "Welcome to lindfors.no", &html, Some(&unsubscribe_url)).await {
        Ok(()) => println!("welcome mail sent"),
        Err(e) => eprintln!("welcome mail failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Rendering: markdown, HTML pages, mail templates
// ---------------------------------------------------------------------------

/// Parse the `---` frontmatter of a newsletter file: (key-value pairs, body).
pub fn parse_frontmatter(md: &str) -> (std::collections::HashMap<String, String>, &str) {
    let mut meta = std::collections::HashMap::new();
    let trimmed = md.trim_start();
    if !trimmed.starts_with("---") {
        return (meta, md);
    }
    let after_first = trimmed[3..].trim_start_matches('\r');
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);
    if let Some(end) = after_first.find("\n---") {
        let front = &after_first[..end];
        let body = after_first[end + 4..].trim_start_matches(['\r', '\n']);
        for line in front.lines() {
            if let Some((k, v)) = line.split_once(':') {
                meta.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
            }
        }
        (meta, body)
    } else {
        (meta, md)
    }
}

/// Markdown to sanitised HTML.
pub fn render_markdown(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let parser = Parser::new_ext(md, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    ammonia::clean(&out)
}

pub fn html_escape(s: &str) -> String {
    // `&` first: escaping it after the others would re-escape the `&` each introduces.
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const PAGE_CSS: &str = r#"
:root { color-scheme: light; }
body { font-family: Georgia, 'Times New Roman', serif; max-width: 480px; margin: 80px auto; padding: 0 24px; color: #1C3240; background: #F0EAE0; }
h1 { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 1.5rem; margin: 0 0 16px 0; }
p { line-height: 1.6; }
a { color: #D4706A; }
.note { font-size: 0.9rem; color: #5A7078; }
.back { margin-top: 32px; }
form { display: flex; gap: 8px; margin-top: 16px; flex-wrap: wrap; }
input[type="email"] { flex: 1; min-width: 200px; padding: 10px 14px; border: 1px solid #E4DED5; border-radius: 6px; font-size: 16px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
button { padding: 10px 18px; background: #D4706A; color: #F0EAE0; border: none; border-radius: 6px; font-size: 15px; font-weight: 600; cursor: pointer; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
button:hover { background: #B85A54; }
button:focus-visible, a:focus-visible, input:focus-visible { outline: 2px solid #2A8F82; outline-offset: 2px; }
.msg { margin-top: 16px; padding: 12px; border-radius: 6px; font-size: 14px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; }
.msg.ok { background: #e8f5e9; color: #2e7d32; }
.msg.err { background: #fce4ec; color: #c62828; }
"#;

/// Shared chrome for the HTML pages. `noindex` because none of these are content.
pub fn page_shell(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta name="robots" content="noindex">
    <title>{title} &middot; lindfors.no</title>
    <style>{css}</style>
</head>
<body>
{body}
    <p class="back"><a href="https://lindfors.no">Back to lindfors.no</a></p>
</body>
</html>"#,
        title = html_escape(title),
        css = PAGE_CSS,
        body = body,
    )
}

/// The confirmation message. Deliberately short: the only thing it has to do is carry
/// one link and make clear what clicking it means. The link is repeated as plain text
/// because some clients strip or fail to linkify styled anchors.
pub fn confirmation_email_template(confirm_url: &str, site_url: &str) -> String {
    let escaped = html_escape(confirm_url);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Confirm your subscription</title>
</head>
<body style="margin: 0; padding: 0; background-color: #F0EAE0; font-family: Georgia, 'Times New Roman', serif;">
    <div style="max-width: 600px; margin: 0 auto; padding: 32px 24px; background-color: #ffffff;">
        <div style="border-bottom: 2px solid #2A8F82; padding-bottom: 16px; margin-bottom: 24px;">
            <a href="{site_url}" style="color: #1C3240; text-decoration: none; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 600;">lindfors.no</a>
        </div>
        <h1 style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 24px; color: #1C3240; margin: 0 0 16px 0; line-height: 1.3;">Confirm your subscription</h1>
        <p style="color: #1C3240; font-size: 17px; line-height: 1.7; margin: 0 0 24px 0;">Someone &mdash; hopefully you &mdash; asked to get new posts from lindfors.no by email. Click below to confirm, and you are on the list.</p>
        <p style="margin: 0 0 24px 0;">
            <a href="{escaped}" style="display: inline-block; background-color: #D4706A; color: #ffffff; text-decoration: none; padding: 12px 24px; border-radius: 6px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 15px; font-weight: 600;">Confirm subscription</a>
        </p>
        <p style="color: #5A7078; font-size: 13px; line-height: 1.6; margin: 0 0 24px 0;">Or paste this into your browser:<br><span style="word-break: break-all;">{escaped}</span></p>
        <div style="border-top: 2px solid #2A8F82; margin-top: 32px; padding-top: 16px;">
            <p style="color: #5A7078; font-size: 13px; line-height: 1.6; margin: 0;">The link works for 48 hours. If you did not sign up, ignore this message &mdash; nothing has been added to any list, and no reminder follows.</p>
        </div>
    </div>
</body>
</html>"#
    )
}

/// The post list, as HTML for the welcome mail. Empty when there is nothing to list.
pub fn recent_posts_html(posts: &[RecentPost]) -> String {
    if posts.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        r#"<h2 style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 17px; color: #1C3240; margin: 32px 0 12px 0;">Recent posts</h2>"#,
    );
    for post in posts {
        out.push_str(&format!(
            r#"<p style="margin: 0 0 14px 0;"><a href="{url}" style="color: #D4706A; text-decoration: none; font-size: 16px; font-weight: 600;">{title}</a><br><span style="color: #5A7078; font-size: 14px; line-height: 1.6;">{description}</span></p>"#,
            url = html_escape(&post.url),
            title = html_escape(&post.title),
            description = html_escape(&post.description),
        ));
    }
    out
}

/// The first message a confirmed subscriber gets. It carries `List-Unsubscribe` like
/// any list mail: it is the first of a series someone can leave.
pub fn welcome_email_template(site_url: &str, unsubscribe_url: &str, posts_html: &str) -> String {
    let unsub = html_escape(unsubscribe_url);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>You are subscribed</title>
</head>
<body style="margin: 0; padding: 0; background-color: #F0EAE0; font-family: Georgia, 'Times New Roman', serif;">
    <div style="max-width: 600px; margin: 0 auto; padding: 32px 24px; background-color: #ffffff;">
        <div style="border-bottom: 2px solid #2A8F82; padding-bottom: 16px; margin-bottom: 24px;">
            <a href="{site_url}" style="color: #1C3240; text-decoration: none; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 600;">lindfors.no</a>
        </div>
        <h1 style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 24px; color: #1C3240; margin: 0 0 16px 0; line-height: 1.3;">You are on the list</h1>
        <p style="color: #1C3240; font-size: 17px; line-height: 1.7; margin: 0 0 16px 0;">Thanks for confirming. You will get new posts from lindfors.no by email &mdash; mostly Rust, aquaculture, sensors, and whatever I have recently broken and had to fix.</p>
        <p style="color: #1C3240; font-size: 17px; line-height: 1.7; margin: 0 0 24px 0;">There is no schedule. Posts go out when they are written, which has been every few weeks and sometimes not for a couple of months. No other mail, ever, and the list is not shared with anyone.</p>
        {posts_html}
        <p style="margin: 0 0 24px 0;">
            <a href="{site_url}/blog/" style="display: inline-block; background-color: #D4706A; color: #ffffff; text-decoration: none; padding: 12px 24px; border-radius: 6px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 15px; font-weight: 600;">Read the archive</a>
        </p>
        <div style="border-top: 2px solid #2A8F82; margin-top: 32px; padding-top: 16px;">
            <p style="color: #5A7078; font-size: 13px; line-height: 1.6; margin: 0;">Changed your mind already? <a href="{unsub}" style="color: #5A7078;">Unsubscribe</a> &mdash; one click, no questions, and it works from any message I send.</p>
        </div>
    </div>
</body>
</html>"#
    )
}

/// An issue, wrapped in the mail template.
pub fn email_template(
    title: &str,
    description: &str,
    date: &str,
    post_url: &str,
    rendered_body: &str,
    site_url: &str,
    unsubscribe_url: &str,
) -> String {
    // Escaped because the URL carries a query string, and `&` opens an entity in an
    // href just as it does in text.
    let unsubscribe_url = html_escape(unsubscribe_url);
    let title = html_escape(title);
    let description = html_escape(description);
    let date = html_escape(date);
    let post_url = html_escape(post_url);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
</head>
<body style="margin: 0; padding: 0; background-color: #F0EAE0; font-family: Georgia, 'Times New Roman', serif;">
    <div style="max-width: 600px; margin: 0 auto; padding: 32px 24px; background-color: #ffffff;">
        <div style="border-bottom: 2px solid #2A8F82; padding-bottom: 16px; margin-bottom: 24px;">
            <a href="{site_url}" style="color: #1C3240; text-decoration: none; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 600;">lindfors.no</a>
        </div>
        <h1 style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 28px; color: #1C3240; margin: 0 0 8px 0; line-height: 1.2;">{title}</h1>
        <p style="color: #5A7078; font-size: 18px; margin: 0 0 16px 0; line-height: 1.5;">{description}</p>
        <p style="color: #5A7078; font-size: 14px; margin: 0 0 24px 0;">{date}</p>
        <div style="color: #1C3240; font-size: 17px; line-height: 1.75;">
            {rendered_body}
        </div>
        <div style="margin-top: 24px; padding: 12px 16px; background-color: #F0EAE0; border-radius: 6px;">
            <a href="{post_url}" style="color: #D4706A; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; font-size: 14px; font-weight: 500;">Read the full post on the site &rarr;</a>
            <span style="color: #5A7078; font-size: 13px; display: block; margin-top: 4px;">For math equations, citations, and interactive features</span>
        </div>
        <div style="border-top: 2px solid #2A8F82; margin-top: 32px; padding-top: 16px;">
            <p style="color: #5A7078; font-size: 13px; margin: 0 0 8px 0;">You received this because you subscribed to the <a href="{site_url}" style="color: #D4706A;">lindfors.no</a> newsletter.</p>
            <a href="{site_url}" style="color: #D4706A; font-size: 13px;">Visit site</a> &middot;
            <a href="{unsubscribe_url}" style="color: #D4706A; font-size: 13px;">Unsubscribe</a>
        </div>
    </div>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_split_from_the_body() {
        let (meta, body) = parse_frontmatter("---\ntitle: \"A post\"\ndate: \"2026-08-28\"\n---\n\nHello *there*.\n");
        assert_eq!(meta["title"], "A post");
        assert_eq!(meta["date"], "2026-08-28");
        assert_eq!(body, "Hello *there*.\n");
        let (meta, body) = parse_frontmatter("No frontmatter");
        assert!(meta.is_empty());
        assert_eq!(body, "No frontmatter");
    }

    #[test]
    fn markdown_is_rendered_and_sanitised() {
        let html = render_markdown("Hello *there* <script>alert(1)</script>\n\n| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(html.contains("<em>there</em>"));
        assert!(!html.contains("<script>"));
        assert!(html.contains("<table>"));
    }

    #[test]
    fn the_welcome_mail_carries_an_escaped_unsubscribe_link() {
        let url = tokens::unsubscribe_link("https://newsletter.lindfors.no", "a@example.com", "sig");
        let html = welcome_email_template("https://lindfors.no", &url, "");
        assert!(html.contains(&html_escape(&url)));
        assert!(!html.contains("Recent posts"));
        let hostile = welcome_email_template("https://lindfors.no", "https://x/?e=a\"><script>", "");
        assert!(!hostile.contains("<script>"));
    }

    #[test]
    fn recent_posts_render_escaped_and_linked() {
        let posts = vec![RecentPost { title: "Tom & Jerry <script>".into(), url: "https://lindfors.no/blog/a/".into(), description: "a < b".into() }];
        let html = recent_posts_html(&posts);
        assert!(html.contains("Recent posts"));
        assert!(html.contains("https://lindfors.no/blog/a/"));
        assert!(html.contains("Tom &amp; Jerry"));
        assert!(!html.contains("<script>"));
        assert_eq!(recent_posts_html(&[]), "");
    }

    #[test]
    fn recent_json_parses_as_the_build_writes_it() {
        let src = r#"[{"title": "A post", "url": "https://lindfors.no/blog/a/", "date": "2026-08-28", "description": "About a thing"}]"#;
        let posts: Vec<RecentPost> = serde_json::from_str(src).unwrap();
        assert_eq!(posts[0].description, "About a thing");
    }

    #[test]
    fn the_issue_template_escapes_what_it_is_given() {
        let html = email_template("T <b>", "d & e", "2026-09-03", "https://lindfors.no/blog/x/?a=1&b=2", "<p>body</p>", "https://lindfors.no", "https://n/api/unsubscribe?email=a%40b&sig=1");
        assert!(html.contains("T &lt;b&gt;"));
        assert!(html.contains("d &amp; e"));
        assert!(html.contains("<p>body</p>"), "the rendered body is trusted, it was sanitised");
        assert!(html.contains("email=a%40b&amp;sig=1"));
    }

    #[test]
    fn pages_escape_the_title() {
        let html = page_shell("A <title>", "<h1>x</h1>");
        assert!(html.contains("A &lt;title&gt;"));
        assert!(html.contains("<h1>x</h1>"));
    }
}
