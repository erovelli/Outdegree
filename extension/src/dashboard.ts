// dashboard.ts — page entry (§6.4).
//
// Ping the service worker first: it ensure-creates the DB and acks, so the DB
// stores exist before the WASM core's rexie opens them (the readiness handshake
// that prevents a wedged empty DB on fresh-install-then-open, §4 / §6.4).

import "./chrome-bridge";
import init, { mount } from "./wasm/browsing_graph_core.js";
// Inlined WASM bytes (see vite.config.ts) — instantiated without fetch so it
// works under CSP `connect-src 'none'`.
import wasmBytes from "virtual:wasm-bytes";

async function main(): Promise<void> {
  // Best-effort readiness ping. The SW may be asleep on first message; retry
  // once. Either way we proceed to init — `store.open()` re-creates the schema,
  // so a missed ack is recoverable and must not block the dashboard.
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      await chrome.runtime.sendMessage({ type: "ready?" });
      break;
    } catch (e) {
      console.warn(`readiness ping attempt ${attempt + 1} failed`, e);
    }
  }
  await init(wasmBytes);
  mount("app");
}

// If startup fails before the WASM core can render its own error state, replace
// the "Loading…" placeholder so the page never hangs silently.
main().catch((e) => {
  console.error("Outdegree failed to start", e);
  const app = document.getElementById("app");
  if (app) {
    app.innerHTML =
      '<div class="bg-empty">Outdegree failed to start. Your browser may not ' +
      "support WebAssembly or local storage in this context. Try reopening the tab.</div>";
  }
});
