// capture.ts — pure event-shaping for the append-only capture layer (§6.1).
//
// These helpers translate Chrome `webNavigation`/`tabs` event details into the
// exact record shapes the Rust derive layer consumes (the camelCase IndexedDB
// contract in crates/core/src/model.rs `Event`). They are kept here, separate
// from the service worker, because the worker registers `chrome.*` listeners at
// module top level and so can't be imported into a unit test — but this contract
// is exactly what we want to test. See extension/src/__tests__/capture.test.ts.

/** A navigation record (`events` and `spa` stores share this shape). */
export interface NavRecord {
  kind: "nav";
  ts: number;
  tabId: number;
  windowId: number;
  toUrl: string;
  transitionType: string;
  qualifiers: string[];
}

/** A new-tab link record: the link's source becomes the child's origin (§7.3). */
export interface LinkRecord {
  kind: "link";
  ts: number;
  newTabId: number;
  sourceTabId: number;
}

/** A tab-close record: lets the read-time pass evict per-tab state precisely. */
export interface CloseRecord {
  kind: "close";
  ts: number;
  tabId: number;
}

/** A browser-startup marker: clears derived state so no phantom edge forms. */
export interface StartRecord {
  kind: "start";
  ts: number;
}

/** Subset of a `webNavigation` committed/history/fragment detail we record. */
export interface NavDetail {
  tabId: number;
  url: string;
  transitionType: string;
  transitionQualifiers: string[];
  timeStamp?: number;
}

/** Subset of an `onCreatedNavigationTarget` detail we record. */
export interface LinkDetail {
  tabId: number;
  sourceTabId: number;
  timeStamp?: number;
}

/**
 * Parse a stored boolean-ish flag (e.g. the SW-owned `paused` flag). The
 * dashboard writes it as the STRING "true"/"false" (storageLocalSet only takes
 * strings), so a plain `!!value` would treat "false" as truthy and wedge capture
 * off. Treats `true`, `"true"`, and `"1"` as on; everything else as off.
 */
export function flagOn(v: unknown): boolean {
  return v === true || v === "true" || v === "1";
}

/**
 * Build a navigation record. `windowId` is resolved separately (it isn't on the
 * event detail); `now` is the firing-time fallback used only if the browser
 * didn't stamp the event.
 */
export function navRecord(d: NavDetail, windowId: number, now: number): NavRecord {
  return {
    kind: "nav",
    ts: d.timeStamp ?? now,
    tabId: d.tabId,
    windowId,
    toUrl: d.url,
    transitionType: d.transitionType,
    qualifiers: d.transitionQualifiers,
  };
}

/** Build a new-tab link record from an `onCreatedNavigationTarget` detail. */
export function linkRecord(d: LinkDetail, now: number): LinkRecord {
  return {
    kind: "link",
    ts: d.timeStamp ?? now,
    newTabId: d.tabId,
    sourceTabId: d.sourceTabId,
  };
}

/** Build a tab-close record. */
export function closeRecord(tabId: number, ts: number): CloseRecord {
  return { kind: "close", ts, tabId };
}

/** Build a browser-startup record. */
export function startRecord(ts: number): StartRecord {
  return { kind: "start", ts };
}

// ── Foreground attention (§F7): tab activation + window focus ─────────────────

/**
 * A tab-activation record (`tabs.onActivated`): which tab became the active tab
 * of a window. Carries only ids — no URL, no title — so it needs no permission
 * beyond the existing set; the derive layer joins it to the tab's page itself.
 */
export interface FocusRecord {
  kind: "focus";
  ts: number;
  tabId: number;
  windowId: number;
}

/**
 * A window-focus record (`windows.onFocusChanged`): which window is focused,
 * with `windowId: -1` meaning the whole browser lost focus (alt-tab away).
 */
export interface WfocusRecord {
  kind: "wfocus";
  ts: number;
  windowId: number;
}

/** Subset of a `tabs.onActivated` activeInfo we record (no timestamp on it). */
export interface ActivatedDetail {
  tabId: number;
  windowId: number;
}

