//! Where the issuer's endpoints are, asked of the issuer itself.
//!
//! This service used to carry Keycloak's paths (`/protocol/openid-connect/...`) in four
//! places. Kanidm, which is what idm.lindfors.no runs, has different ones (`/ui/oauth2`,
//! `/oauth2/token`, `/oauth2/openid/<client>/userinfo`), and the provider after that
//! will have others again. So the paths now come from the one place every OIDC provider
//! publishes them: the discovery document at `<issuer>/.well-known/openid-configuration`.
//!
//! It is fetched once, from the **internal** issuer, on the first request that needs it,
//! and kept for the life of the process. Not at startup: the IdP is a container on this
//! same box with its own start order, and a dashboard that dies because it booted eleven
//! seconds before Kanidm did is worse than one whose first page load says "authentication
//! service unavailable" and whose second works.
//!
//! The document names public URLs — a provider builds them from its configured origin,
//! never from the request — which is exactly what the browser needs and exactly what this
//! process cannot use, having no TLS. So the one URL this process calls, `userinfo`, is
//! re-rooted onto the internal issuer's origin: the same path, on the loopback scheme and
//! host. That holds for any provider behind a proxy that mirrors paths, which is how both
//! Keycloak and Kanidm are fronted here.
//!
//! The document's own `issuer` is checked against the configured public one. The two
//! variables are one server reached two ways, and a mismatch means somebody pointed the
//! internal URL at a different client or realm — which would otherwise show up as every
//! token being rejected, nowhere near the environment file that caused it.

use serde::Deserialize;
use tokio::sync::OnceCell;

/// The discovery document, as far as this service reads it.
#[derive(Deserialize, Clone, Debug)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    /// RP-initiated logout. Keycloak publishes one; Kanidm 1.11 does not, and the page
    /// then signs out locally and says so.
    #[serde(default)]
    pub end_session_endpoint: Option<String>,
}

pub struct Endpoints {
    /// What the browser is told. Public URLs, verbatim from the document.
    pub public: Discovery,
    /// `userinfo_endpoint` re-rooted onto the internal issuer's origin: what this
    /// process actually calls.
    pub internal_userinfo: String,
}

pub struct Provider {
    public_issuer: String,
    internal_issuer: String,
    cell: OnceCell<Endpoints>,
}

impl Provider {
    pub fn new(public_issuer: String, internal_issuer: String) -> Self {
        Self {
            public_issuer,
            internal_issuer,
            cell: OnceCell::new(),
        }
    }

    /// The endpoints, fetching them on the first call and remembering the answer.
    ///
    /// A failure is not remembered: the next request asks again, which is what makes
    /// the lazy fetch tolerate an IdP that is still starting.
    pub async fn endpoints(&self, client: &reqwest::Client) -> Result<&Endpoints, String> {
        self.cell
            .get_or_try_init(|| self.fetch(client))
            .await
    }

    async fn fetch(&self, client: &reqwest::Client) -> Result<Endpoints, String> {
        let url = discovery_url(&self.internal_issuer);
        let response = client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("discovery unreachable at {url}: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "discovery at {url} answered {} -- is OIDC_INTERNAL_ISSUER the right client?",
                response.status().as_u16()
            ));
        }
        let public: Discovery = response
            .json()
            .await
            .map_err(|e| format!("discovery at {url} is not an OIDC document: {e}"))?;

        if trim(&public.issuer) != trim(&self.public_issuer) {
            return Err(format!(
                "OIDC_INTERNAL_ISSUER describes issuer {}, but OIDC_ISSUER is {} -- \
                 they must be the same client reached two ways",
                public.issuer, self.public_issuer
            ));
        }
        // The page's CSP admits the issuer's origin and nothing else, so a token
        // endpoint anywhere else would be a sign-in that appears to work and then dies
        // at the exchange. Say so here, once, rather than in a browser console.
        if origin_of(&public.token_endpoint) != origin_of(&self.public_issuer) {
            eprintln!(
                "lindfors-admin: token endpoint {} is not on the issuer's origin; \
                 the page's CSP will block the token exchange",
                public.token_endpoint
            );
        }

        let internal_userinfo = re_root(&public.userinfo_endpoint, &self.internal_issuer);
        Ok(Endpoints {
            public,
            internal_userinfo,
        })
    }
}

