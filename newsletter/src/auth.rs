//! Who is allowed in.
//!
//! **The token is never parsed here.** It is not decoded, its signature is not checked,
//! and none of its claims are read. It is handed back to the issuer's `userinfo` endpoint
//! and the issuer says whether it is live. That costs a round trip on every admin call
//! and makes the dashboard unavailable when the IdP is down. What it buys is the absence
//! of signature verification, JWKS fetching and key-rotation handling — code whose
//! failure mode is "authenticates the wrong person", written to save a round trip on a
//! page one person opens a few times a week. It is also the only check that works at all
//! against Kanidm, whose access tokens are encrypted (JWE) on purpose and cannot be
//! verified by anyone but Kanidm.
//!
//! The endpoint comes from config, never from the token, so a forged `iss` buys nothing:
//! whatever arrives is presented to *our* issuer, which rejects it. Concretely it is the
//! `userinfo_endpoint` from the issuer's own discovery document, re-rooted onto
//! `OIDC_INTERNAL_ISSUER` — the same server reached over loopback, in plaintext, which is
//! what lets this binary carry no TLS stack at all. See `oidc.rs`.
//!
//! The check is issuer-wide by construction — any live token the issuer will answer
//! `userinfo` for reaches this far, whichever client minted it. `sub` is what actually
//! gates, and it is compared against a single configured account.
//!
//! **There is no rate limiter here, and that is a deployment requirement, not an
//! oversight.** Every unauthenticated request costs one request to the IdP, so an open
//! port in front of this is an amplifier aimed at idm.lindfors.no. The reverse proxy is
//! where that belongs — see README.md. Doing it in-process would mean a second, weaker
//! limiter behind the real one.

use serde::Deserialize;

/// The userinfo response, deserialized as far as it decides anything.
///
/// `sub` only. It is the one claim an account cannot change about itself; `email` can
/// be edited, and can be unverified. In Kanidm it is the account's UUID.
#[derive(Deserialize)]
struct UserInfo {
    sub: String,
}

/// Why a request was refused.
pub enum Denied {
    /// The issuer rejected the token, there was no token, or it belongs to someone
    /// else. All of these are a 401 and none of them says which.
    Unauthorized,
    /// The issuer could not be asked. A 503: the difference between "you are not
    /// allowed in" and "the lock is broken".
    Unavailable(String),
}

/// The bearer token in an `Authorization` header, if there is one.
pub fn bearer(header: Option<&str>) -> Option<&str> {
    header?.strip_prefix("Bearer ").filter(|t| !t.is_empty())
}

/// Ask the issuer whose token this is.
///
/// `Ok(None)` is a rejection by the issuer — expired, revoked, or never valid, which
/// it reports identically and which the caller could not act on differently anyway.
async fn subject_of(
    client: &reqwest::Client,
    userinfo_url: &str,
    token: &str,
) -> Result<Option<String>, String> {
    let response = client
        .get(userinfo_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("userinfo unreachable: {e}"))?;

    match response.status().as_u16() {
        200 => response
            .json::<UserInfo>()
            .await
            .map(|info| Some(info.sub))
            .map_err(|e| format!("userinfo returned unexpected JSON: {e}")),
        401 | 403 => Ok(None),
        other => Err(format!("userinfo answered {other}")),
    }
}

/// Let the request through, or say why not.
pub async fn authorize(
    client: &reqwest::Client,
    userinfo_url: &str,
    admin_subject: &str,
    header: Option<&str>,
) -> Result<(), Denied> {
    let Some(token) = bearer(header) else {
        return Err(Denied::Unauthorized);
    };

    match subject_of(client, userinfo_url, token).await {
        Ok(Some(sub)) if sub == admin_subject => Ok(()),
        Ok(Some(_)) => {
            // A live token for the wrong account. Worth a line: it is either a second
            // user who found the address, or something more interesting. The subject
            // itself is not logged — it identifies a person and proves nothing.
            eprintln!("admin refused: authenticated subject is not ADMIN_SUBJECT");
            Err(Denied::Unauthorized)
        }
        Ok(None) => Err(Denied::Unauthorized),
        Err(e) => Err(Denied::Unavailable(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bearer_token_is_read_out_of_the_header() {
        assert_eq!(bearer(Some("Bearer abc")), Some("abc"));
    }

    /// Everything that is not a bearer token is no token, including the empty one —
    /// which would otherwise be sent to the issuer as a question with an obvious answer.
    #[test]
    fn anything_else_is_no_token() {
        for header in [None, Some(""), Some("abc"), Some("Basic abc"), Some("Bearer ")] {
            assert!(bearer(header).is_none(), "{header:?} must not be a token");
        }
    }

    /// Kanidm's userinfo carries more than `sub`, and a stricter struct would refuse a
    /// perfectly good answer.
    #[test]
    fn userinfo_is_read_for_sub_alone() {
        let info: UserInfo = serde_json::from_str(
            r#"{"sub":"9a2f...","preferred_username":"emil","scopes":["openid"]}"#,
        )
        .unwrap();
        assert_eq!(info.sub, "9a2f...");
    }
}
