// idb.ts — the **sole IndexedDB schema owner** (§4, §6.2).
//
// The service worker ensure-creates the DB on install and on the dashboard
// readiness ping, so by the time the dashboard's rexie opens the DB the stores
// already exist (§4, §6.4). No indexes are required: the fold cursors the
// primary key and time ranges are served by rollups, not raw scans (§4.1).

export const DB_NAME = "browsing_graph";
export const DB_VERSION = 1;

/** Open (and, on first open, create) the database with the §4 schema. */
export function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      // events — the unified, globally-ordered stream (§4.1).
      if (!db.objectStoreNames.contains("events")) {
        db.createObjectStore("events", { keyPath: "id", autoIncrement: true });
      }
      // spa — history-state navigations, separate high-volume store (§4.2).
      if (!db.objectStoreNames.contains("spa")) {
        db.createObjectStore("spa", { keyPath: "id", autoIncrement: true });
      }
      // rollup_days — derived UTC-day aggregates cache (§4.3).
      if (!db.objectStoreNames.contains("rollup_days")) {
        db.createObjectStore("rollup_days", { keyPath: "date" });
      }
      // sessions — derived session index (§4.4).
      if (!db.objectStoreNames.contains("sessions")) {
        db.createObjectStore("sessions", { keyPath: "id" });
      }
      // meta — rollup cursor, positions, prefs (§4.5).
      if (!db.objectStoreNames.contains("meta")) {
        db.createObjectStore("meta", { keyPath: "key" });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
    req.onblocked = () => reject(new Error("IndexedDB open blocked"));
  });
}

let dbPromise: Promise<IDBDatabase> | null = null;

function db(): Promise<IDBDatabase> {
  if (!dbPromise) dbPromise = openDb();
  return dbPromise;
}

/** Idempotent ensure-create for the readiness path (§4, §6.2). */
export async function ensureCreated(): Promise<void> {
  await db();
}

/** Append one record to a store, resolving with its assigned key. */
export async function idbAdd(store: string, record: unknown): Promise<number> {
  const d = await db();
  return new Promise<number>((resolve, reject) => {
    const tx = d.transaction(store, "readwrite");
    const req = tx.objectStore(store).add(record);
    req.onsuccess = () => resolve(req.result as number);
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error ?? new Error("transaction aborted"));
  });
}
