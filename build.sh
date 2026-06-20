#!/usr/bin/env bash
# Build the unpacked extension into dist/ (§9). Load it via chrome://extensions
# (Developer mode → Load unpacked → select dist/).
set -euo pipefail
cd "$(dirname "$0")"

echo "==> Building WASM core (wasm-pack)…"
npm run build:wasm

echo "==> Building extension (vite)…"
npm run build:ext

echo "==> Done. Load the unpacked extension from: $(pwd)/dist"
