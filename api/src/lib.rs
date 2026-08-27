use serde::{Deserialize, Serialize};
use worker::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The body of both `/api/subscribe` and `/api/unsubscribe`: `{"email": "..."}`.
#[derive(Deserialize)]
struct EmailRequest {
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

/// The result of a per-recipient send. `sent` and `failed` exist because the send is
/// no longer one atomic message: it can now half-succeed, and the operator needs to
/// know which addresses to retry.
#[derive(Serialize)]
struct SendResponse {
    success: bool,
    sent: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Ceiling on recipients in one send. Every message is a subrequest and Workers caps
/// those per invocation (50 on the free plan), with one already spent fetching the
/// newsletter markdown. Set below the cap rather than at it, and enforced by refusing
/// the send outright -- see handle_send_newsletter.
const MAX_RECIPIENTS_PER_SEND: usize = 45;

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
    headers.set(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization",
    )?;
    Ok(headers)
}

fn json_response(data: &ApiResponse, status: u16, headers: Headers) -> Result<Response> {
    json_response_value(data, status, headers)
}

fn json_response_value<T: Serialize>(data: &T, status: u16, headers: Headers) -> Result<Response> {
    let body = serde_json::to_string(data).map_err(|e| Error::RustError(e.to_string()))?;
    let mut resp = Response::ok(body)?;
    for (key, val) in headers.entries() {
        resp.headers_mut().set(&key, &val)?;
    }
    resp.headers_mut().set("Content-Type", "application/json")?;
    Ok(resp.with_status(status))
}

/// Validate an address well enough to decide whether to *send mail to it*.
///
/// That is a stricter job than it was. This check used to gate a row in a list, where
/// a nonsense address was merely clutter; it now gates a message leaving the server,
/// and mail to a domain that cannot resolve comes straight back as a bounce -- which
/// is the signal that costs a self-hosted sender its delivery reputation. Two gaps
/// found by the tests and closed here:
///   - the domain was checked only as a whole, so `example-.com` passed: a trailing
///     hyphen was rejected on the last label but on no other. Labels are checked
///     individually now.
///   - the local part had no character restriction at all, so a space or a quote
///     went through. It is limited to RFC 5322 atext plus `.`, which is every
///     unquoted address anyone actually has.
///
/// Still deliberately not a full RFC 5322 parser: quoted local parts, comments and
/// address literals are all legal and all rejected here, because none of them belong
/// in a newsletter signup box and each is a way to smuggle odd bytes downstream.
fn is_valid_email(email: &str) -> bool {
    if email.len() > 254 {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    is_valid_local_part(local) && is_valid_domain(domain)
}

fn is_valid_local_part(local: &str) -> bool {
    !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '.' | '!'
                        | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '/'
                        | '='
                        | '?'
                        | '^'
                        | '_'
                        | '`'
                        | '{'
                        | '|'
                        | '}'
                        | '~'
                )
        })
}

fn is_valid_domain(domain: &str) -> bool {
    if domain.len() > 253 {
        return false;
    }
    let labels: Vec<&str> = domain.split('.').collect();

    // At least one dot: a bare hostname is unroutable from the public internet.
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // An all-digit final label means this is an IP address rather than a name.
        // Punycode TLDs (`xn--p1ai`) carry digits and hyphens, so the test is
        // "not entirely digits" rather than "all letters".
        && labels
            .last()
            .is_some_and(|tld| tld.len() >= 2 && !tld.chars().all(|c| c.is_ascii_digit()))
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

/// The JMAP connection to the subscriber list.
///
/// Read from the environment in one place because three handlers now need the same
/// four values -- subscribe's sibling unsubscribe, the admin listing, and confirm --
/// and four copies of the same lookups is four chances for one to drift into a
/// failure that surfaces only as an opaque 401.
struct ListConfig {
    base_url: String,
    credentials: String,
    list_id: String,
    account_id: Option<String>,
}

impl ListConfig {
    fn from_env(env: &Env) -> Result<Self> {
        Ok(Self {
            base_url: env.var("JMAP_API_URL")?.to_string(),
            credentials: basic_credential(
                &env.var("JMAP_LIST_USER")?.to_string(),
                &env.secret("JMAP_LIST_PASSWORD")?.to_string(),
            ),
            list_id: env.var("STALWART_LIST_ID")?.to_string(),
            account_id: env
                .var("STALWART_LIST_ACCOUNT_ID")
                .ok()
                .map(|v| v.to_string()),
        })
    }
}

/// The postmaster account outgoing mail is sent from.
///
/// Kept separate from `ListConfig` rather than merged into one blob of JMAP settings,
/// because they are genuinely different principals: this one needs neither
/// sysMailingListGet nor sysMailingListUpdate, and its credential is still the
/// pre-encoded `JMAP_CREDENTIALS` rather than the user/password split.
struct SenderConfig {
    base_url: String,
    credentials: String,
    account_id: String,
    identity_id: String,
    from: String,
}

impl SenderConfig {
    fn from_env(env: &Env) -> Result<Self> {
        Ok(Self {
            base_url: env.var("JMAP_API_URL")?.to_string(),
            credentials: env.secret("JMAP_CREDENTIALS")?.to_string(),
            account_id: env.var("JMAP_ACCOUNT_ID")?.to_string(),
            identity_id: env.var("JMAP_IDENTITY_ID")?.to_string(),
            from: "postmaster@lindfors.no".to_string(),
        })
    }
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
            let err_type = result
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
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
        obj.insert(
            "accountId".into(),
            serde_json::Value::String(id.to_string()),
        );
    }

    serde_json::json!({
        "using": ["urn:ietf:params:jmap:core", "urn:stalwart:jmap"],
        "methodCalls": [[method, args, "0"]]
    })
}

/// Add or remove one address in the mailing list's `recipients` map.
/// Requires the sysMailingListUpdate permission.
async fn jmap_set_recipient(
    cfg: &ListConfig,
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
    update.insert(cfg.list_id.clone(), serde_json::Value::Object(patch));

    let body = mailing_list_request(
        cfg.account_id.as_deref(),
        "x:MailingList/set",
        serde_json::json!({ "update": serde_json::Value::Object(update) }),
    );

    let resp = jmap_call(&cfg.base_url, &cfg.credentials, &body).await?;

    for (method, result) in method_responses(&resp) {
        if let Some(err) = method_error(method, result, "notUpdated") {
            return Err(err);
        }
    }

    Ok(())
}

