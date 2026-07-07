# Porting Outdegree to Firefox and Edge

Outdegree ships as a Chromium MV3 extension. This document assesses what it takes
to run it on **Microsoft Edge** (Chromium) and **Mozilla Firefox** (Gecko), and
gives a ready-to-use Firefox manifest variant. **Nothing here changes the
shipping Chrome build** — it is an analysis plus a recommended Firefox build
recipe, so the Chrome manifest stays byte-for-byte what the privacy audit
enforces.

The privacy invariants are non-negotiable on every target:

- `host_permissions: []`, `optional_host_permissions` absent
- `permissions ⊆ { webNavigation, storage, unlimitedStorage }`
- `incognito: "not_allowed"`
- CSP `connect-src 'none'` (no fetch / XHR / WebSocket / sendBeacon / EventSource)
- no `content_scripts`, no `web_accessible_resources`, no remote code
- the single OKLCH provenance hue is the only data color

## Capability surface used

The extension touches a deliberately small slice of the WebExtensions API:

| API | Where | Firefox | Edge |
| --- | --- | --- | --- |
| `webNavigation.onCommitted` / `onCreatedNavigationTarget` / `onHistoryStateUpdated` / `onReferenceFragmentUpdated` | service worker (capture) | ✅ supported | ✅ supported |
| `storage.local` (+ `storage.onChanged`) | SW + dashboard (pause flag, prefs, saved views) | ✅ | ✅ |
| `unlimitedStorage` permission | manifest | ✅ | ✅ |
| `tabs.create` / `tabs.get` / `tabs.onRemoved` | SW + open dashboard | ✅ | ✅ |
| `runtime.onInstalled` / `onStartup` / `onMessage` / `sendMessage` / `getURL` | SW ⇄ dashboard handshake | ✅ | ✅ |
| `action.onClicked` | open dashboard on toolbar click | ✅ (MV3 `action`) | ✅ |
| WebAssembly + `'wasm-unsafe-eval'` CSP | dashboard page (analysis core) | ✅ | ✅ |
| IndexedDB | SW + dashboard (event store, rollups) | ✅ | ✅ |

No `host_permissions` are needed because `webNavigation` events already carry the
URL, and the extension never injects into or reads page content.

## Microsoft Edge — drop-in

Edge is Chromium-based and runs the same MV3 model (background **service
worker**, identical manifest keys, same CSP semantics). The existing build runs
unmodified:

- Same `manifest.json` the Chrome build emits — no changes.
- `incognito: "not_allowed"` is honored (Edge calls it "InPrivate"; the key value
  is the same).
- Distribution differs only in store mechanics (Partner Center vs the Chrome Web
  Store); the package is identical.

**Required changes: none.** Load `dist/` as an unpacked extension, or submit the
same zip to the Edge Add-ons store.

## Mozilla Firefox — small manifest delta, same code

Firefox supports MV3, `webNavigation`, `storage`/`unlimitedStorage`, the `action`
key, `'wasm-unsafe-eval'`, and `incognito: "not_allowed"`. The capture and
analysis code is browser-agnostic (`chrome.*` is aliased to `browser.*` callback
style, which Firefox accepts). Three manifest-level differences matter:

1. **Background type.** Chrome MV3 requires a background **service worker**.
   Firefox MV3 runs the background as an **event page** (`background.scripts`)
   and (depending on version) does not honor `background.service_worker`. The
   portable form is to ship **both** keys — each browser reads the one it
   supports:

   ```jsonc
   "background": {
     "service_worker": "service-worker.js", // Chrome / Edge
     "scripts": ["service-worker.js"],       // Firefox event page
     "type": "module"
   }
   ```

   The capture worker only registers `chrome.*` event listeners synchronously at
   top level and funnels appends through a serialized promise queue, so it
   behaves correctly as an event page (no reliance on `ServiceWorkerGlobalScope`
   specifics, `importScripts`, `clients`, or `FetchEvent`).

2. **`browser_specific_settings.gecko`.** Firefox requires an extension `id`
   (and benefits from a `strict_min_version`). MV3 + `'wasm-unsafe-eval'` is
   stable from Firefox 121+:

   ```jsonc
   "browser_specific_settings": {
     "gecko": {
       "id": "outdegree@erovelli.github.io",
       "strict_min_version": "121.0"
     }
   }
   ```

   This key is **intentionally not added to the Chrome manifest**: Chrome logs an
   "Unrecognized manifest key" warning for it, and the `@crxjs/vite-plugin`
   `defineManifest` type does not include it. It belongs only in the Firefox
   artifact (see the build recipe below).

3. **Packaging/build.** The toolchain (`@crxjs/vite-plugin`) targets Chromium.
   For Firefox, take the same emitted `dist/`, overlay the two manifest deltas
   above, and package with `web-ext`.

### Firefox manifest (full recommended variant)