fn trim(url: &str) -> &str {
    url.trim_end_matches('/')
}

/// `<issuer>/.well-known/openid-configuration`, tolerating a trailing slash on the
/// issuer: it is copied out of a console that shows it both ways, and a doubled slash
/// is a 404 that reads exactly like a wrong client name.
pub fn discovery_url(issuer: &str) -> String {
    format!("{}/.well-known/openid-configuration", trim(issuer))
}

/// The scheme and host of a URL, or the URL unchanged if it has no path to trim.
pub fn origin_of(url: &str) -> &str {
    let Some(rest) = url.find("://").map(|i| i + 3) else {
        return url;
    };
    match url[rest..].find('/') {
        Some(slash) => &url[..rest + slash],
        None => url,
    }
}

/// `url` with its scheme and host replaced by `base`'s. The path and query are kept.
///
/// This is how a public `https://idm.lindfors.no/oauth2/openid/x/userinfo` becomes the
/// `http://127.0.0.1:8447/oauth2/openid/x/userinfo` that a process with no TLS can call.
pub fn re_root(url: &str, base: &str) -> String {
    let origin = origin_of(url);
    format!("{}{}", origin_of(base), &url[origin.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_issuer_reduces_to_its_origin() {
        assert_eq!(
            origin_of("https://idm.lindfors.no/oauth2/openid/lindfors-admin"),
            "https://idm.lindfors.no"
        );
        assert_eq!(origin_of("https://idm.lindfors.no"), "https://idm.lindfors.no");
        assert_eq!(
            origin_of("http://127.0.0.1:8447/oauth2/openid/x"),
            "http://127.0.0.1:8447"
        );
    }

    /// The path the provider published is the path the proxy mirrors; only the origin
    /// changes. A query string, should a provider ever put one there, survives too.
    #[test]
    fn a_public_endpoint_is_re_rooted_onto_the_internal_origin() {
        assert_eq!(
            re_root(
                "https://idm.lindfors.no/oauth2/openid/lindfors-admin/userinfo",
                "http://127.0.0.1:8447/oauth2/openid/lindfors-admin"
            ),
            "http://127.0.0.1:8447/oauth2/openid/lindfors-admin/userinfo"
        );
        assert_eq!(
            re_root("https://a.example/u?x=1", "http://127.0.0.1:1/"),
            "http://127.0.0.1:1/u?x=1"
        );
    }

    /// The issuer is copied out of a console that shows it with and without a trailing
    /// slash. Both must reach the same document.
    #[test]
    fn the_discovery_url_tolerates_a_trailing_slash() {
        let expected = "https://idm.lindfors.no/oauth2/openid/x/.well-known/openid-configuration";
        assert_eq!(discovery_url("https://idm.lindfors.no/oauth2/openid/x"), expected);
        assert_eq!(discovery_url("https://idm.lindfors.no/oauth2/openid/x/"), expected);
    }

    /// Kanidm's document has no `end_session_endpoint`; the field must be optional or
    /// the whole document fails to parse and the dashboard is unusable against it.
    #[test]
    fn a_document_without_end_session_still_parses() {
        let doc: Discovery = serde_json::from_str(
            r#"{"issuer":"https://idm.lindfors.no/oauth2/openid/x",
                "authorization_endpoint":"https://idm.lindfors.no/ui/oauth2",
                "token_endpoint":"https://idm.lindfors.no/oauth2/token",
                "userinfo_endpoint":"https://idm.lindfors.no/oauth2/openid/x/userinfo",
                "jwks_uri":"https://idm.lindfors.no/oauth2/openid/x/public_key.jwk"}"#,
        )
        .unwrap();
        assert!(doc.end_session_endpoint.is_none());
        assert_eq!(doc.authorization_endpoint, "https://idm.lindfors.no/ui/oauth2");
    }
}
