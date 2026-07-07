import { describe, expect, it } from "vitest";
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
} from "../capture";

describe("flagOn (pause-flag parsing)", () => {
  it("treats true / \"true\" / \"1\" as on", () => {
    expect(flagOn(true)).toBe(true);
    expect(flagOn("true")).toBe(true);
    expect(flagOn("1")).toBe(true);
  });

  it("treats the string \"false\" as off (the regression that wedged capture)", () => {
    expect(flagOn("false")).toBe(false);
  });

  it("treats everything else as off", () => {
    expect(flagOn(false)).toBe(false);
    expect(flagOn(undefined)).toBe(false);
    expect(flagOn(null)).toBe(false);
    expect(flagOn("")).toBe(false);
    expect(flagOn("0")).toBe(false);
    expect(flagOn(1)).toBe(false); // storage values are strings, not numbers
  });
});

describe("navRecord (the events/spa contract)", () => {
  const detail = {
    tabId: 7,
    url: "https://example.com/page",
    transitionType: "link",
    transitionQualifiers: ["client_redirect"],
    timeStamp: 1000,
  };

  it("produces the exact camelCase shape the Rust Event::Nav expects", () => {
    expect(navRecord(detail, 3, 9999)).toEqual({
      kind: "nav",
      ts: 1000,
      tabId: 7,
      windowId: 3,
      toUrl: "https://example.com/page",
      transitionType: "link",
      qualifiers: ["client_redirect"],
    });
  });

  it("falls back to the firing-time `now` when the event has no timeStamp", () => {
    const { timeStamp: _omit, ...noTs } = detail;
    expect(navRecord(noTs, 3, 9999).ts).toBe(9999);
  });
});

describe("linkRecord / closeRecord / startRecord", () => {
  it("links the new tab to its source (origin handoff, §7.3)", () => {
    expect(linkRecord({ tabId: 12, sourceTabId: 4, timeStamp: 500 }, 0)).toEqual({
      kind: "link",
      ts: 500,
      newTabId: 12,
      sourceTabId: 4,
    });
  });

  it("link falls back to `now` without a timeStamp", () => {
    expect(linkRecord({ tabId: 12, sourceTabId: 4 }, 777).ts).toBe(777);
  });

  it("shapes close and start markers", () => {
    expect(closeRecord(5, 200)).toEqual({ kind: "close", ts: 200, tabId: 5 });
    expect(startRecord(300)).toEqual({ kind: "start", ts: 300 });
  });
});

describe("focusRecord / wfocusRecord (§F7 foreground attention)", () => {
  it("shapes a tab activation into the exact Event::Focus contract", () => {
    expect(focusRecord({ tabId: 9, windowId: 2 }, 4321)).toEqual({
      kind: "focus",
      ts: 4321,
      tabId: 9,
      windowId: 2,
    });
  });

  it("shapes a window focus change into the exact Event::Wfocus contract", () => {
    expect(wfocusRecord(3, 5000)).toEqual({ kind: "wfocus", ts: 5000, windowId: 3 });
  });

  it("records WINDOW_ID_NONE (browser blurred) as -1", () => {
    expect(wfocusRecord(-1, 100)).toEqual({ kind: "wfocus", ts: 100, windowId: -1 });
  });

  it("normalizes any other negative sentinel to -1 (single 'no window' value)", () => {
    // WINDOW_ID_CURRENT (-2) should never be reported by onFocusChanged, but if
    // any negative id slips through it must read as "browser blurred", not as a
    // real window the derive layer would attribute time to.
    expect(wfocusRecord(-2, 100).windowId).toBe(-1);
  });

  it("carries only ids — no URL or title fields (privacy shape guard)", () => {
    expect(Object.keys(focusRecord({ tabId: 1, windowId: 1 }, 1)).sort()).toEqual([
      "kind",
      "tabId",
      "ts",
      "windowId",
    ]);
    expect(Object.keys(wfocusRecord(1, 1)).sort()).toEqual(["kind", "ts", "windowId"]);
  });
});

describe("badgeStateFor (paused toolbar indicator)", () => {
  it("shows a visible glyph and the paused title while capture is off", () => {
    const s = badgeStateFor(true);
    expect(s.text).not.toBe("");
    expect(s.title).toBe("Outdegree — capture paused");
  });

  it("clears the badge and restores the default title while capture runs", () => {
    expect(badgeStateFor(false)).toEqual({ text: "", title: "Open Outdegree" });
  });
});

describe("findDashboardTab (focus-existing-tab, no \"tabs\" permission)", () => {
  const DASH = "chrome-extension://abcdefghijklmnop/dashboard.html";

  it("returns the first open dashboard TAB, skipping unrelated tabs", () => {
    const contexts = [
      { contextType: "BACKGROUND", tabId: -1, windowId: -1 },
      { contextType: "TAB", documentUrl: "https://example.com/", tabId: 3, windowId: 1 },
      { contextType: "TAB", documentUrl: DASH, tabId: 7, windowId: 2 },
      { contextType: "TAB", documentUrl: DASH, tabId: 9, windowId: 5 },
    ];
    expect(findDashboardTab(contexts, DASH)).toEqual({ tabId: 7, windowId: 2 });
  });

  it("matches the dashboard even with a trailing #fragment or ?query", () => {
    expect(
      findDashboardTab(
        [{ contextType: "TAB", documentUrl: `${DASH}#graph`, tabId: 4, windowId: 1 }],
        DASH
      )
    ).toEqual({ tabId: 4, windowId: 1 });
    expect(
      findDashboardTab(
        [{ contextType: "TAB", documentUrl: `${DASH}?range=day`, tabId: 5, windowId: 1 }],
        DASH
      )
    ).toEqual({ tabId: 5, windowId: 1 });
  });

  it("returns null when no dashboard tab is open", () => {
    expect(
      findDashboardTab(
        [{ contextType: "TAB", documentUrl: "https://example.com/", tabId: 3, windowId: 1 }],
        DASH
      )
    ).toBeNull();
  });

  it("ignores non-TAB contexts and prefix-only lookalikes", () => {
    const contexts = [
      // Same URL, but a POPUP context — must not be treated as a tab.
      { contextType: "POPUP", documentUrl: DASH, tabId: 2, windowId: 1 },
      // dashboard.html2 shares a prefix but is a different page — must not match.
      { contextType: "TAB", documentUrl: `${DASH}2`, tabId: 8, windowId: 1 },
    ];
    expect(findDashboardTab(contexts, DASH)).toBeNull();
  });
});
