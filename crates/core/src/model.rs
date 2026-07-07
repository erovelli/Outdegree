//! Core data model: the captured `Event` stream (the IndexedDB contract, §4.1)
//! plus the derived provenance / edge-kind taxonomy and graph aggregates.
//!
//! Everything here is pure and target-agnostic so it compiles under `cargo test`.

use serde::{Deserialize, Serialize};

/// The unified, globally-ordered capture stream (§4.1).
///
/// All capture kinds share one `id` auto-increment sequence in the `events`
/// store so the read-time pass can order them globally (§7.3). `id`/`ts`/ids
/// are `f64` because they cross the JS boundary as IndexedDB numbers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename = "nav")]
    Nav {
        id: f64,
        ts: f64,
        #[serde(rename = "tabId")]
        tab_id: f64,
        #[serde(rename = "windowId")]
        window_id: f64,
        #[serde(rename = "toUrl")]
        to_url: String,
        #[serde(rename = "transitionType")]
        transition_type: String,
        qualifiers: Vec<String>,
    },
    #[serde(rename = "link")]
    Link {
        id: f64,
        ts: f64,
        #[serde(rename = "newTabId")]
        new_tab_id: f64,
        #[serde(rename = "sourceTabId")]
        source_tab_id: f64,
    },
    #[serde(rename = "close")]
    Close {
        id: f64,
        ts: f64,
        #[serde(rename = "tabId")]
        tab_id: f64,
    },
    /// Tab activation (`chrome.tabs.onActivated`): which tab became active in a
    /// window. Feeds foreground attribution (§F7); carries only ids, so it needs
    /// no permission beyond the existing set.
    #[serde(rename = "focus")]
    Focus {
        id: f64,
        ts: f64,
        #[serde(rename = "tabId")]
        tab_id: f64,
        #[serde(rename = "windowId")]
        window_id: f64,
    },
    /// Window focus change (`chrome.windows.onFocusChanged`): which window is
    /// focused, or `windowId == -1` (Chrome's `WINDOW_ID_NONE`) when the browser
    /// itself is blurred (§F7).
    #[serde(rename = "wfocus")]
    Wfocus {
        id: f64,
        ts: f64,
        #[serde(rename = "windowId")]
        window_id: f64,
    },
    #[serde(rename = "start")]
    Start { id: f64, ts: f64 },
    /// Forward-compat catch-all (§F7): an event `kind` this build does not
    /// recognize (an older dashboard reading a log written by a newer version).
    /// It deserializes here instead of failing the whole batch; the read layer
    /// drops it and the fold skips it, so the dashboard degrades gracefully.
    #[serde(other)]
    Unknown,
}

impl Event {
    /// Global ordering key (the `events` primary key). `Unknown` (a forward-compat
    /// unrecognized kind) has no id in this build; it is dropped before it reaches
    /// any id-consuming path, so the `NAN` here is never observed.
    pub fn id(&self) -> f64 {
        match self {
            Event::Nav { id, .. }
            | Event::Link { id, .. }
            | Event::Close { id, .. }
            | Event::Focus { id, .. }
            | Event::Wfocus { id, .. }
            | Event::Start { id, .. } => *id,
            Event::Unknown => f64::NAN,
        }
    }

    pub fn ts(&self) -> f64 {
        match self {
            Event::Nav { ts, .. }
            | Event::Link { ts, .. }
            | Event::Close { ts, .. }
            | Event::Focus { ts, .. }
            | Event::Wfocus { ts, .. }
            | Event::Start { ts, .. } => *ts,
            Event::Unknown => f64::NAN,
        }
    }
}

/// How a page was arrived at (derived from `transitionType`, §7.2).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Provenance {
    Link,
    Form,
    TypedUrl,
    SearchOrigin,
    Bookmark,
    Start,
    Reload,
    Other,
}

impl Provenance {
    /// `Reload`/`Other` produce no node, edge, or per-tab state change (§7.3 step 2).
    pub fn is_ignored(self) -> bool {
        matches!(self, Provenance::Reload | Provenance::Other)
    }

    /// Only `Link`/`Form` traversals produce a graph edge (§7.2 "Edge?" column).
    pub fn is_edge(self) -> bool {
        matches!(self, Provenance::Link | Provenance::Form)
    }

    /// "Rootless" provenances emit a node visit but no edge and reset the
    /// per-tab chain (typed/search/bookmark/start).
    pub fn is_rootless(self) -> bool {
        matches!(
            self,
            Provenance::TypedUrl
                | Provenance::SearchOrigin
                | Provenance::Bookmark
                | Provenance::Start
        )
    }

