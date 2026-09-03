-- Queries over the RUM stream in OpenObserve (logs.lindfors.no, stream `_rumdata`).
-- Paste into Logs -> SQL mode, or save each as a dashboard panel.
--
-- Why these exist: OpenObserve's Sessions page is hidden unless session replay is
-- on, and replay is off here on purpose (it records screens, and the recorder
-- chunks are not vendored). Everything below reads the plain event rows the SDK
-- sends anyway. Column names are the ones OpenObserve's own UI queries
-- (web/src/views/RUM/AppSessions.vue): session_id, view_url, view_name, type,
-- action_target_name, geo_info_country, usr_*.
--
-- The SDK's automatic click actions never fire for a link that navigates (the page
-- is gone before the action settles), so link clicks are recorded by rum.js as a
-- custom action named `link` with `href`, `internal` and `text` in its context.
-- The context flattens to context_href, context_internal and context_text
-- (confirmed against live rows on 2026-09-03).
--
-- Newsletter issues carry `?issue=<slug>` on every link into the site
-- (`site-tools newsletter gen`), so a visit from an issue is a view whose
-- view_url has that parameter. Per issue, never per reader: see query 6.

-- 1. Pages visited per session, in order. The "links visited" list.
SELECT
  session_id,
  min(_timestamp)                       AS started,
  count(*)                              AS pages,
  array_agg(view_url ORDER BY _timestamp) AS path
FROM "_rumdata"
WHERE type = 'view'
GROUP BY session_id
ORDER BY started DESC
LIMIT 100;

-- 2. Most viewed pages. One row per view event; a long read that flushes several
--    times produces several rows for the same view_id, so count distinct views.
SELECT
  view_url,
  count(DISTINCT view_id)  AS views,
  count(DISTINCT session_id) AS sessions
FROM "_rumdata"
WHERE type = 'view'
GROUP BY view_url
ORDER BY views DESC
LIMIT 50;

-- 3. Where sessions start (landing pages) and from where.
SELECT
  view_url                AS landing,
  view_referrer           AS referrer,
  count(DISTINCT session_id) AS sessions
FROM "_rumdata"
WHERE type = 'view'
GROUP BY view_url, view_referrer
ORDER BY sessions DESC
LIMIT 50;

-- 4. Links clicked, from the custom `link` action. Outbound links included.
SELECT
  action_target_name,
  context_href     AS href,
  context_internal AS internal,
  count(*)         AS clicks
FROM "_rumdata"
WHERE type = 'action'
  AND action_target_name = 'link'
GROUP BY action_target_name, context_href, context_internal
ORDER BY clicks DESC
LIMIT 100;

-- 5. Which links are clicked from which page.
SELECT
  view_url     AS from_page,
  context_href AS to_href,
  count(*)     AS clicks
FROM "_rumdata"
WHERE type = 'action'
  AND action_target_name = 'link'
GROUP BY view_url, context_href
ORDER BY clicks DESC
LIMIT 100;

-- 6. Visits that came from a newsletter issue, per issue and per page. The
--    parameter is the same for every recipient, so this is how many readers an
--    issue brought to the site and where they went, and nothing about who.
SELECT
  regexp_replace(view_url, '^.*[?&]issue=([^&#]+).*$', '\\1') AS issue,
  regexp_replace(view_url, '\\?.*$', '') AS page,
  count(distinct view_id) AS views,
  count(distinct session_id) AS sessions
FROM "_rumdata"
WHERE type = 'view' AND view_url LIKE '%issue=%'
GROUP BY issue, page
ORDER BY issue, views DESC;
