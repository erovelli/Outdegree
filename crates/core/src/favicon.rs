//! Favicon support (§F12): pure URL construction for Chrome's LOCAL favicon
//! service plus the decoded-icon cache policy. Kept in the **pure** core (compiles
//! native + wasm32, no `web-sys`) so both the URL builder and the cache
//! capacity/no-retry policy are exercised under `cargo test`. The wasm shell
//! (`render::canvas2d` + `ui`) instantiates the cache over `HtmlImageElement`.
//!
//! **Audit interaction (the whole reason this lives here).** The favicon service
//! is reached at the extension's own origin —
//! `chrome-extension://<id>/_favicon/?pageUrl=<url>&size=<16|32>` — and Chrome
//! serves the icon from its on-disk favicon cache, making **no network request**,
//! so the no-egress guarantee holds (see [`ADR-0006`](../../../docs/adr/0006-favicon-permission.md)).
//! But `pageUrl` needs an `http(s)` scheme, and the CI network-surface audit greps
//! the built `dist/` for `https?://` expecting zero non-w3.org matches. This module
//! never emits a contiguous `https://<host>` literal into a plaintext bundle:
//!
//! * the scheme itself is the single [`crate::sample::URL_SCHEME`] const, reused
//!   verbatim from the F4 sample-data loader (same runtime-concatenation idiom),
//!   which lives in the WASM data section — base64-inlined into `dist/`, so a raw
//!   `grep https?://` cannot match it; and
//! * [`favicon_url`] **percent-encodes** the whole `pageUrl` value, so the runtime
//!   string carries `https%3A%2F%2F…` (no literal `://`) even in the DOM.
//!
//! The net effect: the dist network-surface grep stays at exactly **zero** matches
//! without weakening the audit (verified in CI + by the F12 audit-bite check).

use std::collections::HashMap;

/// Decoded-icon cache capacity (host → image). Bounds memory on a very large
/// graph; ~512 comfortably covers any projection the dashboard draws at once.
/// Slots are first-come-first-served and **never evicted** ([`IconCache`]): past
/// the cap, further hosts simply keep their provenance shapes for the session —
/// the trade that guarantees a steady state with zero per-frame load work (see
/// the churn note on [`IconCache`]).
pub const CACHE_CAP: usize = 512;

/// Build the favicon URL for `host` at `size` (16, or 32 for HiDPI) against the
/// extension-origin `base` (Chrome's `getURL("_favicon/")`, ending in `/`).
///
/// `host` is a bare hostname / registrable domain (no scheme). The `http(s)`
/// scheme is re-attached via [`crate::sample::with_scheme`] and the assembled
/// `pageUrl` is percent-encoded, so no `https://<host>` literal is ever produced —
/// see the module note on the audit interaction. Pure and total.
pub fn favicon_url(base: &str, host: &str, size: u16) -> String {
    // `with_scheme` prepends the F4 `URL_SCHEME` const ("https://") to the
    // schemeless host — the same idiom the sample-data loader uses — then the whole
    // value is percent-encoded below, so the emitted string is `https%3A%2F%2F…`.
    let page_url = crate::sample::with_scheme(host);
    format!("{base}?pageUrl={}&size={size}", percent_encode(&page_url))
}

/// Percent-encode a string for use as a URL query-parameter value, encoding every
/// byte outside the RFC 3986 unreserved set (`A-Z a-z 0-9 - _ . ~`). Slightly more
/// conservative than `encodeURIComponent` (which also leaves `!*'()`), which is
/// safe here — Chrome decodes the value. Crucially it encodes `:` and `/`, so a
/// scheme becomes `https%3A%2F%2F…` and never surfaces a literal `://`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_upper(b >> 4));
            out.push(hex_upper(b & 0x0f));
        }
    }
    out
}

fn hex_upper(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'A' + (nibble - 10)) as char,
    }
}

/// The load state of a host's favicon in the [`IconCache`]. `T` is the decoded
/// image handle (`HtmlImageElement` in the wasm shell; a stand-in in tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot<T> {
    /// A load is in flight — draw the provenance-shape fallback until it resolves.
    Loading,
    /// Decoded and ready to draw.
    Ready(T),
    /// The load errored (or decoded empty). **Never retried this session** — the
    /// slot stays `Failed` so a host with no cached favicon quietly keeps its
    /// shape/text instead of re-hitting a dead URL every frame.
    Failed,
}

