//! The newsletter, as one service on the machine that already holds its mail.
//!
//! Three things in one binary, on one loopback port, with nginx deciding which name may
//! reach which paths:
//!
//! - the public endpoints a reader hits -- subscribe, confirm, unsubscribe -- routed
//!   from `newsletter.lindfors.no` and nothing else (`public.rs`)
//! - the send an operator triggers with `site-tools newsletter send`, behind
//!   `ADMIN_KEY`, routed from `admin.lindfors.no` (`public.rs`, `send_newsletter`)
//! - the dashboard, behind Kanidm, routed from `admin.lindfors.no` (this file, `auth.rs`,
//!   `oidc.rs`, `rum.rs`, and the compiled-in page under `static/`)
//!
//! State is in the Postgres on this box (`db.rs`, `schema.sql`). It replaced a Cloudflare
//! Worker that kept the list in a Stalwart mailing list, the history in WebDAV filenames
//! and the send lock in an HTTP precondition, because those were ways of not having a
//! database, and this host has one. README.md has the whole argument.
//!
//! Everything it talks to is a port on this same machine, in plaintext: Postgres,
//! Stalwart's JMAP, and two nginx listeners that put a plain-HTTP face on things that only
//! speak TLS (Kanidm, and the site itself for the issue bodies). That is what lets the
//! binary carry no TLS stack and no C, which is what makes cross-compiling it for this
//! host a plain `cargo build --target`.

mod auth;
mod db;
mod mail;
mod oidc;
mod public;
mod ratelimit;
mod rum;
mod tokens;
mod validate;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

/// The dashboard page, compiled in. A deployment is one binary plus an environment file.
const INDEX_HTML: &str = include_str!("../static/index.html");
const ADMIN_JS: &str = include_str!("../static/admin.js");
const ADMIN_AUTH_JS: &str = include_str!("../static/admin-auth.js");

pub struct Config {
    /// `https://lindfors.no`: where links in mail point readers, and the CORS origin.
    pub site_url: String,
    /// The same site reached from this process, through nginx's loopback front for it
    /// (`http://127.0.0.1:8448`): the issue bodies and `recent.json` come from here.
    pub site_internal_url: String,
    /// `https://newsletter.lindfors.no`: where the confirm and unsubscribe links point.
    pub public_url: String,
    /// The origins the site's form may call from. `site_url` and its `www.` variant.
    pub cors_origins: Vec<String>,
    pub confirm_secret: String,
    pub event_log_secret: String,
    pub admin_key: String,
    pub sender: mail::Sender,
    /// The **public** OIDC issuer, handed to the browser; and the same one reached
    /// over loopback, which is what this process fetches. See `oidc.rs`.
    pub issuer: String,
    pub internal_issuer: String,
    pub client_id: String,
    pub admin_subject: String,
    pub rum: Option<rum::RumConfig>,
    pub csp: String,
}

pub struct App {
    pub config: Config,
    pub client: reqwest::Client,
    pub db: db::Db,
    pub limiter: ratelimit::Limiter,
    pub provider: oidc::Provider,
}

/// Read a required variable, or name the one that is missing. Every one of these is
/// fatal at startup rather than at the first request.
fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{name} is not set"))
}

