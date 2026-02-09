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

/// Stalwart PATCH body: atomic add/remove on a principal field.
#[derive(Serialize)]
struct StalwartPatchOp {
    action: &'static str,
    field: &'static str,
    value: String,
}

/// Stalwart principal (partial — only the fields we read).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StalwartPrincipal {
    #[serde(default)]
    external_members: Vec<String>,
}

/// Wrapper for Stalwart GET /api/principal/{id} response.
#[derive(Deserialize)]
struct StalwartGetResponse {
    data: StalwartPrincipal,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cors_headers(req: &Request) -> Result<Headers> {
    let origin = req.headers().get("Origin")?.unwrap_or_default();
    let allowed = if origin.contains("lindfors.no") {
        origin
    } else {
        "https://lindfors.no".to_string()
    };

    let headers = Headers::new();
    headers.set("Access-Control-Allow-Origin", &allowed)?;
    headers.set("Access-Control-Allow-Methods", "POST, GET, OPTIONS")?;
    headers.set("Access-Control-Allow-Headers", "Content-Type")?;
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
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].contains('.')
        && parts[1].len() >= 3
        && email.len() >= 5
}

/// Call the Stalwart Management API.
async fn stalwart_patch(
    api_url: &str,
    api_key: &str,
    list_id: &str,
    ops: &[StalwartPatchOp],
) -> Result<u16> {
    let url = format!("{}/api/principal/{}", api_url, list_id);
    let body = serde_json::to_string(ops).map_err(|e| Error::RustError(e.to_string()))?;

    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {}", api_key))?;
    headers.set("Content-Type", "application/json")?;

    let mut init = RequestInit::new();
    init.with_method(Method::Patch);
    init.with_headers(headers);
    init.with_body(Some(wasm_bindgen::JsValue::from_str(&body)));

    let req = Request::new_with_init(&url, &init)?;
    let resp = Fetch::Request(req).send().await?;
    Ok(resp.status_code())
}

