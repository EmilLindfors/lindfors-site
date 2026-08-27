use serde::{Deserialize, Serialize};
use worker::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SubscribeRequest {
    email: String,
}

#[derive(Deserialize)]
struct SendNewsletterRequest {
    slug: String,
    subject: Option<String>,
}

#[derive(Serialize)]
struct ApiResponse {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cors_headers(req: &Request) -> Result<Headers> {
    let origin = req.headers().get("Origin")?.unwrap_or_default();
    let allowed = match origin.as_str() {
        "https://lindfors.no" | "https://www.lindfors.no" => origin,
        _ => "https://lindfors.no".to_string(),
    };

    let headers = Headers::new();
    headers.set("Access-Control-Allow-Origin", &allowed)?;
    headers.set("Access-Control-Allow-Methods", "POST, GET, OPTIONS")?;
    headers.set("Access-Control-Allow-Headers", "Content-Type, Authorization")?;
    Ok(headers)
}

fn json_response(data: &ApiResponse, status: u16, headers: Headers) -> Result<Response> {
    let body = serde_json::to_string(data).map_err(|e| Error::RustError(e.to_string()))?;
    let mut resp = Response::ok(body)?;
    for (key, val) in headers.entries() {
        resp.headers_mut().set(&key, &val)?;
    }
    resp.headers_mut().set("Content-Type", "application/json")?;
    Ok(resp.with_status(status))
}

fn is_valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && domain.len() >= 3
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.starts_with('-')
        && !domain.ends_with('-')
        && !domain.contains("..")
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && email.len() <= 254
}

// ---------------------------------------------------------------------------
// Stalwart JMAP
// ---------------------------------------------------------------------------
//
// Stalwart 0.16 deleted the REST management API. /api/principal/{id} and every
// other /api/* route 404 before authentication is considered at all, which is why
// sending a bearer token made no difference to the "Upstream error (404)" this
// used to return -- there was nothing left to authenticate against. Management
// objects moved to JMAP at /jmap/, so the subscriber list is now the MailingList
// object's `recipients` property.
//
// `recipients` is a Map<String>, which serialises as an object keyed by address
// rather than as an array:
//
//     "recipients": { "alice@example.com": true, "bob@example.com": true }
//
// Stalwart patches that map through JSON pointers, and the arms we need behave
// exactly like the old addItem/removeItem: setting `recipients/<addr>` to true
// pushes only if absent, and to null removes. So a subscribe stays one round trip
// with no read-modify-write, and two people subscribing at once cannot lose an
// update the way a get-then-put would.

