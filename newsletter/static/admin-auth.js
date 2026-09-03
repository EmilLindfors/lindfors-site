// OIDC authorization code + PKCE against the issuer, by hand.
//
// By hand rather than with a library, because every library that would do this lives on
// a CDN, and this page has no business fetching code from one. A dashboard that reads
// the subscriber list should depend on this machine and the IdP, and on nothing else
// that could change under it. It is about a hundred lines.
//
// Nothing provider-specific is written here. The authorization, token and (optional)
// end-session endpoints arrive in the config the service hands out, which it read from
// the issuer's own discovery document; this file has worked against Keycloak and now
// runs against Kanidm without a line of it knowing which.
//
// An ES module, and nothing here runs on import -- no top-level access to `location`,
// `document` or `sessionStorage` -- so node can load it. tests/admin-auth.test.mjs
// exercises the pure half: the encoding, the S256 challenge, the authorization query,
// the callback parse and the expiry arithmetic.
//
// **The id_token is never trusted here.** This module obtains tokens; it does not
// validate them, and the page draws nothing from their claims. Authorization happens in
// the service, which hands the access token back to the issuer's userinfo endpoint. That
// is also why there is no `nonce` in the authorization request: a nonce exists to bind
// an id_token to this browser session, and an id_token nobody reads proves nothing
// either way.

/**
 * Where the issuer sends the browser back, registered as a redirect URL on the client.
 *
 * The root of this service's own origin, whatever that is. Deriving it from
 * `location.origin` rather than naming a host means the same binary works behind
 * whatever name the reverse proxy gives it, and on localhost while you are setting it
 * up -- both of which have to be registered on the client either way. Kanidm matches
 * redirect URLs exactly, trailing slash included.
 */
export const REDIRECT_PATH = "/";

/** sessionStorage keys. Session, not local: the tab closes and the tokens are gone. */
const PENDING_KEY = "admin.oidc.pending";
const SESSION_KEY = "admin.oidc.session";

/**
 * Treat a token as expired this many milliseconds early, so a request issued just
 * under the wire does not arrive just over it.
 */
const EXPIRY_SKEW_MS = 30000;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/**
 * base64url of a byte array: RFC 4648 section 5, unpadded. `btoa` gives standard
 * base64, and the three characters that differ are exactly the ones that would need
 * escaping in a query string.
 */
export function base64url(bytes) {
    let binary = "";
    for (const byte of bytes) binary += String.fromCharCode(byte);
    return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/**
 * A fresh high-entropy string for a `code_verifier` or a `state`.
 *
 * 32 bytes, which base64url widens to 43 characters -- the minimum RFC 7636 allows for
 * a verifier is 43, so this is the floor rather than a round number that clears it by
 * accident.
 */
export function randomToken(length = 32) {
    return base64url(crypto.getRandomValues(new Uint8Array(length)));
}

/**
 * The `code_challenge` for a verifier: base64url(SHA-256(verifier)), method S256.
 *
 * The `plain` method is not implemented and must not be enabled on the client. It
 * makes the challenge equal to the verifier, so anyone who can read the redirect can
 * complete the exchange -- which is the whole attack PKCE exists to stop. Kanidm only
 * offers S256 and enforces it on public clients, which is the right default.
 */
export async function challengeFromVerifier(verifier) {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier));
    return base64url(new Uint8Array(digest));
}

/**
 * Build the authorization URL. Pure, so a test can read the query back.
 *
 * Parameters are appended rather than assigned, so an endpoint that already carries a
 * query string keeps it.
 */
export function authorizeUrl(config, { state, challenge, redirectUri }) {
    const url = new URL(config.authorizationEndpoint);
    const params = {
        response_type: "code",
        client_id: config.clientId,
        redirect_uri: redirectUri,
        // `openid` alone. The page shows subscriber counts, not a profile: there is no
        // claim it renders, so there is no further scope worth asking for.
        scope: "openid",
        state,
        code_challenge: challenge,
        code_challenge_method: "S256",
    };
    for (const [key, value] of Object.entries(params)) url.searchParams.set(key, value);
    return url.toString();
}

/**
 * Read what the issuer put on the redirect: a code, an error, or neither.
 *
 * An error comes back as a value rather than a throw because it is a normal outcome --
 * the user pressed cancel -- and the page renders it as a sign-in prompt, not a crash.
 */
export function parseCallback(search) {
    const params = new URLSearchParams(search);
    if (params.has("error")) {
        return {
            error: params.get("error"),
            description: params.get("error_description") || "",
        };
    }
    if (params.has("code")) {
        return { code: params.get("code"), state: params.get("state") || "" };
    }
    return null;
}

/**
 * Turn a token response into what gets stored. `expires_in` is seconds from now, and
 * the absolute instant is what a later page load can actually compare against.
 */
export function sessionFromTokenResponse(tokens, now) {
    return {
        accessToken: tokens.access_token,
        refreshToken: tokens.refresh_token || null,
        idToken: tokens.id_token || null,
        expiresAt: now + (tokens.expires_in || 0) * 1000,
    };
}

export function isExpired(session, now) {
    return !session || !session.accessToken || now + EXPIRY_SKEW_MS >= session.expiresAt;
}