    /// Node fill color for the canvas2d renderer (§7.7). Pure so it is testable
    /// and shared with any future renderer.
    ///
    /// The palette is the single blue→violet→magenta→red OKLCH spectrum that is
    /// the *only* color in the product (the data-encoding hue ramp from the
    /// design handoff). Lightness/chroma are held constant (`L≈0.64 C≈0.205`) so
    /// the categories read as evenly-spaced stops on one continuous bar.
    pub fn color(self) -> &'static str {
        match self {
            Provenance::SearchOrigin => "oklch(0.64 0.205 264)",
            // "External" (start_page). A colder cyan-blue that stands clear of
            // Search's periwinkle (264), extending the cold end of the ramp.
            Provenance::Start => "oklch(0.64 0.205 210)",
            Provenance::Link => "oklch(0.64 0.205 288)",
            Provenance::TypedUrl => "oklch(0.64 0.205 312)",
            Provenance::Bookmark => "oklch(0.64 0.205 340)",
            Provenance::Reload => "oklch(0.64 0.205 350)",
            Provenance::Form => "oklch(0.64 0.205 8)",
            Provenance::Other => "oklch(0.64 0.205 30)",
        }
    }

    /// Fold the one category we don't surface separately (`Reload`) into `Other`
    /// for *display* — color, glyph, legend, callout. The data model still records
    /// the precise provenance; this only changes what the user sees.
    ///
    /// `Start` is **not** folded: in practice Chrome reports `start_page` for tabs
    /// opened from another application (the genuine browser-start page is
    /// `chrome://newtab/`, which we drop as non-http), so it surfaces as its own
    /// "External" category.
    pub fn display(self) -> Provenance {
        match self {
            Provenance::Reload => Provenance::Other,
            p => p,
        }
    }

    /// Marker shape encoding provenance *without* relying on color (CVD-safe
    /// redundant channel). Folds through [`Self::display`], so only the seven
    /// surfaced categories map to a shape.
    pub fn shape(self) -> Shape {
        match self.display() {
            Provenance::Link => Shape::Circle,
            Provenance::SearchOrigin => Shape::Triangle,
            Provenance::TypedUrl => Shape::Square,
            Provenance::Bookmark => Shape::Diamond,
            Provenance::Form => Shape::Hexagon,
            Provenance::Start => Shape::Star, // "External" — opened from another app
            _ => Shape::Cross,                // Other (and folded Reload)
        }
    }
}

/// A node-marker shape. Provenance is drawn as one of these so the encoding
/// survives color-vision deficiency (shape is redundant with hue). Pure geometry
/// so both the canvas and the SVG Sankey can share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Circle,
    Square,
    Triangle,
    Diamond,
    Hexagon,
    Cross,
    Star,
}

impl Shape {
    /// Polygon vertices for this marker centered at `(cx, cy)` with "radius" `r`.
    /// `None` means a circle (the caller draws it with an arc / `<circle>`).
    pub fn points(self, cx: f64, cy: f64, r: f64) -> Option<Vec<(f64, f64)>> {
        let pts = match self {
            Shape::Circle => return None,
            Shape::Square => {
                let s = r * 0.86;
                vec![
                    (cx - s, cy - s),
                    (cx + s, cy - s),
                    (cx + s, cy + s),
                    (cx - s, cy + s),
                ]
            }
            Shape::Triangle => vec![
                (cx, cy - r),
                (cx + r * 0.87, cy + r * 0.55),
                (cx - r * 0.87, cy + r * 0.55),
            ],
            Shape::Diamond => vec![(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)],
            Shape::Hexagon => vec![
                (cx - r * 0.5, cy - r * 0.87),
                (cx + r * 0.5, cy - r * 0.87),
                (cx + r, cy),
                (cx + r * 0.5, cy + r * 0.87),
                (cx - r * 0.5, cy + r * 0.87),
                (cx - r, cy),
            ],
            Shape::Cross => {
                let a = r * 0.4;
                vec![
                    (cx - a, cy - r),
                    (cx + a, cy - r),
                    (cx + a, cy - a),
                    (cx + r, cy - a),
                    (cx + r, cy + a),
                    (cx + a, cy + a),
                    (cx + a, cy + r),
                    (cx - a, cy + r),
                    (cx - a, cy + a),
                    (cx - r, cy + a),
                    (cx - r, cy - a),
                    (cx - a, cy - a),
                ]
            }
            Shape::Star => {
                // 5-point star: alternate outer (r) / inner (0.5r) vertices,
                // starting at the top point.
                let inner = r * 0.5;
                (0..10)
                    .map(|i| {
                        let ang =
                            -std::f64::consts::FRAC_PI_2 + (i as f64) * std::f64::consts::PI / 5.0;
                        let rad = if i % 2 == 0 { r } else { inner };
                        (cx + rad * ang.cos(), cy + rad * ang.sin())
                    })
                    .collect()
            }
        };
        Some(pts)
    }

    /// CSS class used to clip a legend/key swatch into this shape (see
    /// `dashboard.css`). The canvas/SVG draw the polygon directly instead.
    pub fn css(self) -> &'static str {
        match self {
            Shape::Circle => "glyph-circle",
            Shape::Square => "glyph-square",
            Shape::Triangle => "glyph-triangle",
            Shape::Diamond => "glyph-diamond",
            Shape::Hexagon => "glyph-hexagon",
            Shape::Cross => "glyph-cross",
            Shape::Star => "glyph-star",
        }
    }
}

