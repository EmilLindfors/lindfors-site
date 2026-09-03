//! The signed links: confirmation (double opt-in) and unsubscribe.
//!
//! Carried over from the Worker, where the construction was chosen because there was
//! no database to hold a pending-subscribers table. There is one now, and the links
//! stay as they were anyway: the pending state is only ever (address, deadline), and
//! a link that carries both under an HMAC is self-authenticating with nothing to expire
//! and nothing to clean up. A row for something that lives for two days would be a
//! table nobody needs.
//!
//! `exp` comes first in the signed string and is decimal digits terminated by ':', so
//! the split between the two fields is unambiguous -- an attacker cannot shift bytes
//! from one field into the other to make a different pair verify under the same
//! signature. Every purpose has its own prefix, so a link that proves someone wants
//! *in* is never a valid instruction to take them *out*.
//!
//! Rotating `CONFIRM_SECRET` invalidates every confirmation link not yet clicked (48
//! hours' worth, the whole revocation story) and every unsubscribe link in delivered
//! mail. The second is survivable only because the typed-address form at
//! `/api/unsubscribe` is a permanent fallback that needs no token.
//!
//! The event pseudonym lives here too, under its own secret: `EVENT_LOG_SECRET` is
//! meant to be stable for the life of the log, and must never be `CONFIRM_SECRET`.

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// How long a confirmation link stays valid. Long enough to survive a weekend and a
/// greylisting delay, short enough that a leaked link is not a standing grant.
pub const CONFIRM_TTL_SECS: u64 = 48 * 60 * 60;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compare without an early return on the first difference, so the time taken does
/// not reveal how much of a submitted signature was correct.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// HMAC-SHA256 over an already-canonicalised payload, hex encoded.
fn sign(secret: &str, payload: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(payload.as_bytes());
    to_hex(&mac.finalize().into_bytes())
}

pub fn confirm_signature(secret: &str, email: &str, exp: u64) -> String {
    sign(secret, &format!("confirm:v1:{exp}:{email}"))
}

pub fn confirm_signature_matches(secret: &str, email: &str, exp: u64, sig: &str) -> bool {
    constant_time_eq(confirm_signature(secret, email, exp).as_bytes(), sig.as_bytes())
}

pub fn unsubscribe_signature(secret: &str, email: &str) -> String {
    sign(secret, &format!("unsub:v1:{email}"))
}

pub fn unsubscribe_signature_matches(secret: &str, email: &str, sig: &str) -> bool {
    constant_time_eq(unsubscribe_signature(secret, email).as_bytes(), sig.as_bytes())
}

/// The stable pseudonym for an address in the event log: 16 hex characters of an HMAC
/// under the log's own secret. Preimage resistance comes from the secret, not the
/// length -- an address space small enough to enumerate would be trivially reversible
/// at any length without it. Identical to the Worker's, so the old log imports as is.
pub fn event_subject(secret: &str, email: &str) -> String {
    sign(secret, &format!("event:v1:{email}"))
        .chars()
        .take(16)
        .collect()
}

/// The confirmation link. The address goes through form encoding because `+` is a
/// legal local-part character and a raw `+` in a query string decodes back as a space,
/// which would silently confirm a different address than the one that was signed.
pub fn confirm_link(public_url: &str, email: &str, exp: u64, sig: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("email", email)
        .append_pair("exp", &exp.to_string())
        .append_pair("sig", sig)
        .finish();
    format!("{}/api/confirm?{query}", public_url.trim_end_matches('/'))
}

/// The per-recipient unsubscribe URL that goes in both the `List-Unsubscribe` header
/// and the footer link.
pub fn unsubscribe_link(public_url: &str, email: &str, sig: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .append_pair("email", email)
        .append_pair("sig", sig)
        .finish();
    format!("{}/api/unsubscribe?{query}", public_url.trim_end_matches('/'))
}

/// The (address, deadline, signature) triple a confirmation link carries.
#[derive(Debug, PartialEq)]
pub struct ConfirmToken {
    pub email: String,
    pub exp: u64,
    pub sig: String,
}