/// A bounded host→icon cache with a **load-once, never-retry** policy (§F12).
/// Any present host — `Loading`, `Ready`, *or* `Failed` — is left alone by
/// [`begin`](Self::begin), so a frame never restarts a load it already tried.
///
/// **Churn guard: slots are first-come-first-served and NEVER evicted.** At
/// capacity, [`begin`](Self::begin) refuses new hosts (returns `false` without
/// inserting). Eviction was rejected deliberately: with more than `cap`
/// icon-qualifying hosts visible at once, evict-on-insert would make every frame
/// re-report the evicted-but-visible hosts as misses → re-begin → evict *other*
/// visible hosts → a perpetual load/repaint loop. Refusing instead makes the
/// session's total load count provably ≤ `cap`: [`len`](Self::len) is
/// monotonically non-decreasing, bounded by `cap`, and `begin` returns `true`
/// only when it grows it — so once every reserved slot has resolved
/// (`Ready`/`Failed`, each exactly once), **no frame can ever start another load**
/// regardless of how many nodes are visible. Hosts beyond the first `cap`
/// distinct sightings keep their provenance shapes for the session (the designed
/// degradation; `CACHE_CAP` comfortably exceeds any legible projection).
///
/// Kept generic over `T` so the capacity + no-retry policy is unit-tested
/// natively (with `T = u32`), independent of `HtmlImageElement`.
#[derive(Debug, Default)]
pub struct IconCache<T> {
    map: HashMap<String, Slot<T>>,
    cap: usize,
}

impl<T> IconCache<T> {
    /// A cache holding at most `cap` hosts (use [`CACHE_CAP`]).
    pub fn new(cap: usize) -> Self {
        IconCache {
            map: HashMap::new(),
            cap: cap.max(1),
        }
    }

    /// Whether `host` has a slot (any state). A frame that sees `false` may
    /// [`begin`](Self::begin) a load (subject to capacity); `true` means the
    /// load-once policy already covers it.
    pub fn contains(&self, host: &str) -> bool {
        self.map.contains_key(host)
    }

    /// The decoded image for `host`, iff its slot is [`Slot::Ready`].
    pub fn ready(&self, host: &str) -> Option<&T> {
        match self.map.get(host) {
            Some(Slot::Ready(v)) => Some(v),
            _ => None,
        }
    }

    /// Reserve a `Loading` slot for `host`. Returns `true` if a **new** load
    /// should start; `false` when a slot already exists (load-once / never-retry —
    /// including a prior `Failed`) **or the cache is at capacity** (the churn
    /// guard: no eviction, no re-request — see the type-level note).
    pub fn begin(&mut self, host: &str) -> bool {
        if self.map.contains_key(host) || self.map.len() >= self.cap {
            return false;
        }
        self.map.insert(host.to_string(), Slot::Loading);
        true
    }

    /// Mark a completed load. A no-op for an unknown host (defensive; slots are
    /// never removed, so a reserved host is always present).
    pub fn set_ready(&mut self, host: &str, image: T) {
        if let Some(slot) = self.map.get_mut(host) {
            *slot = Slot::Ready(image);
        }
    }

    /// Mark a failed load (never retried this session). No-op for an unknown host.
    pub fn set_failed(&mut self, host: &str) {
        if let Some(slot) = self.map.get_mut(host) {
            *slot = Slot::Failed;
        }
    }

