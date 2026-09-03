// The newsletter dashboard.
//
// The service returns records; everything counted, bucketed or drawn happens here. That
// split is on purpose: what is worth measuring about a mailing list changes more often
// than the list does, and this way changing it does not mean rebuilding a binary.
//
// The chart is hand-drawn SVG for the same reason admin-auth.js is hand-rolled PKCE: a
// charting library would come from a CDN, and a page that reads the subscriber list
// should not fetch code from one. A grouped bar chart is about eighty lines.
//
// Nothing here runs on import; tests/admin-dashboard.test.mjs loads the module to
// exercise the bucketing, which is where an off-by-one would quietly misreport a week.

import { accessToken, beginLogin, completeLogin, loadConfig, logout } from "./admin-auth.js";

/** Fixed order, and the colours are bound to the entity, never to its rank. */
const SERIES = [
    { key: "requested", label: "Requested" },
    { key: "confirmed", label: "Confirmed" },
    { key: "unsubscribed", label: "Unsubscribed" },
];

/**
 * Where a sent issue's post lives. Absolute, because this page is served from its own
 * origin and a root-relative link would point back at this service, which has no blog
 * on it.
 */
const SITE_URL = "https://lindfors.no";

/** How far back the rate tiles look. */
const WINDOW_DAYS = 90;

/** How many weeks the chart shows, at most. */
const MAX_WEEKS = 12;

const DAY_MS = 86400000;

// ---------------------------------------------------------------------------
// Aggregation (pure)
// ---------------------------------------------------------------------------

/**
 * The Monday of an instant's week, as `YYYY-MM-DD`.
 *
 * UTC throughout, because the timestamps are UTC and a local-time week boundary would
 * move events between weeks depending on where the page is opened.
 */
export function weekStartUTC(iso) {
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return null;
    // getUTCDay() is Sunday-first; the list's weeks start Monday.
    const offset = (date.getUTCDay() + 6) % 7;
    date.setUTCDate(date.getUTCDate() - offset);
    return date.toISOString().slice(0, 10);
}

/** `n` weeks after a `YYYY-MM-DD` week start. */
export function addWeeks(weekStart, n) {
    return new Date(Date.parse(weekStart + "T00:00:00Z") + n * 7 * DAY_MS)
        .toISOString()
        .slice(0, 10);
}

/**
 * Weekly counts, most recent week last.
 *
 * Weeks with nothing in them are present and zero rather than absent. A chart that
 * silently closes its gaps says the list ticked along steadily through a month when
 * nobody signed up at all.
 */
export function weeklyBuckets(events, now, maxWeeks = MAX_WEEKS) {
    const counts = new Map();
    for (const event of events) {
        const week = weekStartUTC(event.at);
        if (!week) continue;
        if (!counts.has(week)) counts.set(week, { requested: 0, confirmed: 0, unsubscribed: 0 });
        const bucket = counts.get(week);
        if (event.event in bucket) bucket[event.event] += 1;
    }
    if (counts.size === 0) return [];

    const last = weekStartUTC(new Date(now).toISOString());
    const earliest = [...counts.keys()].sort()[0];
    // Start at whichever is later: the window, or the week the log itself begins. There
    // is no point drawing twelve empty weeks before the first event ever recorded.
    const windowStart = addWeeks(last, -(maxWeeks - 1));
    let cursor = windowStart > earliest ? windowStart : earliest;

    const weeks = [];
    while (cursor <= last) {
        weeks.push({
            week: cursor,
            ...(counts.get(cursor) || { requested: 0, confirmed: 0, unsubscribed: 0 }),
        });
        cursor = addWeeks(cursor, 1);
    }
    return weeks;
}

/** Event counts since an instant. */
export function windowCounts(events, sinceIso) {
    const totals = { requested: 0, confirmed: 0, unsubscribed: 0 };
    for (const event of events) {
        if (event.at >= sinceIso && event.event in totals) totals[event.event] += 1;
    }
    return totals;
}

