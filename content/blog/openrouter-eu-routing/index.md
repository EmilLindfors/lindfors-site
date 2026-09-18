+++
title = "OpenRouter's EU routing makes the dbt conversation easier"
description = "Helping with a dbt project without sending customer rows to an unapproved endpoint. Business EU routing offers another route, with retention controls and processor terms still part of the setup."
date = 2026-09-09
draft = true

[taxonomies]
tags = ["llm", "dbt", "governance", "gdpr", "consulting", "innovation"]
categories = ["business"]
series = ["Coding agents and what they cost"]

[extra]
toc = true
skip_audio = true
featured_image = "hero.webp"
+++

A dbt project is a good place to put a coding agent. There is SQL to review, transformation logic to explain, and tests to write. It is also a good place to discover that helping with the code and seeing the customer's data are difficult to separate.

<!-- emil: verbatim phrase from the interview -->
We have been discussing this at work. The starting point was: "we are using claude but dont want it to see the data".

So we started exploring how to use a model served in Europe. When I previously asked OpenRouter about EU routing, it was an Enterprise feature, and the minimum spend I was quoted was $25,000 per month. That was a steep commitment for an exploration. Azure was another route, but the setup was substantial for what we wanted to try.

OpenRouter now documents EU routing on its Business plan as well as Enterprise. An organization administrator can upgrade through the account settings. That gives us another option to evaluate for the dbt workflow. [OpenRouter's routing documentation](https://openrouter.ai/docs/guides/features/in-region-routing)

<!-- more -->

## Where code becomes data

Suppose a uniqueness test fails on a customer table. An agent can read the SQL and suggest that a join duplicates customers. To establish whether that is what happened, it needs evidence. You might inspect duplicate keys, look at the matching rows, or ask the warehouse for a small summary.

dbt supports storing failing test records for inspection. If an agent queries those records and includes the result in its conversation, their contents become input to the model provider. Keeping the warehouse in Europe does not determine where that next request goes. [dbt's failure-storage documentation](https://docs.getdbt.com/reference/resource-configs/store_failures)

You can do useful work without exposing live rows. Give the agent synthetic fixtures, let it propose a query, and run the query yourself. Sometimes that is all you need. But the person in the middle still has to inspect the result and decide what can go back into the conversation. That limits how much of the debugging loop you can delegate.

I wrote about this in [Exploring LLMs so our customers don't have to](/blog/which-llm-are-we-allowed-to-use/): the endpoint needs to be part of the customer's approval. The dbt discussions give that argument a very practical setting. The same issue appears when an assistant reads a support ticket or receives rows from an analytical tool.

## The Business route

For a client that supports a configurable API base, the EU address is:

```text
https://eu.openrouter.ai/api/v1
```

OpenRouter says requests are decrypted in-region and routed to eligible EU provider endpoints. If the requested model has no eligible endpoint, the request fails. The regional catalogue is a subset of what OpenRouter offers globally, so check availability for the model you want. Bringing your own cloud credentials also requires checking where your own deployment runs. [In-region routing](https://openrouter.ai/docs/guides/features/in-region-routing)

This is the part I really like: we can evaluate a regional route through an interface our tooling already supports. Access to frontier models becomes a question of which eligible endpoint meets the task and the customer's requirements. We can try another model without building another integration for it.

That was the point of [making our coding agent's endpoint configurable](/blog/forking-codex-for-any-endpoint/). The work pays off when a new option becomes available. It still requires a client that supports the chosen endpoint and protocol; using Claude today does not mean every Claude-based application can be redirected this way.

<!-- emil: verbatim phrase from the interview -->
My complaint about the alternative was that "running though azure models is a lot of setup". For a customer already organized around Azure, that work may be appropriate. For our exploration, having a Business route through OpenRouter is welcome.

## Deciding what it may see

I would split the dbt workflow into three levels:

| Task | What reaches the model | What to establish |
|---|---|---|
| Review transformations and write tests | SQL, approved documentation, synthetic examples | Whether the repository itself contains confidential information or real records |
| Diagnose a data-quality failure | A limited result or selected example rows | Which fields and records the approved endpoint may receive |
| Work with sensitive personal records | Information covered by additional contractual or legal restrictions | Whether the agreement and the processing purpose permit that specific use |

An approved EU endpoint could let us move more of the second task into the agent's loop. You could ask it to inspect an approved sample, explain a join problem, and propose a regression test. The same pattern could support document extraction or a support assistant, once their inputs are approved too.

Keep the amount of data tied to the question. Diagnosing duplicate keys usually does not need addresses and free-text notes. An aggregate also needs thought if it describes a group small enough to identify someone. These are application design decisions that a regional URL cannot make for you.

## Retention and the agreement

Region and retention are separate settings. OpenRouter lets you require endpoints with a zero-data-retention policy using this provider preference:

```json
{
  "provider": {
    "zdr": true,
    "data_collection": "deny"
  }
}
```

ZDR filters inference endpoints by their retention policy. OpenRouter allows in-memory prompt caching under that policy, and external tools have their own retention terms. The data-collection control restricts collection, including training. Apply the policy centrally where possible, so each developer does not have to remember it on every request. [ZDR documentation](https://openrouter.ai/docs/guides/features/zdr), [data-collection controls](https://openrouter.ai/docs/guides/features/sovereign-ai)

There is a contractual step for sensitive data. OpenRouter's August 26 DPA requires express agreement for its broadly defined Sensitive Data, including employment, financial and health information. Its EU-only addendum requires an election recorded in an order form or other written agreement. The addendum covers API payloads; operational metadata is excluded, and the commitment has legal exceptions. Check what is effective for your account. [OpenRouter DPA, §2.6 and Exhibit A](https://openrouter.ai/data-processing-agreement)

GDPR also requires a lawful basis for processing personal data. Special-category data, such as health information, needs an additional Article 9 condition. A data protection impact assessment is required where the processing is likely to pose high risk. EU routing can support that design, but it supplies none of those decisions. [EDPB lawful-processing guidance](https://www.edpb.europa.eu/sme/be-compliant/process-personal-data-lawfully_en), [impact assessments](https://www.edpb.europa.eu/sme/be-compliant/be-compliant_en)

## What a customer can approve

<!-- emil: verbatim phrase from the interview -->
The reason this matters now is that "companies are increasinly securing their llm usage". We are having conversations about where these tools fit, and customers may require certification or other evidence before approving them.

For those conversations, I want a setup we can describe and demonstrate: which data the agent can read, where inference happens, what gets retained, and who operates each part. If a customer asks for certification, establish which certification and whose service it must cover. A supplier's certificate does not describe our entire application.

OpenRouter offers a workspace guardrail that restricts requests to the EU domain, using `allowed_data_regions: ["europe"]`. A request through a disallowed domain is rejected. That is a useful control to include in the setup and test with synthetic data. [Regional guardrails](https://openrouter.ai/docs/guides/features/in-region-routing)

The next experiment I would run is small: one dbt project, a synthetic failing test, and an eligible model behind the EU endpoint with the intended retention policy. Check whether it can diagnose the failure and write a useful test. Then review what entered the conversation, including tool results and application logs, before deciding which real data it may see.

That is a much more approachable experiment than the Enterprise commitment I was quoted. We already have a concrete task to try it on.