/// The kind of a traversal edge (§7.1). Coloring is per-traversal then aggregated
/// to the dominant kind (decision #1).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum EdgeKind {
    Link,
    SearchLink,
    Form,
}

impl EdgeKind {
    /// Edge stroke color — the same spectrum as node fills but slightly desaturated
    /// (`L≈0.6 C≈0.14`) so edges recede behind the nodes they connect.
    pub fn color(self) -> &'static str {
        match self {
            EdgeKind::SearchLink => "oklch(0.6 0.14 264)",
            EdgeKind::Link => "oklch(0.6 0.14 288)",
            EdgeKind::Form => "oklch(0.6 0.14 8)",
        }
    }
}

/// Per-node provenance histogram. Stored in `rollup_days` (§4.3) and summed at
/// merge; `dominant()` drives node fill.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvBreakdown {
    pub link: u32,
    pub form: u32,
    #[serde(rename = "typedUrl")]
    pub typed_url: u32,
    #[serde(rename = "searchOrigin")]
    pub search_origin: u32,
    pub bookmark: u32,
    pub start: u32,
    pub reload: u32,
    pub other: u32,
}

impl ProvBreakdown {
    pub fn add(&mut self, p: Provenance) {
        match p {
            Provenance::Link => self.link += 1,
            Provenance::Form => self.form += 1,
            Provenance::TypedUrl => self.typed_url += 1,
            Provenance::SearchOrigin => self.search_origin += 1,
            Provenance::Bookmark => self.bookmark += 1,
            Provenance::Start => self.start += 1,
            Provenance::Reload => self.reload += 1,
            Provenance::Other => self.other += 1,
        }
    }

    pub fn merge(&mut self, o: &ProvBreakdown) {
        self.link += o.link;
        self.form += o.form;
        self.typed_url += o.typed_url;
        self.search_origin += o.search_origin;
        self.bookmark += o.bookmark;
        self.start += o.start;
        self.reload += o.reload;
        self.other += o.other;
    }

    /// Dominant provenance (decision #7). Ties broken by the fixed declaration
    /// order below for determinism.
    pub fn dominant(&self) -> Provenance {
        let ranked = [
            (Provenance::Link, self.link),
            (Provenance::Form, self.form),
            (Provenance::TypedUrl, self.typed_url),
            (Provenance::SearchOrigin, self.search_origin),
            (Provenance::Bookmark, self.bookmark),
            (Provenance::Start, self.start),
            (Provenance::Reload, self.reload),
            (Provenance::Other, self.other),
        ];
        let mut best = ranked[0];
        for &(p, c) in &ranked[1..] {
            if c > best.1 {
                best = (p, c);
            }
        }
        best.0
    }
}

/// Per-edge kind histogram. Stored in `rollup_days` (§4.3); `dominant()` drives
/// edge color (decision #1, #7).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KindBreakdown {
    pub link: u32,
    #[serde(rename = "searchLink")]
    pub search_link: u32,
    pub form: u32,
}

impl KindBreakdown {
    pub fn add(&mut self, k: EdgeKind) {
        match k {
            EdgeKind::Link => self.link += 1,
            EdgeKind::SearchLink => self.search_link += 1,
            EdgeKind::Form => self.form += 1,
        }
    }

    pub fn merge(&mut self, o: &KindBreakdown) {
        self.link += o.link;
        self.search_link += o.search_link;
        self.form += o.form;
    }

    pub fn dominant(&self) -> EdgeKind {
        let ranked = [
            (EdgeKind::Link, self.link),
            (EdgeKind::SearchLink, self.search_link),
            (EdgeKind::Form, self.form),
        ];
        let mut best = ranked[0];
        for &(k, c) in &ranked[1..] {
            if c > best.1 {
                best = (k, c);
            }
        }
        best.0
    }
}

/// A node in a projected graph view (§7.1).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeAgg {
    pub key: String,
    pub visits: u32,
    pub prov: ProvBreakdown,
    /// Total time the page was the active page across all visits, in milliseconds
    /// (derived from inter-event timestamp gaps, capped per visit at the idle gap).
    /// Carried alongside `visits` so the UI can rank/size by attention, not just hits.
    #[serde(default)]
    pub dwell_ms: u64,
    /// Total *foreground* time (§F7): time this host was loaded in the focused
    /// window's active tab, attributed per inter-event interval and capped at the
    /// idle gap. `0` for data captured before focus tracking (see `has_focus_signal`).
    #[serde(default)]
    pub fg_dwell_ms: u64,
}

/// An edge in a projected graph view (§7.1).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EdgeAgg {
    pub from: String,
    pub to: String,
    pub weight: u32,
    pub kinds: KindBreakdown,
}

/// A fully merged + filtered graph ready for layout/render (§7.1).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphProjection {
    pub nodes: Vec<NodeAgg>,
    pub edges: Vec<EdgeAgg>,
}

/// Node-key granularity (decision #9). `Hostname` is the storage + default
/// granularity; `Registrable` (eTLD+1) is regrouped at merge time.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum Granularity {
    #[default]
    Hostname,
    Registrable,
}