fn optional(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// A required variable that must be a plaintext URL. This binary carries no TLS, so an
/// `https://` upstream fails when the request is built, far from the environment file
/// that caused it; failing here with the name is the whole value of this function.
fn plaintext(name: &str) -> Result<String, String> {
    let value = required(name)?;
    if !value.starts_with("http://") {
        return Err(format!(
            "{name} must be an http:// URL -- this service is built without TLS and reaches \
             everything over loopback. Got: {value}"
        ));
    }
    Ok(value)
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let site_url = optional("SITE_URL", "https://lindfors.no").trim_end_matches('/').to_string();
        let www = site_url.replacen("://", "://www.", 1);
        let issuer = required("OIDC_ISSUER")?;
        let confirm_secret = required("CONFIRM_SECRET")?;
        let event_log_secret = required("EVENT_LOG_SECRET")?;
        if confirm_secret == event_log_secret {
            // The confirm secret is meant to be rotated; the log secret is meant to last
            // for the life of the log. One value for both makes a rotation orphan every
            // event ever written.
            return Err("EVENT_LOG_SECRET must not equal CONFIRM_SECRET".into());
        }
        Ok(Self {
            cors_origins: vec![site_url.clone(), www],
            site_internal_url: plaintext("SITE_INTERNAL_URL")?,
            public_url: required("PUBLIC_URL")?.trim_end_matches('/').to_string(),
            site_url,
            confirm_secret,
            event_log_secret,
            admin_key: required("ADMIN_KEY")?,
            sender: mail::Sender::new(
                plaintext("JMAP_API_URL")?,
                &required("JMAP_SENDER_USER")?,
                &required("JMAP_SENDER_PASSWORD")?,
                required("JMAP_ACCOUNT_ID")?,
                required("JMAP_IDENTITY_ID")?,
                optional("MAIL_FROM", "postmaster@lindfors.no"),
                optional("MAIL_FROM_NAME", "Emil Lindfors"),
            ),
            internal_issuer: plaintext("OIDC_INTERNAL_ISSUER")?,
            client_id: required("OIDC_CLIENT_ID")?,
            admin_subject: required("ADMIN_SUBJECT")?,
            rum: rum_config()?,
            csp: content_security_policy(&issuer),
            issuer,
        })
    }
}

/// The OpenObserve block is optional as a group: no `O2_URL`, no Readers section.
fn rum_config() -> Result<Option<rum::RumConfig>, String> {
    if std::env::var("O2_URL").map(|v| v.trim().is_empty()).unwrap_or(true) {
        return Ok(None);
    }
    Ok(Some(rum::RumConfig {
        base_url: plaintext("O2_URL")?,
        org: required("O2_ORG")?,
        user: required("O2_USER")?,
        token: required("O2_TOKEN")?,
    }))
}

/// The dashboard's policy: its own scripts only, and one cross-origin host, the issuer.
fn content_security_policy(issuer: &str) -> String {
    format!(
        concat!(
            "default-src 'self'; script-src 'self'; ",
            "style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; ",
            "connect-src 'self' {}; form-action 'self'; frame-ancestors 'none'; ",
            "base-uri 'self'; object-src 'none'"
        ),
        oidc::origin_of(issuer)
    )
}

// ---------------------------------------------------------------------------
// The dashboard
// ---------------------------------------------------------------------------

fn asset(app: &App, content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_SECURITY_POLICY, &app.config.csp),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::HeaderName::from_static("x-robots-tag"), "noindex, nofollow"),
        ],
        body,
    )
        .into_response()
}

async fn index(State(app): State<Arc<App>>) -> Response {
    asset(&app, "text/html; charset=utf-8", INDEX_HTML)
}

async fn admin_js(State(app): State<Arc<App>>) -> Response {
    asset(&app, "text/javascript; charset=utf-8", ADMIN_JS)
}

