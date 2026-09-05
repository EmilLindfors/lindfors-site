// Tests for the Readers section's pure half: week bucketing of daily rows, the
// nanosecond timings, and the path display. Run with: node --test admin/tests/
//
// admin.js guards its DOM entry point on `typeof document`, so node can import it.

import { test } from "node:test";
import assert from "node:assert/strict";

import { consentSummary, displayPath, mergePageLoads, nsToMs, percent, weeklyViews } from "../static/admin.js";

// A Wednesday. The Monday of that week is 2026-08-31.
const NOW = Date.parse("2026-09-02T12:00:00Z");

test("daily rows land in Monday-start weeks, most recent last", () => {
    const weeks = weeklyViews(
        [
            { day: "2026-09-01T00:00:00", views: 39, sessions: 7 },
            { day: "2026-09-02T00:00:00", views: 95, sessions: 18 },
            { day: "2026-08-30T00:00:00", views: 4, sessions: 1 }, // the Sunday before
        ],
        NOW,
        3,
    );
    assert.deepEqual(
        weeks.map((w) => w.week),
        ["2026-08-17", "2026-08-24", "2026-08-31"],
    );
    assert.deepEqual(weeks[2], { week: "2026-08-31", loads: 0, views: 134, sessions: 25 });
    assert.deepEqual(weeks[1], { week: "2026-08-24", loads: 0, views: 4, sessions: 1 });
    assert.deepEqual(weeks[0], { week: "2026-08-17", loads: 0, views: 0, sessions: 0 });
});

test("empty weeks are present and zero, never absent", () => {
    // A chart that closes its gaps says the site ticked along through a month nobody
    // read it. Twelve weeks come back whatever the rows say.
    const weeks = weeklyViews([], NOW);
    assert.equal(weeks.length, 12);
    assert.ok(weeks.every((w) => w.loads === 0 && w.views === 0 && w.sessions === 0));
});

test("rows outside the window are ignored, and counts are numbers", () => {
    const weeks = weeklyViews(
        [
            { day: "2020-01-01T00:00:00", views: 1000, sessions: 1000 },
            { day: "2026-09-01T00:00:00", views: "3", sessions: "2" }, // the store's strings
        ],
        NOW,
        2,
    );
    assert.equal(weeks[1].views, 3);
    assert.equal(weeks[1].sessions, 2);
    assert.equal(weeks[0].views, 0);
});

test("paint timings arrive in nanoseconds and are shown in milliseconds", () => {
    assert.equal(nsToMs(719000000), 719);
    assert.equal(nsToMs("449991666"), 450);
    assert.equal(nsToMs(undefined), null);
});

test("the site's own URLs are shown as paths, others whole", () => {
    assert.equal(displayPath("https://lindfors.no/blog/x/"), "/blog/x/");
    assert.equal(displayPath("https://lindfors.no"), "/");
    assert.equal(displayPath("https://www.google.com/"), "https://www.google.com/");
    assert.equal(displayPath(""), "");
});

// --- issues and subscribers ---------------------------------------------------

import { issueRows, subscriberRows } from "../static/admin.js";

const SENDS = [
    { slug: "first", claimed_at: "2026-08-28T10:00:00Z", finished_at: "2026-08-28T10:01:00Z", status: "sent", sent: 2, failed: [] },
    { slug: "second", claimed_at: "2026-09-01T10:00:00Z", finished_at: null, status: "partial", sent: 1, failed: ["b@example.com"] },
];
const SUBS = [
    { subject: "aaaa", email: "a@example.com", subscribed_at: "2026-08-01T00:00:00Z", source: "migrated" },
    { subject: "bbbb", email: "b@example.com", subscribed_at: "2026-08-02T00:00:00Z", source: "migrated" },
    { subject: "cccc", email: "c@example.com", subscribed_at: "2026-09-02T00:00:00Z", source: "confirmed" },
];
const DELIVERIES = [
    { slug: "first", subject: "aaaa", email: "a@example.com", at: "2026-08-28T10:00:30Z", status: "assumed" },
    { slug: "first", subject: "bbbb", email: "b@example.com", at: "2026-08-28T10:00:31Z", status: "assumed" },
    { slug: "second", subject: "aaaa", email: "a@example.com", at: "2026-09-01T10:00:30Z", status: "sent" },
    { slug: "second", subject: "bbbb", email: "b@example.com", at: "2026-09-01T10:00:31Z", status: "failed" },
];

