// Academic PDF Template for Lindfors Site
// Matches the blog theme styling with Literata + Inter fonts

#let academic(
  title: none,
  author: "Emil Lindfors",
  date: none,
  abstract: none,
  featured-image: none,
  body
) = {
  // Sea theme colors
  let color-text = rgb("#1C3240")        // Deep Sea
  let color-text-secondary = rgb("#5A7078")  // Kelp
  let color-link = rgb("#D4706A")        // Coral
  let color-accent = rgb("#2A8F82")      // Teal
  let color-bg-code = rgb("#F0EBE3")     // Tide Pool
  let color-border = rgb("#E4DED5")      // Seafoam Line

  // Page setup - A4 with proper margins
  set page(
    paper: "a4",
    margin: (x: 2.5cm, y: 2.5cm),
    footer: context [
      #set text(fill: color-text-secondary, size: 9pt)
      #h(1fr)
      #counter(page).display("1 / 1", both: true)
      #h(1fr)
    ]
  )

  // Typography matching blog theme
  // Body text: Literata (serif) for long-form reading
  set text(
    font: ("Literata", "Libertinus Serif", "DejaVu Serif"),
    size: 11pt,
    fill: color-text,
    lang: "en"
  )

  set par(
    justify: true,
    leading: 0.85em,
    first-line-indent: 0em,
  )

  // Heading styles - Inter (sans-serif) for contrast
  set heading(numbering: none)

  show heading.where(level: 1): it => {
    set text(
      font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
      size: 18pt,
      weight: 600,
      fill: color-text
    )
    v(1.5em)
    it.body
    v(0.75em)
  }

  show heading.where(level: 2): it => {
    set text(
      font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
      size: 14pt,
      weight: 600,
      fill: color-text
    )
    v(1.25em)
    it.body
    v(0.5em)
  }

  show heading.where(level: 3): it => {
    set text(
      font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
      size: 12pt,
      weight: 600,
      fill: color-text
    )
    v(1em)
    it.body
    v(0.4em)
  }

  // Link styling - coral like the blog
  show link: it => {
    set text(fill: color-link)
    it
  }

  // Code block styling with teal accent
  show raw.where(block: true): it => {
    set text(
      font: ("JetBrains Mono", "Fira Code", "Consolas", "monospace"),
      size: 9pt
    )
    block(
      fill: color-bg-code,
      stroke: (left: 3pt + color-accent),
      inset: 10pt,
      radius: 4pt,
      width: 100%,
      it
    )
  }

  // Inline code styling
  show raw.where(block: false): it => {
    set text(
      font: ("JetBrains Mono", "Fira Code", "Consolas", "monospace"),
      size: 0.85em
    )
    box(
      fill: color-bg-code,
      inset: (x: 3pt, y: 1pt),
      radius: 2pt,
      it
    )
  }

  // Blockquote styling with teal accent
  show quote: it => {
    block(
      inset: (left: 12pt, y: 8pt),
      stroke: (left: 3pt + color-accent),
      fill: color-bg-code,
      it.body
    )
  }

  // Image styling - constrain width and center
  show figure: it => {
    v(0.5em)
    align(center, block(width: 85%, it.body))
    if it.caption != none {
      align(center,
        text(size: 9pt, fill: color-text-secondary, style: "italic")[
          #it.caption.body
        ]
      )
    }
    v(0.5em)
  }

  // Title block - Inter font
  if title != none {
    align(center)[
      #text(
        font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
        size: 22pt,
        weight: 700,
        fill: color-text
      )[#title]
    ]
    v(0.5em)
  }

  // Author and date - Inter font
  align(center)[
    #text(
      font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
      size: 11pt,
      fill: color-text-secondary
    )[
      #author
      #if date != none [
        #h(1em) | #h(1em) #date
      ]
    ]
  ]

  v(1.5em)

  // Featured image below title
  if featured-image != none {
    align(center,
      block(
        width: 100%,
        clip: true,
        radius: 4pt,
        image(featured-image, width: 100%)
      )
    )
    v(1em)
  }

  // Horizontal rule after header
  line(length: 100%, stroke: 0.5pt + color-border)

  v(1em)

  // Abstract if provided
  if abstract != none {
    block(
      inset: (x: 1.5em, y: 1em),
      fill: color-bg-code,
      radius: 4pt,
    )[
      #text(
        font: ("Inter", "Ubuntu Sans", "DejaVu Sans"),
        weight: 600
      )[Abstract]
      #v(0.3em)
      #abstract
    ]
    v(1em)
  }

  // Main content
  body
}