/// Pull a confirmation token out of decoded key/value pairs -- query string on GET,
/// form body on POST.
pub fn parse_confirm_token<I: Iterator<Item = (String, String)>>(pairs: I) -> Option<ConfirmToken> {
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

/// Pull a signed unsubscribe token out of decoded key/value pairs.
pub fn parse_unsubscribe_token<I: Iterator<Item = (String, String)>>(
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

#[derive(Debug, PartialEq)]
pub enum TokenState {
    Valid,
    Expired,
    Invalid,
}

/// Signature first: an expired-but-authentic link earns the honest "this expired, sign
/// up again"; a forged one must not be told which of its two fields was wrong.
pub fn check_confirm_token(secret: &str, token: &ConfirmToken, now: u64) -> TokenState {
    if !confirm_signature_matches(secret, &token.email, token.exp, &token.sig) {
        return TokenState::Invalid;
    }
    if now > token.exp {
        return TokenState::Expired;
    }
    TokenState::Valid
}

/// Compare a presented admin key against the configured one, in constant time.
pub fn key_matches(presented: &str, secret: &str) -> bool {
    constant_time_eq(presented.as_bytes(), secret.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret";

    #[test]
    fn a_confirmation_link_round_trips() {
        let exp = 1_800_000_000;
        let sig = confirm_signature(SECRET, "a+b@example.com", exp);
        let link = confirm_link("https://newsletter.lindfors.no", "a+b@example.com", exp, &sig);
        let query = link.split_once('?').unwrap().1;
        let token = parse_confirm_token(form_urlencoded::parse(query.as_bytes()).into_owned()).unwrap();
        assert_eq!(token.email, "a+b@example.com", "the plus survives the round trip");
        assert_eq!(check_confirm_token(SECRET, &token, exp - 1), TokenState::Valid);
        assert_eq!(check_confirm_token(SECRET, &token, exp + 1), TokenState::Expired);
    }

    #[test]
    fn a_tampered_link_is_invalid_not_expired() {
        let exp = 1_800_000_000;
        let sig = confirm_signature(SECRET, "a@example.com", exp);
        let forged = ConfirmToken { email: "b@example.com".into(), exp, sig: sig.clone() };
        assert_eq!(check_confirm_token(SECRET, &forged, 0), TokenState::Invalid);
        let later = ConfirmToken { email: "a@example.com".into(), exp: exp + 1, sig };
        assert_eq!(check_confirm_token(SECRET, &later, 0), TokenState::Invalid);
    }

    #[test]
    fn purposes_do_not_cross() {
        // A confirmation signature must never verify as an unsubscribe, and vice versa.
        let c = confirm_signature(SECRET, "a@example.com", 1);
        assert!(!unsubscribe_signature_matches(SECRET, "a@example.com", &c));
        let u = unsubscribe_signature(SECRET, "a@example.com");
        assert!(!confirm_signature_matches(SECRET, "a@example.com", 1, &u));
    }

    #[test]
    fn the_event_pseudonym_matches_the_worker_s() {
        // 16 hex characters, stable for an address, different across addresses.
        let s = event_subject("log-secret", "a@example.com");
        assert_eq!(s.len(), 16);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(s, event_subject("log-secret", "a@example.com"));
        assert_ne!(s, event_subject("log-secret", "b@example.com"));
        assert_ne!(s, event_subject("other", "a@example.com"));
    }

    #[test]
    fn unsubscribe_tokens_parse_from_either_source() {
        let sig = unsubscribe_signature(SECRET, "a@example.com");
        let link = unsubscribe_link("https://newsletter.lindfors.no/", "a@example.com", &sig);
        assert!(link.starts_with("https://newsletter.lindfors.no/api/unsubscribe?"));
        let query = link.split_once('?').unwrap().1;
        let (email, s) = parse_unsubscribe_token(form_urlencoded::parse(query.as_bytes()).into_owned()).unwrap();
        assert!(unsubscribe_signature_matches(SECRET, &email, &s));
        assert!(parse_unsubscribe_token(std::iter::empty()).is_none());
    }

    #[test]
    fn constant_time_eq_is_exact() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
