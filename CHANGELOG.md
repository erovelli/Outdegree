# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Bump the version in `package.json` (the single source of truth) and add an entry
here before tagging a release; the `version v*` tag drives `release.yml`.

## [Unreleased]

### Added
- Contributor & AI-agent scaffolding: `AGENTS.md`, `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md`, `SUPPORT.md`, `CHANGELOG.md`, issue/PR
  templates, `CODEOWNERS`, `.editorconfig`, `.nvmrc`.
- Architecture Decision Records under `docs/adr/`.
- TypeScript unit tests (Vitest) for the capture layer and IndexedDB schema.
- A user-facing error state so the dashboard reports a problem instead of hanging
  on "Loading…" when WASM init or local storage is unavailable.
- CI: least-privilege `permissions`, and a `cargo-deny` advisory gate
  (`deny.toml`).

### Changed
- Single-sourced the version: the manifest derives it from `package.json`; the
  Cargo workspace pins its own.
- Documentation accuracy: corrected the test-count claim, completed the module
  map and capture diagram, and added a non-developer install path.

### Removed
- The unused, never-compiled `webgpu` feature (and its `wgpu`/`bytemuck`
  dependencies). canvas2d remains the renderer; the GPU path can return behind a
  feature when a measured need arises.

## [0.1.0] — 2026-06-20

Initial release.

### Added
- **Capture:** append-only TypeScript service worker recording `webNavigation`
  events (committed navigations, new-tab link origins, history-state/hash SPA
  navigations, tab closes, startup) into one globally-ordered IndexedDB stream.
- **Analysis core (Rust → WASM):** read-time derivation (origins, redirect
  collapse, lifecycle), UTC-day rollups with a bit-identical incremental fold,
  range/granularity projection, graph build with Louvain community detection,
  hubs, frequent-sequence mining, and a Fruchterman–Reingold layout with
  warm-start.
- **Dashboard:** interactive graph view, analytics tables, per-session Sankey
  flow, a raw-event view, time-range and display filters, saved view presets, and
  PNG/SVG/CSV export.
- **Privacy controls:** pause capture, forget a domain, delete a recent time
  range, and export/import your data as a local JSON file.
- **Privacy by construction:** `host_permissions: []`, CSP `connect-src 'none'`,
  `incognito: "not_allowed"`, no content scripts, no remote code — all enforced by
  the browser and audited in CI.

[Unreleased]: https://github.com/erovelli/Outdegree/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/erovelli/Outdegree/releases/tag/v0.1.0
