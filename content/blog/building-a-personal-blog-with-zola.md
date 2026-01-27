+++
title = "Building a Personal Blog with Zola"
description = "A practical guide to creating a personal blog with Zola, featuring automatic PDF generation with Typst and Zotero citation integration."
date = 2025-01-27
[taxonomies]
tags = ["zola", "web", "tutorial", "typst", "zotero"]
[extra]
featured = true
+++

Building a personal blog is one of those projects that seems simple until you start making decisions. What colors? Which fonts? How should the layout work? In this post, I'll walk through some of the more unique aspects of this site—particularly the Zotero citation integration and automatic PDF generation using Typst.

<!-- more -->

## Why Zola?

Zola stands out among static site generators for a few reasons:

- **Single binary** — No Node.js, no Ruby, no Python dependencies. Just one executable.
- **Fast** — Built in Rust, it compiles sites in milliseconds.
- **Batteries included** — Sass compilation, syntax highlighting, and search indexing are built-in.
- **Simple templating** — Uses Tera templates, which feel natural if you've used Jinja2.

But the real power comes from what you can build around it. This site extends Zola with two custom tools: a Zotero citation processor and automatic PDF generation using Typst.

## The Zotero Citation Tool

When writing scientific or academic content, you want proper citations without manually formatting each one. Zotero is the de facto standard for reference management, and Better BibTeX adds stable citekeys that persist across your library.

I built a Rust CLI tool that bridges the gap between Zotero and Zola. It reads directly from Zotero's SQLite database and transforms `@citekey` references in your markdown into properly formatted citations.

### How It Works

The tool connects to two databases:
1. **Zotero's main database** (`zotero.sqlite`) — Contains all bibliographic data
2. **Better BibTeX database** (`better-bibtex.sqlite`) — Maps citekeys to item IDs

When you write a post, you can reference papers naturally:

```markdown
Recent work by @Christiansen2017 has shown significant
improvements in recirculating aquaculture systems.
This aligns with earlier findings [@Boyd2015].
```

The tool recognizes two citation formats:
- `@citekey` — Narrative citations that become "Author (Year)"
- `[@citekey]` — Parenthetical citations that become "(Author, Year)"

Running `zotero-cite process content/blog/my-post/index.md` transforms these into formatted citations with links to a References section automatically appended to the post.

### The Reference Data Structure

The tool extracts rich bibliographic information from Zotero:

```rust
pub struct Reference {
    pub citekey: String,
    pub item_type: String,   // article, book, inproceedings, etc.
    pub title: String,
    pub authors: Vec<Author>,
    pub year: String,
    pub journal: String,
    pub volume: String,
    pub pages: String,
    pub doi: String,
    pub url: String,
}
```

This gets formatted into APA-style citations with clickable DOI links:

> Smith, J. A., & Jones, B. C. (2023). *Title of the Article*. Journal Name, 45(3), 123-145. doi:10.1234/example

### Citation Styles

The tool supports multiple citation styles:

- **APA** (default) — Author-year format with full reference list
- **Numeric** — Numbered references like [1], [2], [3]
- **Numeric-link** — Numbers that link to the reference section

Choose the style with the `--style` flag:

```bash
zotero-cite process content/blog/post.md --style numeric
```

## Automatic PDF Generation with Typst

Every blog post on this site automatically gets a PDF version. This is useful for offline reading, printing, or sharing as a document.

Typst is a modern typesetting system that combines the power of LaTeX with a much friendlier syntax. Unlike LaTeX, it compiles in milliseconds and has a coherent, programmable styling system.

### The Academic Template

I created a custom Typst template that matches the blog's visual style:

