// Tests for the blog content-negotiation middleware.
// Run with: node --test tools/tests/
//
// Lives outside functions/ because every file under that directory is treated as a
// route by Cloudflare Pages.

import { test } from "node:test";
import assert from "node:assert/strict";

import { prefersMarkdown, markdownPathFor } from "../../functions/blog/_middleware.js";

// The Accept string Chrome and Firefox actually send.
const BROWSER =
  "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8";

test("a browser gets HTML", () => {
  assert.equal(prefersMarkdown(BROWSER), false);
});

test("curl's */* gets HTML", () => {
  assert.equal(prefersMarkdown("*/*"), false);
});

test("a missing Accept header gets HTML", () => {
  assert.equal(prefersMarkdown(null), false);
  assert.equal(prefersMarkdown(""), false);
});

test("an explicit markdown request gets markdown", () => {
  assert.equal(prefersMarkdown("text/markdown"), true);
  assert.equal(prefersMarkdown("text/x-markdown"), true);
});

test("markdown wins when it outranks html on q", () => {
  assert.equal(prefersMarkdown("text/markdown;q=0.9,text/html;q=0.5"), true);
});

test("html wins when it outranks markdown on q", () => {
  assert.equal(prefersMarkdown("text/markdown;q=0.5,text/html;q=0.9"), false);
});

test("naming both at equal q prefers markdown", () => {
  assert.equal(prefersMarkdown("text/markdown,text/html"), true);
});

test("q=0 on markdown is a rejection, not a request", () => {
  assert.equal(prefersMarkdown("text/markdown;q=0"), false);
});

test("markdown alongside a wildcard still wins", () => {
  // An agent sending `text/markdown, */*;q=0.1` means it.
  assert.equal(prefersMarkdown("text/markdown, */*;q=0.1"), true);
});

test("whitespace and casing are tolerated", () => {
  assert.equal(prefersMarkdown("  TEXT/MARKDOWN ;q=1.0 "), true);
});

test("a malformed q does not silently reject the type", () => {
  assert.equal(prefersMarkdown("text/markdown;q=banana"), true);
});

test("post URLs map to their markdown asset", () => {
  assert.equal(markdownPathFor("/blog/my-post/"), "/blog/my-post.md");
  assert.equal(markdownPathFor("/blog/my-post"), "/blog/my-post.md");
});

test("the listing page and pagers are not posts", () => {
  assert.equal(markdownPathFor("/blog/"), null);
  assert.equal(markdownPathFor("/blog"), null);
  assert.equal(markdownPathFor("/blog/page/2/"), null);
});

test("the markdown files themselves are not renegotiated", () => {
  assert.equal(markdownPathFor("/blog/my-post.md"), null);
});

test("nested paths under a post are left alone", () => {
  assert.equal(markdownPathFor("/blog/my-post/hero.webp"), null);
});
