// Emil Lindfors - CV
#set document(title: "Emil Lindfors - CV", author: "Emil Lindfors")
#set page(paper: "a4", margin: (x: 1.8cm, y: 1.5cm))
#set text(font: "Libertinus Serif", size: 10pt)
#set par(justify: true)

// Colors
#let accent = rgb("#2c3e50")
#let light-gray = rgb("#7f8c8d")

// Section heading style
#let section(title) = {
  v(0.6em)
  text(weight: "bold", size: 11pt, fill: accent)[#upper(title)]
  v(-0.3em)
  line(length: 100%, stroke: 0.5pt + accent)
  v(0.3em)
}

// Experience entry
#let experience(company, role, dates, location, bullets) = {
  grid(
    columns: (1fr, auto),
    text(weight: "bold")[#company],
    text(style: "italic", fill: light-gray)[#dates]
  )
  text(style: "italic")[#role] + h(1em) + text(fill: light-gray, size: 9pt)[#location]
  v(0.2em)
  for bullet in bullets {
    [- #bullet]
  }
  v(0.4em)
}

// Education entry
#let education(degree, institution, years) = {
  grid(
    columns: (1fr, auto),
    [*#degree* --- #institution],
    text(fill: light-gray, style: "italic")[#years]
  )
  v(0.2em)
}

// ============ HEADER ============
#grid(
  columns: (auto, 1fr),
  column-gutter: 1.2em,
  align: (center, left + horizon),
  box(
    clip: true,
    radius: 4pt,
    image("emil.jpg", width: 2.8cm)
  ),
  [
    #text(size: 24pt, weight: "bold", fill: accent)[Emil Lindfors]
    #v(-0.2em)
    #text(size: 11pt, fill: light-gray)[Senior Software Engineer | PhD Responsible Innovation]
    #v(0.3em)
    #text(size: 9pt)[
      #link("mailto:emil@lindfors.io")[emil\@lindfors.io] #h(0.8em) | #h(0.8em)
      +47 480 78 707 #h(0.8em) | #h(0.8em)
      #link("https://linkedin.com/in/emil-lindfors")[LinkedIn] #h(0.8em) | #h(0.8em)
      #link("https://github.com/EmilLindfors")[GitHub]
      #v(0.1em)
      Norway
    ]
  ]
)

#v(0.3em)

// ============ PROFESSIONAL SUMMARY ============
#section("Professional Summary")

Senior software engineer with 15+ years of experience and a PhD in responsible technological innovation. Deep domain expertise in *aquaculture data platforms*---I've built production systems integrating Fishtalk, Mercatus, IoT sensor data, and ocean models (IMR Norkyst) for major Norwegian salmon farmers. I architect robust, scalable infrastructure in Rust and Python: data pipelines, API integrations, and cloud deployment (AWS, Terraform). Recent work includes LLM integration and AI agent development. My research background brings rigor and long-term thinking to system design.

// ============ TECHNICAL SKILLS ============
#section("Technical Skills")

#grid(
  columns: (auto, 1fr),
  column-gutter: 1em,
  row-gutter: 0.4em,
  text(weight: "bold")[Aquaculture:], [Fishtalk, Mercatus, production data integration, salmon farming domain expertise],
  text(weight: "bold")[Data Platforms:], [API design, ETL pipelines, system integration, real-time sensor data, ocean models (IMR Norkyst)],
  text(weight: "bold")[Cloud/DevOps:], [AWS (Lambda, ECS, S3, Batch), Terraform, GitHub Actions, Docker, CI/CD],
  text(weight: "bold")[Languages:], [Python, Rust, TypeScript, JavaScript],
  text(weight: "bold")[AI/ML:], [LLM integration (Claude, Azure OpenAI), agent development, prompt engineering],
)

// ============ PROFESSIONAL EXPERIENCE ============
#section("Professional Experience")

#experience(
  "AquaCloud",
  "Senior Software Engineer",
  "January 2025 -- Present",
  "Norway",
  (
    [Architected production-grade aquaculture data platform integrating Fishtalk, Mercatus, and other industry systems],
    [Built data pipelines and ETL workflows for major Norwegian salmon farmers handling real-time production data],
    [Integrated IoT sensor data streams and ocean environmental models (IMR Norkyst) for site-level analytics],
    [Designed APIs for third-party integrations; enabled seamless data exchange across the salmon farming value chain],
    [Infrastructure as Code: Terraform for Lambda, ECS, S3, Batch; full CI/CD with GitHub Actions],
    [Implemented Rust core components with Python FFI for performance-critical data processing],
  )
)

