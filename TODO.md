# Site Improvements TODO

## Priority 1: Homepage Hero + Intro
- [x] Add hero section with brief intro and CTA to `index.html`
- [x] Style hero section in `style.scss`

## Priority 2: Newsletter Signup
- [x] Create newsletter signup form component (HTML + CSS)
- [x] Place form in homepage hero section
- [x] Place form at end of every blog post (page.html)
- [x] Place subtle form in site footer (base.html)
- [x] JS handler that POSTs signup JSON to `/api/subscribe`
- [x] Style for both light and dark themes
- [x] Rust Worker: `api/src/lib.rs` — proxies to Stalwart mail server REST API
- [x] POST /api/subscribe (addItem to externalMembers)
- [x] GET /api/unsubscribe (form page) + POST /api/unsubscribe (removeItem)
- [x] GET /api/subscribers (admin, reads members from Stalwart)

## Priority 3: Open Graph + Twitter Card Meta Tags
- [x] Add og:title, og:description, og:type, og:url to base.html
- [x] Add og:image with configurable default (`og_image` in zola.toml)
- [x] Add Twitter card meta tags
- [x] Override per-page for blog posts in page.html (article type, published_time, tags)
- [ ] Create a default OG image (`static/og-default.png`, 1200x630)

## Priority 4: Self-Host Fonts
- [x] Remove Google Fonts CDN link from base.html
- [x] Add @font-face declarations (inline in `<head>` for fast loading)
- [x] Inter: variable woff2 (338K + 372K italic)
- [x] Literata: subset woff2 (35-39K per weight, Latin glyphs)
- [x] Fonts served from `/fonts/`

## Priority 5: Search UI
- [x] Create search page template (`templates/search.html`)
- [x] Uses Zola's built-in elasticlunr.min.js + search_index.en.js
- [x] Debounced input with snippet highlighting
- [x] Supports `?q=` query parameter for direct linking
- [x] Search icon added to navigation header
- [x] Styled for both themes

## Priority 6: Newsletter Generation Script
- [x] Created `scripts/generate-newsletter.sh`
- [x] Extracts frontmatter (title, date, description)
- [x] Strips shortcodes and math for email safety
- [x] Generates email-safe HTML template with inline styles
- [x] Generates cleaned markdown body for newsletter service
- [x] Includes "Read full post" link for features that don't work in email
- [x] Unsubscribe link points to `/api/unsubscribe` form page

## Priority 7: Related Posts
- [x] Add related posts section to page.html (by shared tag matching)
- [x] Style related posts cards (grid layout, responsive)
- [x] Limited to 3 posts max

## Priority 8: JSON-LD Structured Data
- [x] Add Article schema to page.html
- [x] Includes author, dates, headline, description
- [ ] Add Person schema to about page
- [ ] Add WebSite schema to base.html

## Priority 9: Reading Progress Bar
- [x] Add progress bar element to page.html
- [x] JS scroll handler to update width
- [x] Styled thin bar at top (accent color, fixed position below header)

## Priority 10: robots.txt + Canonical URLs
- [x] Created `static/robots.txt` with sitemap reference
- [x] Added `<link rel="canonical">` to base.html (overridable per page)

---

## Architecture

```
lindfors.no (Cloudflare Pages)          lindfors.no/api/* (Cloudflare Worker)
┌──────────────────────────┐            ┌──────────────────────────┐
│  Static site (Zola)      │            │  Rust Worker (WASM)      │
│  - HTML, CSS, JS         │            │  - POST /api/subscribe   │
│  - Newsletter forms POST │───────────>│  - GET  /api/unsubscribe │
│    to /api/subscribe     │            │  - GET  /api/subscribers │
│  - Search (client-side)  │            │         │                │
└──────────────────────────┘            │    ┌────▼─────────────┐  │
                                        │    │ Stalwart Mail    │  │
                                        │    │ (mail.lindfors.no)│  │
                                        │    │ REST API         │  │
                                        │    └──────────────────┘  │
                                        └──────────────────────────┘
```

The Worker proxies subscribe/unsubscribe requests to Stalwart's Management API,
using PATCH with `addItem`/`removeItem` on the mailing list's `externalMembers`.

## Setup Steps

### 1. Create Stalwart mailing list
In Stalwart admin, create a mailing list principal named `newsletter` (or
whatever `STALWART_LIST_ID` is set to). This list uses `externalMembers` to
store subscriber email addresses.

### 2. Generate a Stalwart API key
Create a Bearer token in Stalwart's settings that has permission to read/write
principals.

### 3. Set Worker secrets
```bash
cd api
npx wrangler secret put STALWART_API_KEY
# Paste the Stalwart Bearer token when prompted

npx wrangler secret put ADMIN_KEY
# Enter a strong random string when prompted (protects /api/subscribers)
```

### 4. Deploy the Worker
```bash
cd api
npx wrangler deploy
```
This compiles Rust to WASM and deploys the Worker. The `routes` config in
`api/wrangler.toml` routes `lindfors.no/api/*` to this Worker.

### 5. Deploy the site
```bash
git push  # Cloudflare Pages auto-deploys the static site
```

### 6. Test
```bash
# Subscribe
curl -X POST https://lindfors.no/api/subscribe \
  -H 'Content-Type: application/json' \
  -d '{"email": "test@example.com"}'

# List subscribers (admin)
curl "https://lindfors.no/api/subscribers?key=YOUR_ADMIN_KEY"

# Unsubscribe (browser)
# Visit https://lindfors.no/api/unsubscribe and enter email
```

---

## Remaining
- [ ] Default OG image (static/og-default.png, 1200x630)
- [ ] Person/WebSite JSON-LD schemas
- [ ] Newsletter send workflow (compose email, send to newsletter@lindfors.no via Stalwart)
