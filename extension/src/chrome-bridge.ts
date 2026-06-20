// chrome-bridge.ts — the thin JS surface the WASM core calls through (§6.3).
//
// Exposes only: local-storage get/set (for the pause flag and UI prefs) and a
// **local file download** for export. There is deliberately no network sink here
// — see the local-only guarantee (§12.1) and store.rs's export comment.

declare global {
  interface Window {
    chromeBridge: ChromeBridge;
  }
}

export interface ChromeBridge {
  storageLocalGet: (k: string) => Promise<string | null>;
  storageLocalSet: (k: string, v: string) => void;
  downloadJson: (name: string, json: string) => void;
}

const chromeBridge: ChromeBridge = {
  storageLocalGet: (k: string) =>
    chrome.storage.local.get(k).then((o) => (o[k] ?? null) as string | null),

  storageLocalSet: (k: string, v: string) => {
    void chrome.storage.local.set({ [k]: v });
  },

  // Export = a Blob written to the user's disk via an object URL. A *download*,
  // never an upload — the only path by which data leaves the extension (§7.8).
  downloadJson: (name: string, json: string) => {
    const url = URL.createObjectURL(new Blob([json], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.click();
    URL.revokeObjectURL(url);
  },
};

(globalThis as unknown as { chromeBridge: ChromeBridge }).chromeBridge = chromeBridge;

export {};