/**
 * Confirmed as a share of requested, or null when nobody asked.
 *
 * An approximation at the edges: a confirmation counted here may belong to a request
 * made just before the window opened. Links last 48 hours and the window is 90 days, so
 * the error is bounded by two days at one end of ninety. It is the number that says
 * whether the confirmation mail is arriving at all, which no exact alternative does.
 */
export function confirmationRate(counts) {
    if (!counts.requested) return null;
    return counts.confirmed / counts.requested;
}

/** Y-axis ticks: 0 to a rounded ceiling, in 1/2/5 x 10^n steps. */
export function niceTicks(max, target = 4) {
    if (!(max > 0)) return [0, 1];
    const rough = max / target;
    const magnitude = Math.pow(10, Math.floor(Math.log10(rough)));
    const step = [1, 2, 5, 10].find((m) => m * magnitude >= rough) * magnitude;
    const ticks = [];
    for (let value = 0; value < max + step; value += step) ticks.push(value);
    return ticks;
}

/** `2026-08-24` as `24 Aug`. */
export function shortDate(weekStart) {
    const date = new Date(weekStart + "T00:00:00Z");
    const month = date.toLocaleString("en", { month: "short", timeZone: "UTC" });
    return date.getUTCDate() + " " + month;
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

const SVG_NS = "http://www.w3.org/2000/svg";

function el(tag, attrs = {}, text) {
    const node = document.createElementNS(SVG_NS, tag);
    for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, value);
    if (text !== undefined) node.textContent = text;
    return node;
}

/**
 * A column with a rounded cap and a square foot.
 *
 * `rect` rounds all four corners, which lifts the bar off its own baseline and makes
 * every value look slightly smaller than it is. The data end is the only end that gets
 * a radius.
 */
function columnPath(x, y, width, height, radius = 4) {
    const r = Math.max(0, Math.min(radius, width / 2, height));
    return [
        "M", x, y + height,
        "L", x, y + r,
        "Q", x, y, x + r, y,
        "L", x + width - r, y,
        "Q", x + width, y, x + width, y + r,
        "L", x + width, y + height,
        "Z",
    ].join(" ");
}

/**
 * Grouped columns, one group per week.
 *
 * Drawn into a viewBox and scaled by CSS, so there is no resize handler and no redraw
 * on a theme change -- the fills are CSS custom properties, which follow `data-theme`
 * on their own.
 */
