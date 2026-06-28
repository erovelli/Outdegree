# 3. Append-only event store with a read-time fold

- **Status:** Accepted
- **Date:** 2026-06-28

## Context

The capture layer fires from many independent listeners (committed navigations,
new-tab link origins, SPA navigations, tab closes, startup), sometimes needing an
async lookup (resolving `windowId`). If the worker also maintained derived,
per-tab state, a kill/restart mid-sequence could corrupt it, and concurrent
handlers could write in the wrong order.

## Decision

The service worker writes a **single, globally-ordered, append-only `events`
stream** (one IndexedDB auto-increment `id` sequence) and keeps **no derived or
per-tab state**. All appends are funnelled through one serialized promise queue,
so id assignment matches firing order even across async `windowId` resolution.

Everything derived (origins, redirect collapse, sessions, rollups) is
reconstructed **at read time**, in strict global `id` order, by a pure fold over
the stream. The fold carries a complete `DeriveState` checkpoint, so processing
`id > watermark` from the checkpoint is **bit-identical** to a from-scratch
recompute.

## Consequences

- The capture layer is trivially correct under the MV3 kill/restart lifecycle —
  its only state is the queue tail (a single promise), reset harmlessly on
  restart.
- Recovery is free: "Rebuild from raw events" re-derives everything; a corrupted
  rollup cache is never load-bearing.
- The bit-identical property is **enforced by a property test** at every watermark
  split (`crates/core/tests/prop.rs`). Changes to `derive.rs`/`rollup.rs` must
  keep it green.
- Cost: an append-only store grows over time. `unlimitedStorage` covers months to
  a year; the user can forget a domain or delete a time range, and a future
  compaction pass could prune. This is an accepted known limitation, not a bug.