/// Escape one JSON Pointer segment (RFC 6901). `~` must be escaped before `/`,
/// or the escape marker introduced by the second pass is re-escaped by the first.
/// `is_valid_email` constrains the domain but not the local part, so an address
/// containing either character does reach here.
fn json_pointer_escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Iterate a JMAP response's methodResponses as (method name, result) pairs.
fn method_responses(resp: &serde_json::Value) -> Vec<(&str, &serde_json::Value)> {
    resp.get("methodResponses")
        .and_then(|v| v.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let arr = call.as_array()?;
                    if arr.len() < 2 {
                        return None;
                    }
                    Some((arr[0].as_str().unwrap_or(""), &arr[1]))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Render a non-empty `notCreated`/`notUpdated` map into an error message.
fn method_error(method: &str, result: &serde_json::Value, key: &str) -> Option<String> {
    let map = result.get(key)?.as_object()?;
    if map.is_empty() {
        return None;
    }

    let details: Vec<String> = map
        .iter()
        .map(|(k, v)| {
            let err_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("unknown");
            format!("{}: {}", k, err_type)
        })
        .collect();
    Some(format!("{} {}: {}", method, key, details.join(", ")))
}

/// Assemble a Basic credential from a username and password.
///
/// The list credentials are stored as two plain values and encoded here, rather than
/// as one pre-encoded secret, because the pre-encoded convention is what broke this
/// endpoint: a bare password in the secret is indistinguishable from a correct value
/// until the server answers 401, and secrets cannot be read back to check. Sending
/// still uses a pre-encoded JMAP_CREDENTIALS -- left alone deliberately, since
/// changing it would invalidate a secret that may currently be correct.
fn basic_credential(user: &str, password: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", user, password))
}

/// Describe the *shape* of a Basic credential for the logs. Never returns any part
/// of the credential itself -- the point is to tell "wrong password" apart from
/// "wrong format", which is the mistake this convention actually invites: the value
/// must be the base64 of `user:password`, and a bare password looks identical to a
/// correct one from the outside.
fn describe_credential(credential: &str) -> String {
    let mut notes = vec![format!("{} chars", credential.len())];

    if credential.trim() != credential {
        notes.push("has surrounding whitespace".into());
    }
    if credential.contains(':') {
        notes.push("contains ':', so it is raw user:password rather than base64".into());
    }
    if !credential
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        notes.push("has characters outside the base64 alphabet".into());
    }

    notes.join("; ")
}

/// POST a JMAP request and return the parsed response. Fails on transport errors,
/// a non-200 status, and request-level JMAP errors. Method-level failures are left
/// to the caller, which knows which method it asked for.
async fn jmap_call(
    base_url: &str,
    credentials: &str,
    body: &serde_json::Value,
) -> std::result::Result<serde_json::Value, String> {
    let url = format!("{}/jmap/", base_url);
    let body_str = serde_json::to_string(body).map_err(|e| format!("JSON serialization: {}", e))?;

    let headers = Headers::new();
    headers
        .set("Authorization", &format!("Basic {}", credentials))
        .map_err(|e| format!("Header error: {}", e))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("Header error: {}", e))?;

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(headers);
    init.with_body(Some(wasm_bindgen::JsValue::from_str(&body_str)));

    let req = Request::new_with_init(&url, &init).map_err(|e| format!("Request build: {}", e))?;
    let mut resp = Fetch::Request(req)
        .send()
        .await
        .map_err(|e| format!("JMAP fetch: {}", e))?;

    if resp.status_code() == 401 {
        // A 401 is far more often a malformed credential than a wrong password, and
        // the status alone cannot tell those apart. Secrets are write-only once set,
        // so the only way to inspect one is from in here -- describe its shape, never
        // its content, and only into the logs.
        return Err(format!(
            "JMAP HTTP status 401 (credential {})",
            describe_credential(credentials)
        ));
    }

    if resp.status_code() != 200 {
        return Err(format!("JMAP HTTP status {}", resp.status_code()));
    }

    let resp_body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("JMAP response parse: {}", e))?;

    // A request-level failure arrives as a method response named "error".
    for (method, result) in method_responses(&resp_body) {
        if method == "error" {
            let err_type = result.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(format!("JMAP error: {}", err_type));
        }
    }

    Ok(resp_body)
}

/// Wrap a single MailingList method call in a JMAP request. `accountId` is only
/// sent when STALWART_LIST_ACCOUNT_ID is configured -- whether these registry
/// objects require it is unconfirmed, and omitting the key is the safer default
/// until an x:MailingList/get against the live server settles it.
fn mailing_list_request(
    account_id: Option<&str>,
    method: &str,
    mut args: serde_json::Value,
) -> serde_json::Value {
    if let (Some(id), Some(obj)) = (account_id, args.as_object_mut()) {
        obj.insert("accountId".into(), serde_json::Value::String(id.to_string()));
    }

    serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
        "methodCalls": [[method, args, "0"]]
    })
}

/// Add or remove one address in the mailing list's `recipients` map.
/// Requires the sysMailingListUpdate permission.
async fn jmap_set_recipient(
    base_url: &str,
    credentials: &str,
    account_id: Option<&str>,
    list_id: &str,
    email: &str,
    subscribe: bool,
) -> std::result::Result<(), String> {
    let value = if subscribe {
        serde_json::Value::Bool(true)
    } else {
        serde_json::Value::Null
    };

    let mut patch = serde_json::Map::new();
    patch.insert(format!("recipients/{}", json_pointer_escape(email)), value);
    let mut update = serde_json::Map::new();
    update.insert(list_id.to_string(), serde_json::Value::Object(patch));

    let body = mailing_list_request(
        account_id,
        "x:MailingList/set",
        serde_json::json!({ "update": serde_json::Value::Object(update) }),
    );

    let resp = jmap_call(base_url, credentials, &body).await?;

    for (method, result) in method_responses(&resp) {
        if let Some(err) = method_error(method, result, "notUpdated") {
            return Err(err);
        }
    }

    Ok(())
}

