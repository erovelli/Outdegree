// service-worker.ts — append-only capture into one globally-ordered stream (§6.1).
//
// Listeners are registered synchronously at top level, and each navigation/link/
// close/start handler **returns its write promise** so Chrome keeps the worker
// alive until the IndexedDB commit. No per-tab or derived state is kept here —
// all derivation happens at read time on the dashboard (§1).

import { ensureCreated, idbAdd } from "./idb";

// The pause flag is SW-owned and lives in chrome.storage.local (§4.5).
let paused = false;
chrome.storage.local.get("paused").then((o) => {
  paused = !!o.paused;
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.paused) paused = !!changes.paused.newValue;
});

// Skip the extension's own pages (e.g. the dashboard) so opening it is not
// recorded as a navigation (§6.1, M0 accept criteria).
const OWN_URL_PREFIX = chrome.runtime.getURL("");

// ── Schema lifecycle ──────────────────────────────────────────────────────────
chrome.runtime.onInstalled.addListener(() => {
  void ensureCreated();
});

chrome.runtime.onStartup.addListener(() =>
  idbAdd("events", { kind: "start", ts: Date.now() })
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
  // windowId is not on onCommitted details; it only scopes sessions, so we
  // default it and keep the append synchronous to preserve global id order
  // (§6.1). No per-tab state is kept here by design.
  return idbAdd("events", {
    kind: "nav",
    ts: d.timeStamp ?? Date.now(),
    tabId: d.tabId,
    windowId: 0,
    toUrl: d.url,
    transitionType: d.transitionType,
    qualifiers: d.transitionQualifiers,
  });
});

// In-page (history.pushState) navigations go to the separate high-volume store,
// never read by the default pass (§4.2).
chrome.webNavigation.onHistoryStateUpdated.addListener((d) => {
  if (d.frameId !== 0 || paused) return;
  if (d.url.startsWith(OWN_URL_PREFIX)) return;
  return idbAdd("spa", {
    kind: "nav",
    ts: d.timeStamp ?? Date.now(),
    tabId: d.tabId,
    windowId: 0,
    toUrl: d.url,
    transitionType: d.transitionType,
    qualifiers: d.transitionQualifiers,
  });
});

// Open-link-in-new-tab: the link's source becomes the child's origin (§7.3).
chrome.webNavigation.onCreatedNavigationTarget.addListener((d) => {
  if (paused) return;
  return idbAdd("events", {
    kind: "link",
    ts: d.timeStamp ?? Date.now(),
    newTabId: d.tabId,
    sourceTabId: d.sourceTabId,
  });
});

// Tab close: lets the read-time pass evict per-tab state precisely (§7.3).
chrome.tabs.onRemoved.addListener((tabId) => {
  if (paused) return;
  return idbAdd("events", { kind: "close", ts: Date.now(), tabId });
});

// Toolbar click opens the dashboard.
chrome.action.onClicked.addListener(() => {
  void chrome.tabs.create({ url: chrome.runtime.getURL("dashboard.html") });
});
