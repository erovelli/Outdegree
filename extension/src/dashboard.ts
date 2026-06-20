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
  try {
    await chrome.runtime.sendMessage({ type: "ready?" });
  } catch (e) {
    // The SW may be asleep on first message; a second send wakes it.
    console.warn("readiness ping retry", e);
    await chrome.runtime.sendMessage({ type: "ready?" });
  }
  await init(wasmBytes);
  mount("app");
}

void main();