#experience(
  "Høgskulen på Vestlandet (HVL)",
  "Software Engineer",
  "July 2024 -- January 2025",
  "Bergen",
  (
    [Built Rust backend system integrated with Microsoft Teams for internal communication workflows],
    [Automated research group newsletter: weekly submission aggregation, admin approval flow, automated distribution],
    [Designed integrations with Microsoft ecosystem (Teams, Power Automate, email) for operational efficiency],
    [Contributed to university-wide technology strategy and responsible technology implementation],
  )
)

#experience(
  "Høgskulen på Vestlandet (HVL)",
  "PhD Researcher",
  "December 2019 -- January 2025",
  "Bergen",
  (
    [PhD: "The Evolution of Technological Trajectories: Responsible Innovation and Regional Industrial Path Development"],
    [Researched salmon farming industry transformation in Norway, Australia, and USA; 92 interviews, comparative case studies],
    [Published 3 peer-reviewed articles (Sustainability, Marine Policy, Regional Studies) with 1 under review],
    [Developed frameworks for understanding how technology adoption shapes regional industrial development],
  )
)

#experience(
  "Hatch",
  "Norway Representative",
  "January 2018 -- December 2019",
  "Bergen / Netherlands",
  (
    [First non-founder employee at the global aquaculture startup accelerator],
    [Participated in due diligence for Aqua-Spark's seed investment in Hatch],
    [Evaluated startups and managed dealflow across two accelerator cohorts (8 international startups)],
    [Contributed to investment discussions, marketing, outreach, and portfolio company support],
  )
)

#experience(
  "Lindfors Foundry",
  "Founder & Software Engineer",
  "2011 -- Present",
  "Bergen",
  (
    [14+ years building production web systems and domain-specific software solutions],
    [Led digitalization project for sea lice counting in salmon farming using computer vision],
    [Built custom data dashboards and reporting tools for aquaculture clients],
    [Full-stack development: React/Vue frontends, Python/Rust backends, PostgreSQL databases],
    [Home infrastructure: self-hosted services, IoT sensors (ESP32), home automation systems],
  )
)

// ============ EDUCATION ============
#section("Education")

#education("PhD, Technological Innovation", "Høgskulen på Vestlandet", "2019 -- 2025")
#education("Master, Innovation & Entrepreneurship", "Bergen University College", "2017 -- 2019")
#education("Bachelor, Sustainable Aquaculture", "University of Bergen", "2014 -- 2017")

// ============ PUBLICATIONS ============
#section("Selected Publications")

#set text(size: 9pt)
- Lindfors, E. (2022). Radical path transformation of the Norwegian and Tasmanian salmon farming industries. _Regional Studies, Regional Science_.
- Lindfors, E. & Jakobsen, S-E. (2022). Sustainable regional industry development through co-evolution. _Marine Policy_.
- Lindfors, E. et al. (2021). Place-Based Directionality of Innovation: Tasmanian Salmon Farming. _Sustainability_.
#set text(size: 10pt)

// ============ OPEN SOURCE ============
#section("Open Source")

#grid(
  columns: (auto, 1fr),
  column-gutter: 1em,
  row-gutter: 0.3em,
  text(weight: "bold")[a2a-rs:], [Rust implementation of Google's Agent-to-Agent protocol for multi-agent communication],
  text(weight: "bold")[rust-browser-mcp:], [MCP server in Rust for AI-powered web navigation using Gecko or WebDriver backends],
  text(weight: "bold")[roms-rs:], [Experimental Discontinuous Galerkin solver for coastal ocean modeling---simulating Norwegian coast currents],
)
#text(size: 9pt, fill: light-gray)[See #link("https://github.com/EmilLindfors")[github.com/EmilLindfors] for more projects]

// ============ ACHIEVEMENTS & LANGUAGES ============
#v(0.2em)
#grid(
  columns: (1fr, 1fr),
  column-gutter: 2em,
  [
    #section("Achievements")
    - *Winner*, Fishackathon Bergen 2016
    - *Project Lead*, AquaHack 2018
  ],
  [
    #section("Languages")
    - Swedish (Native)
    - Norwegian & English (Fluent)
  ]
)
