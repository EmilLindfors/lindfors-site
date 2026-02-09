use serde::{Deserialize, Serialize};
use worker::*;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SubscribeRequest {
    email: String,
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

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[event(fetch, respond_with_errors)]
pub async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/api/subscribe", handle_subscribe)
        .post_async("/api/unsubscribe", handle_unsubscribe_post)
        .get_async("/api/unsubscribe", handle_unsubscribe_page)
        .get_async("/api/subscribers", handle_subscribers)
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