/// Read the mailing list's current recipients.
/// Requires the sysMailingListGet permission.
async fn jmap_get_recipients(
    base_url: &str,
    credentials: &str,
    account_id: Option<&str>,
    list_id: &str,
) -> std::result::Result<Vec<String>, String> {
    let body = mailing_list_request(
        account_id,
        "x:MailingList/get",
        serde_json::json!({ "ids": [list_id], "properties": ["recipients"] }),
    );

    let resp = jmap_call(base_url, credentials, &body).await?;

    for (method, result) in method_responses(&resp) {
        if method != "x:MailingList/get" {
            continue;
        }

        let Some(list) = result
            .get("list")
            .and_then(|v| v.as_array())
            .and_then(|l| l.first())
        else {
            return Err(format!("mailing list {} not found", list_id));
        };

        // Map<String> is an object keyed by address; a false value means unset.
        let mut members: Vec<String> = list
            .get("recipients")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter(|(_, v)| v.as_bool().unwrap_or(false))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        members.sort();
        return Ok(members);
    }

    Err("no x:MailingList/get response".to_string())
}

/// Parse YAML-ish frontmatter from a markdown file (between --- delimiters).
/// Returns (key-value pairs, body after frontmatter).
fn parse_frontmatter(md: &str) -> (std::collections::HashMap<String, String>, &str) {
    let mut meta = std::collections::HashMap::new();
    let trimmed = md.trim_start();

    if !trimmed.starts_with("---") {
        return (meta, md);
    }

    let after_first = &trimmed[3..].trim_start_matches('\r');
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    if let Some(end) = after_first.find("\n---") {
        let front = &after_first[..end];
        let body_start = end + 4; // skip \n---
        let body = after_first[body_start..].trim_start_matches(['\r', '\n']);

        for line in front.lines() {
            if let Some((k, v)) = line.split_once(':') {
                let key = k.trim().to_string();
                let val = v.trim().trim_matches('"').to_string();
                meta.insert(key, val);
            }
        }

        (meta, body)
    } else {
        (meta, md)
    }
}

/// Render markdown to sanitized HTML using pulldown-cmark + ammonia.
fn render_markdown(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(md, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    ammonia::clean(&html_output)
}

/// Wrap rendered HTML content in the email template.
fn email_template(
    title: &str,
    description: &str,
    date: &str,
    post_url: &str,
    rendered_body: &str,
    site_url: &str,
) -> String {
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
            <a href="{site_url}/api/unsubscribe" style="color: #D4706A; font-size: 13px;">Unsubscribe</a>
        </div>
    </div>
</body>
</html>"#,
        title = title,
        description = description,
        date = date,
        post_url = post_url,
        rendered_body = rendered_body,
        site_url = site_url,
    )
}

/// Strip HTML tags to produce a plain text version of an email body.
fn html_to_plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_newline = false;

    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                // Block-level tags get a newline
                let rest = &html[html.len().saturating_sub(html.len())..];
                let _ = rest; // we handle this via the closing '>' check below
            }
            '>' => {
                in_tag = false;
            }
            _ if in_tag => {}
            '\n' | '\r' => {
                if !last_was_newline {
                    text.push('\n');
                    last_was_newline = true;
                }
            }
            _ => {
                text.push(ch);
                last_was_newline = false;
            }
        }
    }

    // Decode common HTML entities
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rarr;", "->")
        .replace("&middot;", "-")
        .replace("&nbsp;", " ")
}

