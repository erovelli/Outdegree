# Contributing to Outdegree

Thanks for your interest! Outdegree is a local-only Chrome MV3 extension that
visualizes your own browsing as a navigation graph. It's a portfolio project, so
contributions are welcome but reviewed on a best-effort basis.

For a deeper map of the codebase and the rules that must never be broken, read
**[AGENTS.md](AGENTS.md)** first — it applies to humans and AI agents alike.

## Prerequisites

- **Rust** stable with the `wasm32-unknown-unknown` target (`rust-toolchain.toml`
  pins the channel + components).
- **wasm-pack** (`cargo install wasm-pack`).
- **Node 20** (see `.nvmrc`; matches CI).

## Setup & build

```bash
rustup target add wasm32-unknown-unknown
npm install
./build.sh        # builds WASM + extension into dist/
```

Then load it: `chrome://extensions` → enable **Developer mode** → **Load
unpacked** → select `dist/`. Click the toolbar icon to open the dashboard.

## Before you open a PR — run the checks CI runs

```bash
cargo fmt --all -- --check
cargo test -p browsing-graph-core
cargo clippy -p browsing-graph-core --lib --tests -- -D warnings
cargo clippy -p browsing-graph-core --target wasm32-unknown-unknown --all-targets -- -D warnings
npm run typecheck
npm run test:ts
npm run build
```

All of these must pass. The build step also runs the two **privacy audits**
(manifest + network surface) — see below.

If your change touches the capture layer (the service worker / event shapes),
also walk the relevant rows of [`docs/M0-CHECKLIST.md`](docs/M0-CHECKLIST.md) in a
real Chrome, since that path can't be fully unit-tested.

## The rules that gate every change

This is a privacy extension; its guarantees are **browser-enforced and
CI-audited**. A PR that does any of the following will fail CI and won't be
merged:

- Adds any network call (`fetch`/`XHR`/`WebSocket`/`EventSource`/`sendBeacon`/
  `importScripts`/remote `import()`), telemetry, or analytics.
- Widens `permissions` beyond `{ webNavigation, storage, unlimitedStorage }`, adds
  `host_permissions`, content scripts, or web-accessible resources.
- Loosens the CSP (`connect-src 'none'` must stay) or `incognito: "not_allowed"`.

See [SECURITY.md](SECURITY.md) and
[AGENTS.md](AGENTS.md#non-negotiable-invariants) for the full list and the
rationale.

## Conventions

- Keep the pure Rust core (`model`/`interpret`/`derive`/`rollup`/`project`/
  `graph`/`layout`/`flow`/`search`/`svg`/`export`/`views`) free of WASM/DOM
  dependencies so it keeps running under `cargo test`. WASM-only code lives behind
  `cfg(target_arch = "wasm32")`.
- If you change `derive.rs`/`rollup.rs`, keep the **fold == from-scratch
  recompute** invariant green (`crates/core/tests/prop.rs`).
- Add/keep rustdoc on public items in the pure core; comments explain *why*.
- Bump `package.json` version (the single source of truth) and add a
  `CHANGELOG.md` entry for user-visible changes. Bump the "Last updated" date in
  `docs/privacy-policy.md` if you change anything that affects it.

## Reporting bugs & requesting features

Use the issue templates. **Never paste exported browsing data** into an issue.
Security issues: see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree your contributions are licensed under the project's
[MIT License](LICENSE).
