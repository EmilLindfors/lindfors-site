+++
title = "About"
description = "AI capability lead and senior data engineer at Fraktal Oslo, PhD in innovation studies. Agent-powered data pipelines, talk-to-your-data, agent-assisted coding, Rust, and innovation theory applied to all of it."
template = "simple-page.html"
[extra]
toc = false
+++

<div class="profile-header">
    <img src="/emil.jpg" alt="Emil Lindfors" class="profile-image">
    <div class="profile-links">
        <a href="/cv.pdf" class="profile-link">
            <svg class="profile-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path><polyline points="14 2 14 8 20 8"></polyline><line x1="16" y1="13" x2="8" y2="13"></line><line x1="16" y1="17" x2="8" y2="17"></line><polyline points="10 9 9 9 8 9"></polyline></svg>
            CV
        </a>
        <a href="https://github.com/EmilLindfors" class="profile-link">
            <svg class="profile-icon" viewBox="0 0 24 24" fill="currentColor"><path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z"/></svg>
            GitHub
        </a>
        <a href="https://linkedin.com/in/emil-havbruk" class="profile-link">
            <svg class="profile-icon" viewBox="0 0 24 24" fill="currentColor"><path d="M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433c-1.144 0-2.063-.926-2.063-2.065 0-1.138.92-2.063 2.063-2.063 1.14 0 2.064.925 2.064 2.063 0 1.139-.925 2.065-2.064 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003z"/></svg>
            LinkedIn
        </a>
    </div>
</div>

I lead the AI work at [Fraktal Oslo](https://fraktal.no), a consultancy that builds data platforms, and I am a senior data engineer there. Before that: fifteen years of writing software, five of them alongside a PhD in innovation studies. This site is where the two halves meet.

## What I do

<!-- emil -->
I work a lot on how to implement agent-powered pipelines and talk-to-your-data kinds of capabilities, and agent-powered coding.

In practice that means three things. Finding out which models and endpoints a customer can use, what they cost and what they leak, before the customer has to ask. Putting a semantic layer under an agent so that "what was revenue last quarter" gets one answer instead of two. And running coding agents against every kind of endpoint we can find, local models included, so the recommendation we give is tested and not guessed.

The platforms underneath are Rust and Python on open standards, Apache Iceberg among them. We have watched customers adopt technology for years, and the pattern is always the same: the thing that wins is the interface that gets standardised, and the customer who can switch is the one who is fine. So vendor lock-in is not something we like, and modularity is a design goal, not a slogan.

## The theory half

My PhD was in innovation studies: responsible innovation, regional industrial path development and evolutionary economic geography, with salmon farming as the case. I did 92 interviews across Norway, Australia and the United States about how an industry takes up a new technology, and what decides whether a region ends up locked into the old path or transformed by the new one.

<!-- emil -->
Innovation is inherently experimental, so it's hard to foresee the future, and thus we need to be able to change.

That sentence is most of what I took from the PhD into the AI work. The posts here use the theory on purpose, one lens at a time: dominant designs and eras of ferment on what is happening to BI, explore versus exploit on [how a company ends up on one vendor](/blog/which-llm-are-we-allowed-to-use/), path dependence on [why this blog looks the way it does](/blog/building-a-personal-blog-with-zola/). Innovation theory was built on cases like cars, steel and salmon, and it transfers to software better than I expected.

## The site

Everything here is text in one git repo: a [Zola](https://www.getzola.org/) site, a Rust CLI that generates the PDFs, the audio, the share images and the plain-markdown copies, and a Rust Cloudflare Worker for the newsletter. The [infrastructure series](/series/) explains each piece. Opinions are mine, not my employer's.

## Publications

- Lindfors, E. T. (2025). [The Evolution of Technological Trajectories – Responsible Innovation and Regional Industrial Path Development in the Salmon Farming Industry](https://hdl.handle.net/11250/3199992). PhD thesis, Western Norway University of Applied Sciences.

- Hopp, J. Z., Coffay, M., & Lindfors, E. T. (2023). [Inclusion in the global innovation system for CRISPR salmon in Norway](https://doi.org/10.1080/00291951.2023.2197622). *Norsk Geografisk Tidsskrift - Norwegian Journal of Geography*, 77(1), 10-20.

- Lindfors, E. T. (2022). [Radical path transformation of the Norwegian and Tasmanian salmon farming industries](https://doi.org/10.1080/21681376.2022.2148555). *Regional Studies, Regional Science*, 9(1), 757-775.

- Lindfors, E. T., & Jakobsen, S.-E. (2022). [Sustainable regional industry development through co-evolution](https://doi.org/10.1016/j.marpol.2021.104855). *Marine Policy*, 135, 104855.

- Fløysand, A., Lindfors, E. T., Jakobsen, S.-E., & Coenen, L. (2021). [Place-Based Directionality of Innovation: Tasmanian Salmon Farming and Responsible Innovation](https://doi.org/10.3390/su13010062). *Sustainability*, 13(1), 62.

## Open Source

I maintain several open source projects:

- **[a2a-rs](https://github.com/EmilLindfors/a2a-rs)** — Rust implementation of Google's Agent-to-Agent protocol
- **[rust-browser-mcp](https://github.com/EmilLindfors/rust-browser-mcp)** — MCP server for AI-powered web navigation
- **[strata](https://github.com/EmilLindfors/strata)** — Rust data platform that covers ingest, transform and present in one tool: where dbt starts at the warehouse, strata also does the ingest side. The present layer, with a semantic layer and metrics views, is what I am building now; the Norwegian aquaculture dataset in the BI post runs on it
- **[roms-rs](https://github.com/EmilLindfors/roms-rs)** — Discontinuous Galerkin solver for coastal ocean modeling
- **[lindfors-site](https://github.com/EmilLindfors/lindfors-site)** — this site: Zola, the site-tools CLI, the newsletter Worker

