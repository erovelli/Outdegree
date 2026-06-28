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
