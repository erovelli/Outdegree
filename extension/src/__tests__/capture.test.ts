import { describe, expect, it } from "vitest";
import {
  closeRecord,
  flagOn,
  linkRecord,
  navRecord,
  startRecord,
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
