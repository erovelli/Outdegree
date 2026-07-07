import { defineManifest } from "@crxjs/vite-plugin";
// Single source of truth for the version: derive it from package.json so the
// manifest and the npm package can never disagree (the store requires monotonic
// versions). Vite/esbuild inlines this JSON at config-build time.
import pkg from "../package.json";

// Manifest (§5). The privacy guarantees are browser-enforced here:
//  • host_permissions: []          → no granted ability to contact any origin
//  • connect-src 'none' (CSP)      → fetch/XHR/WebSocket/sendBeacon are blocked
//  • incognito: "not_allowed"      → incognito browsing is never observed
// 'wasm-unsafe-eval' is required to instantiate the dashboard WASM module.
//
// The `favicon` permission (§F12, docs/adr/0006) unlocks Chrome's LOCAL favicon
// service at the extension's own origin (chrome-extension://<id>/_favicon/) so the
// dashboard can label sites with their icons. It makes NO network request — Chrome
// serves the icon from its on-disk cache — so the no-egress guarantee is intact.
// It is Chromium-only; the Firefox overlay (scripts/build-firefox.mjs) strips it.
export default defineManifest({
  manifest_version: 3,
  name: "Outdegree",
  version: pkg.version,
  description:
    "Records your navigations as a directed graph you can explore over time.",
  permissions: ["webNavigation", "storage", "unlimitedStorage", "favicon"],
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
