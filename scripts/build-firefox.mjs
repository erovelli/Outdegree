// Firefox overlay build: take the exact Chromium `dist/` that the privacy audit
// enforces and produce a Firefox-loadable `dist-firefox/` by applying ONLY the
// documented manifest deltas from docs/PORTING.md — nothing else changes, so
// every privacy invariant (host_permissions:[], permissions allowlist, CSP
// connect-src 'none', incognito not_allowed, no content_scripts / WAR) stays
// byte-identical and passes the same CI gate pointed at dist-firefox/manifest.json.
//
// Deltas (see docs/PORTING.md):
//   1. background: ship `scripts` alongside `service_worker` (Firefox event page).
//   2. browser_specific_settings.gecko: Firefox extension id + strict_min_version.
//   3. permissions: STRIP "favicon" — the `_favicon` service is Chromium-only, so
//      declaring it on Firefox would only produce an install warning (§F12). The
//      dashboard's site-icons feature already runtime-guards on the permission
//      being declared, so stripping it here makes the feature cleanly inert on
//      Firefox. This is why the Firefox manifest audit's allowlist is THREE
//      permissions while the Chrome one is FOUR (see .github/workflows/ci.yml).
//
// The compiled bundle (WASM, dashboard, service worker) is browser-agnostic and
// is copied verbatim; web-ext is a dev-only tool and contributes nothing to it.
//
// Usage: node scripts/build-firefox.mjs   (run `npm run build` first — needs dist/)
import { cpSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(ROOT, "dist");
const DIST_FF = join(ROOT, "dist-firefox");
const SRC_MANIFEST = join(DIST, "manifest.json");
const FF_MANIFEST = join(DIST_FF, "manifest.json");

// docs/PORTING.md §"Mozilla Firefox": Firefox MV3 requires an extension id (and
// benefits from a strict_min_version). MV3 + 'wasm-unsafe-eval' is stable from
// Firefox 121+. This key is intentionally NOT in the Chrome manifest (Chrome logs
// "Unrecognized manifest key" and @crxjs/vite-plugin's type omits it).
const GECKO = { id: "outdegree@erovelli.github.io", strict_min_version: "121.0" };

if (!existsSync(SRC_MANIFEST)) {
  console.error(
    `✗ ${SRC_MANIFEST} not found — run \`npm run build\` before build:firefox.`
  );
  process.exit(1);
}

// 1) Fresh, byte-identical copy of the Chromium dist/. Clear any stale overlay so
//    a removed dist/ file can never linger here.
rmSync(DIST_FF, { recursive: true, force: true });
cpSync(DIST, DIST_FF, { recursive: true });

// 2) Apply EXACTLY the two documented manifest deltas to the overlay copy. The
//    dist/ manifest is never touched — only dist-firefox/manifest.json.
const manifest = JSON.parse(readFileSync(FF_MANIFEST, "utf8"));

// Delta 1 — Background. Chrome/Edge read `service_worker`; Firefox MV3 runs the
// same file as an event page via `scripts`. Ship both keys (each browser reads
// the one it supports) and derive `scripts` from the emitted service_worker so we
// track the @crxjs loader filename (service-worker-loader.js) rather than hardcode.
const sw = manifest.background?.service_worker;
if (!sw) {
  console.error(
    "✗ dist manifest has no background.service_worker — unexpected build output."
  );
  process.exit(1);
}
manifest.background = {
  service_worker: sw,
  scripts: [sw],
  type: manifest.background.type ?? "module",
};

// Delta 2 — Firefox extension id + minimum version.
manifest.browser_specific_settings = { gecko: { ...GECKO } };

// Delta 3 — Strip the Chromium-only `favicon` permission (§F12). Firefox has no
// `_favicon` service, so declaring it would only surface an install warning; the
// dashboard guards the feature on the permission being present, so removing it
// here keeps site icons inert on Firefox with no source changes.
const hadFavicon = Array.isArray(manifest.permissions)
  && manifest.permissions.includes("favicon");
if (hadFavicon) {
  manifest.permissions = manifest.permissions.filter((p) => p !== "favicon");
}

writeFileSync(FF_MANIFEST, JSON.stringify(manifest, null, 2) + "\n");

console.log(
  `✓ dist-firefox/ ready — overlaid background.scripts=["${sw}"] + ` +
    `browser_specific_settings.gecko { id: "${GECKO.id}", ` +
    `strict_min_version: "${GECKO.strict_min_version}" }` +
    (hadFavicon ? ' + stripped Chromium-only "favicon" permission' : "")
);
