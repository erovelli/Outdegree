# 4. Inline the WASM as base64 to satisfy `connect-src 'none'`

- **Status:** Accepted
- **Date:** 2026-06-28

## Context

The headline privacy guarantee is that the extension makes **no network
requests** — enforced by CSP `connect-src 'none'`. But the default
`wasm-bindgen --target web` glue loads the `.wasm` with `fetch()` (even for the
extension's own packaged file). Under `connect-src 'none'`, `fetch` — including of
same-origin resources — is browser-blocked, so the default path would fail to
instantiate.

## Decision

A small Vite plugin (`vite.config.ts`) **base64-inlines the `.wasm` into the JS
bundle** and the dashboard instantiates it with `WebAssembly.instantiate(bytes)`
instead of fetching. The plugin also neutralizes wasm-bindgen's dead `fetch`
self-loader and disables Vite's modulepreload polyfill (its only other `fetch`),
leaving the built bundle with **zero network surface**.

## Consequences

- The extension instantiates WASM with `connect-src 'none'` intact — the privacy
  guarantee and a working dashboard coexist.
- The CI "network-surface audit" greps `dist/` for `fetch`/`XHR`/`WebSocket`/etc.
  and any external URL, so a regression here fails the build.
- Cost: base64 inflates the payload ~33% and the inlined bundle is large. The
  release workflow runs `wasm-opt -Os` to claw size back; a CI size note keeps it
  visible. Acceptable for a single-page dashboard with no preloading.
