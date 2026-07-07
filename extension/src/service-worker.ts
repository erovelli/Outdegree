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
  badgeStateFor,
  closeRecord,
  findDashboardTab,
  flagOn,
  focusRecord,
  linkRecord,
  navRecord,
  startRecord,
  wfocusRecord,
  type DashboardTabRef,
  type NavDetail,
} from "./capture";

// Neutral, achromatic gray for the paused badge background. The badge is chrome,
// not data, so it deliberately stays off the single provenance hue (design
// constraint); ~Chrome's own disabled-gray.
const PAUSED_BADGE_COLOR = "#5f6368";

// Reflect the pause flag onto the toolbar icon so capture-off is visible without
// opening the dashboard (§4.5). Text/title come from the pure badgeStateFor;
// setBadgeText("") clears it when running.
function applyBadge(isPaused: boolean): void {
  const { text, title } = badgeStateFor(isPaused);
  void chrome.action.setBadgeText({ text });
  void chrome.action.setTitle({ title });
  if (isPaused) {
    void chrome.action.setBadgeBackgroundColor({ color: PAUSED_BADGE_COLOR });
  }
}

// The pause flag is SW-owned and lives in chrome.storage.local (§4.5). The
// dashboard writes it as the STRING "true"/"false", so parse it via the shared
// flagOn helper (a plain `!!value` would treat "false" as truthy). This
// top-level read runs on every worker start — including onStartup after a
// browser restart, which resets the badge — so it re-applies the badge there.
let paused = false;
chrome.storage.local.get("paused").then((o) => {
  paused = flagOn(o.paused);
  applyBadge(paused);
});
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "local" && changes.paused) {
    paused = flagOn(changes.paused.newValue);
    applyBadge(paused);
  }
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

// Tab activation → "focus" (§F7): which tab is on-screen in a window. Ids only
// (no URL/title), no new permission; the derive layer attributes foreground
// time by joining it to the tab's already-captured page.
chrome.tabs.onActivated.addListener((activeInfo) => {
  if (paused) return;
  return enqueue(() => idbAdd("events", focusRecord(activeInfo, Date.now())));
});

// Window focus → "wfocus" (§F7). Chrome reports WINDOW_ID_NONE (-1) when the
// whole browser is blurred (alt-tab away) — recorded as -1 so attribution stops.
chrome.windows.onFocusChanged.addListener((windowId) => {
  if (paused) return;
  return enqueue(() => idbAdd("events", wfocusRecord(windowId, Date.now())));
});

// Toolbar click focuses an already-open dashboard tab, else opens one — so
// clicking repeatedly doesn't pile up duplicate tabs. The open tab is found via
// runtime.getContexts (MV3, Chrome 116+) rather than tabs.query({url:...}):
// URL-matching queries require the "tabs" permission, which must never be added.
chrome.action.onClicked.addListener(() => {
  void openOrFocusDashboard();
});

// Look up an open dashboard tab through getContexts, degrading gracefully if the
// API is unavailable (older Chrome) or throws — the caller then opens a new tab.
async function findOpenDashboardTab(
  dashboardUrl: string
): Promise<DashboardTabRef | null> {
  if (typeof chrome.runtime.getContexts !== "function") return null;
  try {
    const contexts = await chrome.runtime.getContexts({ contextTypes: ["TAB"] });
    return findDashboardTab(contexts, dashboardUrl);
  } catch {
    return null;
  }
}

async function openOrFocusDashboard(): Promise<void> {
  const dashboardUrl = chrome.runtime.getURL("dashboard.html");
  const existing = await findOpenDashboardTab(dashboardUrl);
  if (existing) {
    try {
      // Activating a tab by id and focusing a window need no "tabs" permission.
      await chrome.tabs.update(existing.tabId, { active: true });
      if (existing.windowId >= 0) {
        await chrome.windows.update(existing.windowId, { focused: true });
      }
      return;
    } catch {
      // The tab/window vanished between lookup and focus — open a fresh one.
    }
  }
  await chrome.tabs.create({ url: dashboardUrl });
}