// ---------------------------------------------------------------------------
// Browser flow
// ---------------------------------------------------------------------------

function readJson(key) {
    try {
        const raw = sessionStorage.getItem(key);
        return raw ? JSON.parse(raw) : null;
    } catch (e) {
        // A tab with storage disabled, or a half-written value. Either way the answer
        // is "no session", which sends the user through the login flow again.
        return null;
    }
}

function writeJson(key, value) {
    try {
        sessionStorage.setItem(key, JSON.stringify(value));
    } catch (e) {
        // Storage refused. The flow still works within this page load; it just will not
        // survive a reload. Better to fail quietly than to block sign-in over it.
    }
}

function redirectUri() {
    return new URL(REDIRECT_PATH, location.origin).href;
}

/**
 * The issuer, the client and the endpoints, from the service rather than hardcoded
 * here. The service read them from the issuer's discovery document.
 */
export async function loadConfig() {
    const response = await fetch("/api/config", {
        headers: { Accept: "application/json" },
    });
    if (!response.ok) throw new Error("Could not read admin config (" + response.status + ")");
    return response.json();
}

/** Leave for the issuer. Does not return: the browser navigates away. */
export async function beginLogin(config) {
    const verifier = randomToken();
    const state = randomToken(16);
    // Written before the redirect, read after it. A verifier that never comes back is
    // simply overwritten by the next attempt.
    writeJson(PENDING_KEY, { verifier, state });
    location.assign(
        authorizeUrl(config, {
            state,
            challenge: await challengeFromVerifier(verifier),
            redirectUri: redirectUri(),
        }),
    );
}

async function exchange(config, body) {
    const response = await fetch(config.tokenEndpoint, {
        method: "POST",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams(body).toString(),
    });
    if (!response.ok) throw new Error("Token endpoint answered " + response.status);
    return sessionFromTokenResponse(await response.json(), Date.now());
}

/**
 * Finish the flow on the redirect back. Returns the session, or null if this page load
 * is not a callback.
 *
 * The pending verifier is cleared whatever happens, so a replayed callback URL cannot
 * be exchanged a second time. The query string is stripped from the address bar for the
 * same reason: a code in a URL survives in history and in anything the user pastes.
 */
export async function completeLogin(config) {
    const callback = parseCallback(location.search);
    if (!callback) return null;

    const pending = readJson(PENDING_KEY);
    sessionStorage.removeItem(PENDING_KEY);
    history.replaceState(null, "", REDIRECT_PATH);

    if (callback.error) {
        throw new Error(callback.description || callback.error);
    }
    // No pending verifier, or a state that does not match the one we sent: this
    // redirect was not started by this tab. Refuse rather than exchange it.
    if (!pending || pending.state !== callback.state) {
        throw new Error("Sign-in could not be verified. Try again.");
    }

    const session = await exchange(config, {
        grant_type: "authorization_code",
        client_id: config.clientId,
        code: callback.code,
        redirect_uri: redirectUri(),
        code_verifier: pending.verifier,
    });
    writeJson(SESSION_KEY, session);
    return session;
}

/**
 * A live access token, or null if the user has to sign in.
 *
 * Refreshes when the stored one is spent. A refresh that fails is treated as no session
 * at all: the issuer has ended it, and the only move left is a new login.
 */
export async function accessToken(config) {
    let session = readJson(SESSION_KEY);
    if (!isExpired(session, Date.now())) return session.accessToken;

    if (session && session.refreshToken) {
        try {
            session = await exchange(config, {
                grant_type: "refresh_token",
                client_id: config.clientId,
                refresh_token: session.refreshToken,
            });
            writeJson(SESSION_KEY, session);
            return session.accessToken;
        } catch (e) {
            // Fall through to signed-out.
        }
    }

    sessionStorage.removeItem(SESSION_KEY);
    return null;
}

/**
 * Sign out at the issuer as well as here, when the issuer offers a way to.
 *
 * Dropping the local tokens alone leaves the issuer's own browser session standing, so
 * the next sign-in completes without a password prompt and looks like the logout never
 * happened. Keycloak publishes an `end_session_endpoint` for exactly this. Kanidm 1.11
 * does not, so against it this is a local sign-out only: the tokens this page holds are
 * gone, and the session at idm.lindfors.no is ended from its own UI. The sign-in panel
 * says so.
 */
export function logout(config) {
    const session = readJson(SESSION_KEY);
    sessionStorage.removeItem(SESSION_KEY);
    sessionStorage.removeItem(PENDING_KEY);

    if (!config.endSessionEndpoint) {
        location.replace(new URL("/?signed_out=local", location.origin).href);
        return;
    }

    const url = new URL(config.endSessionEndpoint);
    const params = { post_logout_redirect_uri: new URL("/", location.origin).href };
    // Keycloak requires one of these two alongside a post-logout redirect. The id_token
    // is the better hint; client_id is the fallback when there is nothing stored.
    if (session && session.idToken) params.id_token_hint = session.idToken;
    else params.client_id = config.clientId;
    for (const [key, value] of Object.entries(params)) url.searchParams.set(key, value);

    location.assign(url.toString());
}