test("an issue's missing count is current subscribers without a sent or assumed delivery", () => {
    const rows = issueRows(SENDS, DELIVERIES, SUBS);
    assert.equal(rows[0].slug, "first");
    assert.equal(rows[0].missing, 1, "c joined after the first issue");
    assert.equal(rows[0].when, "2026-08-28");
    assert.equal(rows[1].missing, 2, "b failed and c never got it");
    assert.equal(rows[1].failed, 1);
    assert.equal(rows[1].when, "2026-09-01", "claimed_at when nothing finished");
});

test("a subscriber's issues list is in send order and marks the assumed ones", () => {
    const rows = subscriberRows(SUBS, DELIVERIES, SENDS);
    assert.deepEqual(rows[0].had, ["first (assumed)", "second"]);
    assert.equal(rows[0].issues, 2);
    assert.equal(rows[0].of, 2);
    assert.deepEqual(rows[1].had, ["first (assumed)"], "a failed delivery is not a delivery");
    assert.deepEqual(rows[2].had, []);
    assert.equal(rows[2].since, "2026-09-02");
});

test("the ping's daily loads land in the same weeks as the views", () => {
    const weeks = weeklyViews(
        [{ day: "2026-09-01T00:00:00", views: 3, sessions: 2 }],
        NOW,
        2,
        [
            { day: "2026-09-01T00:00:00", loads: "40", allowed: 3, denied: 1 },
            { day: "2026-08-30T00:00:00", loads: 5, allowed: 0, denied: 0 },
        ],
    );
    assert.deepEqual(weeks[1], { week: "2026-08-31", loads: 40, views: 3, sessions: 2 });
    assert.deepEqual(weeks[0], { week: "2026-08-24", loads: 5, views: 0, sessions: 0 });
});

test("consent summary: coverage is measured loads over all loads, yes is allow over presses", () => {
    const since = Date.parse("2026-08-25T00:00:00Z");
    const summary = consentSummary(
        [
            { day: "2026-09-01T00:00:00", views: 30, sessions: 9, measured: "12" },
            { day: "2026-08-01T00:00:00", views: 99, sessions: 99, measured: 99 }, // outside the window
        ],
        [
            { day: "2026-09-01T00:00:00", loads: 100, allowed: 10, denied: 5 },
            { day: "2026-08-01T00:00:00", loads: 1000, allowed: 0, denied: 0 },
        ],
        [
            { choice: "allow", presses: 8 },
            { choice: "deny", presses: "24" },
        ],
        since,
    );
    assert.deepEqual(summary, {
        loads: 100,
        measured: 12,
        coverage: 0.12,
        shown: 85,
        allow: 8,
        deny: 24,
        yes: 0.25,
    });
});

test("consent summary: ratios are null, not zero, before anything has been counted", () => {
    const summary = consentSummary([], [], [], 0);
    assert.equal(summary.coverage, null);
    assert.equal(summary.yes, null);
    assert.equal(summary.shown, 0);
    assert.equal(percent(summary.coverage), "—");
    assert.equal(percent(0.126), "13%");
});

test("pages merge with the ping's loads on the URL, most loaded first", () => {
    const rows = mergePageLoads(
        [
            { url: "https://lindfors.no/", views: 52, sessions: 25 },
            { url: "https://lindfors.no/blog/a/", views: 18, sessions: 8 },
        ],
        [
            { url: "https://lindfors.no/blog/a/", loads: 300 },
            { url: "https://lindfors.no/blog/b/", loads: 7 }, // no consented view at all
            { url: "https://lindfors.no/", loads: "120" },
        ],
    );
    assert.deepEqual(rows, [
        { url: "https://lindfors.no/blog/a/", loads: 300, views: 18, sessions: 8 },
        { url: "https://lindfors.no/", loads: 120, views: 52, sessions: 25 },
        { url: "https://lindfors.no/blog/b/", loads: 7, views: 0, sessions: 0 },
    ]);
});