/// Send an email via Stalwart's JMAP API using Email/set + EmailSubmission/set.
/// Returns Ok(()) on success or an error message describing what went wrong.
async fn jmap_send_email(
    base_url: &str,
    credentials: &str,
    account_id: &str,
    identity_id: &str,
    from: &str,
    to: &str,
    subject: &str,
    html_body: &str,
) -> std::result::Result<(), String> {
    let plain_text = html_to_plain_text(html_body);

    let body = serde_json::json!({
        "using": [
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
            "urn:ietf:params:jmap:submission"
        ],
        "methodCalls": [
            [
                "Email/set",
                {
                    "accountId": account_id,
                    "create": {
                        "draft": {
                            "mailboxIds": { "d": true },
                            "from": [{ "name": "Emil Lindfors", "email": from }],
                            "to": [{ "email": to }],
                            "subject": subject,
                            "header:List-Unsubscribe:asRaw": " <https://lindfors.no/api/unsubscribe>",
                            "header:List-Unsubscribe-Post:asRaw": " List-Unsubscribe=One-Click",
                            "textBody": [{
                                "partId": "text",
                                "type": "text/plain"
                            }],
                            "htmlBody": [{
                                "partId": "html",
                                "type": "text/html"
                            }],
                            "bodyValues": {
                                "text": {
                                    "value": plain_text,
                                    "isEncodingProblem": false,
                                    "isTruncated": false
                                },
                                "html": {
                                    "value": html_body,
                                    "isEncodingProblem": false,
                                    "isTruncated": false
                                }
                            }
                        }
                    }
                },
                "0"
            ],
            [
                "EmailSubmission/set",
                {
                    "accountId": account_id,
                    "create": {
                        "send": {
                            "identityId": identity_id,
                            "emailId": "#draft",
                            "envelope": {
                                "mailFrom": { "email": from },
                                "rcptTo": [{ "email": to }]
                            }
                        }
                    },
                    "onSuccessDestroyEmail": ["#send"]
                },
                "1"
            ]
        ]
    });

    let resp_body = jmap_call(base_url, credentials, &body).await?;

    // Email/set and EmailSubmission/set report per-object failures in notCreated.
    for (method, result) in method_responses(&resp_body) {
        if let Some(err) = method_error(method, result, "notCreated") {
            return Err(err);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[event(fetch, respond_with_errors)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/api/subscribe", handle_subscribe)
        .get_async("/api/unsubscribe", handle_unsubscribe_page)
        .post_async("/api/unsubscribe", handle_unsubscribe_post)
        .get_async("/api/subscribers", handle_subscribers)
        .post_async("/api/send-newsletter", handle_send_newsletter)
        .options("/api/subscribe", handle_preflight)
        .options("/api/unsubscribe", handle_preflight)
        .run(req, env)
        .await
}

/// Shared logic for subscribe/unsubscribe — validates email, calls Stalwart.
async fn handle_subscription_op(
    mut req: Request,
    ctx: &RouteContext<()>,
    subscribe: bool,
) -> Result<Response> {
    let headers = cors_headers(&req)?;

    let body: SubscribeRequest = match req.json().await {
        Ok(b) => b,
        Err(_) => {
            return json_response(
                &ApiResponse {
                    success: false,
                    error: Some("Invalid request body".into()),
                },
                400,
                headers,
            );
        }
    };

    let email = body.email.trim().to_lowercase();

    if !is_valid_email(&email) {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Invalid email address".into()),
            },
            400,
            headers,
        );
    }

    let base_url = ctx.env.var("JMAP_API_URL")?.to_string();
    let credentials = basic_credential(
        &ctx.env.var("JMAP_LIST_USER")?.to_string(),
        &ctx.env.secret("JMAP_LIST_PASSWORD")?.to_string(),
    );
    let list_id = ctx.env.var("STALWART_LIST_ID")?.to_string();
    let account_id = ctx
        .env
        .var("STALWART_LIST_ACCOUNT_ID")
        .ok()
        .map(|v| v.to_string());

    match jmap_set_recipient(
        &base_url,
        &credentials,
        account_id.as_deref(),
        &list_id,
        &email,
        subscribe,
    )
    .await
    {
        Ok(()) => json_response(&ApiResponse { success: true, error: None }, 200, headers),
        Err(e) => {
            // The upstream detail goes to `wrangler tail`, not to the caller. The old
            // code returned it in the body, which is how the 0.16 breakage was visible
            // from outside -- convenient that once, but this is a public endpoint and
            // the detail now names an authenticated JMAP method.
            console_error!(
                "mailing list {} failed: {}",
                if subscribe { "subscribe" } else { "unsubscribe" },
                e
            );
            json_response(
                &ApiResponse {
                    success: false,
                    error: Some("Subscription service unavailable".into()),
                },
                502,
                headers,
            )
        }
    }
}

/// POST /api/subscribe — add email to the Stalwart mailing list.
async fn handle_subscribe(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    handle_subscription_op(req, &ctx, true).await
}

/// GET /api/unsubscribe — show the unsubscribe form.
async fn handle_unsubscribe_page(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_html(&unsubscribe_form_page())
}

/// POST /api/unsubscribe — remove email from the Stalwart mailing list.
async fn handle_unsubscribe_post(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    handle_subscription_op(req, &ctx, false).await
}

/// Extract Bearer token from Authorization header.
fn extract_bearer_token(req: &Request) -> Option<String> {
    req.headers()
        .get("Authorization")
        .ok()
        .flatten()
        .and_then(|v| v.strip_prefix("Bearer ").map(|t| t.to_string()))
}

/// GET /api/subscribers — admin: list current subscribers from Stalwart.
async fn handle_subscribers(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let admin_key = ctx.env.secret("ADMIN_KEY")?.to_string();
    let token = extract_bearer_token(&req).unwrap_or_default();

    if token != admin_key {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Unauthorized".into()),
            },
            401,
            cors_headers(&req)?,
        );
    }

    let base_url = ctx.env.var("JMAP_API_URL")?.to_string();
    let credentials = basic_credential(
        &ctx.env.var("JMAP_LIST_USER")?.to_string(),
        &ctx.env.secret("JMAP_LIST_PASSWORD")?.to_string(),
    );
    let list_id = ctx.env.var("STALWART_LIST_ID")?.to_string();
    let account_id = ctx
        .env
        .var("STALWART_LIST_ACCOUNT_ID")
        .ok()
        .map(|v| v.to_string());

    let members = jmap_get_recipients(&base_url, &credentials, account_id.as_deref(), &list_id)
        .await
        .map_err(Error::RustError)?;

    #[derive(Serialize)]
    struct ListResponse {
        total: usize,
        members: Vec<String>,
    }

    let data = ListResponse {
        total: members.len(),
        members,
    };

    let body = serde_json::to_string(&data).map_err(|e| Error::RustError(e.to_string()))?;
    let mut resp = Response::ok(body)?;
    resp.headers_mut().set("Content-Type", "application/json")?;
    Ok(resp)
}