```typ
#let academic(
  title: none,
  author: "Emil Lindfors",
  date: none,
  abstract: none,
  body
) = {
  set page(paper: "a4", margin: (x: 2.5cm, y: 2.5cm))

  set text(
    font: ("Ubuntu Sans", "DejaVu Sans"),
    size: 11pt,
    lang: "en"
  )

  // Title block
  if title != none {
    align(center)[
      #text(size: 22pt, weight: 700)[#title]
    ]
  }

  // Author and date
  align(center)[
    #text(fill: rgb("#6c757d"))[
      #author #h(1em) | #h(1em) #date
    ]
  ]

  body
}
```

The template handles:
- Page layout with proper margins
- Typography matching the blog theme
- Styled code blocks with monospace fonts
- Blockquotes with colored left borders
- Clickable links

### The Generation Pipeline

The PDF generation script does several things:

1. **Extracts frontmatter** — Parses the TOML header for title and date
2. **Preprocesses content** — Converts HTML reference links to plain text for PDF
3. **Renders markdown** — Uses the `cmarker` Typst package to convert markdown
4. **Compiles to PDF** — Typst generates the final document

The key is the `cmarker` package, which renders markdown directly in Typst:

```typ
#import "@preview/cmarker:0.1.8"
#import "@preview/mitex:0.2.6": mitex

#cmarker.render(
  read("content.md"),
  math: mitex,
  smart-punctuation: true,
)
```

The `mitex` package handles LaTeX math notation, so equations work seamlessly between the web version and the PDF.

### Preprocessing for PDF

One challenge is that HTML elements in markdown (like the citation links) don't render in Typst. The script preprocesses these:

```bash
# Convert HTML reference paragraphs to markdown lists
sed -i -E 's/<p[^>]*class="reference"[^>]*>/- /g' "$TEMP_DIR/content.md"

# Convert HTML emphasis to markdown
sed -i -E 's/<em>/*/g' "$TEMP_DIR/content.md"
sed -i -E 's/<\/em>/*/g' "$TEMP_DIR/content.md"

# Convert HTML links to markdown links
sed -i -E 's/<a href="([^"]+)">([^<]+)<\/a>/[\2](\1)/g' "$TEMP_DIR/content.md"
```

This ensures the References section looks clean in both formats.

## The Build Pipeline

Everything comes together in a single build script:

```bash
#!/bin/bash
set -e

# Build citation tool if needed
if [ ! -f "$CITE_TOOL" ]; then
    cargo build --release --manifest-path tools/zotero-cite/Cargo.toml
fi

# Process all markdown files with citations
for file in $(find content -name "*.md" -type f); do
    if grep -q '@[a-zA-Z]' "$file" 2>/dev/null; then
        zotero-cite process "$file" --output "$file"
    fi
done

# Generate PDFs for all blog posts
for post in content/blog/*/index.md; do
    ./scripts/generate-pdf.sh "$post"
done

# Build with Zola
zola build
```

The script:
1. Compiles the Rust citation tool if needed
2. Finds all markdown files with `@citations` and processes them
3. Generates PDFs for each blog post
4. Builds the static site with Zola

## Lessons Learned

Building these tools taught me a few things:

1. **SQLite is everywhere** — Zotero's database is just SQLite. So is Firefox's history, your browser's cookies, and countless other apps. Learning to query it opens up powerful integrations.

2. **Typst is production-ready** — I was skeptical of a LaTeX alternative, but Typst compiles in milliseconds, has excellent documentation, and produces beautiful output.

3. **Preprocessing is often simpler than parsing** — Rather than building a proper HTML-to-Typst converter, sed and awk handle the few patterns I need.

4. **Rust makes CLI tools pleasant** — With clap for argument parsing, thiserror for errors, and rusqlite for database access, the citation tool is under 650 lines.

## Conclusion

The combination of Zola, Typst, and a custom citation tool creates a workflow that's fast to use and produces both web and print-ready content. The investment in building these tools pays off every time I write a new post—citations just work, and PDFs generate automatically.

The full source for this site, including the citation tool and PDF generation scripts, is available on [GitHub](https://github.com/emillindfors/lindfors-site).
