//! IndexedDB access via rexie (§7.8): id-range event reads, rollup/session
//! read-merge-write, the derive cursor, layout positions, privacy deletes, and
//! local export/import.
//!
//! The DB is opened only after the SW readiness ack (§6.4); the schema mirrors
//! `idb.ts` (the SW remains the authoritative owner) as a self-sufficient guard.

use crate::model::Event;
use crate::rollup::{merge_bucket, DayBucket, DeriveState, SessionRec};
use std::collections::HashMap;
use wasm_bindgen::JsValue;

use rexie::{KeyRange, ObjectStore, Rexie, TransactionMode};

fn je<E: std::fmt::Display>(e: E) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// Internal `meta` record shape (`keyPath: "key"`). Large/structured values are
/// stored as a JSON string for round-trip fidelity (avoids Map/Object ambiguity).
#[derive(serde::Serialize, serde::Deserialize)]
struct MetaRecord {
    key: String,
    json: String,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Cursor {
    watermark: f64,
    #[serde(rename = "deriveState")]
    derive_state: DeriveState,
}

pub struct Db {
    rexie: Rexie,
}

const STORES: [&str; 5] = ["events", "spa", "rollup_days", "sessions", "meta"];

/// Open the database (after the SW readiness ack). Mirrors the §4 schema so it is
/// also self-sufficient if the stores somehow do not yet exist.
pub async fn open() -> Result<Db, JsValue> {
    let rexie = Rexie::builder("browsing_graph")
        .version(1)
        .add_object_store(
            ObjectStore::new("events")
                .key_path("id")
                .auto_increment(true),
        )
        .add_object_store(ObjectStore::new("spa").key_path("id").auto_increment(true))
        .add_object_store(ObjectStore::new("rollup_days").key_path("date"))
        .add_object_store(ObjectStore::new("sessions").key_path("id"))
        .add_object_store(ObjectStore::new("meta").key_path("key"))
        .build()
        .await
        .map_err(je)?;
    Ok(Db { rexie })
}

impl Db {
    // ── events ─────────────────────────────────────────────────────────────────