```jsonc
{
  "manifest_version": 3,
  "name": "Outdegree",
  "version": "0.1.0",
  "description": "Records your navigations as a directed graph you can explore over time.",
  "permissions": ["webNavigation", "storage", "unlimitedStorage"],
  "host_permissions": [],
  "incognito": "not_allowed",
  "background": {
    "service_worker": "service-worker.js",
    "scripts": ["service-worker.js"],
    "type": "module"
  },
  "action": { "default_title": "Open Outdegree" },
  "content_security_policy": {
    "extension_pages": "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'none'; object-src 'self'"
  },
  "browser_specific_settings": {
    "gecko": { "id": "outdegree@erovelli.github.io", "strict_min_version": "121.0" }
  },
  "icons": { "16": "icons/16.png", "32": "icons/32.png", "48": "icons/48.png", "128": "icons/128.png" }
}
```

Every privacy invariant above is preserved verbatim: empty `host_permissions`,
the same three permissions, `connect-src 'none'`, `incognito: "not_allowed"`, and
no content scripts or web-accessible resources.

### Firefox build recipe (wired into CI)

This is implemented as `scripts/build-firefox.mjs` + the `build:firefox` npm
script. `npm run build:firefox`:

1. runs the normal `npm run build` (`build:wasm` + `vite build`),
2. copies `dist/` to `dist-firefox/` (a byte-identical copy of the compiled
   bundle),
3. rewrites **only** `dist-firefox/manifest.json` with the two deltas — background
   `scripts` (derived from the emitted `service_worker`, so it tracks the
   `@crxjs` `service-worker-loader.js` filename) + `browser_specific_settings.gecko`,
4. runs `web-ext lint --source-dir dist-firefox`.

`npm run package:firefox` then runs `web-ext build` to produce the Firefox
`.zip`. `dist-firefox/` (like `dist/`) is generated and git-ignored. `web-ext` is
a **devDependency only** — nothing from it enters the shipped bundle, so the
network-surface audit is unaffected.

The same manifest privacy audit (`.github/workflows/ci.yml`) is now pointed at
`dist-firefox/manifest.json` **in CI** (the `firefox` job) — none of the deltas
touch the audited keys (`host_permissions`, `permissions`, `incognito`, CSP,
`content_scripts`, `web_accessible_resources`), so the Firefox artifact passes the
identical gate. The `release.yml` workflow overlays the same two deltas onto the
optimized `dist/` and attaches `outdegree-firefox-vX.Y.Z.zip` to the GitHub
Release alongside the Chrome/Edge zip (AMO submission stays manual for now).

#### web-ext lint policy

CI runs `web-ext lint` at its default severity: **lint errors fail the build;
warnings do not** (we deliberately do **not** pass `--warnings-as-errors`). On the
current bundle it reports **0 errors** and a small set of known-acceptable
warnings, each expected and none a privacy or correctness problem:

- **`BACKGROUND_SERVICE_WORKER_IGNORED`** — expected and intended: Firefox ignores
  `background.service_worker` and runs `background.scripts` instead. Shipping both
  keys is exactly the portable form; each browser reads the one it supports.
- **`INCOMPATIBLE_API` / `ANDROID_INCOMPATIBLE_API` (`runtime.getContexts`)** — the
  toolbar "focus an already-open dashboard tab" optimization uses `runtime.getContexts`
  (Firefox ≥ 129); below that it isn't implemented, and the code already degrades
  to opening a new tab (the same fallback it uses on older Chrome). `strict_min_version`
  stays `121.0` because that is where MV3 + `'wasm-unsafe-eval'` is stable; the
  degradation is cosmetic (no capture impact).
- **`UNSAFE_VAR_ASSIGNMENT` (`innerHTML`)** — a heuristic flag on the dashboard's
  own UI code, which builds markup from local, already-in-store data on the
  extension's own page. There is no remote or untrusted input (CSP is
  `connect-src 'none'`, no content scripts, no remote code), so it is a
  false positive for this extension.
- **`MISSING_DATA_COLLECTION_PERMISSIONS`** — a forthcoming AMO requirement to
  declare data collection. Outdegree collects and transmits nothing; the accurate
  declaration is `browser_specific_settings.gecko.data_collection_permissions:
  { required: ["none"] }`. It is intentionally **not** added yet: this change keeps
  strictly to the two documented deltas and AMO submission is manual/out of scope.
  Add it when the AMO listing is prepared.

## Known caveats

- **Firefox event-page lifecycle.** Event pages can be torn down and respawned
  like service workers; the capture queue already tolerates this (the queue tail
  is a single promise reset harmlessly on restart), so ordering is preserved.
- **`tabs.create` focus.** Minor cosmetic differences in how a newly created
  dashboard tab is focused across browsers; no behavioral impact on capture.
- **WASM size.** The dashboard module is large; all targets instantiate it on the
  extension page under `'wasm-unsafe-eval'` with no network fetch.

## Summary

| Target | Effort | Code changes | Manifest changes |
| --- | --- | --- | --- |
| **Edge** | none | none | none |
| **Firefox** | small | none | add `background.scripts` + `browser_specific_settings.gecko` in a Firefox-only build artifact |

Edge is a drop-in. Firefox needs only a Firefox-specific manifest overlay (two
keys) applied to the same compiled bundle — no source changes and no relaxation
of any privacy guarantee.
