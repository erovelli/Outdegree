// service-worker.ts — append-only capture into one globally-ordered stream (§6.1).
//
// Listeners are registered synchronously at top level. All `events` appends are
// funneled through one serialized promise queue, so even when a handler needs an
// async lookup (resolving windowId) the records are still committed in strict
// firing order — preserving the global id order the read-time pass depends on.
// Each handler returns its queued promise so Chrome keeps the worker alive until
// the IndexedDB commit. No per-tab or derived state is kept (the queue tail is a
// single promise, reset harmlessly on restart).

import { ensureCreated, idbAdd } from "./idb";

// The pause flag is SW-owned and lives in chrome.storage.local (§4.5). The
// dashboard writes it as the STRING "true"/"false" (chromeBridge.storageLocalSet
// only takes strings), so parse it the same way the dashboard reads it — a plain
// `!!value` would treat the string "false" as truthy and wedge capture off.
function flagOn(v: unknown): boolean {
  return v === true || v === "true" || v === "1";
}
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

// ── Schema lifecycle ──────────────────────────────────────────────────────────
chrome.runtime.onInstalled.addListener(() => {
  void ensureCreated();
});

chrome.runtime.onStartup.addListener(() =>
  enqueue(() => idbAdd("events", { kind: "start", ts: Date.now() }))
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
chrome.webNavigation.onCommitted.addListener((d) => {
  if (d.frameId !== 0 || paused) return;
  if (d.url.startsWith(OWN_URL_PREFIX)) return;
  const ts = d.timeStamp ?? Date.now();
  return enqueue(async () =>
    idbAdd("events", {
      kind: "nav",
      ts,
      tabId: d.tabId,
      windowId: await resolveWindowId(d.tabId),
      toUrl: d.url,
      transitionType: d.transitionType,
      qualifiers: d.transitionQualifiers,
    })
  );
});

// In-page (history.pushState) navigations go to the separate high-volume store,
// never read by the default pass (§4.2).
chrome.webNavigation.onHistoryStateUpdated.addListener((d) => {
  if (d.frameId !== 0 || paused) return;
  if (d.url.startsWith(OWN_URL_PREFIX)) return;
  const ts = d.timeStamp ?? Date.now();
  return enqueue(async () =>
    idbAdd("spa", {
      kind: "nav",
      ts,
      tabId: d.tabId,
      windowId: await resolveWindowId(d.tabId),
      toUrl: d.url,
      transitionType: d.transitionType,
      qualifiers: d.transitionQualifiers,
    })
  );
});

// Hash-route (#/...) SPA navigations — same opt-in `spa` store as pushState, so
// hash-routed apps (older SPAs, docs sites) aren't invisible to the in-app view.
chrome.webNavigation.onReferenceFragmentUpdated.addListener((d) => {
  if (d.frameId !== 0 || paused) return;
  if (d.url.startsWith(OWN_URL_PREFIX)) return;
  const ts = d.timeStamp ?? Date.now();
  return enqueue(async () =>
    idbAdd("spa", {
      kind: "nav",
      ts,
      tabId: d.tabId,
      windowId: await resolveWindowId(d.tabId),
      toUrl: d.url,
      transitionType: d.transitionType,
      qualifiers: d.transitionQualifiers,
    })
  );
});

// Open-link-in-new-tab: the link's source becomes the child's origin (§7.3).
chrome.webNavigation.onCreatedNavigationTarget.addListener((d) => {
  if (paused) return;
  const ts = d.timeStamp ?? Date.now();
  return enqueue(() =>
    idbAdd("events", {
      kind: "link",
      ts,
      newTabId: d.tabId,
      sourceTabId: d.sourceTabId,
    })
  );
});

// Tab close: lets the read-time pass evict per-tab state precisely (§7.3).
chrome.tabs.onRemoved.addListener((tabId) => {
  if (paused) return;
  const ts = Date.now();
  return enqueue(() => idbAdd("events", { kind: "close", ts, tabId }));
});

// Toolbar click opens the dashboard.
chrome.action.onClicked.addListener(() => {
  void chrome.tabs.create({ url: chrome.runtime.getURL("dashboard.html") });
});
