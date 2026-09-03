//! Sending mail through Stalwart, on loopback, over JMAP.
//!
//! Email/set creates a draft and EmailSubmission/set sends it, in one request, as the
//! Worker did -- but to `http://127.0.0.1:8080/jmap/` rather than across the internet
//! to the same server. The account is postmaster's; its credential is an app password
//! in the environment file and nowhere else.
//!
//! `unsubscribe_url` is `Some` for list mail and `None` for transactional mail. The
//! confirmation message is the second kind: its recipient is by definition not on the
//! list yet, so advertising `List-Unsubscribe` on it would offer to remove an address
//! that was never added, and a client honouring One-Click would fire a POST at an
//! address this service cannot act on.

use base64::Engine;
use serde_json::{json, Value};

pub struct Sender {
    /// `http://127.0.0.1:8080`; the code appends `/jmap/`.
    pub base_url: String,
    /// `Basic` credential, assembled from user and password here and never stored
    /// pre-encoded: a bare password in a secret reads as a plain 401 and the two cannot
    /// be told apart from outside.
    pub credential: String,
    pub account_id: String,
    pub identity_id: String,
    pub from: String,
    pub from_name: String,
}

impl Sender {
    pub fn new(
        base_url: String,
        user: &str,
        password: &str,
        account_id: String,
        identity_id: String,
        from: String,
        from_name: String,
    ) -> Self {
        Self {
            base_url,
            credential: base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}")),
            account_id,
            identity_id,
            from,
            from_name,
        }
    }
}