function renderChart(frame, tooltip, weeks, series = SERIES, label = "Weekly newsletter events. The same figures are in the table view below.") {
    frame.querySelectorAll("svg").forEach((node) => node.remove());
    if (weeks.length === 0) return;

    const W = 720, H = 230;
    const M = { top: 18, right: 8, bottom: 28, left: 32 };
    const plotW = W - M.left - M.right;
    const plotH = H - M.top - M.bottom;

    const max = Math.max(1, ...weeks.flatMap((w) => series.map((s) => w[s.key])));
    const ticks = niceTicks(max);
    const ceiling = ticks[ticks.length - 1];
    const y = (value) => M.top + plotH - (value / ceiling) * plotH;

    const bandW = plotW / weeks.length;
    const BAND_PAD = 6;
    const GAP = 2; // the surface gap: white separates the bars, not a stroke
    const barW = Math.min(24, (bandW - 2 * BAND_PAD - GAP * (series.length - 1)) / series.length);

    const svg = el("svg", {
        viewBox: `0 0 ${W} ${H}`,
        role: "img",
        "aria-label": label,
    });

    // Gridlines and y ticks. Solid hairlines one step off the surface: a dashed grid
    // reads as a threshold when it is only a grid.
    for (const tick of ticks) {
        svg.appendChild(el("line", {
            class: "grid-line",
            x1: M.left, x2: W - M.right, y1: y(tick), y2: y(tick),
        }));
        svg.appendChild(el("text", {
            class: "axis-text", x: M.left - 6, y: y(tick) + 3, "text-anchor": "end",
        }, String(tick)));
    }

    weeks.forEach((week, index) => {
        const bandX = M.left + index * bandW;

        series.forEach((s, slot) => {
            const value = week[s.key];
            if (value <= 0) return; // an absent bar, not a zero-height sliver
            const x = bandX + BAND_PAD + slot * (barW + GAP);
            const height = M.top + plotH - y(value);
            svg.appendChild(el("path", {
                class: "bar--" + s.key,
                d: columnPath(x, y(value), barW, height),
            }));

            // Direct labels, sparingly: the most recent week only, and only where the
            // text fits inside the bar's own width. A number over every column is the
            // thing nobody reads.
            const isLast = index === weeks.length - 1;
            const fits = String(value).length * 6 <= barW;
            if (isLast && fits) {
                svg.appendChild(el("text", {
                    class: "axis-text", x: x + barW / 2, y: y(value) - 5, "text-anchor": "middle",
                }, String(value)));
            }
        });

        svg.appendChild(el("text", {
            class: "axis-text",
            x: bandX + bandW / 2,
            y: H - 10,
            "text-anchor": "middle",
        }, shortDate(week.week)));

        // One hit target per week, the full height of the plot. Bars this thin are an
        // unfair thing to ask anyone to land on, and a band also answers the question
        // actually being asked -- what happened that week, across all three series.
        const hit = el("rect", {
            class: "hit",
            x: bandX, y: M.top, width: bandW, height: plotH,
            tabindex: "0",
            role: "button",
            "aria-label": describeWeek(week, series),
        });
        const show = () => showTooltip(frame, tooltip, hit, week, series);
        const hide = () => { tooltip.hidden = true; };
        hit.addEventListener("mouseenter", show);
        hit.addEventListener("focus", show);
        hit.addEventListener("mouseleave", hide);
        hit.addEventListener("blur", hide);
        svg.appendChild(hit);
    });

    frame.insertBefore(svg, tooltip);
}

function describeWeek(week, series = SERIES) {
    const parts = series.map((s) => `${week[s.key]} ${s.label.toLowerCase()}`);
    return `Week of ${shortDate(week.week)}: ` + parts.join(", ");
}

function showTooltip(frame, tooltip, hit, week, allSeries = SERIES) {
    tooltip.replaceChildren();

    const heading = document.createElement("div");
    heading.textContent = "Week of " + shortDate(week.week);
    tooltip.appendChild(heading);

    for (const series of allSeries) {
        const row = document.createElement("div");
        row.className = "tooltip-row";
        const swatch = document.createElement("span");
        swatch.className = "legend-swatch swatch--" + series.key;
        const value = document.createElement("b");
        value.textContent = String(week[series.key]);
        row.append(swatch, value, document.createTextNode(series.label.toLowerCase()));
        tooltip.appendChild(row);
    }

    tooltip.hidden = false;

    const frameBox = frame.getBoundingClientRect();
    const hitBox = hit.getBoundingClientRect();
    const left = hitBox.left - frameBox.left + hitBox.width / 2 - tooltip.offsetWidth / 2;
    tooltip.style.left = Math.max(0, Math.min(left, frameBox.width - tooltip.offsetWidth)) + "px";
    tooltip.style.top = "0px";
}

function tile(label, value, note, hero = false) {
    const node = document.createElement("div");
    node.className = hero ? "tile tile--hero" : "tile";

    const labelNode = document.createElement("span");
    labelNode.className = "tile-label";
    labelNode.textContent = label;

    const valueNode = document.createElement("div");
    valueNode.className = "tile-value";
    valueNode.textContent = value;

    node.append(labelNode, valueNode);
    if (note) {
        const noteNode = document.createElement("span");
        noteNode.className = "tile-note";
        noteNode.textContent = note;
        node.appendChild(noteNode);
    }
    return node;
}