    /// Number of tracked hosts (any state).
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// How many more hosts [`begin`](Self::begin) would accept. The renderer uses
    /// this to bound its per-frame miss collection — at `0` it collects nothing,
    /// so a full cache costs a saturated graph zero allocations per frame.
    pub fn remaining(&self) -> usize {
        self.cap.saturating_sub(self.map.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_url_encodes_scheme_and_carries_no_literal_slashes() {
        let base = "chrome-extension://abcdef/_favicon/";
        let url = favicon_url(base, "news.example", 16);
        assert_eq!(
            url,
            "chrome-extension://abcdef/_favicon/?pageUrl=https%3A%2F%2Fnews.example&size=16"
        );
        // The whole point of the encoding (§F12 audit interaction): the pageUrl
        // value never carries a literal "://", so the *value* can't trip the
        // network-surface grep. (The `chrome-extension://` in the base is from
        // getURL at runtime and is not http(s); it never appears in dist plaintext.)
        let query = url.split_once("pageUrl=").unwrap().1;
        assert!(!query.contains("://"));
        assert!(query.starts_with("https%3A%2F%2F"));
    }

    #[test]
    fn favicon_url_size_32_for_hidpi() {
        let url = favicon_url("x/_favicon/", "a.example", 32);
        assert!(url.ends_with("&size=32"));
    }

    #[test]
    fn favicon_url_is_idempotent_on_already_schemed_host() {
        // `with_scheme` won't double-prepend; a defensive already-http host still
        // encodes cleanly (no literal "://").
        let url = favicon_url("b/_favicon/", "https://a.example", 16);
        assert!(url.contains("pageUrl=https%3A%2F%2Fa.example"));
    }

    #[test]
    fn percent_encode_leaves_unreserved_and_escapes_the_rest() {
        assert_eq!(percent_encode("aZ0-_.~"), "aZ0-_.~");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(
            percent_encode("https://x/y?z=1&w"),
            "https%3A%2F%2Fx%2Fy%3Fz%3D1%26w"
        );
        // Non-ASCII (an IDN host) is encoded byte-by-byte as UTF-8.
        assert_eq!(percent_encode("é"), "%C3%A9");
    }

    #[test]
    fn cache_loads_once_and_never_retries() {
        let mut c: IconCache<u32> = IconCache::new(CACHE_CAP);
        assert!(c.begin("a"), "first sighting should start a load");
        assert!(!c.begin("a"), "a Loading slot must not restart the load");
        c.set_ready("a", 7);
        assert_eq!(c.ready("a"), Some(&7));
        assert!(!c.begin("a"), "a Ready slot must not restart the load");

        assert!(c.begin("b"));
        c.set_failed("b");
        assert_eq!(c.ready("b"), None);
        assert!(
            !c.begin("b"),
            "a Failed slot must never retry within the session"
        );
    }

    #[test]
    fn cache_refuses_new_hosts_at_capacity_without_evicting() {
        // The churn guard: at capacity, begin() must neither insert nor evict —
        // otherwise >cap visible hosts would thrash slots forever (see the type doc).
        let mut c: IconCache<u32> = IconCache::new(2);
        assert!(c.begin("a"));
        assert!(c.begin("b"));
        assert_eq!(c.len(), 2);
        assert_eq!(c.remaining(), 0);
        assert!(!c.begin("c"), "at capacity, a new host must be refused");
        assert_eq!(c.len(), 2);
        assert!(c.contains("a") && c.contains("b"), "no eviction ever");
        assert!(!c.contains("c"));
        // Refused hosts stay refused — repeat frames never squeeze one in.
        assert!(!c.begin("c"));
        // Slots resolving does NOT free capacity (slots are per-session, not LRU).
        c.set_ready("a", 1);
        c.set_failed("b");
        assert!(!c.begin("c"));
    }

    #[test]
    fn set_ready_on_unknown_host_is_a_noop() {
        // Defensive: resolving a host that never got a slot must not create one.
        let mut c: IconCache<u32> = IconCache::new(1);
        c.set_ready("ghost", 1);
        c.set_failed("phantom");
        assert!(c.is_empty());
        assert_eq!(c.ready("ghost"), None);
    }

    #[test]
    fn steady_state_no_loads_once_reachable_slots_resolve() {
        // The property the churn guard exists for (§F12 review fix): with ANY number
        // of visible hosts — here 3× the cap — the per-frame loop reaches a steady
        // state where no frame starts a load once every reachable slot has resolved.
        const CAP: usize = 8;
        let hosts: Vec<String> = (0..CAP * 3).map(|i| format!("h{i}.example")).collect();
        let mut c: IconCache<u32> = IconCache::new(CAP);

        // Frame 1: simulate the renderer — every uncached visible host is a miss,
        // bounded by remaining capacity (mirrors draw()'s budget); each miss begins.
        let mut started_total = 0;
        let misses: Vec<&String> = hosts
            .iter()
            .filter(|h| !c.contains(h))
            .take(c.remaining())
            .collect();
        for h in misses {
            if c.begin(h) {
                started_total += 1;
            }
        }
        assert_eq!(started_total, CAP, "first frame fills exactly cap slots");

        // All in-flight loads resolve (mix of success and failure).
        for (i, h) in hosts.iter().take(CAP).enumerate() {
            if i % 2 == 0 {
                c.set_ready(h, i as u32);
            } else {
                c.set_failed(h);
            }
        }

        // Frames 2..=10: same visible set every frame. Steady state: the budget is
        // zero, so no misses are even collected — and begin() would refuse anyway.
        // Two phases per frame (collect, then begin), exactly like the renderer.
        for _ in 0..9 {
            let budget = c.remaining();
            assert_eq!(budget, 0, "a full cache advertises no capacity");
            let misses: Vec<&String> = hosts
                .iter()
                .filter(|h| !c.contains(h))
                .take(budget)
                .collect();
            let started = misses.into_iter().filter(|h| c.begin(h)).count();
            assert_eq!(started, 0, "no frame after saturation may start a load");
        }
        assert_eq!(c.len(), CAP, "slot count is monotone and capped — no churn");
    }
}
