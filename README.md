# Outdegree — Browsing Graph

**100% local · no network · open source.**

A Chrome MV3 extension that records your web navigations as a directed graph you
can explore over time (session → day → week → month → year). The capture layer is
a tiny append-only TypeScript service worker; all the interesting work —
deriving origins, collapsing redirects, rolling up days, detecting communities,
laying out the graph — runs as a **Rust → WebAssembly** core on the dashboard
page.

> *Outdegree* is the repository / codename; the extension's Web Store display
> title is **Browsing Graph**. (They need not match — §12.5 of the spec.)

---

## Why it's private by construction

The privacy story is **browser-enforced**, not just promised:

| Guarantee | Enforced by |
|---|---|
| No network egress | `host_permissions: []` **and** CSP `connect-src 'none'` — `fetch`/`XHR`/`WebSocket`/`sendBeacon` are blocked by the browser |
| Never records incognito | `"incognito": "not_allowed"` in the manifest |
| No remote code | no content scripts, no `<all_urls>`, `psl` embeds the Public Suffix List at compile time, UI runs in CSR (no hydration callback) |
| Only data-out path | a **user-initiated local file download** (export) — a `Blob` to disk, never an upload |

See [`docs/privacy-policy.md`](docs/privacy-policy.md).

---

## Architecture

```
Service Worker (TypeScript, append-only)        Dashboard page (Rust → WASM)
  onCommitted  → append "nav"   ┐                  derive   global-order read-time pass
  onCreatedNavigationTarget → "link"  │ ONE id      rollup   UTC-day buckets + sessions
  tabs.onRemoved → "close"           │ sequence     project  range merge + filters
  onStartup    → "start"      ┘  (store: events)    graph    Louvain · hubs · PrefixSpan
  onHistoryStateUpdated → "spa" (separate store)    layout   Fruchterman–Reingold
                                                     render   canvas2d   ui   store(rexie)
```

Key commitments:

- **WASM never runs in the service worker** (MV3 sync listeners vs async WASM).
  All Rust lives on the dashboard page.
- **The service worker is append-only** — no derived or per-tab state. Everything
  is reconstructed at read time, in **strict global id order**, over one unified
  `events` store.
- The read-time pass carries a **complete `DeriveState` checkpoint**, so an
  incremental fold over `id > watermark` is **bit-identical** to a from-scratch
  recompute (verified at every watermark split — see the tests).

The pure core (`interpret` / `derive` / `rollup` / `project` / `graph` / `layout`)
depends only on `url`, `psl`, and `petgraph`, and runs under `cargo test`. Only
`store` / `render` / `ui` / `bridge` are WASM-only (gated behind
`cfg(target_arch = "wasm32")`).

---

## Repository layout

```
crates/core/src/
  model.rs        Event stream, provenance/edge-kind taxonomy, aggregates
  interpret.rs    transition classification, host(), ICANN eTLD+1, node_key()
  derive.rs       global-order read-time pass (origins, redirects, lifecycle)
  rollup.rs       DeriveState checkpoint, fold, UTC-day buckets, sessions
  project.rs      bucket merge, granularity regroup, display filters
  graph.rs        petgraph build, Louvain, hubs, top_edges, frequent_sequences
  layout.rs       Fruchterman–Reingold with warm-start
  store.rs        rexie reads/writes, privacy deletes, export/import   [wasm]
  render/canvas2d.rs  graph renderer                                   [wasm]
  ui/             shell, filters, graph view, tables, sankey, picker   [wasm]
  bridge.rs       externs → chromeBridge; mount() entry                [wasm]
extension/src/
  service-worker.ts  append-only capture
  idb.ts             sole IndexedDB schema owner
  chrome-bridge.ts   storage + local download surface
  dashboard.ts       readiness ping → init WASM → mount
extension/{dashboard.html, manifest.config.ts, icons/}
vite.config.ts · package.json · build.sh · docs/privacy-policy.md
```

---

## Build & install

Prerequisites: a recent **Rust** toolchain with the `wasm32-unknown-unknown`
target, **wasm-pack**, and **Node 18+**.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-pack          # if not installed
npm install
./build.sh                       # builds WASM + extension into dist/
```

Then load it: open `chrome://extensions`, enable **Developer mode**, click
**Load unpacked**, and select the `dist/` directory. Click the toolbar icon to
open the dashboard.

Scripts:

| Command | What it does |
|---|---|
| `npm run build` | `build:wasm` + `build:ext` → `dist/` |
| `npm run dev` | dev WASM build + Vite dev server |
| `npm run typecheck` | `tsc --noEmit` over the TypeScript layer |
| `npm run test:core` / `cargo test` | run the pure-core test suite |

---

## Testing

The load-bearing logic is pure Rust and fully covered by `cargo test`:

- **interpret** — every `transitionType`; `erovelli.github.io → github.io`,
  `gist.github.com → github.com`, IP/`localhost` fallback, non-http(s) dropped.
- **derive** — the §1 interleaving trace (new-tab origin = source's *then-current*
  page), two-tab interleaving, client-redirect bursts, `forward_back` search-prov,
  rootless chain reset, and lifecycle markers (`Start` clears state → no phantom
  edge; `Close` flushes its buffer).
- **rollup** — `fold == from-scratch recompute` verified at **every** watermark
  split (covering redirect/session boundaries straddling the watermark, backward
  clock jumps, and destructive-edit rebuilds); UTC-day bucketing.
- **project / graph / layout** — bucket-merge sums, eTLD+1 regroup, self-loop
  drop, Louvain on two cliques, PrefixSpan support, deterministic warm-start.

```bash
cargo test          # ~30 tests, all pure, no browser required
```

---

## Privacy controls

- **Pause** capture at any time (toolbar).
- **Forget a domain** or **delete a recent time range** (raw records removed,
  rollups rebuilt).
- **Export** your data to a local JSON file; **import** it back. Uninstalling the
  extension removes all stored data.

---

## License

MIT © Evan Rovelli. See [`LICENSE`](LICENSE).
