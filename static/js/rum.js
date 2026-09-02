// Real user monitoring: sample first, then load the SDK off the critical path.
//
// This file is deliberately tiny and is the only thing base.html loads for RUM. The
// SDK bundle is 178 KB, 61 KB over the wire, against ~16 KB for the whole rest of
// this site's JavaScript, so it does not go in front of the page:
//
//   - Visitors who are not sampled in never fetch it at all. The sampling that used
//     to happen inside the SDK now happens here, before the request, which is the
//     only place it can save anything. The SDK is then initialised with
//     sessionSampleRate 100, because the decision has already been made -- sampling
//     in both places would multiply the two rates together.
//   - Sampled-in visitors get it after `load`, on an idle callback capped at 2s, so
//     it competes with nothing the reader is waiting for.
//
// Deferring costs less than it looks. The SDK registers its PerformanceObservers
// with buffered: true, so first paint, LCP, navigation and resource timing are read
// back out of the performance buffer and survive a late init. What a late init does
// lose is errors thrown before it, so there is a trap below that holds them and
// replays them through addError once the SDK is up.
//
// It also loses anyone who leaves before `load` fires, which biases the numbers
// against the fastest bounces. The alternative is blocking the page on telemetry,
// which is a worse trade on a site whose whole point is text.
//
// Configuration arrives as data- attributes on this file's own <script> tag, written
// from [extra.rum] in zola.toml. It cannot arrive as an inline <script>: the CSP
// ships script-src 'self' with no nonce and no 'unsafe-inline' (see static/_headers),
// so a JSON blob in the page would be dropped silently. Nothing here is secret --
// the client token is a write-only ingest key, readable in the page source either way.
//
// Externalised from base.html for CSP; see theme.js.
(function () {
    var el = document.currentScript || document.querySelector('script[data-oo-rum]');
    if (!el) return;

    var cfg = el.dataset;

    // An enabled flag with no application registered in OpenObserve yet is the normal
    // state of a half-finished setup, and the SDK's own failure mode for it is a
    // console warning on every page load. Say nothing and do nothing instead.
    if (!cfg.applicationId || !cfg.clientToken || !cfg.site || !cfg.bundle) return;

    // --- consent, before anything else ---------------------------------------
    // The SDK sets a first-party session cookie (_oo_s), which is analytics and not
    // something the site needs, so it needs a yes. Nothing below runs, and the SDK is
    // never fetched, until the reader has pressed Allow. The answer is kept in
    // localStorage, which is the one thing that counts as strictly necessary: it is
    // how "No thanks" is remembered as well as "Allow". The bar is markup in base.html
    // (CSP: no inline handlers) and the footer's Analytics link reopens it.
    var CONSENT_KEY = 'oo-rum-consent';
    function readConsent() { try { return localStorage.getItem(CONSENT_KEY); } catch (e) { return null; } }
    function writeConsent(v) { try { localStorage.setItem(CONSENT_KEY, v); } catch (e) {} }
    var bar = document.getElementById('consent-bar');
    function showBar() { if (bar) bar.hidden = false; }
    function hideBar() { if (bar) bar.hidden = true; }
    function clearSessionCookies() {
        ['_oo_s', '_oo_s_v2'].forEach(function (n) { document.cookie = n + '=; Max-Age=0; path=/'; });
    }

    if (bar) {
        bar.addEventListener('click', function (e) {
            var b = e.target && e.target.closest ? e.target.closest('[data-consent]') : null;
            if (!b) return;
            var v = b.getAttribute('data-consent') === 'allow' ? 'allow' : 'deny';
            writeConsent(v);
            hideBar();
            if (v === 'allow') { start(); } else { clearSessionCookies(); }
        });
        var openers = document.querySelectorAll('[data-consent-open]');
        for (var i = 0; i < openers.length; i++) {
            openers[i].addEventListener('click', function (e) { e.preventDefault(); showBar(); });
        }
    }

    var consent = readConsent();
    if (consent === 'allow') {
        start();
    } else if (consent === null) {
        showBar();
    }

    var started = false;
    function start() {
    if (started) return;
    started = true;

    // --- sample, before spending any bytes -----------------------------------
    var rate = parseInt(cfg.sessionSampleRate, 10);
    if (isNaN(rate)) rate = 100;

    // The roll is remembered for the tab, so a reader moving between posts is one
    // session throughout rather than a series of half-journeys. sessionStorage
    // throws outright in some privacy modes instead of returning null, so the roll
    // has to survive that; an unrecorded decision is better than no telemetry.
    var sampled;
    try {
        var seen = sessionStorage.getItem('oo-rum-sampled');
        if (seen === null) {
            sampled = Math.random() * 100 < rate;
            sessionStorage.setItem('oo-rum-sampled', sampled ? '1' : '0');
        } else {
            sampled = seen === '1';
        }
    } catch (e) {
        sampled = Math.random() * 100 < rate;
    }
    if (!sampled) return;

    // --- hold anything thrown before the SDK exists --------------------------
    // Capped, because a page erroring in a loop should not grow an array until the
    // idle callback runs.
    var early = [];
    function onError(e) { if (early.length < 10) early.push(e.error || e.message); }
    function onRejection(e) { if (early.length < 10) early.push(e.reason); }
    window.addEventListener('error', onError);
    window.addEventListener('unhandledrejection', onRejection);

    function init() {
        window.removeEventListener('error', onError);
        window.removeEventListener('unhandledrejection', onRejection);
        if (!window.OO_RUM) return;

        window.OO_RUM.init({
            applicationId: cfg.applicationId,
            clientToken: cfg.clientToken,
            site: cfg.site,
            organizationIdentifier: cfg.organization || 'default',
            service: cfg.service || 'lindfors-site',
            env: cfg.env || 'production',
            version: cfg.version || undefined,
            apiVersion: cfg.apiVersion || 'v1',
            insecureHTTP: false,

            // Off by default, and the single biggest lever on ingest volume: see
            // track_resources in zola.toml. The view events carry first paint, LCP
            // and view_resource_count regardless, so this is a debugging mode rather
            // than a steady-state metric.
            trackResources: cfg.trackResources === 'true',
            trackLongTasks: true,
            trackUserInteractions: true,

            // Already sampled above; this must stay at 100 or the two rates compound.
            sessionSampleRate: 100,

            // The newsletter field is the only place anyone types anything on this
            // site, and an email address is exactly the thing with no business in a
            // telemetry store. 'mask-user-input' is the floor once interactions are
            // tracked, because interaction events otherwise carry the text of what
            // was acted on.
            defaultPrivacyLevel: 'mask-user-input',

            // Session replay is off. It would lazily fetch a recorder chunk from a
            // bundle/chunks/ path that is not vendored, so a non-zero rate here is a
            // 404 rather than a feature; scripts/fetch-rum.sh has the note on
            // enabling it. startSessionReplayRecording() is deliberately never called.
            sessionReplaySampleRate: 0
        });

        early.forEach(function (err) { window.OO_RUM.addError(err); });
        early = [];

        // Clicks on links, recorded by hand. The SDK's own click tracking waits for
        // page activity to settle before it turns a click into an action, and a link
        // that navigates destroys the page first, so on a static site no internal
        // link ever becomes an action: the unload beacon carries the view update and
        // nothing else (verified 2026-09-02 from headless Chrome). A custom action is
        // recorded at once and rides that beacon. Delegated on document in the capture
        // phase so it sees the click before anything that might stop it; a click in
        // the first couple of seconds, before the SDK is up, is lost.
        document.addEventListener('click', function (e) {
            var a = e.target && e.target.closest ? e.target.closest('a[href]') : null;
            if (!a || e.button !== 0) return;
            var url;
            try { url = new URL(a.href, location.href); } catch (err) { return; }
            if (url.protocol !== 'http:' && url.protocol !== 'https:') return;
            window.OO_RUM.addAction('link', {
                href: url.href,
                internal: url.origin === location.origin,
                text: (a.textContent || '').trim().slice(0, 80)
            });
        }, true);
    }

    function load() {
        var s = document.createElement('script');
        s.src = cfg.bundle;
        s.async = true;
        s.onload = init;
        document.head.appendChild(s);
    }

    // The 2s cap matters: on a page that never goes idle, an uncapped
    // requestIdleCallback can wait indefinitely and the session is simply never
    // reported.
    function whenIdle() {
        if (window.requestIdleCallback) {
            requestIdleCallback(load, { timeout: 2000 });
        } else {
            setTimeout(load, 1000);
        }
    }

    if (document.readyState === 'complete') {
        whenIdle();
    } else {
        window.addEventListener('load', whenIdle);
    }
    } // start()

    // No allowedTracingUrls. The only same-origin fetch on this site is the
    // newsletter POST to /api/subscribe, and tracing it means injecting
    // x-openobserve-* headers, which turns a CORS-simple request into a preflighted
    // one against a Worker that answers no OPTIONS.

    // No setUser either: there is no login, so every id would be invented here.
})();
