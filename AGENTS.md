# AGENTS.md

Operating manual for AI coding agents (and humans) working in this repo. It is
the single source of truth for how to build, test, and change Outdegree without
breaking the two things that define it: the **green build** and the
**browser-enforced local-only privacy guarantees**.

> If you read nothing else, read **[Non-negotiable invariants](#non-negotiable-invariants)**
> and run the **[verification gate](#verification-gate)** before declaring any change done.

---

## What this is

**Outdegree** is a Chrome MV3 extension that records your web navigations as a
directed graph you can explore over time (session → day → week → month → year).
It is **100% local, makes no network requests, and is open source**. A tiny
append-only TypeScript service worker captures navigation events; all the
analysis (deriving origins, collapsing redirects, rolling up days, community
detection, layout) runs as a **Rust → WebAssembly** core on the dashboard page.

It is a portfolio project — calibrate effort to a polished hobby app, not an
enterprise service. **Never** add telemetry, analytics, crash reporting, or any
network egress (see invariants).

---

## Quickstart commands

Run from the repo root. The crate name is `browsing-graph-core`; the product and
npm package are `outdegree` — they differ for historical reasons, this is expected.

```bash
# One-time toolchain
rustup target add wasm32-unknown-unknown
cargo install wasm-pack            # if not already installed
npm install

# Build the unpacked extension into dist/
./build.sh                         # == npm run build (build:wasm + build:ext)

# Dev server (dev WASM + Vite)
npm run dev
```

### The checks (mirror CI exactly)

```bash
# Rust pure core (native): tests + format + lint
cargo test -p browsing-graph-core
cargo fmt --all -- --check
cargo clippy -p browsing-graph-core --lib --tests -- -D warnings

# Rust WASM shell + wasm-only tests: type-check & lint on the wasm target
cargo clippy -p browsing-graph-core --target wasm32-unknown-unknown --all-targets -- -D warnings

# TypeScript: type-check + unit tests
npm run typecheck
npm run test:ts

# Full extension build (also what the CI privacy audits run against)
npm run build
```

The wasm-bindgen **browser** test suite (`crates/core/tests/browser.rs`) runs in
headless Chrome via `wasm-pack test --headless --chrome crates/core`. It needs a
Chrome + chromedriver, so it usually runs in CI rather than locally.

---

## Architecture map

```
Service Worker (TypeScript, append-only)        Dashboard page (Rust → WASM)
  onCommitted               → "nav"    ┐           derive   read-time global-order pass
  onCreatedNavigationTarget → "link"   │           rollup   UTC-day buckets + sessions
  tabs.onRemoved            → "close"  │  ONE id   project  range merge + display filters
  tabs.onActivated          → "focus"  │  sequence graph    Louvain · hubs · PrefixSpan
  windows.onFocusChanged    → "wfocus" │  (events) layout   Fruchterman–Reingold
  onStartup                 → "start"  ┘           render   canvas2d · svg · flow (Sankey)
  onHistoryStateUpdated     → "nav" (spa store)
  onReferenceFragmentUpdated→ "nav" (spa store)
```

Two layers, two languages:

| Path | Language | Compiles under | Notes |
|---|---|---|---|
| `crates/core/src/{model,interpret,derive,rollup,project,graph,layout,camera3,flow,search,svg,export,views}.rs` | Rust | **native + wasm32** | **Pure.** Runs under `cargo test`. Depends only on `url`, `psl`, `petgraph`, `serde`. This is the load-bearing logic. |
| `crates/core/src/{store,render/*,ui/*,bridge}.rs` | Rust | **wasm32 only** | Gated behind `cfg(target_arch = "wasm32")`. IndexedDB (rexie), canvas2d renderer, the dashboard UI, and the JS bridge. |
| `extension/src/{service-worker,idb,chrome-bridge,dashboard,capture}.ts` | TypeScript | n/a | The capture layer + glue. `capture.ts` holds the pure event-shaping helpers (unit-tested); `idb.ts` is the sole IndexedDB schema owner. |

Load-bearing design commitments (do not violate without strong reason):

- **WASM never runs in the service worker.** MV3 listeners are synchronous; WASM
  init is async. All Rust lives on the dashboard page.
- **The service worker is append-only.** No derived or per-tab state. Everything
  is reconstructed at read time, in strict global `id` order, over one unified
  `events` store. Appends are funnelled through a serialized promise queue so id
  assignment matches firing order even across async `windowId` resolution.
- **The fold is bit-identical incremental vs. from-scratch.** The read-time pass
  carries a complete `DeriveState` checkpoint, so folding `id > watermark` from
  the checkpoint equals a full recompute. This is **property-tested at every
  watermark split** (`crates/core/tests/prop.rs`) — if you touch
  `derive.rs`/`rollup.rs`, keep that invariant.
- **WASM is instantiated from inlined bytes, not `fetch`.** `vite.config.ts`
  base64-inlines the `.wasm` so it instantiates under CSP `connect-src 'none'`.
- **The Public Suffix List is embedded at compile time** (`psl` crate) — never
  fetched at runtime.

---

## Non-negotiable invariants

These are **browser-enforced and CI-audited** (see `.github/workflows/ci.yml`
"Audit emitted manifest" + "Audit built bundle for network surface"). A change
that breaks any of them will fail CI and must never ship.

```
# PRIVACY / SECURITY — do not weaken any of these:
- NO network egress in the shipped extension. Never add fetch / XMLHttpRequest /
  WebSocket / EventSource / navigator.sendBeacon / importScripts / dynamic import()
  of a remote URL. The only data-out path is a user-initiated local file download.
- NO telemetry, analytics, crash/error reporting, or "phone home" of any kind.
- permissions ⊆ { webNavigation, storage, unlimitedStorage, favicon }. host_permissions: [].
- CSP must keep connect-src 'none' (and no 'unsafe-inline').
- incognito: "not_allowed".
- NO content_scripts, NO web_accessible_resources, NO remotely-hosted code.
- The Public Suffix List stays embedded at compile time (no runtime fetch).
- The single OKLCH provenance hue is the only data color (design constraint).
  Favicons (F12) are identity marks, not a data-color channel — the provenance
  ring/shape around each icon stays the data encoding (ADR-0006).
```

The `favicon` permission (added in F12, [ADR-0006](docs/adr/0006-favicon-permission.md))
is the **only** permission beyond the original three, and the only invariant this
release cycle deliberately widened. It unlocks Chrome's **local** favicon service at
the extension's own origin (`chrome-extension://<id>/_favicon/`) to label sites with
their icons. It makes **no network request** — Chrome serves the icon from its
on-disk cache — so the no-egress guarantee, `connect-src 'none'`, and
`host_permissions: []` all still hold, and the CI network-surface audit stays at
zero. Two subtleties to preserve if you touch this:

- **The scheme must not leak into `dist/`.** `pageUrl` needs an `http(s)` scheme,
  but the network-surface audit greps `dist/` for `https?://`. All favicon URL
  building lives in the Rust→WASM core (`crate::favicon`), which is base64-inlined
  (so string constants are scrambled), it **reuses the F4 `URL_SCHEME` const**
  rather than adding a second scheme literal, **and** it percent-encodes the value
  (`https%3A%2F%2F…`, no literal `://`). Keep it that way — do not put a favicon URL
  in TypeScript, and do not weaken the audit.
- **`favicon` is Chromium-only.** `scripts/build-firefox.mjs` strips it from the
  Firefox overlay, so the two CI manifest audits differ on purpose: the `web` job's
  allowlist is **four** permissions (incl. `favicon`); the `firefox` job's is
  **three**. The dashboard runtime-guards the feature on the permission being
  declared, so it no-ops on Firefox. Keep both allowlists in step with
  `extension/manifest.config.ts` + the overlay.

If a task seems to require any of the above, **stop and surface it** rather than
implementing it. CI-side tooling (coverage, advisory scans, badges) is fine —
those never touch the shipped extension.

---

## Conventions

- **Rust:** stable toolchain (`rust-toolchain.toml`), `rustfmt` defaults
  (edition 2021), clippy clean under `-D warnings` on **both** native and wasm32.
  The pure core forbids `unsafe` (`#![cfg_attr(not(target_arch = "wasm32"), forbid(unsafe_code))]`);
  the wasm shell can't (wasm-bindgen's generated glue is `unsafe`). Public items
  in the pure core carry rustdoc; match that bar.
- **TypeScript:** strict mode (`tsconfig.json`). Keep pure logic in `capture.ts`
  so it stays unit-testable; the service worker registers `chrome.*` listeners at
  module top level, so it cannot be imported into a test.
- **Comments** reference the design spec by section (e.g. `§7.3`); preserve that
  style when editing nearby code. Explain *why*, not *what*.
- **IndexedDB schema** is owned by `extension/src/idb.ts` (the SW creates the
  stores); `crates/core/src/store.rs` mirrors it as a self-sufficient guard. Keep
  the two in sync, and bump `DB_VERSION` with a migration if you change stores.
- **Versioning:** `package.json` is the single source of truth. The manifest
  imports it; the Cargo workspace pins its own. Bump it + `CHANGELOG.md` before a
  release tag.

---

## Gotchas

- `dist/` and `extension/src/wasm/` are **generated** (git-ignored). Don't edit
  them or commit them; run `./build.sh`.
- `wasm-opt` is **disabled by default** (so the build works offline); the release
  workflow enables it. Don't expect a size-optimized local `.wasm`.
- `proptest` is **native-only** (it does not target wasm32) — see the
  `cfg(not(target_arch = "wasm32"))` dev-dependency gate.
- `cargo test` runs the **pure** suite (native). The wasm-only tests
  (`coverage.rs`, `browser.rs`) need `wasm-pack test`.
- Editing `extension/manifest.config.ts` changes the **emitted** `dist/manifest.json`
  — re-run `npm run build` and confirm the CI manifest-privacy audit still passes.
- The dashboard reads the pause flag as the **string** `"true"`/`"false"` from
  `chrome.storage.local` — a plain `!!value` treats `"false"` as truthy. Use the
  `flagOn` helper (`capture.ts`).

---

## Verification gate

Before declaring any code change done, run and pass:

```bash
cargo fmt --all -- --check
cargo test -p browsing-graph-core
cargo clippy -p browsing-graph-core --lib --tests -- -D warnings
cargo clippy -p browsing-graph-core --target wasm32-unknown-unknown --all-targets -- -D warnings
npm run typecheck
npm run test:ts
npm run build      # then confirm dist/manifest.json + bundle still pass the privacy audits
```

For capture-layer changes that can't be unit-tested, also walk the relevant rows
of `docs/M0-CHECKLIST.md` in a real Chrome.

---

## Where things live

- Build/CI: `build.sh`, `vite.config.ts`, `package.json`, `.github/workflows/`.
- Docs: `README.md`, `docs/` (privacy policy, store listing, porting, screenshots,
  M0 checklist), `docs/adr/` (architecture decision records).
- Contributor process: `CONTRIBUTING.md`. Security model: `SECURITY.md`.