async fn admin_auth_js(State(app): State<Arc<App>>) -> Response {
    asset(&app, "text/javascript; charset=utf-8", ADMIN_AUTH_JS)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientConfig {
    issuer: String,
    client_id: String,
    authorization_endpoint: String,
    token_endpoint: String,
    end_session_endpoint: Option<String>,
}

/// The public values the login flow needs, relayed from the issuer's discovery document.
async fn client_config(State(app): State<Arc<App>>) -> Response {
    match app.provider.endpoints(&app.client).await {
        Ok(endpoints) => Json(ClientConfig {
            issuer: app.config.issuer.clone(),
            client_id: app.config.client_id.clone(),
            authorization_endpoint: endpoints.public.authorization_endpoint.clone(),
            token_endpoint: endpoints.public.token_endpoint.clone(),
            end_session_endpoint: endpoints.public.end_session_endpoint.clone(),
        })
        .into_response(),
        Err(e) => {
            eprintln!("OIDC discovery failed: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "Authentication service unavailable").into_response()
        }
    }
}

/// What the dashboard draws. Every section is independently optional, because the
/// sources fail independently.
#[derive(Serialize)]
struct Overview {
    #[serde(skip_serializing_if = "Option::is_none")]
    subscribers: Option<i64>,
    events: Vec<db::EventRecord>,
    sends: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rum: Option<rum::RumOverview>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
}

async fn overview(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let presented = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    if auth::bearer(presented).is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }
    let endpoints = match app.provider.endpoints(&app.client).await {
        Ok(endpoints) => endpoints,
        Err(e) => {
            eprintln!("OIDC discovery failed: {e}");
            return (StatusCode::SERVICE_UNAVAILABLE, "Authentication service unavailable").into_response();
        }
    };
    if let Err(denied) = auth::authorize(&app.client, &endpoints.internal_userinfo, &app.config.admin_subject, presented).await {
        return match denied {
            auth::Denied::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
            auth::Denied::Unavailable(e) => {
                eprintln!("userinfo failed: {e}");
                (StatusCode::SERVICE_UNAVAILABLE, "Authentication service unavailable").into_response()
            }
        };
    }

    let mut errors = Vec::new();
    let subscribers = match app.db.subscriber_count().await {
        Ok(n) => Some(n),
        Err(e) => {
            errors.push(format!("subscriber list: {e}"));
            None
        }
    };
    let events = app.db.events().await.unwrap_or_else(|e| {
        errors.push(format!("event log: {e}"));
        Vec::new()
    });
    let sends = app.db.sends().await.unwrap_or_else(|e| {
        errors.push(format!("send log: {e}"));
        Vec::new()
    });
    let rum = match &app.config.rum {
        Some(config) => Some(rum::overview(&app.client, config, &mut errors).await),
        None => None,
    };

    Json(Overview { subscribers, events, sends, rum, errors }).into_response()
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("lindfors-newsletter: {e}");
            eprintln!("See newsletter/README.md for the full environment.");
            std::process::exit(1);
        }
    };

    let db = match db::Db::connect(&required("DATABASE_URL").unwrap_or_default()) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("lindfors-newsletter: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = db.check().await {
        eprintln!("lindfors-newsletter: {e}");
        std::process::exit(1);
    }

    // Loopback by default. nginx is what reaches this, and it decides which name may
    // reach which paths; nothing outside the machine talks to this port.
    let bind = optional("ADMIN_BIND", "127.0.0.1:8788");
    let addr: SocketAddr = match bind.parse() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("lindfors-newsletter: ADMIN_BIND is not an address: {e}");
            std::process::exit(1);
        }
    };

    let provider = oidc::Provider::new(config.issuer.clone(), config.internal_issuer.clone());
    let app = Arc::new(App {
        config,
        client: reqwest::Client::new(),
        db,
        limiter: ratelimit::Limiter::new(60),
        provider,
    });

    let router = Router::new()
        // Public, routed from newsletter.lindfors.no.
        .route("/api/subscribe", post(public::subscribe).options(public::preflight))
        .route("/api/confirm", get(public::confirm_page).post(public::confirm))
        .route("/api/unsubscribe", get(public::unsubscribe_page).post(public::unsubscribe).options(public::preflight))
        // Operator, routed from admin.lindfors.no.
        .route("/api/send-newsletter", post(public::send_newsletter))
        .route("/", get(index))
        .route("/admin.js", get(admin_js))
        .route("/admin-auth.js", get(admin_auth_js))
        .route("/api/config", get(client_config))
        .route("/api/overview", get(overview))
        .with_state(app);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("lindfors-newsletter: cannot bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    println!("lindfors-newsletter listening on http://{addr}");

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown())
        .await
        .expect("server failed");
}

/// Stop on SIGTERM as well as Ctrl-C, so `rc-service lindfors-newsletter stop` is a
/// clean exit rather than a kill after start-stop-daemon's timeout.
async fn shutdown() {
    let interrupt = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sig) = signal(SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = interrupt => {}
        _ = terminate => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_https_upstream_is_refused_by_name() {
        std::env::set_var("TEST_UPSTREAM", "https://mail.lindfors.no");
        let message = plaintext("TEST_UPSTREAM").unwrap_err();
        assert!(message.contains("TEST_UPSTREAM"), "{message}");
        std::env::remove_var("TEST_UPSTREAM");
    }

    #[test]
    fn the_policy_admits_only_the_configured_issuer() {
        let csp = content_security_policy("https://idm.lindfors.no/oauth2/openid/lindfors-admin");
        assert!(csp.contains("connect-src 'self' https://idm.lindfors.no;"), "{csp}");
        assert!(!csp.contains("oauth2"), "{csp}");
        assert!(csp.contains("script-src 'self';"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    }
}
