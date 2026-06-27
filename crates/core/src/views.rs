//! Saved/named views: a small, persisted list of view configurations (time
//! range + display filters + node granularity + in-app-nav mode) the user can
//! name, re-apply, and delete. Stored locally as JSON under one
//! `chrome.storage.local` key — no network, no new permission.
//!
//! Kept pure so the (de)serialization, name de-duplication, and ordering are
//! unit-tested under `cargo test`; the dashboard's wasm glue only reads/writes
//! the JSON blob and maps a `SavedView` to/from the live `App` state.

use crate::model::Granularity;
use crate::project::{Filters, TimeRange};
use serde::{Deserialize, Serialize};

/// `chrome.storage.local` key the saved-views list is persisted under.
pub const STORAGE_KEY: &str = "savedViews";

/// One named snapshot of the dashboard's view controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedView {
    pub name: String,
    pub range: TimeRange,
    pub gran: Granularity,
    pub filters: Filters,
    /// The "in-app navigations" toggle (folds `events` + `spa`), captured so a
    /// saved view restores the same data source it was created against.
    #[serde(default)]
    pub spa_mode: bool,
}

/// Parse the persisted JSON list. Lenient: any malformed/absent blob yields an
/// empty list so a corrupt value can never break the dashboard.
pub fn parse(json: &str) -> Vec<SavedView> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Serialize the list for persistence.
pub fn serialize(views: &[SavedView]) -> String {
    serde_json::to_string(views).unwrap_or_else(|_| "[]".to_string())
}

/// Case-insensitive trimmed-name equality — the identity used for upsert/remove
/// so "Work" and "work " are the same saved view.
fn same_name(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Insert `view`, replacing any existing view with the same (case-insensitive)
/// name, then keep the list ordered by name (case-insensitive) for a stable,
/// predictable menu. Returns the updated list.
pub fn upsert(mut views: Vec<SavedView>, mut view: SavedView) -> Vec<SavedView> {
    view.name = view.name.trim().to_string(); // normalize: no stray whitespace
    views.retain(|v| !same_name(&v.name, &view.name));
    views.push(view);
    views.sort_by_key(|v| v.name.to_lowercase());
    views
}

/// Remove the view with the given (case-insensitive) name, if present.
pub fn remove(mut views: Vec<SavedView>, name: &str) -> Vec<SavedView> {
    views.retain(|v| !same_name(&v.name, name));
    views
}

/// Look up a view by (case-insensitive) name.
pub fn find<'a>(views: &'a [SavedView], name: &str) -> Option<&'a SavedView> {
    views.iter().find(|v| same_name(&v.name, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Provenance;

    fn view(name: &str, range: TimeRange) -> SavedView {
        SavedView {
            name: name.to_string(),
            range,
            gran: Granularity::Hostname,
            filters: Filters::default(),
            spa_mode: false,
        }
    }

    #[test]
    fn roundtrips_through_json() {
        let v = SavedView {
            name: "Work".into(),
            range: TimeRange::Week,
            gran: Granularity::Registrable,
            filters: Filters {
                min_visits: 3,
                hide_search_hubs: true,
                hide_isolated: true,
                provenance_in: Some(vec![Provenance::Link, Provenance::TypedUrl]),
            },
            spa_mode: true,
        };
        let list = vec![v.clone()];
        let json = serialize(&list);
        assert_eq!(parse(&json), list);
    }

    #[test]
    fn parse_is_lenient_on_garbage() {
        assert!(parse("").is_empty());
        assert!(parse("not json").is_empty());
        assert!(parse("{}").is_empty());
    }

    #[test]
    fn upsert_replaces_by_name_case_insensitively_and_sorts() {
        let mut list = vec![];
        list = upsert(list, view("Zebra", TimeRange::Day));
        list = upsert(list, view("apple", TimeRange::Week));
        // Same name (different case/whitespace) replaces, not duplicates.
        list = upsert(list, view(" ZEBRA ", TimeRange::Month));
        assert_eq!(list.len(), 2);
        // Sorted case-insensitively, names normalized (trimmed): apple, ZEBRA.
        assert_eq!(list[0].name, "apple");
        assert_eq!(list[1].name, "ZEBRA");
        assert_eq!(list[1].range, TimeRange::Month, "replaced, not appended");
    }

    #[test]
    fn remove_and_find_are_case_insensitive() {
        let list = vec![view("Reading", TimeRange::Year)];
        assert!(find(&list, "reading").is_some());
        let list = remove(list, "READING");
        assert!(list.is_empty());
    }
}
