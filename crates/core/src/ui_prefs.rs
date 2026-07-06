//! Persisted dashboard UI preferences (§7.7): the subset of view controls that
//! survive a reload, stored as one JSON document under a single
//! `chrome.storage.local` key, so the dashboard reopens the way it was left.
//!
//! Kept pure — like [`crate::views`] — so the (de)serialization round-trip and
//! the lenient-fallback behaviour are unit-tested under `cargo test`; the wasm
//! glue in `ui` only reads the blob on open and writes it back on every control
//! change. Reuses `TimeRange` / `Granularity` / the `Filters` fields verbatim so
//! the persisted shape matches what saved views already store.

use crate::model::Granularity;
use crate::project::TimeRange;
use serde::{Deserialize, Serialize};

/// `chrome.storage.local` key the UI-prefs document is persisted under.
pub const STORAGE_KEY: &str = "uiPrefs";

/// The persistable view — a strict subset of the dashboard's internal `View`.
/// The Raw-events view is deliberately excluded: it stays reachable only from the
/// settings menu, so it can never be persisted or restored as the landing view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrefView {
    #[default]
    Graph,
    Sankey,
    Tables,
}

/// The persisted view controls. Exactly the fields the spec calls for — the
/// view, time range, node granularity, the three display filters that survive a
/// reload, the in-app-navigations toggle, and the layout lock. Transient state
/// (focus/drill, camera, hover, the legend provenance filter, the selected
/// session/day, the session-picker query, and the Raw view) is intentionally
/// *not* stored.
///
/// Every field carries `#[serde(default)]` (container-level), so a document
/// missing any of them — one written by an older or newer build — still parses,
/// each absent field falling back to its current default. Unknown fields are
/// ignored (serde's default), keeping the format forward-compatible.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiPrefs {
    pub view: PrefView,
    pub time_range: TimeRange,
    pub granularity: Granularity,
    pub min_visits: u32,
    pub hide_search_hubs: bool,
    pub hide_isolated: bool,
    pub spa_mode: bool,
    pub locked: bool,
}

/// Parse the persisted JSON document. Lenient by construction: a malformed blob,
/// or one carrying an unknown enum value (e.g. a view this build doesn't know, or
/// a persisted `"raw"`), yields the all-defaults `UiPrefs` rather than an error —
/// so a corrupt or forward-dated value can never break the dashboard, it only
/// falls back to the default view state.
pub fn parse(json: &str) -> UiPrefs {
    serde_json::from_str(json).unwrap_or_default()
}

/// Serialize the prefs for persistence.
pub fn serialize(prefs: &UiPrefs) -> String {
    serde_json::to_string(prefs).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_json() {
        let prefs = UiPrefs {
            view: PrefView::Tables,
            time_range: TimeRange::Week,
            granularity: Granularity::Registrable,
            min_visits: 5,
            hide_search_hubs: true,
            hide_isolated: true,
            spa_mode: true,
            locked: true,
        };
        let json = serialize(&prefs);
        assert_eq!(parse(&json), prefs);
    }

    #[test]
    fn defaults_match_the_dashboard_defaults() {
        // The zero-config landing state: Graph + Year + Hostname + no filters.
        let d = UiPrefs::default();
        assert_eq!(d.view, PrefView::Graph);
        assert_eq!(d.time_range, TimeRange::Year);
        assert_eq!(d.granularity, Granularity::Hostname);
        assert_eq!(d.min_visits, 0);
        assert!(!d.hide_search_hubs && !d.hide_isolated && !d.spa_mode && !d.locked);
    }

    #[test]
    fn parse_is_lenient_on_garbage() {
        // Missing key / unparsable JSON → silent fall back to defaults.
        assert_eq!(parse(""), UiPrefs::default());
        assert_eq!(parse("not json"), UiPrefs::default());
        assert_eq!(parse("[]"), UiPrefs::default());
        assert_eq!(parse("{}"), UiPrefs::default());
    }

    #[test]
    fn unknown_enum_value_falls_back_to_defaults() {
        // A persisted Raw view (which `PrefView` can't represent) or any unknown
        // enum value can't parse, so the whole document falls back to defaults —
        // never an error, and never the Raw view.
        assert_eq!(
            parse(r#"{"view":"raw","timeRange":"week"}"#),
            UiPrefs::default()
        );
        assert_eq!(parse(r#"{"timeRange":"decade"}"#), UiPrefs::default());
    }

    #[test]
    fn missing_fields_default_individually() {
        // A partial document (older build, or hand-edited) restores the fields it
        // carries and defaults the rest.
        let p = parse(r#"{"timeRange":"month","minVisits":3,"hideIsolated":true}"#);
        assert_eq!(p.time_range, TimeRange::Month);
        assert_eq!(p.min_visits, 3);
        assert!(p.hide_isolated);
        // untouched fields keep their defaults
        assert_eq!(p.view, PrefView::Graph);
        assert_eq!(p.granularity, Granularity::Hostname);
        assert!(!p.hide_search_hubs && !p.spa_mode && !p.locked);
    }

    #[test]
    fn unknown_fields_are_ignored_for_forward_compat() {
        // A field a future build added is dropped; the known ones still apply.
        let p = parse(r#"{"view":"sankey","futureKnob":42,"theme":"dark"}"#);
        assert_eq!(p.view, PrefView::Sankey);
        assert_eq!(p.time_range, TimeRange::Year);
    }
}