function renderTiles(container, data, counts) {
    container.replaceChildren();

    // The one hero figure, and the only number on the page that does not come from the
    // event log: the log begins when it was deployed, the list does not.
    container.appendChild(
        tile(
            "Subscribers",
            data.subscribers === undefined ? "—" : data.subscribers.toLocaleString(),
            data.subscribers === undefined ? "list unavailable" : "confirmed, on the list now",
            true,
        ),
    );

    const rate = confirmationRate(counts);
    container.append(
        tile("Confirmed", counts.confirmed.toLocaleString(), `last ${WINDOW_DAYS} days`),
        tile(
            "Confirmation rate",
            rate === null ? "—" : Math.round(rate * 100) + "%",
            rate === null ? "no requests yet" : `${counts.confirmed} of ${counts.requested} requests`,
        ),
        tile("Unsubscribed", counts.unsubscribed.toLocaleString(), `last ${WINDOW_DAYS} days`),
        tile("Issues sent", data.sends.length.toLocaleString(), "since the send log began"),
    );
}

function renderWeeksTable(table, weeks) {
    table.replaceChildren();

    const head = table.createTHead().insertRow();
    for (const heading of ["Week", ...SERIES.map((s) => s.label)]) {
        const th = document.createElement("th");
        th.textContent = heading;
        if (heading !== "Week") th.className = "num";
        head.appendChild(th);
    }

    const body = table.createTBody();
    for (const week of [...weeks].reverse()) {
        const row = body.insertRow();
        row.insertCell().textContent = week.week;
        for (const series of SERIES) {
            const cell = row.insertCell();
            cell.className = "num";
            cell.textContent = String(week[series.key]);
        }
    }
}

/**
 * The issues, one row each, with what the record says and how many current
 * subscribers are still without it. Pure: the counting is here, not in the service.
 */
export function issueRows(sends, deliveries, subscribers) {
    const got = new Map(); // slug -> Set of subjects with a sent or assumed delivery
    for (const d of deliveries) {
        if (d.status !== "sent" && d.status !== "assumed") continue;
        if (!got.has(d.slug)) got.set(d.slug, new Set());
        got.get(d.slug).add(d.subject);
    }
    return sends.map((send) => {
        const have = got.get(send.slug) || new Set();
        const missing = subscribers.filter((s) => !have.has(s.subject)).length;
        return {
            slug: send.slug,
            when: (send.finished_at || send.claimed_at || "").slice(0, 10),
            status: send.status,
            sent: send.sent,
            failed: (send.failed || []).length,
            missing,
        };
    });
}

/** Each subscriber with the slugs they have received, in send order. */
export function subscriberRows(subscribers, deliveries, sends) {
    const order = new Map(sends.map((s, i) => [s.slug, i]));
    const bySubject = new Map();
    for (const d of deliveries) {
        if (d.status === "failed") continue;
        if (!bySubject.has(d.subject)) bySubject.set(d.subject, []);
        bySubject.get(d.subject).push(d);
    }
    return subscribers.map((s) => {
        const had = (bySubject.get(s.subject) || [])
            .sort((a, b) => (order.get(a.slug) ?? 0) - (order.get(b.slug) ?? 0))
            .map((d) => (d.status === "assumed" ? d.slug + " (assumed)" : d.slug));
        return {
            email: s.email,
            since: (s.subscribed_at || "").slice(0, 10),
            source: s.source,
            issues: had.length,
            of: sends.length,
            had,
        };
    });
}

function renderSends(container, sends, deliveries, subscribers) {
    const rows = issueRows(sends, deliveries, subscribers);
    renderRows(
        container,
        rows,
        [
            { key: "slug", label: "Issue", link: true, text: (v) => v },
            { key: "when", label: "Sent" },
            { key: "status", label: "Status" },
            { key: "sent", label: "Sent to", num: true },
            { key: "failed", label: "Failed", num: true },
            { key: "missing", label: "Missing", num: true },
        ],
        "Nothing has been sent yet.",
    );
    // The link column wants a URL; the slug becomes one here rather than in the data.
    container.querySelectorAll("a").forEach((a) => {
        const slug = a.textContent;
        a.href = SITE_URL + "/blog/" + slug + "/";
        a.title = a.href;
    });
}

