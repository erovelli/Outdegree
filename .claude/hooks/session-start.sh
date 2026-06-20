#!/bin/bash
# SessionStart hook: prepare the toolchain so `cargo test` and `npm run build`
# work immediately in Claude Code on the web. Synchronous + idempotent.
set -euo pipefail

# Only run in remote (web) environments; a no-op locally.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-.}"

echo "[session-start] preparing Rust + Node toolchain…"

# Rust: ensure the wasm32 target (rust-toolchain.toml also requests clippy/rustfmt,
# which rustup installs on first use).
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# wasm-pack is needed for `npm run build:wasm`. Prefer the prebuilt installer;
# fall back to building from source.
if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "[session-start] installing wasm-pack…"
  curl -sSf https://rustwasm.github.io/wasm-pack/installer/init.sh | sh \
    || cargo install wasm-pack
fi

# JS dev dependencies. `npm install` (not `ci`) so the cached container layer is
# reused across sessions.
echo "[session-start] installing JS deps…"
npm install

# Warm the cargo registry + dependency build cache so the first test run is fast.
echo "[session-start] warming cargo cache…"
cargo fetch >/dev/null 2>&1 || true

echo "[session-start] done."
