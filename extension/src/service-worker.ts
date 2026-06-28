// service-worker.ts — append-only capture into one globally-ordered stream (§6.1).
//
// Listeners are registered synchronously at top level. All `events` appends are
// funneled through one serialized promise queue, so even when a handler needs an
// async lookup (resolving windowId) the records are still committed in strict
// firing order — preserving the global id order the read-time pass depends on.
// Each handler returns its queued promise so Chrome keeps the worker alive until
// the IndexedDB commit. No per-tab or derived state is kept (the queue tail is a
// single promise, reset harmlessly on restart).
//
// Event records are shaped by the pure helpers in capture.ts (unit-tested); this
// file owns only the Chrome wiring, the pause gate, and the serialized queue.

import { ensureCreated, idbAdd } from "./idb";
import {
  closeRecord,
  flagOn,
  linkRecord,
  navRecord,
  startRecord,
  type NavDetail,
} from "./capture";

// The pause flag is SW-owned and lives in chrome.storage.local (§4.5). The
// dashboard writes it as the STRING "true"/"false", so parse it via the shared
// flagOn helper (a plain `!!value` would treat "false" as truthy).
let paused = false;
chrome.storage.local.get("paused").then((o) => {
  paused = flagOn(o.paused);
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.paused) paused = flagOn(changes.paused.newValue);
});

// Skip the extension's own pages (e.g. the dashboard) so opening it is not
// recorded as a navigation (§6.1, M0 accept criteria).
const OWN_URL_PREFIX = chrome.runtime.getURL("");

// Serialize all appends in firing order. `tail` is the only state here; chaining
// each job onto it guarantees IndexedDB `add()` (and thus id assignment) happens
// in the order events fired, even across async windowId resolution.
let tail: Promise<unknown> = Promise.resolve();
function enqueue(job: () => Promise<unknown>): Promise<unknown> {
  const run = tail.then(job, job);
  tail = run.catch(() => {});
  return run;
}

// windowId is not on navigation event details. Resolving it via chrome.tabs.get
// (windowId is available without the "tabs" permission) inside the serialized
// queue gives real per-window sessions without reordering appends. Falls back to
// 0 if the tab is already gone.
async function resolveWindowId(tabId: number): Promise<number> {
  try {
    const tab = await chrome.tabs.get(tabId);
    return tab.windowId ?? 0;
  } catch {
    return 0;
  }
}

// Shared top-level-frame nav handler for onCommitted (→ events) and the two SPA
// triggers (→ spa). Skips subframes, paused capture, and the extension's own
// pages, then appends the shaped record in firing order.
function handleNav(store: "events" | "spa", d: NavDetail & { frameId: number }) {
  if (d.frameId !== 0 || paused) return;
  if (d.url.startsWith(OWN_URL_PREFIX)) return;
  const now = Date.now();
  return enqueue(async () =>
    idbAdd(store, navRecord(d, await resolveWindowId(d.tabId), now))
  );
}

// ── Schema lifecycle ──────────────────────────────────────────────────────────
chrome.runtime.onInstalled.addListener(() => {
  void ensureCreated();
});

chrome.runtime.onStartup.addListener(() =>
  enqueue(() => idbAdd("events", startRecord(Date.now())))
);

// Readiness handshake: ensure-create the DB, then ack so the dashboard opens
// only after the stores exist (§4, §6.4). Returning true keeps the message
// channel open for the async response.
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === "ready?") {
    ensureCreated()
      .then(() => sendResponse("ready"))
      .catch(() => sendResponse("error"));
    return true;
  }
  return undefined;
});

// ── Capture ─────────────────────────────────────────────────────────────────
chrome.webNavigation.onCommitted.addListener((d) => handleNav("events", d));

// In-page (history.pushState) navigations go to the separate high-volume store,
// never read by the default pass (§4.2).
chrome.webNavigation.onHistoryStateUpdated.addListener((d) => handleNav("spa", d));

// Hash-route (#/...) SPA navigations — same opt-in `spa` store as pushState, so
// hash-routed apps (older SPAs, docs sites) aren't invisible to the in-app view.
chrome.webNavigation.onReferenceFragmentUpdated.addListener((d) =>
  handleNav("spa", d)
);

// Open-link-in-new-tab: the link's source becomes the child's origin (§7.3).
chrome.webNavigation.onCreatedNavigationTarget.addListener((d) => {
  if (paused) return;
  const now = Date.now();
  return enqueue(() => idbAdd("events", linkRecord(d, now)));
});

// Tab close: lets the read-time pass evict per-tab state precisely (§7.3).
chrome.tabs.onRemoved.addListener((tabId) => {
  if (paused) return;
  return enqueue(() => idbAdd("events", closeRecord(tabId, Date.now())));
});

// Toolbar click opens the dashboard.
chrome.action.onClicked.addListener(() => {
  void chrome.tabs.create({ url: chrome.runtime.getURL("dashboard.html") });
});