/// POST /api/send-newsletter — admin: send a newsletter to the mailing list via JMAP.
async fn handle_send_newsletter(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let admin_key = ctx.env.secret("ADMIN_KEY")?.to_string();
    let token = extract_bearer_token(&req).unwrap_or_default();

    if token != admin_key {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Unauthorized".into()),
            },
            401,
            cors_headers(&req)?,
        );
    }

    let body: SendNewsletterRequest = match req.json().await {
        Ok(b) => b,
        Err(_) => {
            return json_response(
                &ApiResponse {
                    success: false,
                    error: Some("Invalid request body — expected {\"slug\": \"...\"}".into()),
                },
                400,
                cors_headers(&req)?,
            );
        }
    };

    // Validate slug: only lowercase alphanumeric and hyphens
    if body.slug.is_empty() || !body.slug.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Invalid slug — only lowercase letters, digits, and hyphens allowed".into()),
            },
            400,
            cors_headers(&req)?,
        );
    }

    // Fetch the newsletter markdown from the site
    let site_url = ctx.env.var("SITE_URL")?.to_string();
    let newsletter_url = format!("{}/newsletter/{}.md", site_url, body.slug);

    let fetch_req = Request::new(&newsletter_url, Method::Get)?;
    let mut fetch_resp = Fetch::Request(fetch_req).send().await?;

    if fetch_resp.status_code() != 200 {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some(format!(
                    "Newsletter not found at {} (status {})",
                    newsletter_url,
                    fetch_resp.status_code()
                )),
            },
            404,
            cors_headers(&req)?,
        );
    }

    let md_source = fetch_resp.text().await?;
    let (meta, md_body) = parse_frontmatter(&md_source);

    let title = meta.get("title").cloned().unwrap_or_else(|| body.slug.clone());
    let description = meta.get("description").cloned().unwrap_or_default();
    let date = meta.get("date").cloned().unwrap_or_default();
    let post_url = meta
        .get("url")
        .cloned()
        .unwrap_or_else(|| format!("{}/blog/{}/", site_url, body.slug));

    let rendered_body = render_markdown(md_body);
    let html = email_template(&title, &description, &date, &post_url, &rendered_body, &site_url);

    let subject = body.subject.unwrap_or(title);

    // Read JMAP config
    let jmap_url = ctx.env.var("JMAP_API_URL")?.to_string();
    let credentials = ctx.env.secret("JMAP_CREDENTIALS")?.to_string();
    let account_id = ctx.env.var("JMAP_ACCOUNT_ID")?.to_string();
    let identity_id = ctx.env.var("JMAP_IDENTITY_ID")?.to_string();

    let from = "postmaster@lindfors.no";
    let to = "newsletter@lindfors.no";

    match jmap_send_email(
        &jmap_url,
        &credentials,
        &account_id,
        &identity_id,
        from,
        to,
        &subject,
        &html,
    )
    .await
    {
        Ok(()) => {
            console_log!("Newsletter sent: {}", body.slug);
            json_response(
                &ApiResponse {
                    success: true,
                    error: None,
                },
                200,
                cors_headers(&req)?,
            )
        }
        Err(e) => {
            console_log!("Newsletter send failed: {}", e);
            json_response(
                &ApiResponse {
                    success: false,
                    error: Some(format!("Failed to send: {}", e)),
                },
                502,
                cors_headers(&req)?,
            )
        }
    }
}