function renderSubscribers(container, subscribers, deliveries, sends) {
    const rows = subscriberRows(subscribers, deliveries, sends);
    renderRows(
        container,
        rows,
        [
            { key: "email", label: "Address" },
            { key: "since", label: "Since" },
            { key: "source", label: "Source" },
            { key: "issues", label: "Issues had", num: true },
            { key: "had", label: "Which", text: (v) => (v.length ? v.join(", ") : "none yet") },
        ],
        "Nobody is on the list.",
    );
}

function renderWarnings(container, errors) {
    if (errors.length === 0) {
        container.hidden = true;
        return;
    }
    container.replaceChildren();
    const heading = document.createElement("strong");
    heading.textContent = "Some of this could not be read:";
    const list = document.createElement("ul");
    for (const error of errors) {
        const item = document.createElement("li");
        const code = document.createElement("code");
        code.textContent = error;
        item.appendChild(code);
        list.appendChild(item);
    }
    container.append(heading, list);
    container.hidden = false;
}

// ---------------------------------------------------------------------------
// Readers: what the public site's RUM sent to OpenObserve
// ---------------------------------------------------------------------------

const READER_SERIES = [
    { key: "views", label: "Page views" },
    { key: "sessions", label: "Sessions" },
];

/**
 * Daily `{day, views, sessions}` rows into the same Monday-start weeks the newsletter
 * chart uses, most recent week last, empty weeks present and zero. Pure.
 *
 * Sessions are distinct per day in the store and summed here, so a session that spans
 * midnight UTC counts twice in its week. On this site's numbers that is a rounding
 * error, and the alternative is a second query per week.
 */
export function weeklyViews(daily, now, maxWeeks = MAX_WEEKS) {
    const last = weekStartUTC(new Date(now).toISOString());
    const weeks = [];
    for (let i = maxWeeks - 1; i >= 0; i--) {
        weeks.push({ week: addWeeks(last, -i), views: 0, sessions: 0 });
    }
    for (const row of daily) {
        const week = weekStartUTC(row.day);
        const bucket = weeks.find((w) => w.week === week);
        if (!bucket) continue;
        bucket.views += Number(row.views) || 0;
        bucket.sessions += Number(row.sessions) || 0;
    }
    return weeks;
}

/** Nanoseconds, as the SDK reports paint timings, to a whole number of milliseconds. */
export function nsToMs(value) {
    const n = Number(value);
    return Number.isFinite(n) ? Math.round(n / 1e6) : null;
}

/** `https://lindfors.no/blog/x/` reads better as `/blog/x/` in a narrow column. */
export function displayPath(url) {
    if (!url) return "";
    return url.startsWith(SITE_URL) ? url.slice(SITE_URL.length) || "/" : url;
}

/**
 * A table from rows, with the columns spelled out: `{key, label, num, link, text}`.
 * `num` right-aligns, `link` renders the cell as a link to the raw value, `text`
 * transforms the value for display.
 */
