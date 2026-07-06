# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Bump the version in `package.json` (the single source of truth) and add an entry
here before tagging a release; the `version v*` tag drives `release.yml`.

## [Unreleased]

### Added
- **Toolbar affordances: focus, don't duplicate; and a visible paused badge.**
  Clicking the toolbar icon now focuses an already-open dashboard tab (activating
  it and raising its window, even across windows) instead of piling up a new tab
  on every click; it opens a fresh tab only when none is open. The open tab is
  located via `runtime.getContexts` (MV3, Chrome 116+), never a URL-matching
  `tabs.query`, so no new permission is needed — it degrades to opening a new tab
  on older Chrome. While capture is paused, the icon shows a neutral-gray "⏸"
  badge and a "capture paused" tooltip, so capture-off is visible without opening
  the dashboard; the badge is applied on worker start (covering browser restart)
  and on every change to the pause flag, and cleared when capture resumes. Stays
  100% local: no new permission, no manifest change, no network.
- **Persistent dashboard preferences.** The dashboard now reopens the way you
  left it: the active view, time range, node granularity, min-visits threshold,
  hide-search-hubs / hide-singletons toggles, in-app-navigations mode, and the
  layout lock are written through to `chrome.storage.local` (a single `uiPrefs`
  JSON document) on every change and restored before the first render, so there's
  no flash of default state. Applying a saved view updates these too. Transient
  state (drill-down focus, camera, hover, the legend filter, the selected
  session/day, and the raw-events view) is intentionally not persisted. Restore is
  fully lenient — a missing, corrupt, or forward-dated value silently falls back to
  the defaults. Stays 100% local: no new permission, no network.

### Changed
- Renamed the view-switcher's **Sankey** segment to **Sessions** — the button
  opens the session picker, and the Sankey flow diagram lives inside it, so the
  label now names what the button does.

## [1.1.0] — 2026-07-02

### Added
- **Activity heatmap in the Sankey session picker.** A GitHub-style contribution
  grid (a full rolling year, 53 weeks × 7 days) now sits atop the session list, so
  a day can be picked directly instead of scrolling months of sessions. Days are
  shaded by that day's visit total, binned into quartiles of the busiest day, using
  the single provenance hue at rising opacity (empty days stay achromatic). Picking
  a day scopes the list to it and auto-selects that day's most recent session. Days
  bucket on a DST-safe local calendar-day key; all shading is CSS-class-driven to
  preserve the strict `connect-src 'none'` CSP.

### Changed
- Decluttered the session list: a single date header ("Today · Wed, Jul 1 · N
  sessions") now anchors the scoped day, and each session card shows only its time
  range instead of repeating the date. The heatmap is pinned at its natural height
  so a session-heavy day can no longer clip its bottom rows or intensity key.

## [1.0.3] — 2026-06-29

### Fixed
- The `--all-features` added in 1.0.2 made `wasm-opt` emit a newer GC heap type
  that older Chrome / Node reject (`Unknown heap type -14`) — so the optimized
  bundle could fail to instantiate on anything but the very latest engines, and
  the 1.0.2 release build failed its own smoke test. Reverted to a plain
  `wasm-opt -Os`: with the pinned binaryen, binaryen reads wasm-bindgen's
  `target_features` and enables exactly the needed features (preserving the
  externref table) without enabling GC. Verified launchable on Node 20, Node 22,
  and a real Chromium. (1.0.2 has no published artifact; use 1.0.3.)

## [1.0.2] — 2026-06-29

### Fixed
- Release builds still crashed at startup with
  `WebAssembly.Table.grow(): failed to grow table` — the binaryen that
  `apt-get install binaryen` puts on the CI runner is old enough to mangle
  wasm-bindgen's externref table even with `--all-features`. The release workflow
  now installs a **pinned** binaryen (`version_119`), which optimizes the table
  correctly. (1.0.1 shipped the same crash; 1.0.0/1.0.1 GitHub Release zips are
  not launchable — use 1.0.2.)

### Added
- A release **smoke test** (`scripts/smoke-extension.mjs`): instantiates the
  optimized WASM through the real wasm-bindgen glue (running the externref-table
  init) and fails the release if it traps — so a non-launchable bundle can never
  be published again.

## [1.0.1] — 2026-06-29

### Fixed
- Dashboard failed to start on Web Store / release builds with
  `RangeError: WebAssembly.Table.grow(): failed to grow table` — the release
  `wasm-opt -Os` step mangled wasm-bindgen's externref table. It now runs
  `wasm-opt -Os --all-features`, which preserves the reference-types table.
  (Local `./build.sh` builds were never affected, since they skip `wasm-opt`.)

## [1.0.0] — 2026-06-29

First public release — published to the Chrome Web Store.

### Capture
- Append-only TypeScript service worker recording `webNavigation` events
  (committed navigations, new-tab link origins, history-state/hash SPA
  navigations, tab closes, startup) into one globally-ordered IndexedDB stream.

### Analysis core (Rust → WASM)
- Read-time derivation (origins, redirect collapse, lifecycle), UTC-day rollups
  with a bit-identical incremental fold, range/granularity projection, graph build
  with Louvain community detection, hubs, frequent-sequence mining, and a
  Fruchterman–Reingold layout with warm-start.

### Dashboard
- Interactive graph view, analytics tables, per-session Sankey flow, a raw-event
  view, time-range and display filters, saved view presets, and PNG/SVG/CSV
  export. A user-facing error state replaces a silent "Loading…" hang when local
  storage or WASM init is unavailable.

### Privacy controls
- Pause capture, forget a domain, delete a recent time range, and export/import
  your data as a local JSON file.

### Privacy by construction
- `host_permissions: []`, CSP `connect-src 'none'`, `incognito: "not_allowed"`,
  no content scripts, no remote code — all browser-enforced and gated in CI by the
  manifest-privacy and network-surface audits.

### Engineering & tooling
- Contributor & AI-agent scaffolding (`AGENTS.md`, `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md`, `SUPPORT.md`, issue/PR templates,
  `CODEOWNERS`, `.editorconfig`, `.nvmrc`), Architecture Decision Records,
  property-tested pure core plus Vitest tests for the capture layer and IndexedDB
  schema, a single-sourced version, least-privilege CI `permissions`, a
  `cargo-deny` advisory gate, and release build-provenance with a reproducible
  artifact. The unused, never-compiled `webgpu` feature was removed (canvas2d
  remains the renderer).

[Unreleased]: https://github.com/erovelli/Outdegree/compare/v1.0.3...HEAD
[1.0.3]: https://github.com/erovelli/Outdegree/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/erovelli/Outdegree/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/erovelli/Outdegree/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/erovelli/Outdegree/releases/tag/v1.0.0
