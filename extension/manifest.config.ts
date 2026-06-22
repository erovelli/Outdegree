import { defineManifest } from "@crxjs/vite-plugin";

// Manifest (§5). The privacy guarantees are browser-enforced here:
//  • host_permissions: []          → no granted ability to contact any origin
//  • connect-src 'none' (CSP)      → fetch/XHR/WebSocket/sendBeacon are blocked
//  • incognito: "not_allowed"      → incognito browsing is never observed
// 'wasm-unsafe-eval' is required to instantiate the dashboard WASM module.
export default defineManifest({
  manifest_version: 3,
  name: "Outdegree",
  version: "0.1.0",
  description:
    "Records your navigations as a directed graph you can explore over time.",
  permissions: ["webNavigation", "storage", "unlimitedStorage"],
  host_permissions: [],
  incognito: "not_allowed",
  background: { service_worker: "src/service-worker.ts", type: "module" },
  action: { default_title: "Open Outdegree" },
  content_security_policy: {
    extension_pages:
      "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'none'; object-src 'self'",
  },
  icons: {
    "16": "icons/16.png",
    "32": "icons/32.png",
    "48": "icons/48.png",
    "128": "icons/128.png",
  },
});
