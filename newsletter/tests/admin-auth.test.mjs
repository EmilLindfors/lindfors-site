// Tests for the admin PKCE flow's pure parts.
// Run with: node --test admin/tests/admin-auth.test.mjs
//
// Only what can be checked without a browser: encoding, the challenge derivation, the
// authorization query, the callback parse, and expiry arithmetic. The redirect itself,
// sessionStorage, and the token exchange need a browser or a live issuer, and the
// module is split so that the untestable half is the thin one.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
    authorizeUrl,
    base64url,
    challengeFromVerifier,
    isExpired,
    parseCallback,
    randomToken,
    sessionFromTokenResponse,
} from "../static/admin-auth.js";

// The shape /api/config answers with: the endpoints come from the issuer's discovery
// document, relayed by the service. These are Kanidm's.
const CONFIG = {
    issuer: "https://idm.lindfors.no/oauth2/openid/lindfors-admin",
    clientId: "lindfors-admin",
    authorizationEndpoint: "https://idm.lindfors.no/ui/oauth2",
    tokenEndpoint: "https://idm.lindfors.no/oauth2/token",
    endSessionEndpoint: null,
};
const REDIRECT = "https://admin.lindfors.no/";

// --- encoding ---------------------------------------------------------------

test("base64url uses the URL alphabet and drops padding", () => {
    // 0xFB 0xFF encodes to "+/8=" in standard base64: one byte from each substituted
    // character, plus the padding.
    assert.equal(base64url(new Uint8Array([0xfb, 0xff])), "-_8");
});

test("base64url round-trips through atob", () => {
    const bytes = new Uint8Array([0, 1, 127, 128, 255]);
    const encoded = base64url(bytes).replace(/-/g, "+").replace(/_/g, "/");
    const decoded = Uint8Array.from(atob(encoded), (c) => c.charCodeAt(0));
    assert.deepEqual([...decoded], [...bytes]);
});

// --- PKCE -------------------------------------------------------------------

test("a verifier is long enough for RFC 7636", () => {
    // 43 is the specified minimum, and 32 random bytes is exactly that once encoded.
    assert.equal(randomToken().length, 43);
    assert.match(randomToken(), /^[A-Za-z0-9\-_]+$/);
});

test("two verifiers differ", () => {
    assert.notEqual(randomToken(), randomToken());
});

test("the challenge is base64url(SHA-256(verifier))", async () => {
    // The worked example from RFC 7636 appendix B.
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert.equal(
        await challengeFromVerifier(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
    );
});

test("the challenge is not the verifier", async () => {
    // Guards against a silent fall back to code_challenge_method=plain, where the
    // challenge equals the verifier and PKCE stops protecting anything.
    const verifier = randomToken();
    assert.notEqual(await challengeFromVerifier(verifier), verifier);
});

// --- the authorization request ----------------------------------------------

test("the authorization URL asks for a code with S256", () => {
    const url = new URL(
        authorizeUrl(CONFIG, { state: "st", challenge: "ch", redirectUri: REDIRECT }),
    );
    assert.equal(url.origin + url.pathname, CONFIG.authorizationEndpoint);
    assert.equal(url.searchParams.get("response_type"), "code");
    assert.equal(url.searchParams.get("code_challenge_method"), "S256");
    assert.equal(url.searchParams.get("code_challenge"), "ch");
    assert.equal(url.searchParams.get("state"), "st");
    assert.equal(url.searchParams.get("client_id"), CONFIG.clientId);
    assert.equal(url.searchParams.get("redirect_uri"), REDIRECT);
});

test("the authorization URL never carries a secret", () => {
    const url = authorizeUrl(CONFIG, { state: "st", challenge: "ch", redirectUri: REDIRECT });
    // A public client has no secret to send, and sending one from a browser would
    // publish it. This is the check that notices if someone adds one.
    assert.ok(!url.includes("client_secret"));
});

test("the authorization URL is the endpoint the issuer published, nothing guessed", () => {
    // Keycloak's path was hardcoded here once. Against Kanidm that would be a 404 on
    // the first click, so the endpoint has to be taken from the config verbatim.
    const url = new URL(
        authorizeUrl(CONFIG, { state: "st", challenge: "ch", redirectUri: REDIRECT }),
    );
    assert.ok(!url.pathname.includes("openid-connect"));
    assert.equal(url.href.split("?")[0], CONFIG.authorizationEndpoint);
});

test("an authorization endpoint with its own query keeps it", () => {
    const url = new URL(
        authorizeUrl(
            { ...CONFIG, authorizationEndpoint: "https://idp.example/auth?tenant=t1" },
            { state: "st", challenge: "ch", redirectUri: REDIRECT },
        ),
    );
    assert.equal(url.searchParams.get("tenant"), "t1");
    assert.equal(url.searchParams.get("state"), "st");
});

// --- the callback -----------------------------------------------------------

test("a callback with a code is parsed", () => {
    assert.deepEqual(parseCallback("?code=abc&state=xyz"), { code: "abc", state: "xyz" });
});

test("a cancelled sign-in is an error, not a code", () => {
    const result = parseCallback("?error=access_denied&error_description=Nope");
    assert.equal(result.error, "access_denied");
    assert.equal(result.description, "Nope");
    assert.equal(result.code, undefined);
});

test("an error wins over a code in the same query", () => {
    // Belt and braces: a redirect carrying both must not be exchanged.
    assert.equal(parseCallback("?code=abc&error=access_denied").code, undefined);
});

test("a plain page load is not a callback", () => {
    assert.equal(parseCallback(""), null);
    assert.equal(parseCallback("?foo=bar"), null);
});

// --- session arithmetic -----------------------------------------------------

test("expires_in becomes an absolute instant", () => {
    const session = sessionFromTokenResponse(
        { access_token: "a", refresh_token: "r", id_token: "i", expires_in: 300 },
        1_000_000,
    );
    assert.equal(session.expiresAt, 1_000_000 + 300_000);
    assert.equal(session.accessToken, "a");
    assert.equal(session.refreshToken, "r");
});

test("a token response without a refresh token is still a session", () => {
    const session = sessionFromTokenResponse({ access_token: "a", expires_in: 60 }, 0);
    assert.equal(session.refreshToken, null);
    assert.equal(session.idToken, null);
});

test("a fresh token is not expired", () => {
    const session = sessionFromTokenResponse({ access_token: "a", expires_in: 300 }, 0);
    assert.equal(isExpired(session, 0), false);
});

test("a token is spent early, by the skew", () => {
    const session = sessionFromTokenResponse({ access_token: "a", expires_in: 300 }, 0);
    // 30s of skew: still live at 269s, spent at 271s, though the token itself has
    // 300s on it either way.
    assert.equal(isExpired(session, 269_000), false);
    assert.equal(isExpired(session, 271_000), true);
});

test("no session is expired", () => {
    assert.equal(isExpired(null, 0), true);
    assert.equal(isExpired({ expiresAt: Number.MAX_SAFE_INTEGER }, 0), true);
});
