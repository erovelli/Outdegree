// Release smoke test: instantiate the (optimized) WASM through the real
// wasm-bindgen glue, which runs `__wbindgen_init_externref_table`. A bad
// `wasm-opt` pass mangles that table so the grow traps at startup — exactly the
// failure that shipped twice. Running init here makes the release FAIL instead
// of publishing a dashboard that dies on open. Browser-free, so it's reliable in
// CI. Usage: node scripts/smoke-extension.mjs [wasmDir]  (default extension/src/wasm)
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const dir = process.argv[2] ?? "extension/src/wasm";
// Stub the JS bridge so building the import object never throws (these are only
// *called* at runtime, never during init).
globalThis.chromeBridge = new Proxy({}, { get: () => () => {} });

const glue = await import(pathToFileURL(`${dir}/browsing_graph_core.js`).href);
const init = glue.default;

try {
  await init(readFileSync(`${dir}/browsing_graph_core_bg.wasm`));
  console.log("✓ smoke: WASM init OK (externref table grew) — artifact is launchable");
  process.exit(0);
} catch (e) {
  console.error("✗ smoke: WASM init FAILED — broken release artifact:", e?.message ?? e);
  process.exit(1);
}
