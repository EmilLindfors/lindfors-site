// Tests for the Readers section's pure half: week bucketing of daily rows, the
// nanosecond timings, and the path display. Run with: node --test admin/tests/
//
// admin.js guards its DOM entry point on `typeof document`, so node can import it.

import { test } from "node:test";
import assert from "node:assert/strict";

import { displayPath, nsToMs, weeklyViews } from "../static/admin.js";

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
    assert.deepEqual(weeks[2], { week: "2026-08-31", views: 134, sessions: 25 });
    assert.deepEqual(weeks[1], { week: "2026-08-24", views: 4, sessions: 1 });
    assert.deepEqual(weeks[0], { week: "2026-08-17", views: 0, sessions: 0 });
});

test("empty weeks are present and zero, never absent", () => {
    // A chart that closes its gaps says the site ticked along through a month nobody
    // read it. Twelve weeks come back whatever the rows say.
    const weeks = weeklyViews([], NOW);
    assert.equal(weeks.length, 12);
    assert.ok(weeks.every((w) => w.views === 0 && w.sessions === 0));
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