    /// Read events with `id > watermark`, ascending (the fold cursor, §4.1).
    pub async fn read_events_after(&self, watermark: f64) -> Result<Vec<Event>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["events"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("events").map_err(je)?;
        let range = KeyRange::lower_bound(&JsValue::from_f64(watermark), Some(true)).map_err(je)?;
        let values = store.get_all(Some(range), None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    /// Read events in an inclusive id range (the Sankey cursor from
    /// `sessions.{startId,endId}`, §4.4).
    pub async fn read_events_id_range(
        &self,
        start_id: f64,
        end_id: f64,
    ) -> Result<Vec<Event>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["events"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("events").map_err(je)?;
        let range = KeyRange::bound(
            &JsValue::from_f64(start_id),
            &JsValue::from_f64(end_id),
            Some(false),
            Some(false),
        )
        .map_err(je)?;
        let values = store.get_all(Some(range), None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    /// Read the first `limit` events (ascending id) without materializing the whole
    /// store — for the bounded Raw view. Pair with [`Self::count_events`] for the total.
    pub async fn read_events_head(&self, limit: u32) -> Result<Vec<Event>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["events"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("events").map_err(je)?;
        let values = store.get_all(None, Some(limit)).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    pub async fn count_events(&self) -> Result<u32, JsValue> {
        self.count_store("events").await
    }

    /// Record count of the `spa` (history-state navigations) store.
    pub async fn count_spa(&self) -> Result<u32, JsValue> {
        self.count_store("spa").await
    }

    /// Record count of the derived `rollup_days` cache (one row per UTC day).
    pub async fn count_rollup_days(&self) -> Result<u32, JsValue> {
        self.count_store("rollup_days").await
    }

    /// Record count of the derived `sessions` index.
    pub async fn count_sessions(&self) -> Result<u32, JsValue> {
        self.count_store("sessions").await
    }

    async fn count_store(&self, store_name: &str) -> Result<u32, JsValue> {
        let tx = self
            .rexie
            .transaction(&[store_name], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store(store_name).map_err(je)?;
        let n = store.count(None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        Ok(n)
    }

    /// Read all `spa` (history-state) records for the opt-in in-app navs view
    /// (§4.2). They are `Nav`-shaped.
    pub async fn read_spa(&self) -> Result<Vec<Event>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["spa"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("spa").map_err(je)?;
        let values = store.get_all(None, None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    // ── rollups ────────────────────────────────────────────────────────────────

    pub async fn read_all_rollups(&self) -> Result<Vec<DayBucket>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["rollup_days"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("rollup_days").map_err(je)?;
        let values = store.get_all(None, None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    /// Merge fold deltas into the touched `rollup_days` buckets (read-modify-write).
    pub async fn write_rollups(&self, deltas: &[DayBucket]) -> Result<(), JsValue> {
        if deltas.is_empty() {
            return Ok(());
        }
        let tx = self
            .rexie
            .transaction(&["rollup_days"], TransactionMode::ReadWrite)
            .map_err(je)?;
        let store = tx.store("rollup_days").map_err(je)?;
        for delta in deltas {
            let existing = store
                .get(JsValue::from_str(&delta.date))
                .await
                .map_err(je)?;
            let mut bucket: DayBucket = match existing {
                Some(v) if !v.is_undefined() && !v.is_null() => {
                    serde_wasm_bindgen::from_value(v).map_err(je)?
                }
                _ => DayBucket {
                    date: delta.date.clone(),
                    ..Default::default()
                },
            };
            merge_bucket(&mut bucket, delta);
            let val = serde_wasm_bindgen::to_value(&bucket).map_err(je)?;
            store.put(&val, None).await.map_err(je)?;
        }
        tx.done().await.map_err(je)?;
        Ok(())
    }

    // ── sessions ───────────────────────────────────────────────────────────────

    pub async fn read_sessions(&self) -> Result<Vec<SessionRec>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["sessions"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("sessions").map_err(je)?;
        let values = store.get_all(None, None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        deserialize_all(values)
    }

    pub async fn write_sessions(&self, recs: &[SessionRec]) -> Result<(), JsValue> {
        if recs.is_empty() {
            return Ok(());
        }
        let tx = self
            .rexie
            .transaction(&["sessions"], TransactionMode::ReadWrite)
            .map_err(je)?;
        let store = tx.store("sessions").map_err(je)?;
        for r in recs {
            let val = serde_wasm_bindgen::to_value(r).map_err(je)?;
            store.put(&val, None).await.map_err(je)?;
        }
        tx.done().await.map_err(je)?;
        Ok(())
    }

    // ── meta: derive cursor + positions ─────────────────────────────────────────

    pub async fn read_cursor(&self) -> Result<(f64, DeriveState), JsValue> {
        match self.read_meta_json("rollupCursor").await? {
            Some(json) => {
                let c: Cursor = serde_json::from_str(&json).map_err(je)?;
                Ok((c.watermark, c.derive_state))
            }
            None => Ok((0.0, DeriveState::default())),
        }
    }

    pub async fn write_cursor(&self, watermark: f64, state: &DeriveState) -> Result<(), JsValue> {
        let c = Cursor {
            watermark,
            derive_state: state.clone(),
        };
        let json = serde_json::to_string(&c).map_err(je)?;
        self.write_meta_json("rollupCursor", &json).await
    }

    pub async fn read_positions(&self) -> Result<HashMap<String, (f32, f32)>, JsValue> {
        match self.read_meta_json("positions").await? {
            Some(json) => serde_json::from_str(&json).map_err(je),
            None => Ok(HashMap::new()),
        }
    }

    pub async fn write_positions(&self, pos: &HashMap<String, (f32, f32)>) -> Result<(), JsValue> {
        let json = serde_json::to_string(pos).map_err(je)?;
        self.write_meta_json("positions", &json).await
    }

    async fn read_meta_json(&self, key: &str) -> Result<Option<String>, JsValue> {
        let tx = self
            .rexie
            .transaction(&["meta"], TransactionMode::ReadOnly)
            .map_err(je)?;
        let store = tx.store("meta").map_err(je)?;
        let v = store.get(JsValue::from_str(key)).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        match v {
            Some(v) if !v.is_undefined() && !v.is_null() => {
                let rec: MetaRecord = serde_wasm_bindgen::from_value(v).map_err(je)?;
                Ok(Some(rec.json))
            }
            _ => Ok(None),
        }
    }

    async fn write_meta_json(&self, key: &str, json: &str) -> Result<(), JsValue> {
        let tx = self
            .rexie
            .transaction(&["meta"], TransactionMode::ReadWrite)
            .map_err(je)?;
        let store = tx.store("meta").map_err(je)?;
        let rec = MetaRecord {
            key: key.to_string(),
            json: json.to_string(),
        };
        let val = serde_wasm_bindgen::to_value(&rec).map_err(je)?;
        store.put(&val, None).await.map_err(je)?;
        tx.done().await.map_err(je)?;
        Ok(())
    }

    /// Read an epoch-ms timestamp from a `meta` row (a plain JSON number). Absent
    /// or unparsable → `None`, so a missing/corrupt key just reads as "never".
    async fn read_meta_f64(&self, key: &str) -> Result<Option<f64>, JsValue> {
        match self.read_meta_json(key).await? {
            Some(json) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    async fn write_meta_f64(&self, key: &str, value: f64) -> Result<(), JsValue> {
        let json = serde_json::to_string(&value).map_err(je)?;
        self.write_meta_json(key, &json).await
    }

    // ── onboarding markers (§F4): `onboarded` + `demoData`, stored in `meta` ─────

    /// Read a boolean `meta` flag (a plain JSON bool). Absent, unparsable, or
    /// non-boolean reads as `false` — used for the onboarding markers `onboarded`
    /// (welcome dismissed) and `demoData` (sample dataset loaded). Both live in the
    /// `meta` store, so [`Self::clear_all`] wipes them (Exit sample relies on this
    /// to re-surface the welcome overlay).
    pub async fn read_meta_bool(&self, key: &str) -> Result<bool, JsValue> {
        match self.read_meta_json(key).await? {
            Some(json) => Ok(serde_json::from_str::<bool>(&json).unwrap_or(false)),
            None => Ok(false),
        }
    }

    /// Write a boolean `meta` flag (see [`Self::read_meta_bool`]).
    pub async fn write_meta_bool(&self, key: &str, value: bool) -> Result<(), JsValue> {
        let json = serde_json::to_string(&value).map_err(je)?;
        self.write_meta_json(key, &json).await
    }

    // ── backup-nudge state (§8.4): timestamps in the `meta` store, no new store ──

    /// Last successful JSON export (epoch ms), or `None` if never exported.
    pub async fn read_last_export_ts(&self) -> Result<Option<f64>, JsValue> {
        self.read_meta_f64("lastExportTs").await
    }

    /// Stamp a successful JSON export so the backup nudge resets (§8.4).
    pub async fn write_last_export_ts(&self, ts: f64) -> Result<(), JsValue> {
        self.write_meta_f64("lastExportTs", ts).await
    }

    /// Instant (epoch ms) until which the backup nudge is snoozed, or `None`.
    pub async fn read_nudge_snoozed_until(&self) -> Result<Option<f64>, JsValue> {
        self.read_meta_f64("nudgeSnoozedUntil").await
    }

    /// Snooze the backup nudge until `until` (epoch ms).
    pub async fn write_nudge_snoozed_until(&self, until: f64) -> Result<(), JsValue> {
        self.write_meta_f64("nudgeSnoozedUntil", until).await
    }

    // ── privacy: forget / delete / export / import (§8) ─────────────────────────

    /// Delete every event/spa record whose URL host matches `domain` (hostname or
    /// registrable), then clear the rollup + cursor for a lazy rebuild (§7.8).
    pub async fn forget_domain(&self, domain: &str) -> Result<(), JsValue> {
        self.delete_events_where(|e| event_matches_host(e, domain))
            .await?;
        self.reset_derivation().await
    }

    /// Delete every event/spa record whose `ts` falls in `[from_ts, to_ts]`, then
    /// invalidate the rollup cache (§7.8).
    pub async fn delete_range(&self, from_ts: f64, to_ts: f64) -> Result<(), JsValue> {
        self.delete_events_where(|e| {
            let ts = e.ts();
            ts >= from_ts && ts <= to_ts
        })
        .await?;
        self.reset_derivation().await
    }

    async fn delete_events_where<F: Fn(&Event) -> bool>(&self, pred: F) -> Result<(), JsValue> {
        for store_name in ["events", "spa"] {
            let tx = self
                .rexie
                .transaction(&[store_name], TransactionMode::ReadWrite)
                .map_err(je)?;
            let store = tx.store(store_name).map_err(je)?;
            let pairs = store.scan(None, None, None, None).await.map_err(je)?;
            for (key, value) in pairs {
                if let Ok(ev) = serde_wasm_bindgen::from_value::<Event>(value) {
                    if pred(&ev) {
                        store.delete(key).await.map_err(je)?;
                    }
                }
            }
            tx.done().await.map_err(je)?;
        }
        Ok(())
    }

    /// Wipe **every** store — `events`, `spa`, `rollup_days`, `sessions`, and all
    /// of `meta` (derive cursor, layout positions, `lastExportTs`, snooze) — for
    /// the "Delete all data" control (§8). Preferences in `chrome.storage.local`
    /// (pause / uiPrefs / savedViews) live outside IndexedDB and are deliberately
    /// left intact. Still local-only: this only clears storage, it sends nothing.
    pub async fn clear_all(&self) -> Result<(), JsValue> {
        let tx = self
            .rexie
            .transaction(&STORES, TransactionMode::ReadWrite)
            .map_err(je)?;
        for store_name in STORES {
            tx.store(store_name)
                .map_err(je)?
                .clear()
                .await
                .map_err(je)?;
        }
        tx.done().await.map_err(je)?;
        Ok(())
    }

    /// Clear `rollup_days` + `sessions` and reset the derive cursor so the next
    /// open rebuilds the whole graph from the raw `events` (destructive-edit
    /// invalidation, §4.3; also the manual "Rebuild from raw events" recovery).
    pub async fn reset_derivation(&self) -> Result<(), JsValue> {
        let tx = self
            .rexie
            .transaction(&["rollup_days", "sessions"], TransactionMode::ReadWrite)
            .map_err(je)?;
        tx.store("rollup_days")
            .map_err(je)?
            .clear()
            .await
            .map_err(je)?;
        tx.store("sessions")
            .map_err(je)?
            .clear()
            .await
            .map_err(je)?;
        tx.done().await.map_err(je)?;
        self.write_cursor(0.0, &DeriveState::default()).await
    }

    /// Export every store as a JSON document (a local download; never a network
    /// call — see the §12.1 local-only guarantee). DO NOT add a network sink here.
    pub async fn export_json(&self) -> Result<String, JsValue> {
        let mut doc = serde_json::Map::new();
        doc.insert("version".into(), serde_json::json!(1));
        for store_name in STORES {
            let tx = self
                .rexie
                .transaction(&[store_name], TransactionMode::ReadOnly)
                .map_err(je)?;
            let store = tx.store(store_name).map_err(je)?;
            let values = store.get_all(None, None).await.map_err(je)?;
            tx.done().await.map_err(je)?;
            let arr: Vec<serde_json::Value> = values
                .into_iter()
                .map(serde_wasm_bindgen::from_value)
                .collect::<Result<_, _>>()
                .map_err(je)?;
            doc.insert(store_name.into(), serde_json::Value::Array(arr));
        }
        serde_json::to_string(&serde_json::Value::Object(doc)).map_err(je)
    }

    /// Replace all stores from an exported JSON document (local file only).
    pub async fn import_json(&self, json: &str) -> Result<(), JsValue> {
        let doc: serde_json::Value = serde_json::from_str(json).map_err(je)?;
        for store_name in STORES {
            let tx = self
                .rexie
                .transaction(&[store_name], TransactionMode::ReadWrite)
                .map_err(je)?;
            let store = tx.store(store_name).map_err(je)?;
            store.clear().await.map_err(je)?;
            if let Some(serde_json::Value::Array(items)) = doc.get(store_name) {
                for item in items {
                    let val = serde_wasm_bindgen::to_value(item).map_err(je)?;
                    store.put(&val, None).await.map_err(je)?;
                }
            }
            tx.done().await.map_err(je)?;
        }
        Ok(())
    }
}

fn deserialize_all<T: serde::de::DeserializeOwned>(
    values: Vec<JsValue>,
) -> Result<Vec<T>, JsValue> {
    values
        .into_iter()
        .map(|v| serde_wasm_bindgen::from_value(v).map_err(je))
        .collect()
}

fn event_matches_host(e: &Event, domain: &str) -> bool {
    if let Event::Nav { to_url, .. } = e {
        if let Some(h) = crate::interpret::host(to_url) {
            return h == domain || crate::interpret::registrable(&h) == domain;
        }
    }
    false
}