function renderRows(container, rows, columns, emptyText) {
    container.replaceChildren();
    if (!rows || rows.length === 0) {
        const empty = document.createElement("p");
        empty.className = "empty";
        empty.textContent = emptyText;
        container.appendChild(empty);
        return;
    }

    const table = document.createElement("table");
    const head = table.createTHead().insertRow();
    for (const column of columns) {
        const th = document.createElement("th");
        th.textContent = column.label;
        if (column.num) th.className = "num";
        head.appendChild(th);
    }
    const body = table.createTBody();
    for (const row of rows) {
        const tr = body.insertRow();
        for (const column of columns) {
            const raw = row[column.key];
            const shown = column.text ? column.text(raw, row) : String(raw ?? "");
            const td = tr.insertCell();
            if (column.num) {
                td.className = "num";
                td.textContent = Number(raw ?? 0).toLocaleString();
            } else if (column.link && raw) {
                td.className = "trunc";
                const link = document.createElement("a");
                link.href = String(raw);
                link.rel = "noreferrer";
                link.textContent = shown;
                link.title = String(raw);
                td.appendChild(link);
            } else {
                td.className = "trunc";
                td.textContent = shown;
                td.title = shown;
            }
        }
    }
    // URLs do not wrap, so the table scrolls sideways inside its own box rather than
    // pushing the next column of the grid off the page.
    const scroll = document.createElement("div");
    scroll.className = "table-scroll";
    scroll.appendChild(table);
    container.appendChild(scroll);
}