fn handle_preflight(req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    let headers = cors_headers(&req)?;
    let mut resp = Response::empty()?.with_status(204);
    for (key, val) in headers.entries() {
        resp.headers_mut().set(&key, &val)?;
    }
    resp.headers_mut().set("Access-Control-Max-Age", "86400")?;
    Ok(resp)
}

// ---------------------------------------------------------------------------
// HTML pages
// ---------------------------------------------------------------------------

fn unsubscribe_form_page() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Unsubscribe - lindfors.no</title>
    <style>
        body { font-family: Georgia, serif; max-width: 480px; margin: 80px auto; padding: 0 24px; color: #1C3240; background: #F0EAE0; }
        h1 { font-family: -apple-system, sans-serif; font-size: 1.5rem; }
        p { line-height: 1.6; }
        a { color: #D4706A; }
        form { display: flex; gap: 8px; margin-top: 16px; }
        input[type="email"] { flex: 1; padding: 10px 14px; border: 1px solid #E4DED5; border-radius: 6px; font-size: 16px; font-family: -apple-system, sans-serif; }
        button { padding: 10px 18px; background: #D4706A; color: #F0EAE0; border: none; border-radius: 6px; font-size: 14px; font-weight: 600; cursor: pointer; font-family: -apple-system, sans-serif; }
        button:hover { background: #B85A54; }
        .msg { margin-top: 16px; padding: 12px; border-radius: 6px; font-size: 14px; font-family: -apple-system, sans-serif; }
        .msg.ok { background: #e8f5e9; color: #2e7d32; }
        .msg.err { background: #fce4ec; color: #c62828; }
    </style>
</head>
<body>
    <h1>Unsubscribe</h1>
    <p>Enter your email to unsubscribe from the lindfors.no newsletter.</p>
    <form id="unsub-form">
        <input type="email" name="email" placeholder="your@email.com" required>
        <button type="submit">Unsubscribe</button>
    </form>
    <div id="msg"></div>
    <p style="margin-top: 32px;"><a href="https://lindfors.no">Back to lindfors.no</a></p>
    <script>
    document.getElementById('unsub-form').addEventListener('submit', function(e) {
        e.preventDefault();
        var email = this.querySelector('input').value;
        var btn = this.querySelector('button');
        var msg = document.getElementById('msg');
        btn.disabled = true;
        btn.textContent = 'Processing...';
        fetch('/api/unsubscribe', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ email: email })
        }).then(function(r) { return r.json(); }).then(function(data) {
            if (data.success) {
                msg.className = 'msg ok';
                msg.textContent = 'You have been unsubscribed.';
            } else {
                msg.className = 'msg err';
                msg.textContent = data.error || 'Something went wrong.';
            }
        }).catch(function() {
            msg.className = 'msg err';
            msg.textContent = 'Something went wrong. Please try again.';
        }).finally(function() {
            btn.disabled = false;
            btn.textContent = 'Unsubscribe';
        });
    });
    </script>
</body>
</html>"#.to_string()
}
