# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Bump the version in `package.json` (the single source of truth) and add an entry
here before tagging a release; the `version v*` tag drives `release.yml`.

## [Unreleased]

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

[Unreleased]: https://github.com/erovelli/Outdegree/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/erovelli/Outdegree/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/erovelli/Outdegree/releases/tag/v1.0.0
