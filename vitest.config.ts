import { defineConfig } from "vitest/config";

// Standalone test config (kept separate from vite.config.ts so the crx build
// plugin doesn't run during unit tests). Covers the pure capture helpers and the
// IndexedDB schema; the Rust analysis core is covered by `cargo test`.
export default defineConfig({
  test: {
    include: ["extension/src/__tests__/**/*.test.ts"],
    environment: "node",
  },
});
