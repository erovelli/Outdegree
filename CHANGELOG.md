# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Bump the version in `package.json` (the single source of truth) and add an entry
here before tagging a release; the `version v*` tag drives `release.yml`.

## [Unreleased]

### Added
- **Foreground attention: focused time per site, not just navigation gaps.** The
  service worker now records two new (id-only) event kinds in the same global
  stream: `focus` from `tabs.onActivated` (which tab is on-screen in a window)
  and `wfocus` from `windows.onFocusChanged` (which window is focused; `-1` when
  the whole browser is blurred). Both are pause-gated and serialized exactly like
  nav/link/close, carry no URL or title, and need **no new permission** — the set
  stays exactly `webNavigation`/`storage`/`unlimitedStorage`, still 100% local.
  The derive pass attributes each interval between consecutive events (capped at
  the 30-minute idle gap) to the page loaded in the focused window's active tab —
  and to nothing at all while the browser is blurred, the active tab is unknown
  or closed, or that tab has no http(s) page. The result is a per-site
  foreground-dwell total alongside the existing gap-based estimate (which is
  unchanged), plus a per-day "has focus signal" marker. Tables' "Time spent" now
  shows real foreground time when the displayed window has focus data, and the
  gap estimate prefixed **≈** (with an explanatory tooltip) when it doesn't — so
  pre-upgrade history stays honest. The CSV export carries both columns; the Raw
  view header now breaks the shown events down per kind (surfacing the extra
  volume; compaction remains the ADR-0003 follow-up). On first open after the
  upgrade an existing install automatically rebuilds its derived cache from the
  raw events (a new `derivedSchemaVersion` meta key), so foreground time appears
  with no user action; the fold==recompute property test now interleaves focus
  events at every watermark split, and the interpreter skips unrecognized future
  event kinds instead of erroring.
- **Time navigation: step backward/forward through your history.** The range
  control (Session / Day / Week / Month / Year) is now flanked by **‹ / ›** step
  buttons and a window label showing the actual bounds of what's on screen
  ("Wed, Jul 1" for Day, "Jun 23 – 29" for Week, "Jun 30 – Jul 6" across months,
  years added when a span crosses a calendar year). **‹** steps the displayed
  window back exactly one range duration; **›** steps forward and snaps back to
  the live view when the window reaches the latest data. On the Session range,
  ‹/› walk your recorded sessions one at a time (the label shows that session's
  time range). Stepping is clamped before your earliest recorded day (‹ disables
  there; › disables at latest), but empty windows inside your history remain
  traversable — the empty state plus the window label make gaps navigable.
  While viewing a past window a **Latest ↩** chip appears; clicking it (or
  pressing › at the second-newest window) returns to the live view. Keyboard:
  **ArrowLeft / ArrowRight** step the window whenever focus isn't in a text
  field and no modal/welcome overlay is open. Everything downstream follows the
  displayed window: the graph, Tables (KPI cards and their vs-previous-period
  delta chips, the daily-visits sparkline, "Surging this period", Activity,
  journeys), and the CSV/PNG/SVG exports. Anchored historical views hold still —
  the background live-refresh keeps folding new events but never re-renders a
  past window out from under you; returning to **Latest** shows everything
  captured meanwhile. The anchor is deliberately transient: it is never
  persisted (not in uiPrefs, not in saved views), so the dashboard always
  reopens live.

- **First-run onboarding: a welcome overlay and an inlined sample dataset.** On a
  fresh, empty install the dashboard now greets you with a centered glass welcome
  overlay (in the app's monochrome idiom, the single provenance hue as the only
  accent) explaining what Outdegree does, its 100%-local privacy stance, and how
  to start — with **Start recording** (dismisses and remembers you're onboarded;
  Esc does the same) and **Load sample data**. "Load sample data" imports a
  committed, deterministic ~3-week synthetic browsing fixture (generic
  news/docs/dev/mail/video host clusters with a realistic provenance mix) so the
  Graph, Sessions, and Tables views are populated to explore immediately — its
  offset timestamps are shifted to "now" on load so it always looks recent.
  While the sample is loaded, capture is forced paused (so your real browsing
  never interleaves) and a **Sample data · Exit sample** chip sits by the REC
  indicator; **Exit sample** wipes the demo and returns to a clean, empty
  dashboard with the welcome overlay. The overlay is re-openable anytime from the
  settings menu's **Show welcome**, and the backup nudge is suppressed while the
  demo is loaded. Stays 100% local and audit-clean: the fixture is inlined into
  the bundle at build time (no fetch), and its URLs are stored schemeless — the
  http(s) scheme is re-attached in the WASM core at load time — so the built
  bundle carries zero network surface.
- **Data stewardship: storage readout, delete-all, import confirmation, and a
  backup nudge.** The settings menu's Data section now opens with a read-only
  **Storage** line — event / rollup / session record counts plus an approximate
  size from `navigator.storage.estimate()` (a local StorageManager API; when it's
  unavailable it shows counts only), refreshed each time the menu opens. A new
  **Delete all data…** item wipes every IndexedDB store (events, spa, rollups,
  sessions, and all meta) behind a type-**DELETE**-to-confirm gate, landing on a
  working empty dashboard that captures anew; your preferences (pause, view
  settings, saved views) are kept. **Import JSON** now asks first — a modal warns
  that importing replaces your current data (with the current event count) and
  suggests exporting first, so Cancel truly aborts with nothing touched. And a
  small, dismissible **backup nudge** appears near the settings gear once the log
  is large (> 5,000 events) and there's been no export in 60 days: "Export now"
  runs the local JSON export and clears it; "Snooze" hides it for 30 days. The
  export path stamps a `lastExportTs` in the meta store so the nudge resets. The
  nudge decision is a pure, unit-tested function and never blocks interaction or
  appears on the empty/no-data state. Stays 100% local: no new permission, no
  network — the only data-out path remains a user-initiated file download.
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

### Internal
- Split the dashboard's monolithic `ui/app.rs` into focused modules (`ui/chrome`,
  `ui/settings`, `ui/modal`, `ui/saved_views`, `ui/shortcuts`, `ui/onboarding`),
  leaving `ui/app.rs` as the thin composition root. Pure code move — no behavior
  change.

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