/// Fetch the current external members of a Stalwart mailing list.
async fn stalwart_get_members(
    api_url: &str,
    api_key: &str,
    list_id: &str,
) -> Result<Vec<String>> {
    let url = format!("{}/api/principal/{}", api_url, list_id);

    let headers = Headers::new();
    headers.set("Authorization", &format!("Bearer {}", api_key))?;

    let mut init = RequestInit::new();
    init.with_method(Method::Get);
    init.with_headers(headers);

    let req = Request::new_with_init(&url, &init)?;
    let mut resp = Fetch::Request(req).send().await?;

    if resp.status_code() != 200 {
        return Err(Error::RustError(format!(
            "Stalwart API returned {}",
            resp.status_code()
        )));
    }

    let principal: StalwartGetResponse = resp.json().await?;
    Ok(principal.data.external_members)
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

/// Render markdown to HTML using pulldown-cmark.
fn render_markdown(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(md, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
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

/// Send an email via Stalwart's JMAP API using Email/set + EmailSubmission/set.
async fn jmap_send_email(
    base_url: &str,
    credentials: &str,
    account_id: &str,
    identity_id: &str,
    from: &str,
    to: &str,
    subject: &str,
    html_body: &str,
) -> Result<u16> {
    let url = format!("{}/jmap/", base_url);

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
                            "htmlBody": [{
                                "partId": "html",
                                "type": "text/html"
                            }],
                            "bodyValues": {
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

    let body_str =
        serde_json::to_string(&body).map_err(|e| Error::RustError(e.to_string()))?;

    let headers = Headers::new();
    headers.set("Authorization", &format!("Basic {}", credentials))?;
    headers.set("Content-Type", "application/json")?;

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_headers(headers);
    init.with_body(Some(wasm_bindgen::JsValue::from_str(&body_str)));

    let req = Request::new_with_init(&url, &init)?;
    let resp = Fetch::Request(req).send().await?;
    Ok(resp.status_code())
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

/// POST /api/subscribe — add email to the Stalwart mailing list.
async fn handle_subscribe(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
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

    let api_url = ctx.env.var("STALWART_API_URL")?.to_string();
    let api_key = ctx.env.secret("STALWART_API_KEY")?.to_string();
    let list_id = ctx.env.var("STALWART_LIST_ID")?.to_string();

    let ops = [StalwartPatchOp {
        action: "addItem",
        field: "externalMembers",
        value: email,
    }];

    match stalwart_patch(&api_url, &api_key, &list_id, &ops).await {
        Ok(status) if status < 300 => {
            json_response(&ApiResponse { success: true, error: None }, 200, headers)
        }
        Ok(status) => json_response(
            &ApiResponse {
                success: false,
                error: Some(format!("Upstream error ({})", status)),
            },
            502,
            headers,
        ),
        Err(_) => json_response(
            &ApiResponse {
                success: false,
                error: Some("Subscription failed".into()),
            },
            500,
            headers,
        ),
    }
}

/// GET /api/unsubscribe — show the unsubscribe form.
async fn handle_unsubscribe_page(_req: Request, _ctx: RouteContext<()>) -> Result<Response> {
    Response::from_html(&unsubscribe_form_page())
}

/// POST /api/unsubscribe — remove email from the Stalwart mailing list.
async fn handle_unsubscribe_post(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
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

    let api_url = ctx.env.var("STALWART_API_URL")?.to_string();
    let api_key = ctx.env.secret("STALWART_API_KEY")?.to_string();
    let list_id = ctx.env.var("STALWART_LIST_ID")?.to_string();

    let ops = [StalwartPatchOp {
        action: "removeItem",
        field: "externalMembers",
        value: email,
    }];

    match stalwart_patch(&api_url, &api_key, &list_id, &ops).await {
        Ok(status) if status < 300 => {
            json_response(&ApiResponse { success: true, error: None }, 200, headers)
        }
        Ok(_) | Err(_) => json_response(
            &ApiResponse {
                success: false,
                error: Some("Unsubscribe failed".into()),
            },
            500,
            headers,
        ),
    }
}

/// GET /api/subscribers?key=... — admin: list current subscribers from Stalwart.
async fn handle_subscribers(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let url = req.url()?;
    let params: std::collections::HashMap<String, String> =
        url.query_pairs().into_owned().collect();

    let key = params.get("key").cloned().unwrap_or_default();
    let admin_key = ctx.env.secret("ADMIN_KEY")?.to_string();

    if key != admin_key {
        return json_response(
            &ApiResponse {
                success: false,
                error: Some("Unauthorized".into()),
            },
            401,
            cors_headers(&req)?,
        );
    }

    let api_url = ctx.env.var("STALWART_API_URL")?.to_string();
    let api_key = ctx.env.secret("STALWART_API_KEY")?.to_string();
    let list_id = ctx.env.var("STALWART_LIST_ID")?.to_string();

    let members = stalwart_get_members(&api_url, &api_key, &list_id).await?;

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

/// POST /api/send-newsletter?key=... — admin: send a newsletter to the mailing list via JMAP.
async fn handle_send_newsletter(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let url = req.url()?;
    let params: std::collections::HashMap<String, String> =
        url.query_pairs().into_owned().collect();

    let key = params.get("key").cloned().unwrap_or_default();
    let admin_key = ctx.env.secret("ADMIN_KEY")?.to_string();

    if key != admin_key {
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
        &to,
        &subject,
        &html,
    )
    .await
    {
        Ok(status) if status == 200 => json_response(
            &ApiResponse {
                success: true,
                error: None,
            },
            200,
            cors_headers(&req)?,
        ),
        Ok(status) => json_response(
            &ApiResponse {
                success: false,
                error: Some(format!("JMAP request failed (status {})", status)),
            },
            502,
            cors_headers(&req)?,
        ),
        Err(e) => json_response(
            &ApiResponse {
                success: false,
                error: Some(format!("Failed to send: {}", e)),
            },
            500,
            cors_headers(&req)?,
        ),
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