function renderReaders(rum) {
    const section = document.getElementById("readers");
    if (!rum) {
        section.hidden = true;
        return;
    }
    section.hidden = false;

    const weeks = weeklyViews(rum.daily || [], Date.now());
    const sinceMs = Date.parse(rum.since + "T00:00:00Z");
    const recent = (rum.daily || []).filter((row) => Date.parse(row.day) >= sinceMs);
    const views = recent.reduce((sum, row) => sum + (Number(row.views) || 0), 0);
    const sessions = recent.reduce((sum, row) => sum + (Number(row.sessions) || 0), 0);
    const vitals = (rum.vitals || [])[0] || {};

    document.getElementById("readers-note").textContent =
        "Only readers who allowed analytics, on top of the session sampling. Tables cover " +
        "the 30 days since " + rum.since + "; the chart shows twelve weeks. A view is one page " +
        "in one tab; a session is one browser for fifteen quiet minutes.";

    const lcp = nsToMs(vitals.lcp_p75);
    const tiles = document.getElementById("readers-tiles");
    tiles.replaceChildren(
        tile("Page views", views.toLocaleString(), "last 30 days"),
        tile("Sessions", sessions.toLocaleString(), "last 30 days, summed by day"),
        tile(
            "Largest paint",
            lcp === null ? "—" : lcp.toLocaleString() + " ms",
            lcp === null
                ? "no paint timings yet"
                : "p75 over " + Number(vitals.views || 0).toLocaleString() + " measured views",
        ),
        tile(
            "First byte",
            nsToMs(vitals.ttfb_p75) === null ? "—" : nsToMs(vitals.ttfb_p75).toLocaleString() + " ms",
            "p75, the edge's share of the wait",
        ),
    );

    renderChart(
        document.getElementById("readers-chart"),
        document.getElementById("readers-tooltip"),
        weeks,
        READER_SERIES,
        "Weekly page views and sessions.",
    );

    renderRows(
        document.getElementById("readers-pages"),
        rum.pages,
        [
            { key: "url", label: "Page", link: true, text: displayPath },
            { key: "views", label: "Views", num: true },
            { key: "sessions", label: "Sessions", num: true },
        ],
        "No page views in the window.",
    );
    renderRows(
        document.getElementById("readers-referrers"),
        rum.referrers,
        [
            { key: "referrer", label: "Referrer", text: (v) => (v ? displayPath(v) : "(direct, or none sent)") },
            { key: "sessions", label: "Sessions", num: true },
        ],
        "No sessions in the window.",
    );
    renderRows(
        document.getElementById("readers-links"),
        rum.links,
        [
            { key: "href", label: "Link", link: true, text: displayPath },
            { key: "internal", label: "", text: (v) => (v === true || v === "true" ? "" : "out") },
            { key: "clicks", label: "Clicks", num: true },
        ],
        "No link clicks in the window.",
    );
    renderRows(
        document.getElementById("readers-countries"),
        rum.countries,
        [
            { key: "country", label: "Country", text: (v) => v || "(unknown)" },
            { key: "sessions", label: "Sessions", num: true },
        ],
        "No sessions in the window.",
    );
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

function show(id) {
    for (const panel of ["panel-loading", "panel-signin", "panel-error", "dashboard"]) {
        document.getElementById(panel).hidden = panel !== id;
    }
}

function fail(message) {
    const panel = document.getElementById("panel-error");
    panel.textContent = message;
    show("panel-error");
}

function render(data) {
    const events = data.events || [];
    const since = new Date(Date.now() - WINDOW_DAYS * DAY_MS).toISOString();

    renderWarnings(document.getElementById("warnings"), data.errors || []);
    renderTiles(document.getElementById("tiles"), data, windowCounts(events, since));

    const weeks = weeklyBuckets(events, Date.now());
    const frame = document.getElementById("chart-frame");
    const note = document.getElementById("chart-note");

    if (weeks.length === 0) {
        frame.querySelectorAll("svg").forEach((node) => node.remove());
        note.textContent = "The event log has nothing in it yet.";
    } else {
        // The caveat that keeps the two halves of this page honest: the subscriber tile
        // counts everyone, and every figure derived from the log starts the day the log
        // was deployed. Naming that date is the difference between a chart that is
        // incomplete and one that is wrong.
        note.textContent =
            "Weeks begin Monday, UTC. The log's first entry is " +
            events[0].at.slice(0, 10) +
            "; anything before that was never recorded.";
        renderChart(frame, document.getElementById("tooltip"), weeks);
    }

    renderWeeksTable(document.getElementById("weeks-table"), weeks);
    renderSends(document.getElementById("sends"), data.sends || [], data.deliveries || [], data.subscriber_list || []);
    renderSubscribers(document.getElementById("subscribers"), data.subscriber_list || [], data.deliveries || [], data.sends || []);
    renderReaders(data.rum);
    show("dashboard");
}

async function boot() {
    let config;
    try {
        config = await loadConfig();
    } catch (e) {
        fail("Could not reach the API: " + e.message);
        return;
    }

    document.getElementById("sign-in").addEventListener("click", () => beginLogin(config));
    document.getElementById("sign-out").addEventListener("click", () => logout(config));

    // Back from a local-only sign-out (an issuer with no end_session endpoint, which is
    // Kanidm). The tokens are gone, but the session at the issuer is not, and the only
    // honest thing is to say so where the sign-in button is.
    if (new URLSearchParams(location.search).get("signed_out") === "local") {
        history.replaceState(null, "", "/");
        const note = document.createElement("p");
        note.textContent =
            "Signed out here. Your session at " +
            new URL(config.issuer).host +
            " is still open; end it there too if this is not your own device.";
        document.getElementById("panel-signin").prepend(note);
        show("panel-signin");
        return;
    }

    try {
        await completeLogin(config);
    } catch (e) {
        // A cancelled or unverifiable sign-in is not a broken page. Say what happened
        // on the sign-in panel itself, so the button the user needs stays in front of
        // them rather than being replaced by an error.
        const message = document.createElement("p");
        message.textContent = e.message;
        document.getElementById("panel-signin").prepend(message);
        show("panel-signin");
        return;
    }

    const token = await accessToken(config);
    if (!token) {
        show("panel-signin");
        return;
    }

    document.getElementById("sign-out").hidden = false;

    const response = await fetch("/api/overview", {
        headers: { Authorization: "Bearer " + token, Accept: "application/json" },
    });

    if (response.status === 401) {
        // The Worker disagrees with our token. Nothing to do but start again.
        document.getElementById("sign-out").hidden = true;
        show("panel-signin");
        return;
    }
    if (!response.ok) {
        fail("The API answered " + response.status + ".");
        return;
    }

    render(await response.json());
}

if (typeof document !== "undefined") {
    boot().catch((e) => fail(e.message));
}
