---
name: verify
description: Build the extension and drive the real dashboard in Chromium (Playwright + --load-extension) to verify a change end-to-end.
---

# Verify Outdegree changes at the dashboard surface

The product surface is the extension's dashboard page running inside Chromium
with the MV3 service worker live — not the Vite dev server.

## Recipe that works

1. `npm run build` — emits the unpacked extension into `dist/`.
2. Drive it with the globally-installed Playwright (`/opt/node22/lib/node_modules/playwright/index.mjs`
   in the CCW container; plain `import 'playwright'` elsewhere):

```js
import { chromium } from 'playwright';
const ctx = await chromium.launchPersistentContext(tmpProfileDir, {
  headless: true,
  args: [
    '--disable-extensions-except=/abs/path/to/dist',
    '--load-extension=/abs/path/to/dist',
    '--headless=new',          // extensions need the *new* headless mode
  ],
  viewport: { width: 1280, height: 800 },
});
let [sw] = ctx.serviceWorkers();
if (!sw) sw = await ctx.waitForEvent('serviceworker');
const extId = new URL(sw.url()).host;
const page = await ctx.newPage();
await page.goto(`chrome-extension://${extId}/dashboard.html`);
```

3. A fresh profile lands on the first-run overlay (`#bg-welcome`). Click
   **"Load sample data"** (exact text) to populate a real graph (§F4 demo
   dataset), then wait for `#bg-canvas`.
4. Attach `page.on('pageerror')` + console-error listeners — the WASM UI fails
   quietly otherwise. Screenshot for evidence.

## Useful handles

- Canvas: `#bg-canvas` (full-bleed; hover sets class `grabbable` — probe a grid
  of `mouse.move` points and read the class to find a node under the cursor).
- Chips/controls by id: `#bg-persp` (2D/3D toggle, in the bottom-right zoom
  toolbar), `#seg-gran`, `#chip-minvisits`, `#rng-week` etc.,
  `#bg-legend-rows .legend-row`, `#bg-focus-label`, `#bg-count-nodes`. Zoom
  toolbar: `#bg-zoom-in|bg-zoom-out|bg-fit|bg-lock`. View switcher:
  `#vw-graph|sankey|tables`.
- Prefs persist in `chrome.storage.local` (`uiPrefs`) — reload the page to
  verify persistence; reuse the same profile dir to keep IndexedDB data.

## Gotchas

- Keyboard shortcuts on the canvas need it focused (`canvas.focus()` after a
  click on empty space — a click on a node drills into it).
- One `mouse.wheel` tick ≈ one 1.1× zoom step; send several for a visible change.
