# 1. Record architecture decisions

- **Status:** Accepted
- **Date:** 2026-06-28

## Context

Outdegree makes several non-obvious, load-bearing architectural choices that a
new contributor (or AI agent) could accidentally undo — e.g. *why is the analysis
core Rust→WASM on the dashboard instead of in the service worker?* The README
asserts these decisions but doesn't record the alternatives or the reasoning, so
the "why" is easy to lose.

## Decision

Keep a lightweight log of Architecture Decision Records (ADRs) in `docs/adr/`,
one Markdown file per decision, numbered sequentially. Each ADR states the
context, the decision, and its consequences. Format inspired by Michael Nygard's
[original ADR post](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions).

We record only decisions that are (a) hard to reverse or (b) easy to break
without realizing it. Superseded ADRs are kept and marked, not deleted.

## Consequences

- The rationale behind the invariants lives next to the code, so reviewers and
  agents can avoid breaking them and newcomers ramp faster.
- A small ongoing cost: significant architectural changes should add or supersede
  an ADR as part of the PR.

## Index

- [0002 — Run the analysis core as Rust→WASM on the dashboard, not in the service worker](0002-wasm-on-dashboard-not-service-worker.md)
- [0003 — Append-only event store with a read-time fold](0003-append-only-event-store.md)
- [0004 — Inline the WASM as base64 to satisfy `connect-src 'none'`](0004-inline-wasm-bytes.md)
- [0005 — Embed the Public Suffix List at compile time](0005-embed-public-suffix-list.md)
