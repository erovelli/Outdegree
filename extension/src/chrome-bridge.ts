// chrome-bridge.ts — the thin JS surface the WASM core calls through (§6.3).
//
// Exposes only: local-storage get/set (for the pause flag and UI prefs) and a
// **local file download** for export. There is deliberately no network sink here
// — see the local-only guarantee (§12.1) and store.rs's export comment.

// The committed onboarding sample fixture, imported as a raw string so Vite
// inlines it into the bundle at build time (no fetch — the CSP is
// `connect-src 'none'`). The fixture stores URLs schemeless and timestamps as
// offsets; the WASM core (crates/core/src/sample.rs) re-attaches the scheme and
// shifts to absolute time on load. Keeping the scheme out of this inlined text is
// what keeps the CI dist/ network-surface audit (`grep https?://`) clean (§F4).
import sampleDataRaw from "./sample-data.json?raw";

declare global {
  interface Window {
    chromeBridge: ChromeBridge;
  }
}

export interface ChromeBridge {
  storageLocalGet: (k: string) => Promise<string | null>;
  storageLocalSet: (k: string, v: string) => void;
  downloadText: (name: string, mime: string, body: string) => void;
  downloadDataUrl: (name: string, dataUrl: string) => void;
  /** Raw text of the committed onboarding sample fixture (§F4). */
  sampleData: () => string;
  /** Extension-origin base URL for Chrome's local favicon service, or "" (§F12). */
  faviconBase: () => string;
}

const chromeBridge: ChromeBridge = {
  storageLocalGet: (k: string) =>
    chrome.storage.local.get(k).then((o) => (o[k] ?? null) as string | null),

  storageLocalSet: (k: string, v: string) => {
    void chrome.storage.local.set({ [k]: v });
  },

  // Export = a Blob written to the user's disk via an object URL. A *download*,
  // never an upload — the only path by which data leaves the extension (§7.8).
  downloadText: (name: string, mime: string, body: string) => {
    const url = URL.createObjectURL(new Blob([body], { type: mime }));
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.click();
    URL.revokeObjectURL(url);
  },

  // Download from a data: URL (PNG bytes from canvas.toDataURL). Same local-only
  // download path as downloadText — no network sink (§7.8).
  downloadDataUrl: (name: string, dataUrl: string) => {
    const a = document.createElement("a");
    a.href = dataUrl;
    a.download = name;
    a.click();
  },

  // Hand the inlined fixture text to the WASM core, which materializes it
  // (offset→absolute timestamps + scheme prepend) before importing (§F4).
  sampleData: () => sampleDataRaw,

  // Base URL for Chrome's LOCAL favicon service (§F12). Chrome serves site icons
  // from its own on-disk favicon cache at the extension's origin
  // (chrome-extension://<id>/_favicon/) — NO network request, so the no-egress
  // guarantee holds (see docs/adr/0006). getURL() returns an origin-relative URL at
  // runtime, so no https:// literal enters the bundle. Guarded on the manifest
  // actually declaring the `favicon` permission: the Firefox overlay strips it
  // (Chromium-only API), so this returns "" there and the WASM core's site-icons
  // feature stays completely inert (no URLs built, no <img> emitted). Any failure
  // (older browsers, no chrome.runtime) also degrades to "".
  faviconBase: () => {
    try {
      const m = chrome.runtime.getManifest();
      if (!m.permissions || !m.permissions.includes("favicon")) return "";
      return chrome.runtime.getURL("_favicon/");
    } catch {
      return "";
    }
  },
};

(globalThis as unknown as { chromeBridge: ChromeBridge }).chromeBridge = chromeBridge;

export {};