/// Iterate a JMAP response's methodResponses as (method name, result) pairs.
pub fn method_responses(resp: &Value) -> Vec<(&str, &Value)> {
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
pub fn method_error(method: &str, result: &Value, key: &str) -> Option<String> {
    let map = result.get(key)?.as_object()?;
    if map.is_empty() {
        return None;
    }
    let details: Vec<String> = map
        .iter()
        .map(|(k, v)| {
            let err_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("unknown");
            format!("{k}: {err_type}")
        })
        .collect();
    Some(format!("{method} {key}: {}", details.join(", ")))
}

/// POST a JMAP request. Fails on transport errors, a non-200 status, and request-level
/// JMAP errors; method-level failures are left to the caller, which knows what it asked.
async fn jmap_call(client: &reqwest::Client, sender: &Sender, body: &Value) -> Result<Value, String> {
    let url = format!("{}/jmap/", sender.base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header("Authorization", format!("Basic {}", sender.credential))
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("JMAP unreachable: {e}"))?;

    let status = response.status().as_u16();
    if status == 401 {
        return Err("JMAP answered 401: the sender credential is refused".into());
    }
    if status != 200 {
        return Err(format!("JMAP answered {status}"));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("JMAP response parse: {e}"))?;
    for (method, result) in method_responses(&body) {
        if method == "error" {
            let err_type = result.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(format!("JMAP error: {err_type}"));
        }
    }
    Ok(body)
}

/// The JMAP request that creates and submits one message. Pure, so a test can read
/// the headers back.
pub fn send_request(
    sender: &Sender,
    to: &str,
    subject: &str,
    html_body: &str,
    unsubscribe_url: Option<&str>,
) -> Value {
    let mut draft = json!({
        "mailboxIds": { "d": true },
        "from": [{ "name": sender.from_name, "email": sender.from }],
        "to": [{ "email": to }],
        "subject": subject,
        "textBody": [{ "partId": "text", "type": "text/plain" }],
        "htmlBody": [{ "partId": "html", "type": "text/html" }],
        "bodyValues": {
            "text": { "value": html_to_plain_text(html_body), "isEncodingProblem": false, "isTruncated": false },
            "html": { "value": html_body, "isEncodingProblem": false, "isTruncated": false }
        }
    });

    if let (Some(url), Some(obj)) = (unsubscribe_url, draft.as_object_mut()) {
        obj.insert("header:List-Unsubscribe:asRaw".into(), Value::String(format!(" <{url}>")));
        obj.insert(
            "header:List-Unsubscribe-Post:asRaw".into(),
            Value::String(" List-Unsubscribe=One-Click".into()),
        );
    }

    json!({
        "using": [
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
            "urn:ietf:params:jmap:submission"
        ],
        "methodCalls": [
            ["Email/set", { "accountId": sender.account_id, "create": { "draft": draft } }, "0"],
            ["EmailSubmission/set", {
                "accountId": sender.account_id,
                "create": { "send": {
                    "identityId": sender.identity_id,
                    "emailId": "#draft",
                    "envelope": {
                        "mailFrom": { "email": sender.from },
                        "rcptTo": [{ "email": to }]
                    }
                } },
                "onSuccessDestroyEmail": ["#send"]
            }, "1"]
        ]
    })
}

/// Send one message. `Ok(())` means Stalwart accepted it for delivery.
pub async fn send(
    client: &reqwest::Client,
    sender: &Sender,
    to: &str,
    subject: &str,
    html_body: &str,
    unsubscribe_url: Option<&str>,
) -> Result<(), String> {
    let body = send_request(sender, to, subject, html_body, unsubscribe_url);
    let response = jmap_call(client, sender, &body).await?;
    for (method, result) in method_responses(&response) {
        if let Some(err) = method_error(method, result, "notCreated") {
            return Err(err);
        }
    }
    Ok(())
}

/// Strip tags to produce the plain-text alternative of an HTML message.
pub fn html_to_plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_newline = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
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
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rarr;", "->")
        .replace("&middot;", "-")
        .replace("&mdash;", "--")
        .replace("&rsquo;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender() -> Sender {
        Sender::new(
            "http://127.0.0.1:8080".into(),
            "postmaster@lindfors.no",
            "pw",
            "c2".into(),
            "b".into(),
            "postmaster@lindfors.no".into(),
            "Emil Lindfors".into(),
        )
    }

    #[test]
    fn the_credential_is_user_colon_password() {
        assert_eq!(sender().credential, "cG9zdG1hc3RlckBsaW5kZm9ycy5ubzpwdw==");
    }

    /// List mail carries both RFC 8058 headers; transactional mail carries neither.
    #[test]
    fn list_unsubscribe_headers_follow_the_url() {
        let with = send_request(&sender(), "a@example.com", "s", "<p>x</p>", Some("https://u/1"));
        let draft = &with["methodCalls"][0][1]["create"]["draft"];
        assert_eq!(draft["header:List-Unsubscribe:asRaw"], " <https://u/1>");
        assert_eq!(draft["header:List-Unsubscribe-Post:asRaw"], " List-Unsubscribe=One-Click");

        let without = send_request(&sender(), "a@example.com", "s", "<p>x</p>", None);
        let draft = &without["methodCalls"][0][1]["create"]["draft"];
        assert!(draft.get("header:List-Unsubscribe:asRaw").is_none());
    }

    #[test]
    fn the_submission_references_the_draft_and_destroys_it_after() {
        let req = send_request(&sender(), "a@example.com", "s", "<p>x</p>", None);
        let submission = &req["methodCalls"][1][1];
        assert_eq!(submission["create"]["send"]["emailId"], "#draft");
        assert_eq!(submission["create"]["send"]["envelope"]["rcptTo"][0]["email"], "a@example.com");
        assert_eq!(submission["onSuccessDestroyEmail"][0], "#send");
    }

    #[test]
    fn plain_text_drops_tags_and_decodes_entities() {
        assert_eq!(html_to_plain_text("<p>a &amp; b</p>\n\n<p>c</p>"), "a & b\nc");
    }

    #[test]
    fn method_errors_are_read_out_of_not_created() {
        let resp = json!({"methodResponses": [["Email/set", {"notCreated": {"draft": {"type": "invalidProperties"}}}, "0"]]});
        let (m, r) = method_responses(&resp)[0];
        assert_eq!(method_error(m, r, "notCreated").unwrap(), "Email/set notCreated: draft: invalidProperties");
        assert!(method_error(m, r, "notUpdated").is_none());
    }
}
