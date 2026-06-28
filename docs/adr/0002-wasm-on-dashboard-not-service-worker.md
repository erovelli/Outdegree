# 2. Run the analysis core as Rust→WASM on the dashboard, not in the service worker

- **Status:** Accepted
- **Date:** 2026-06-28

## Context

The capture layer must observe `webNavigation` events; the analysis layer
(deriving origins, collapsing redirects, rolling up days, community detection,
layout) is heavy and benefits from a typed, testable language. A natural
temptation is to do everything in the MV3 service worker.

Two forces conflict:

1. **MV3 service-worker event listeners are synchronous and the worker is killed
   aggressively** when idle. Listeners must register at top level and finish
   quickly.
2. **WebAssembly instantiation is asynchronous** and the analysis works on the
   *whole* history, which is far more work than an event handler should do.

## Decision

Split the system in two:

- The **service worker is a thin, append-only TypeScript capture layer** — it
  registers synchronous listeners and appends raw events. No WASM, no derived
  state.
- **All Rust → WASM analysis runs on the dashboard page**, where async
  instantiation and longer-running computation are fine, and where the results
  are actually displayed.

## Consequences

- Capture stays within the MV3 lifecycle constraints and can't be wedged by a
  slow async WASM init.
- The analysis core is a pure Rust crate that compiles and runs under
  `cargo test` on the host — fast, deterministic, no browser needed.
- The two layers communicate only through the IndexedDB event stream, keeping a
  clean, inspectable contract (see [ADR 0003](0003-append-only-event-store.md)).
- Cost: the dashboard re-folds on open. We mitigate this with a checkpointed
  incremental fold so only new events are processed.