/// Read the mailing list's current recipients.
/// Requires the sysMailingListGet permission.
async fn jmap_get_recipients(cfg: &ListConfig) -> std::result::Result<Vec<String>, String> {
    let body = mailing_list_request(
        cfg.account_id.as_deref(),
        "x:MailingList/get",
        serde_json::json!({ "ids": [cfg.list_id], "properties": ["recipients"] }),
    );

    let resp = jmap_call(&cfg.base_url, &cfg.credentials, &body).await?;

    for (method, result) in method_responses(&resp) {
        if method != "x:MailingList/get" {
            continue;
        }

        let Some(list) = result
            .get("list")
            .and_then(|v| v.as_array())
            .and_then(|l| l.first())
        else {
            return Err(format!("mailing list {} not found", cfg.list_id));
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
    unsubscribe_url: &str,
) -> String {
    // Escaped because the URL carries a query string, and `&` opens an entity in an
    // href just as it does in text.
    let unsubscribe_url = html_escape(unsubscribe_url);
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
</html>"#,
        title = title,
        description = description,
        date = date,
        post_url = post_url,
        rendered_body = rendered_body,
        site_url = site_url,
        unsubscribe_url = unsubscribe_url,
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
///
/// `unsubscribe_url` is `Some` for list mail and `None` for transactional mail.
/// The confirmation message is the second kind: its recipient is by definition not
/// on the list yet, so advertising List-Unsubscribe on it would offer to remove an
/// address that was never added -- and a client that honours One-Click would fire a
/// POST at an address the Worker cannot act on.
async fn jmap_send_email(
    cfg: &SenderConfig,
    to: &str,
    subject: &str,
    html_body: &str,
    unsubscribe_url: Option<&str>,
) -> std::result::Result<(), String> {
    let plain_text = html_to_plain_text(html_body);

    let mut draft = serde_json::json!({
        "mailboxIds": { "d": true },
        "from": [{ "name": "Emil Lindfors", "email": cfg.from }],
        "to": [{ "email": to }],
        "subject": subject,
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
    });

    if let (Some(url), Some(obj)) = (unsubscribe_url, draft.as_object_mut()) {
        obj.insert(
            "header:List-Unsubscribe:asRaw".into(),
            serde_json::Value::String(format!(" <{}>", url)),
        );
        obj.insert(
            "header:List-Unsubscribe-Post:asRaw".into(),
            serde_json::Value::String(" List-Unsubscribe=One-Click".into()),
        );
    }

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
                    "accountId": cfg.account_id,
                    "create": { "draft": draft }
                },
                "0"
            ],
            [
                "EmailSubmission/set",
                {
                    "accountId": cfg.account_id,
                    "create": {
                        "send": {
                            "identityId": cfg.identity_id,
                            "emailId": "#draft",
                            "envelope": {
                                "mailFrom": { "email": cfg.from },
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

    let resp_body = jmap_call(&cfg.base_url, &cfg.credentials, &body).await?;

    // Email/set and EmailSubmission/set report per-object failures in notCreated.
    for (method, result) in method_responses(&resp_body) {
        if let Some(err) = method_error(method, result, "notCreated") {
            return Err(err);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Confirmation tokens (double opt-in)
// ---------------------------------------------------------------------------
//
// /api/subscribe no longer touches the list. It mails a signed link, and only the
// click on that link adds the address -- so an address reaches `recipients` only
// once someone has proved they can read mail at it.
//
// The usual shape for this is a pending-subscribers table, which this system has
// nowhere to put: there is no database, and adding one for a row that lives for
// two days would be the largest structural change on the site. It is not needed.
// The pending state is only ever (address, deadline), and both values can travel
// in the link itself provided the Worker can tell its own links from forged ones.
// That is exactly HMAC: sign `exp:email` with a server-side secret, and a link is
// self-authenticating with no storage at all.
//
// `exp` comes first in the signed string and is decimal digits terminated by ':',
// so the split between the two fields is unambiguous -- an attacker cannot shift
// bytes from one field into the other to make a different pair verify under the
// same signature.
//
// Rotating CONFIRM_SECRET invalidates every link that has not yet been clicked.
// With a 48-hour window that is a small, bounded cost, and it is the whole
// revocation story: there is no issued-token list to walk.

/// How long a confirmation link stays valid. Long enough to survive a weekend and
/// a greylisting delay, short enough that a leaked link is not a standing grant.
const CONFIRM_TTL_SECS: u64 = 48 * 60 * 60;

/// Current wall-clock time in seconds. Workers freeze `Date.now()` between I/O
/// operations, which is harmless here: the value only needs to be right to within
/// far less than the 48-hour window.
fn now_secs() -> u64 {
    Date::now().as_millis() / 1000
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// Compare two byte strings without an early return on the first difference, so
/// the time taken does not reveal how much of a submitted signature was correct.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// HMAC-SHA256 over an already-canonicalised payload, hex encoded.
///
/// Every caller must include a purpose prefix in `payload`. Two kinds of token now
/// share this secret, and without the prefix a link that proves someone wants *in*
/// would be a valid instruction to take them *out*, and vice versa. The `v1` marker
/// lets the format change later without old links silently verifying under new rules.
fn sign(secret: &str, payload: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(payload.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

fn confirm_signature(secret: &str, email: &str, exp: u64) -> String {
    sign(secret, &format!("confirm:v1:{}:{}", exp, email))
}

/// Verify a submitted signature against the address and expiry it claims to cover.
fn confirm_signature_matches(secret: &str, email: &str, exp: u64, sig: &str) -> bool {
    constant_time_eq(
        confirm_signature(secret, email, exp).as_bytes(),
        sig.as_bytes(),
    )
}

// ---------------------------------------------------------------------------
// Unsubscribe tokens
// ---------------------------------------------------------------------------
//
// One-click unsubscribe (RFC 8058) needs the List-Unsubscribe URL to identify its
// recipient, because the POST a mail client sends carries nothing but the literal
// body `List-Unsubscribe=One-Click`. That is why the newsletter is now sent per
// recipient rather than fanned out by Stalwart from one message: a single shared
// message can only carry a single shared URL, and a single shared URL cannot say
// who is unsubscribing.
//
// **These deliberately do not expire.** A confirmation link is an invitation and
// should go stale; an unsubscribe link sits in a mailbox for as long as the
// subscriber keeps the message, and one that stops working is how a reader who
// wanted to leave quietly ends up reporting the mail as spam instead. The token is
// an *identifier*, not an authorisation -- `/api/unsubscribe` is deliberately open,
// so the signature grants nothing that a bare POST to the JSON endpoint does not.
// It is here to name the recipient and to make the URL tamper-evident.
//
// The cost of never expiring: rotating CONFIRM_SECRET strands every unsubscribe
// link in already-delivered mail. Survivable, and only because the typed-address
// form at /api/unsubscribe is a permanent fallback that needs no token at all.

fn unsubscribe_signature(secret: &str, email: &str) -> String {
    sign(secret, &format!("unsub:v1:{}", email))
}

fn unsubscribe_signature_matches(secret: &str, email: &str, sig: &str) -> bool {
    constant_time_eq(
        unsubscribe_signature(secret, email).as_bytes(),
        sig.as_bytes(),
    )
}

/// Build the per-recipient unsubscribe URL that goes in both the `List-Unsubscribe`
/// header and the footer link. Form-encoded for the same reason as `confirm_link`.
fn unsubscribe_link(site_url: &str, email: &str, sig: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("email", email)
        .append_pair("sig", sig)
        .finish();
    format!("{}/api/unsubscribe?{}", site_url, query)
}

/// Pull a signed unsubscribe token out of decoded key/value pairs.
fn parse_unsubscribe_token<I: Iterator<Item = (String, String)>>(
    pairs: I,
) -> Option<(String, String)> {
    let (mut email, mut sig) = (None, None);
    for (key, value) in pairs {
        match key.as_str() {
            "email" => email = Some(value),
            "sig" => sig = Some(value),
            _ => {}
        }
    }
    Some((email?, sig?))
}

/// Build the confirmation link. The address goes through form encoding because
/// `is_valid_email` permits `+` in the local part, and a raw `+` in a query string
/// decodes back as a space -- which would silently confirm a different address
/// than the one that was signed, and then fail the signature check.
fn confirm_link(site_url: &str, email: &str, exp: u64, sig: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("email", email)
        .append_pair("exp", &exp.to_string())
        .append_pair("sig", sig)
        .finish();
    format!("{}/api/confirm?{}", site_url, query)
}

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------
//
// Double opt-in makes one abuse case worse before it makes it better: /api/subscribe
// used to add a row, and now it makes this server send mail to any address a stranger
// types. Unrated, that is a mail bomb aimed by anyone at anyone, from a self-hosted
// sender whose reputation is the whole asset. So the limiter is not a follow-up to
// this change, it is part of it.
//
// Cloudflare's rate limiting binding caps `period` at 60 seconds, so what these
// bindings actually bound is burst rate, not daily volume: a caller pacing itself
// under the limit can still trickle. That is the ceiling of this mechanism, not a
// misconfiguration -- sustained-abuse limits need a WAF rule, which is dashboard
// state rather than anything expressible here.

/// Consume one token from a rate limiter binding.
///
/// **Fails closed.** A missing or erroring binding denies the request. The
/// alternative -- carrying on unlimited when the limiter cannot be reached -- turns
/// a config mistake into a silently unprotected mail sender, which is the exact
/// failure this is here to prevent. It is loud instead: signup returns 503 and the
/// reason is in the log.
async fn rate_limit_allows(
    env: &Env,
    binding: &str,
    key: &str,
) -> std::result::Result<bool, String> {
    let limiter = env
        .rate_limiter(binding)
        .map_err(|e| format!("rate limiter {} unavailable: {}", binding, e))?;
    limiter
        .limit(key.to_string())
        .await
        .map(|outcome| outcome.success)
        .map_err(|e| format!("rate limiter {} failed: {}", binding, e))
}

/// The caller's IP as Cloudflare sees it. `CF-Connecting-IP` is set by the edge and
/// cannot be spoofed by the client, unlike `X-Forwarded-For`. The fallback keys every
/// header-less request together, which is the conservative direction: they share one
/// bucket rather than each getting a fresh one.
fn client_ip(req: &Request) -> String {
    req.headers()
        .get("CF-Connecting-IP")
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string())
}

/// Apply a limiter and, if it denies, produce the 429 to return.
async fn enforce_rate_limit(
    env: &Env,
    binding: &str,
    key: &str,
    headers: &Headers,
) -> Result<Option<Response>> {
    match rate_limit_allows(env, binding, key).await {
        Ok(true) => Ok(None),
        Ok(false) => Ok(Some(json_response(
            &ApiResponse {
                success: false,
                error: Some("Too many requests — please wait a minute and try again.".into()),
            },
            429,
            headers.clone(),
        )?)),
        Err(e) => {
            console_error!("{}", e);
            Ok(Some(json_response(
                &ApiResponse {
                    success: false,
                    error: Some("Service temporarily unavailable".into()),
                },
                503,
                headers.clone(),
            )?))
        }
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[event(fetch, respond_with_errors)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/api/subscribe", handle_subscribe)
        .get_async("/api/confirm", handle_confirm_page)
        .post_async("/api/confirm", handle_confirm)
        .get_async("/api/unsubscribe", handle_unsubscribe_page)
        .post_async("/api/unsubscribe", handle_unsubscribe_post)
        .get_async("/api/subscribers", handle_subscribers)
        .post_async("/api/send-newsletter", handle_send_newsletter)
        .options("/api/subscribe", handle_preflight)
        .options("/api/unsubscribe", handle_preflight)
        .run(req, env)
        .await
}

/// Read and validate the `email` field of a JSON body, or produce the 400 to return
/// in its place.
///
/// Only `/api/subscribe` uses this now. Unsubscribe reads its body as text instead,
/// because it has to accept a form encoding that `req.json()` cannot parse.
async fn read_email_field(
    req: &mut Request,
    headers: &Headers,
) -> Result<std::result::Result<String, Response>> {
    let reject = |message: &str| -> Result<Response> {
        json_response(
            &ApiResponse {
                success: false,
                error: Some(message.into()),
            },
            400,
            headers.clone(),
        )
    };

    let Ok(body) = req.json::<EmailRequest>().await else {
        return Ok(Err(reject("Invalid request body")?));
    };

    let email = body.email.trim().to_lowercase();
    if !is_valid_email(&email) {
        return Ok(Err(reject("Invalid email address")?));
    }

    Ok(Ok(email))
}

/// POST /api/subscribe — mail a signed confirmation link. Does **not** touch the list.
///
/// The response is identical whether the address is already subscribed, newly pending,
/// or nonexistent. Reporting "already subscribed" would turn this into a membership
/// oracle for any address a stranger cares to type, and the uniform answer costs
/// nothing: confirming an address that is already on the list sets
/// `recipients/<addr>` to true a second time, which is a no-op.
async fn handle_subscribe(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let headers = cors_headers(&req)?;

    let email = match read_email_field(&mut req, &headers).await? {
        Ok(email) => email,
        Err(resp) => return Ok(resp),
    };

    // Two limiters, because they bound different attacks and neither covers the other.
    // Per-IP stops one source spraying confirmation mail at many addresses; per-address
    // stops many sources converging on one inbox. The per-address check runs second so
    // a spraying client is rejected before its target's own budget is spent.
    for (binding, key) in [
        ("SUBSCRIBE_IP_LIMITER", client_ip(&req)),
        ("SUBSCRIBE_EMAIL_LIMITER", email.clone()),
    ] {
        if let Some(resp) = enforce_rate_limit(&ctx.env, binding, &key, &headers).await? {
            return Ok(resp);
        }
    }

    let site_url = ctx.env.var("SITE_URL")?.to_string();
    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();
    let exp = now_secs() + CONFIRM_TTL_SECS;
    let sig = confirm_signature(&secret, &email, exp);
    let link = confirm_link(&site_url, &email, exp, &sig);

    match jmap_send_email(
        &SenderConfig::from_env(&ctx.env)?,
        &email,
        "Confirm your subscription to lindfors.no",
        &confirmation_email_template(&link, &site_url),
        None,
    )
    .await
    {
        Ok(()) => {
            // The address is deliberately absent from this line. The log records that a
            // confirmation went out, not who is mid-signup.
            console_log!("Confirmation email sent");
            json_response(
                &ApiResponse {
                    success: true,
                    error: None,
                },
                200,
                headers,
            )
        }
        Err(e) => {
            console_error!("Confirmation send failed: {}", e);
            json_response(
                &ApiResponse {
                    success: false,
                    error: Some("Could not send the confirmation email. Please try again.".into()),
                },
                502,
                headers,
            )
        }
    }
}

/// GET /api/unsubscribe — with a signed token, a one-button confirmation; without
/// one, the typed-address form.
///
/// **Performs nothing either way**, for the same reason `/api/confirm` does not: this
/// URL is printed in the footer of every newsletter, and mail scanners fetch every
/// link they find. A GET that unsubscribed would let Outlook Safe Links quietly empty
/// the list one reader at a time -- a far worse failure here than on confirm, because
/// nobody notices they have been unsubscribed until they wonder why the mail stopped.
async fn handle_unsubscribe_page(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();

    let token = parse_unsubscribe_token(req.url()?.query_pairs().into_owned())
        .filter(|(email, sig)| unsubscribe_signature_matches(&secret, email, sig));

    // An absent or bad token is not an error worth a page of its own: fall through to
    // the form, which needs no token and is the permanent way back for anyone whose
    // link has been mangled or whose secret was rotated.
    let Some((email, sig)) = token else {
        return Response::from_html(unsubscribe_form_page());
    };

    Response::from_html(page_shell(
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
    ))
}

/// POST /api/unsubscribe — remove an address from the list.
///
/// Three kinds of caller reach this, and they disagree about everything except the
/// outcome:
///   - the site's form, sending JSON `{"email": "..."}` and expecting JSON back
///   - the button on the page above, posting a signed token as form fields
///   - an RFC 8058 one-click unsubscribe, posting the fixed body
///     `List-Unsubscribe=One-Click` with the token in the *URL* -- the body carries
///     nothing identifying at all, which is why the URL has to
///
/// The last of those is why this endpoint used to be broken: it called `req.json()`,
/// which fails on a form body, so every one-click unsubscribe got a 400 while the
/// message that prompted it advertised one-click support.
///
/// Still deliberately unauthenticated in the untokened JSON case. Subscribing needs
/// proof of control because it creates an obligation; unsubscribing removes one, and
/// the worst an unauthenticated caller achieves is stopping mail to an address that is
/// not theirs. A subscriber who cannot leave easily reports the mail as spam instead.
async fn handle_unsubscribe_post(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let headers = cors_headers(&req)?;
    let url = req.url()?;

    if let Some(resp) = enforce_rate_limit(
        &ctx.env,
        "API_IP_LIMITER",
        &format!("unsub:{}", client_ip(&req)),
        &headers,
    )
    .await?
    {
        return Ok(resp);
    }

    // Read the body once, as text, and decide what it is afterwards. `req.json()` is
    // what made the form-encoded case impossible to handle.
    let body = req.text().await.unwrap_or_default();
    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();

    // A signed token beats the body: it identifies the recipient without trusting the
    // caller to name themselves, and it is the only thing a one-click POST provides.
    let signed = parse_unsubscribe_token(form_urlencoded::parse(body.as_bytes()).into_owned())
        .or_else(|| parse_unsubscribe_token(url.query_pairs().into_owned()))
        .filter(|(email, sig)| unsubscribe_signature_matches(&secret, email, sig));

    if let Some((email, _)) = signed {
        return match jmap_set_recipient(&ListConfig::from_env(&ctx.env)?, &email, false).await {
            Ok(()) => {
                console_log!("Unsubscribed via signed link");
                // HTML, even for the one-click POST. RFC 8058 says the response body is
                // never shown to the user, so a page costs the machine nothing and is
                // what the human pressing the button needs.
                Response::from_html(page_shell(
                    "Unsubscribed",
                    r#"<h1>Unsubscribed</h1>
<p>That address has been removed and will not receive further newsletters.</p>
<p class="note">Changed your mind? You can sign up again from the site.</p>"#,
                ))
            }
            Err(e) => {
                console_error!("Signed unsubscribe failed: {}", e);
                Ok(Response::from_html(page_shell(
                    "Something went wrong",
                    r#"<h1>Something went wrong</h1>
<p>The link was valid, but the change could not be saved. Please try again in a few minutes.</p>"#,
                ))?
                .with_status(502))
            }
        };
    }

    // No usable token: the site's JSON form, which answers in JSON.
    let Ok(parsed) = serde_json::from_str::<EmailRequest>(&body) else {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Invalid request body".into()),
            },
            400,
            headers,
        );
    };

    let email = parsed.email.trim().to_lowercase();
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

    match jmap_set_recipient(&ListConfig::from_env(&ctx.env)?, &email, false).await {
        Ok(()) => json_response(
            &ApiResponse {
                success: true,
                error: None,
            },
            200,
            headers,
        ),
        Err(e) => {
            // The upstream detail goes to `wrangler tail`, not to the caller: this is a
            // public endpoint and the detail names an authenticated JMAP method.
            console_error!("mailing list unsubscribe failed: {}", e);
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

    let list = ListConfig::from_env(&ctx.env)?;

    let members = jmap_get_recipients(&list).await.map_err(Error::RustError)?;

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
    if body.slug.is_empty()
        || !body
            .slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some(
                    "Invalid slug — only lowercase letters, digits, and hyphens allowed".into(),
                ),
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

    let title = meta
        .get("title")
        .cloned()
        .unwrap_or_else(|| body.slug.clone());
    let description = meta.get("description").cloned().unwrap_or_default();
    let date = meta.get("date").cloned().unwrap_or_default();
    let post_url = meta
        .get("url")
        .cloned()
        .unwrap_or_else(|| format!("{}/blog/{}/", site_url, body.slug));

    let rendered_body = render_markdown(md_body);
    let subject = body.subject.unwrap_or(title.clone());

    // One message per recipient, rather than one message to the list alias for
    // Stalwart to fan out.
    //
    // The fan-out was cheaper and it is genuinely a loss, but it made one-click
    // unsubscribe impossible: every recipient received byte-identical headers, so
    // `List-Unsubscribe` could only ever name a URL that did not know who was
    // clicking it. The header claimed RFC 8058 support the endpoint could not
    // honour. Sending individually is what lets each message carry its own signed
    // URL -- in the header and in the footer link, so leaving costs one click
    // instead of typing your own address into a form.
    let list = ListConfig::from_env(&ctx.env)?;
    let recipients = jmap_get_recipients(&list).await.map_err(Error::RustError)?;

    if recipients.len() > MAX_RECIPIENTS_PER_SEND {
        // Refused rather than truncated. Each send is a subrequest, and Workers caps
        // those per invocation; quietly mailing the first 45 of a longer list and
        // reporting success is the worst available outcome. Past this point the send
        // needs batching or a queue, which is P10's problem.
        console_error!(
            "Refusing to send: {} recipients exceeds the {} cap",
            recipients.len(),
            MAX_RECIPIENTS_PER_SEND
        );
        return json_response_value(
            &SendResponse {
                success: false,
                sent: 0,
                failed: vec![],
                error: Some(format!(
                    "List has {} recipients, above the {} the Worker can send in one \
                     invocation. Sending would silently truncate.",
                    recipients.len(),
                    MAX_RECIPIENTS_PER_SEND
                )),
            },
            400,
            cors_headers(&req)?,
        );
    }

    let sender = SenderConfig::from_env(&ctx.env)?;
    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();

    let mut sent = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for email in &recipients {
        let unsubscribe_url =
            unsubscribe_link(&site_url, email, &unsubscribe_signature(&secret, email));
        let html = email_template(
            &title,
            &description,
            &date,
            &post_url,
            &rendered_body,
            &site_url,
            &unsubscribe_url,
        );

        match jmap_send_email(&sender, email, &subject, &html, Some(&unsubscribe_url)).await {
            Ok(()) => sent += 1,
            Err(e) => {
                // The address is named here because this endpoint is admin-only and
                // the operator needs to know exactly who to retry.
                console_error!("Newsletter send to {} failed: {}", email, e);
                failed.push(email.clone());
            }
        }
    }

    console_log!(
        "Newsletter {}: {} sent, {} failed",
        body.slug,
        sent,
        failed.len()
    );

    // A partial send is reported as a failure, not a success with a footnote. It
    // needs someone to look at it, and the CLI keys off the HTTP status.
    let all_ok = failed.is_empty();
    json_response_value(
        &SendResponse {
            success: all_ok,
            sent,
            failed,
            error: if all_ok {
                None
            } else {
                Some("Some recipients could not be reached; see `failed`.".into())
            },
        },
        if all_ok { 200 } else { 502 },
        cors_headers(&req)?,
    )
}

// ---------------------------------------------------------------------------
// Confirmation flow
// ---------------------------------------------------------------------------

/// The (address, deadline, signature) triple carried by a confirmation link.
struct ConfirmToken {
    email: String,
    exp: u64,
    sig: String,
}

/// Pull a token out of decoded key/value pairs -- query string on GET, form body on
/// POST. `exp` must parse as a `u64`, which is also what keeps the signed
/// `confirm:v1:<exp>:<email>` string unambiguous: no other split of those bytes
/// leaves a valid decimal expiry on the left.
fn parse_confirm_token<I: Iterator<Item = (String, String)>>(pairs: I) -> Option<ConfirmToken> {
    let (mut email, mut exp, mut sig) = (None, None, None);
    for (key, value) in pairs {
        match key.as_str() {
            "email" => email = Some(value),
            "exp" => exp = Some(value),
            "sig" => sig = Some(value),
            _ => {}
        }
    }
    Some(ConfirmToken {
        email: email?,
        exp: exp?.parse().ok()?,
        sig: sig?,
    })
}

/// What a token turned out to be.
enum TokenState {
    Valid,
    Expired,
    Invalid,
}

fn check_confirm_token(secret: &str, token: &ConfirmToken) -> TokenState {
    check_confirm_token_at(secret, token, now_secs())
}

/// The decision itself, with the clock passed in. Split out from `check_confirm_token`
/// so it can be tested: `now_secs` goes through `Date::now`, which is a JS call and
/// panics outside a runtime, and the expiry boundary is exactly the part worth a test.
fn check_confirm_token_at(secret: &str, token: &ConfirmToken, now: u64) -> TokenState {
    // Signature first. An expired-but-authentic link earns the honest "this expired,
    // sign up again"; a forged one must not be told which of its two fields was wrong.
    if !confirm_signature_matches(secret, &token.email, token.exp, &token.sig) {
        return TokenState::Invalid;
    }
    if now > token.exp {
        return TokenState::Expired;
    }
    TokenState::Valid
}

/// GET /api/confirm — render the confirmation button. **Performs nothing.**
///
/// The subscription happens on POST, not on this GET, because links in mail get
/// fetched by things that are not the recipient: Outlook Safe Links, corporate
/// scanners, and any client that prefetches. A GET that subscribed would let those
/// confirm on the reader's behalf, which is precisely the human act this whole
/// mechanism exists to require. It also restores the HTTP contract that a GET is
/// safe to repeat.
///
/// The form is plain HTML with hidden fields rather than a fetch, so it works with
/// JavaScript disabled and needs nothing from the CSP.
async fn handle_confirm_page(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();

    let Some(token) = parse_confirm_token(req.url()?.query_pairs().into_owned()) else {
        return confirm_error_page(TokenState::Invalid);
    };

    match check_confirm_token(&secret, &token) {
        TokenState::Valid => Response::from_html(page_shell(
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
        )),
        state => confirm_error_page(state),
    }
}

/// POST /api/confirm — verify the signed link and add the address to the list.
///
/// This is the step that makes the consent demonstrable: reaching it requires having
/// read a message delivered to the address, and having clicked. Subscribing an
/// address already on the list is a no-op, so a second click is harmless.
async fn handle_confirm(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let url = req.url()?;

    match rate_limit_allows(
        &ctx.env,
        "API_IP_LIMITER",
        &format!("confirm:{}", client_ip(&req)),
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            return Ok(Response::from_html(page_shell(
                "Too many requests",
                "<h1>Too many requests</h1><p>Please wait a minute and click the link again.</p>",
            ))?
            .with_status(429));
        }
        Err(e) => {
            console_error!("{}", e);
            return Ok(Response::from_html(page_shell(
                "Temporarily unavailable",
                "<h1>Temporarily unavailable</h1><p>Please try the link again in a few minutes.</p>",
            ))?
            .with_status(503));
        }
    }

    // The form posts the token in the body; the query string is accepted too, so the
    // endpoint stays usable from curl without a form encoding.
    let body = req.text().await.unwrap_or_default();
    let token = parse_confirm_token(form_urlencoded::parse(body.as_bytes()).into_owned())
        .or_else(|| parse_confirm_token(url.query_pairs().into_owned()));

    let Some(token) = token else {
        return confirm_error_page(TokenState::Invalid);
    };

    let secret = ctx.env.secret("CONFIRM_SECRET")?.to_string();
    match check_confirm_token(&secret, &token) {
        TokenState::Valid => {}
        state => return confirm_error_page(state),
    }

    let list = ListConfig::from_env(&ctx.env)?;

    match jmap_set_recipient(&list, &token.email, true).await {
        Ok(()) => {
            console_log!("Subscription confirmed");
            Response::from_html(page_shell(
                "You are subscribed",
                r#"<h1>You&rsquo;re subscribed</h1>
<p>New posts will arrive by email. Every issue carries an unsubscribe link, and you can also <a href="/api/unsubscribe">unsubscribe here</a> at any time.</p>"#,
            ))
        }
        Err(e) => {
            console_error!("Confirm failed: {}", e);
            Ok(Response::from_html(page_shell(
                "Something went wrong",
                r#"<h1>Something went wrong</h1>
<p>The link was valid, but the subscription could not be saved. Please try again in a few minutes &mdash; the link keeps working until it expires.</p>"#,
            ))?
            .with_status(502))
        }
    }
}

/// Render the page for a token that did not check out. `Valid` cannot reach here.
fn confirm_error_page(state: TokenState) -> Result<Response> {
    let (status, title, body) = match state {
        TokenState::Expired => (
            410,
            "Link expired",
            r#"<h1>Link expired</h1>
<p>Confirmation links are good for 48 hours, and this one is past that. Nothing was added.</p>
<p><a href="https://lindfors.no">Sign up again on lindfors.no</a> and a fresh link will be on its way.</p>"#,
        ),
        // An expired link is told so above; everything else gets one answer, so a
        // forged link learns nothing about which part of it was wrong.
        _ => (
            400,
            "Invalid link",
            r#"<h1>Invalid link</h1>
<p>This confirmation link isn&rsquo;t valid. It may have been altered in transit, or truncated by a mail client that wrapped the line.</p>
<p><a href="https://lindfors.no">Sign up again on lindfors.no</a> to get a fresh one.</p>"#,
        ),
    };
    Ok(Response::from_html(page_shell(title, body))?.with_status(status))
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
//
// The Worker serves three pages of its own -- confirm, unsubscribe, and the
// outcomes of both. They are reached from links in email, so they are the first
// thing a new subscriber sees after the message itself and should not look like
// they belong to a different site. One shell, one stylesheet, no framework.

fn html_escape(s: &str) -> String {
    // `&` first: escaping it after the others would re-escape the `&` each of them
    // introduces.
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

/// Shared chrome for the Worker's HTML pages. `noindex` because none of these are
/// content: a confirmation outcome in a search index would be noise at best.
fn page_shell(title: &str, body: &str) -> String {
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

/// The confirmation message. Deliberately short: the only thing it has to do is
/// carry one link and make clear what clicking it means.
///
/// The link is repeated as plain text under the button because some clients strip
/// or fail to linkify styled anchors, and a confirmation mail whose link cannot be
/// reached is a subscriber lost silently.
fn confirmation_email_template(confirm_url: &str, site_url: &str) -> String {
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
</html>"#,
        site_url = site_url,
        escaped = escaped,
    )
}

/// GET /api/unsubscribe — the address is typed here rather than carried in a signed
/// link because a single message is fanned out to the list by Stalwart, so the Worker
/// never sees an individual recipient to mint a per-subscriber link for.
fn unsubscribe_form_page() -> String {
    page_shell(
        "Unsubscribe",
        r#"    <h1>Unsubscribe</h1>
    <p>Enter your email to unsubscribe from the lindfors.no newsletter.</p>
    <form id="unsub-form">
        <input type="email" name="email" placeholder="your@email.com" required aria-label="Email address">
        <button type="submit">Unsubscribe</button>
    </form>
    <div id="msg"></div>
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
    </script>"#,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// Everything here is pure logic that runs on the host target. Anything touching
// `worker`'s JS bindings -- fetch, Date::now, the rate limiter -- cannot run
// outside a runtime, which is why the clock is a parameter in
// `check_confirm_token_at` and why these tests stop at the JMAP boundary.

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-not-the-real-one";

    fn token(email: &str, exp: u64, secret: &str) -> ConfirmToken {
        ConfirmToken {
            email: email.to_string(),
            exp,
            sig: confirm_signature(secret, email, exp),
        }
    }

    #[test]
    fn signature_is_deterministic() {
        assert_eq!(
            confirm_signature(SECRET, "a@example.com", 1000),
            confirm_signature(SECRET, "a@example.com", 1000)
        );
    }

    #[test]
    fn signature_is_hex_sha256_width() {
        let sig = confirm_signature(SECRET, "a@example.com", 1000);
        assert_eq!(sig.len(), 64);
        assert!(sig
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn signature_binds_every_field() {
        let base = confirm_signature(SECRET, "a@example.com", 1000);
        assert_ne!(base, confirm_signature(SECRET, "b@example.com", 1000));
        assert_ne!(base, confirm_signature(SECRET, "a@example.com", 1001));
        assert_ne!(
            base,
            confirm_signature("other-secret", "a@example.com", 1000)
        );
    }

    /// The whole point of the scheme: a signature for one address must not confirm
    /// another. Without this, anyone holding their own valid link could subscribe
    /// any address they liked.
    #[test]
    fn signature_does_not_transfer_to_another_address() {
        let mine = token("attacker@example.com", 9999, SECRET);
        assert!(!confirm_signature_matches(
            SECRET,
            "victim@example.com",
            mine.exp,
            &mine.sig
        ));
    }

    /// `exp` leads the signed string and is decimal digits terminated by ':', so no
    /// alternative split of the same bytes is a well-formed (exp, email) pair. If
    /// the order were reversed, `exp=1` + `email=":2:a@b.com"` and `exp=12` +
    /// `email="a@b.com"` could be made to collide.
    #[test]
    fn field_boundary_cannot_be_shifted() {
        assert_ne!(
            confirm_signature(SECRET, "a@example.com", 12),
            confirm_signature(SECRET, ":a@example.com", 1)
        );
    }

    #[test]
    fn constant_time_eq_matches_normal_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn valid_token_passes_before_expiry() {
        let t = token("a@example.com", 1000, SECRET);
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 999),
            TokenState::Valid
        ));
        // The boundary itself is still valid -- `now > exp` expires, not `>=`.
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 1000),
            TokenState::Valid
        ));
    }

    #[test]
    fn token_expires_one_second_past_the_deadline() {
        let t = token("a@example.com", 1000, SECRET);
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 1001),
            TokenState::Expired
        ));
    }

    /// A forged link reads as Invalid whether or not its claimed expiry has passed,
    /// so the response cannot be used to probe which field was rejected.
    #[test]
    fn forged_signature_is_invalid_not_expired() {
        let t = ConfirmToken {
            email: "a@example.com".into(),
            exp: 1000,
            sig: "0".repeat(64),
        };
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 1),
            TokenState::Invalid
        ));
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 99999),
            TokenState::Invalid
        ));
    }

    #[test]
    fn token_signed_with_another_secret_is_invalid() {
        let t = token("a@example.com", 1000, "some-other-secret");
        assert!(matches!(
            check_confirm_token_at(SECRET, &t, 999),
            TokenState::Invalid
        ));
    }

    /// The reason the link is form-encoded rather than interpolated. `+` is legal in
    /// a local part and extremely common (`emil+news@`), and a raw `+` in a query
    /// string decodes back as a space -- which would confirm a different address than
    /// the one that was signed, and then fail its own signature check.
    #[test]
    fn plus_addressing_round_trips_through_the_link() {
        let email = "emil+news@example.com";
        let exp = 1_700_000_000u64;
        let sig = confirm_signature(SECRET, email, exp);
        let link = confirm_link("https://lindfors.no", email, exp, &sig);

        assert!(
            !link.contains("emil+news"),
            "raw '+' survived into the URL: {link}"
        );

        let url = Url::parse(&link).expect("link parses");
        let parsed = parse_confirm_token(url.query_pairs().into_owned()).expect("token parses");
        assert_eq!(parsed.email, email);
        assert_eq!(parsed.exp, exp);
        assert!(matches!(
            check_confirm_token_at(SECRET, &parsed, exp - 1),
            TokenState::Valid
        ));
    }

    /// The POST path reads the same fields out of a form body.
    #[test]
    fn token_round_trips_through_a_form_body() {
        let email = "a+b@example.com";
        let exp = 42u64;
        let sig = confirm_signature(SECRET, email, exp);
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("email", email)
            .append_pair("exp", &exp.to_string())
            .append_pair("sig", &sig)
            .finish();

        let parsed = parse_confirm_token(form_urlencoded::parse(body.as_bytes()).into_owned())
            .expect("token parses");
        assert_eq!(parsed.email, email);
        assert_eq!(parsed.exp, exp);
        assert_eq!(parsed.sig, sig);
    }

    #[test]
    fn incomplete_or_malformed_tokens_do_not_parse() {
        let missing_sig = [
            ("email".to_string(), "a@b.com".to_string()),
            ("exp".to_string(), "1".to_string()),
        ];
        assert!(parse_confirm_token(missing_sig.into_iter()).is_none());

        let bad_exp = [
            ("email".to_string(), "a@b.com".to_string()),
            ("exp".to_string(), "not-a-number".to_string()),
            ("sig".to_string(), "x".to_string()),
        ];
        assert!(parse_confirm_token(bad_exp.into_iter()).is_none());

        assert!(parse_confirm_token(std::iter::empty()).is_none());
    }

    #[test]
    fn confirm_link_points_at_the_confirm_endpoint() {
        let link = confirm_link("https://lindfors.no", "a@b.com", 1, "deadbeef");
        assert!(link.starts_with("https://lindfors.no/api/confirm?"));
    }

    #[test]
    fn html_escape_does_not_double_escape() {
        assert_eq!(html_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
        // `&` handled first, so the entities the later passes introduce stay intact.
        assert_eq!(html_escape("<"), "&lt;");
    }

    /// The confirm page interpolates the address into an attribute; it is only ever
    /// an address we signed, but the escaping is what makes that not matter.
    #[test]
    fn escaped_address_cannot_break_out_of_an_attribute() {
        let escaped = html_escape("a\"><script>alert(1)</script>@b.com");
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('"'));
    }

    #[test]
    fn json_pointer_escape_orders_its_replacements() {
        // `~` before `/`, or the `~1` from the second pass gets re-escaped to `~01`.
        assert_eq!(json_pointer_escape("a~b"), "a~0b");
        assert_eq!(json_pointer_escape("a/b"), "a~1b");
        assert_eq!(json_pointer_escape("a~/b"), "a~0~1b");
    }

    #[test]
    fn email_validation_accepts_ordinary_addresses() {
        for email in [
            "a@b.co",
            "emil@lindfors.no",
            "emil+news@lindfors.no",
            "first.last@sub.example.com",
        ] {
            assert!(is_valid_email(email), "should accept {email}");
        }
    }

    #[test]
    fn email_validation_rejects_malformed_addresses() {
        for email in [
            "",
            "no-at-sign",
            "@example.com",
            ".leading@example.com",
            "trailing.@example.com",
            "double..dot@example.com",
            "a@b",
            "a@.example.com",
            "a@-example.com",
            "a@example-.com",
            "a@-example.com",
            "a@example.c",
            "a@1.2.3.4",
            "a b@example.com",
            "a\"b@example.com",
            "a,b@example.com",
            "a@example.com ",
            "a@example..com",
            "a@exam ple.com",
        ] {
            assert!(!is_valid_email(email), "should reject {email:?}");
        }
    }

    #[test]
    fn email_validation_enforces_length_limits() {
        let long_local = format!("{}@example.com", "a".repeat(65));
        assert!(!is_valid_email(&long_local));

        let long_total = format!("{}@{}.com", "a".repeat(64), "b".repeat(200));
        assert!(!is_valid_email(&long_total));
    }

    /// The link appears twice, and both times html-escaped: its query string joins
    /// three parameters with `&`, which is an entity opener in both an attribute and
    /// a text node. The raw form must not appear at all.
    #[test]
    fn confirmation_email_carries_the_escaped_link_twice() {
        let link = confirm_link("https://lindfors.no", "a@b.com", 1, "deadbeef");
        assert!(
            link.contains('&'),
            "precondition: the link has a multi-param query"
        );

        let html = confirmation_email_template(&link, "https://lindfors.no");
        // Once as the button href, once as copyable text for clients that strip it.
        assert_eq!(html.matches(&html_escape(&link)).count(), 2);
        assert!(
            !html.contains(&link),
            "unescaped link leaked into the markup"
        );
    }

    /// ...and the text/plain alternative turns it back into a working URL, since
    /// `&amp;` in a plain-text body is just wrong characters.
    #[test]
    fn plain_text_alternative_restores_the_link() {
        let link = confirm_link("https://lindfors.no", "a@b.com", 1, "deadbeef");
        let html = confirmation_email_template(&link, "https://lindfors.no");
        assert!(html_to_plain_text(&html).contains(&link));
    }

    // --- unsubscribe tokens ---

    /// The reason both payloads carry a purpose prefix. One secret signs two kinds of
    /// token, and without domain separation a link proving someone wants *in* would be
    /// a valid instruction to take them *out*.
    #[test]
    fn confirm_and_unsubscribe_tokens_are_not_interchangeable() {
        let email = "a@example.com";
        let exp = 1000u64;

        let confirm = confirm_signature(SECRET, email, exp);
        let unsub = unsubscribe_signature(SECRET, email);
        assert_ne!(confirm, unsub);

        // Neither verifies as the other.
        assert!(!unsubscribe_signature_matches(SECRET, email, &confirm));
        assert!(!confirm_signature_matches(SECRET, email, exp, &unsub));
    }

    #[test]
    fn unsubscribe_signature_binds_the_address() {
        let sig = unsubscribe_signature(SECRET, "a@example.com");
        assert!(unsubscribe_signature_matches(SECRET, "a@example.com", &sig));
        assert!(!unsubscribe_signature_matches(
            SECRET,
            "b@example.com",
            &sig
        ));
        assert!(!unsubscribe_signature_matches(
            "other-secret",
            "a@example.com",
            &sig
        ));
    }

    /// Unsubscribe links have no expiry field at all -- they sit in a mailbox for as
    /// long as the subscriber keeps the message, and one that goes stale is how
    /// someone who wanted to leave ends up reporting the mail as spam instead.
    #[test]
    fn unsubscribe_link_carries_no_expiry() {
        let link = unsubscribe_link("https://lindfors.no", "a@b.com", "deadbeef");
        assert!(link.starts_with("https://lindfors.no/api/unsubscribe?"));
        assert!(!link.contains("exp="));
    }

    #[test]
    fn unsubscribe_link_round_trips_plus_addressing() {
        let email = "emil+news@example.com";
        let sig = unsubscribe_signature(SECRET, email);
        let link = unsubscribe_link("https://lindfors.no", email, &sig);
        assert!(
            !link.contains("emil+news"),
            "raw '+' survived into the URL: {link}"
        );

        let url = Url::parse(&link).expect("link parses");
        let (parsed_email, parsed_sig) =
            parse_unsubscribe_token(url.query_pairs().into_owned()).expect("token parses");
        assert_eq!(parsed_email, email);
        assert!(unsubscribe_signature_matches(
            SECRET,
            &parsed_email,
            &parsed_sig
        ));
    }

    /// The shape an RFC 8058 client actually sends: the token is in the URL, and the
    /// body is the fixed string with nothing identifying in it.
    #[test]
    fn one_click_body_carries_no_token_so_the_url_must() {
        let body = "List-Unsubscribe=One-Click";
        assert!(
            parse_unsubscribe_token(form_urlencoded::parse(body.as_bytes()).into_owned()).is_none()
        );

        let email = "a@b.com";
        let sig = unsubscribe_signature(SECRET, email);
        let url = Url::parse(&unsubscribe_link("https://lindfors.no", email, &sig)).unwrap();
        let (e, sg) = parse_unsubscribe_token(url.query_pairs().into_owned()).unwrap();
        assert_eq!(e, email);
        assert!(unsubscribe_signature_matches(SECRET, &e, &sg));
    }

    #[test]
    fn unsubscribe_token_needs_both_fields() {
        let only_email = [("email".to_string(), "a@b.com".to_string())];
        assert!(parse_unsubscribe_token(only_email.into_iter()).is_none());

        let only_sig = [("sig".to_string(), "abc".to_string())];
        assert!(parse_unsubscribe_token(only_sig.into_iter()).is_none());
    }

    /// Every newsletter now carries the recipient's own link, escaped, in the footer.
    #[test]
    fn newsletter_footer_carries_the_recipient_link() {
        let email = "a+b@example.com";
        let link = unsubscribe_link(
            "https://lindfors.no",
            email,
            &unsubscribe_signature(SECRET, email),
        );
        let html = email_template(
            "Title",
            "Description",
            "2026-01-01",
            "https://lindfors.no/blog/x/",
            "<p>body</p>",
            "https://lindfors.no",
            &link,
        );

        assert!(html.contains(&html_escape(&link)));
        assert!(
            !html.contains(&link),
            "unescaped link leaked into the markup"
        );
        // The old un-personalised footer target must be gone.
        assert!(!html.contains(r#"href="https://lindfors.no/api/unsubscribe""#));
    }

    #[test]
    fn worker_pages_are_noindex() {
        assert!(page_shell("T", "<p>x</p>").contains(r#"<meta name="robots" content="noindex">"#));
        assert!(unsubscribe_form_page().contains("noindex"));
    }
}