/** Build a tab-activation record from an `onActivated` activeInfo. */
export function focusRecord(d: ActivatedDetail, ts: number): FocusRecord {
  return { kind: "focus", ts, tabId: d.tabId, windowId: d.windowId };
}

/**
 * Build a window-focus record. Chrome reports `WINDOW_ID_NONE` (-1) when the
 * browser is blurred; any other sentinel below 0 (e.g. `WINDOW_ID_CURRENT`, -2,
 * which onFocusChanged should never emit) is normalized to -1 so the stored
 * stream has exactly one "no window focused" value for the derive layer.
 */
export function wfocusRecord(windowId: number, ts: number): WfocusRecord {
  return { kind: "wfocus", ts, windowId: windowId < 0 ? -1 : windowId };
}

// ── Toolbar affordances (§ pure helpers for service-worker.ts) ────────────────

/**
 * Glyph shown on the toolbar badge while capture is paused. `⏸` (U+23F8) reads
 * unambiguously as "pause"; if it renders illegibly at badge size on some
 * platform, swap it for the ASCII fallback `"II"` — this is the single point of
 * change and `badgeStateFor` stays otherwise identical.
 */
const PAUSED_BADGE_TEXT = "⏸";

/** The toolbar badge text + hover title derived purely from the pause flag. */
export interface BadgeState {
  /** `chrome.action.setBadgeText` value ("" clears the badge). */
  text: string;
  /** `chrome.action.setTitle` value. */
  title: string;
}

/**
 * Map the (already-parsed) pause flag to the toolbar badge text and hover title.
 * Paused → a visible glyph + a "capture paused" title; running → an empty badge
 * (cleared) + the default "Open Outdegree" title. The neutral-gray badge
 * background is applied by the caller (it is chrome, not data, so it stays
 * achromatic — off the single provenance hue). Kept pure so it is unit-testable
 * without the service worker's `chrome.*` listeners.
 */
export function badgeStateFor(paused: boolean): BadgeState {
  return paused
    ? { text: PAUSED_BADGE_TEXT, title: "Outdegree — capture paused" }
    : { text: "", title: "Open Outdegree" };
}

/** A resolved dashboard tab: enough to activate it and focus its window. */
export interface DashboardTabRef {
  tabId: number;
  windowId: number;
}

/**
 * The subset of a `chrome.runtime.ExtensionContext` the dashboard-tab picker
 * reads. Declared structurally so the real `getContexts` result is assignable
 * without coupling the pure helper to the full Chrome type.
 */
export interface TabContext {
  contextType: string;
  documentUrl?: string;
  tabId: number;
  windowId: number;
}

/**
 * True when `documentUrl` is the dashboard page — exact match, or the dashboard
 * URL followed by a `#`fragment or `?`query. A bare `startsWith` would also
 * accept a `dashboard.html2` lookalike, so the separator is required.
 */
function isDashboardUrl(documentUrl: string | undefined, dashboardUrl: string): boolean {
  if (!documentUrl) return false;
  return (
    documentUrl === dashboardUrl ||
    documentUrl.startsWith(`${dashboardUrl}#`) ||
    documentUrl.startsWith(`${dashboardUrl}?`)
  );
}

/**
 * Find the first already-open dashboard tab among extension contexts, so a
 * toolbar click can focus it instead of duplicating it. Only `TAB` contexts
 * count; the caller passes the URL from `chrome.runtime.getURL("dashboard.html")`.
 * Returns `null` when none match (the caller then opens a fresh tab). Pure:
 * matching against `getContexts` avoids `tabs.query({url})`, which would require
 * the "tabs" permission the extension deliberately never requests.
 */
export function findDashboardTab(
  contexts: readonly TabContext[],
  dashboardUrl: string
): DashboardTabRef | null {
  for (const c of contexts) {
    if (
      c.contextType === "TAB" &&
      c.tabId >= 0 &&
      isDashboardUrl(c.documentUrl, dashboardUrl)
    ) {
      return { tabId: c.tabId, windowId: c.windowId };
    }
  }
  return null;
}
