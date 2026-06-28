# 5. Embed the Public Suffix List at compile time

- **Status:** Accepted
- **Date:** 2026-06-28

## Context

Grouping hosts by registrable domain (eTLD+1) — so `gist.github.com` and
`github.com` collapse correctly, but `foo.github.io` and `bar.github.io` stay
distinct — requires the [Public Suffix List](https://publicsuffix.org/). Many
libraries fetch or periodically update the PSL over the network.

## Decision

Use the `psl` crate, which **embeds the Public Suffix List at compile time**. The
eTLD+1 logic (`interpret::registrable`) is pure and needs no runtime data.

## Consequences

- No runtime network request for domain grouping — consistent with
  `connect-src 'none'` and the no-network guarantee.
- Domain grouping is deterministic and testable on the host under `cargo test`.
- Cost: the embedded list is only as fresh as the pinned `psl` version. Dependabot
  proposes updates; a stale entry would at worst mis-group a brand-new TLD, which
  is cosmetic. Accepted trade-off.
