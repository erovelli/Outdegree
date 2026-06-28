import "fake-indexeddb/auto";
import { IDBFactory } from "fake-indexeddb";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Each test gets a fresh in-memory IndexedDB and a fresh module cache (idb.ts
// memoizes its open-db promise), so id sequences and schema state don't leak.
beforeEach(() => {
  (globalThis as { indexedDB: IDBFactory }).indexedDB = new IDBFactory();
  vi.resetModules();
});

describe("idb schema (the sole IndexedDB schema owner, §4)", () => {
  it("creates the five §4 object stores on first open", async () => {
    const { openDb } = await import("../idb");
    const db = await openDb();
    expect([...db.objectStoreNames].sort()).toEqual([
      "events",
      "meta",
      "rollup_days",
      "sessions",
      "spa",
    ]);
    db.close();
  });

  it("assigns ascending ids to appended events (global ordering, §4.1)", async () => {
    const { idbAdd } = await import("../idb");
    const id1 = await idbAdd("events", { kind: "start", ts: 1 });
    const id2 = await idbAdd("events", { kind: "nav", ts: 2 });
    const id3 = await idbAdd("events", { kind: "close", ts: 3 });
    expect(id1).toBeLessThan(id2);
    expect(id2).toBeLessThan(id3);
  });

  it("ensureCreated is idempotent (the readiness path, §6.4)", async () => {
    const { ensureCreated, openDb } = await import("../idb");
    await ensureCreated();
    await ensureCreated();
    const db = await openDb();
    expect(db.objectStoreNames.contains("events")).toBe(true);
    expect(db.objectStoreNames.contains("spa")).toBe(true);
    db.close();
  });

  it("rejects on a write to an unknown store", async () => {
    const { idbAdd } = await import("../idb");
    await expect(idbAdd("nope", { x: 1 })).rejects.toBeTruthy();
  });
});
