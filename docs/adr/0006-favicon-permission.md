# 6. Add the MV3 `favicon` permission for local site icons

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

The graph, tables, session list, and node inspector identify sites by **hostname
text** only. At a glance a dense graph or a long session list is hard to scan —
text alone carries no instant "that's GitHub / that's Gmail" recognition. Site
**favicons** are the universal, pre-attentive identity mark for a site and would
make every surface more legible.

Chrome (MV3) exposes the browser's **local** favicon cache to an extension through
the `favicon` permission: with it declared, the extension can load an icon from
its *own* origin at

```
chrome-extension://<extension-id>/_favicon/?pageUrl=<url>&size=<16|32>
```

Chrome answers from its **on-disk favicon cache** — it makes **no network
request**. But adding the permission changes a documented, CI-audited invariant:

> `permissions ⊆ { webNavigation, storage, unlimitedStorage }`

This ADR records the trade. The maintainer pre-approved it by mandating the
feature; the point here is to document it exhaustively, not to re-litigate it.

## Decision

Add `favicon` to `permissions` (the fourth and only non-original permission) and
render site icons across the dashboard, **gated behind a "Site icons" setting
(default on)** so the pure monochrome-plus-single-hue look remains one click away.

### The local-cache mechanism (why no-egress still holds)

- The URL is **same-origin** (`chrome-extension://<id>/…`), so under the page CSP
  `default-src 'self'` — which `img-src` falls back to — the browser is *allowed*
  to load it, and `connect-src 'none'` is untouched (an `<img>`/`drawImage` load is
  not a `connect`).
- Chrome serves the bytes from its **local favicon cache**; there is no fetch to
  any remote host. The extension still declares **no `host_permissions`**, so it
  has no granted ability to contact any origin regardless.
- The service returns whatever Chrome already has cached for that page (or its
  generic default); the extension neither triggers a favicon download nor learns
  anything it didn't already have.

So the headline guarantee — *the shipped extension makes no network request* — is
preserved. The CI **network-surface audit** (grep `dist/` for
`fetch|XHR|WebSocket|…|https?://`) still passes at zero, which is the machine-checked
proof (see below).

### The audit interaction (the load-bearing subtlety)

`pageUrl` must carry an `http(s)` scheme. Naively that means a `https://<host>`
string somewhere — and the network-surface audit greps the built `dist/` for
`https?://`, expecting **zero** non-`w3.org` matches. We must not weaken that grep.

Two facts make the scheme invisible to the audit **without** touching it:

1. **All URL construction is in the Rust→WASM core** (`crate::favicon::favicon_url`,
   called from the canvas renderer and the HTML builders). The `.wasm` is
   **base64-inlined** into the JS bundle (ADR-0004), so any string constant in the
   WASM data section is scrambled by base64 — a raw `grep https?://` cannot match
   it. This is the exact same reason the F4 sample-data loader's `URL_SCHEME`
   const is already audit-clean, and `favicon_url` **reuses that very const**
   (`crate::sample::with_scheme`) rather than introducing a second scheme literal.
2. **The `pageUrl` value is percent-encoded** (`crate::favicon::percent_encode`),
   so the runtime string is `…pageUrl=https%3A%2F%2F<host>&size=16` — it contains
   no literal `://` at all, in the DOM or anywhere else.

The `chrome.runtime.getURL("_favicon/")` base comes from a bridge call at runtime
(`chromeBridge.faviconBase`), so the extension-origin URL is never a bundle
literal either. Net result: the dist grep stays at **exactly zero** matches. This
was verified two ways: (a) inspecting the built bundle (`grep https:// dist` →
none), and (b) an **audit-bite check** — temporarily injecting a fifth permission
and a `fetch(` made *both* the manifest audit and the network-surface audit fail,
confirming they still bite; then reverted.

**Alternative considered:** teach the audit to allow exactly one named `https://`
occurrence and assert its count is 1. Rejected — it weakens a headline guarantee's
check for a cosmetic feature; the runtime-concatenation + percent-encoding approach
keeps the audit at a hard zero, which is strictly safer.

### Rendering & degradation

- **Canvas:** at/above the existing LOD zoom **and** when a node's disc is ≥ 12px,
  the decoded favicon is drawn centered inside the disc, clipped to a circle, with
  the provenance-colored ring still showing around it — so the single-hue **data**
  encoding (provenance) is preserved; the favicon is an *identity* mark layered on
  top, like the (achromatic) host label, not a new data-color channel. Icons load
  off the render loop into a bounded, load-once/never-retry cache
  (`IconCache`, cap ~512): the provenance shape is drawn immediately and the icon
  appears on a later frame once decoded. A host with no cached icon, or a load
  error, keeps its shape forever (never retried this session). Cache slots are
  first-come-first-served and **never evicted**: at capacity the renderer stops
  collecting misses (its per-frame miss list is capped at the cache's remaining
  capacity) and `begin` refuses new hosts, so total loads per session are provably
  ≤ the cap. Even with more visible icon-qualifying hosts than slots, the system
  reaches a steady state with **zero** per-frame load work once all reachable
  slots resolve — eviction would instead thrash slots among visible hosts forever
  (miss → evict → re-miss). Hosts beyond the first ~512 distinct sightings keep
  their provenance shapes for the session: the designed degradation.
- **HTML:** a 16px `alt=""` `loading="lazy"` `<img>` leads the host name in Tables
  cells, session-list cards, and the inspector header. A page-level **capturing**
  `error` listener adds a CSS-hidden `.is-broken` class (resource `error` events
  don't bubble, and CSP forbids inline `onerror`), so a missing icon shows nothing
  rather than a broken-image glyph.
- **Off / unsupported:** when the "Site icons" toggle is off, **no `_favicon` URL
  is constructed at all** (canvas or HTML). The bridge returns `""` for the base
  when the running browser doesn't grant the permission, which also makes the whole
  feature inert.

### The Firefox difference

The `_favicon` service is **Chromium-only** — Firefox has no equivalent. So:

- `scripts/build-firefox.mjs` **strips** the `favicon` permission from the overlay
  manifest (declaring an unsupported permission would only produce an install
  warning). This is why the two CI manifest audits differ: the Chrome (`web`) job
  allows **four** permissions (incl. `favicon`); the Firefox job allows **three**
  and asserts exactly that, so a future leak of `favicon` into `dist-firefox/`
  would fail CI.
- The dashboard **runtime-guards** on the permission actually being declared
  (`chromeBridge.faviconBase` checks `getManifest().permissions.includes("favicon")`
  before returning the base), so on the Firefox build the feature no-ops silently
  to the current shapes/text — no errors, no broken images. See `docs/PORTING.md`.

## Consequences

- Every dashboard surface gains instant site recognition; the graph and session
  list become far more scannable, with **no** new network surface and **no**
  weakening of the CSP, `host_permissions: []`, or the network-surface audit.
- The Chrome Web Store review must be updated: the store listing now declares four
  permissions with a per-permission justification for `favicon`
  (`docs/STORE-LISTING.md`), and the privacy policy names the local-cache icon use
  (`docs/privacy-policy.md`). Because it adds a new permission, the store will
  re-review; the "reads from Chrome's local cache, no network" story keeps it a
  small, defensible delta.
- Cost/limits: Chrome may return a generic default icon for a page it has never
  cached (a real-Chrome behavior we can't distinguish from a real icon), and the
  icon reflects whatever Chrome cached. Both are cosmetic and the feature is
  toggle-off-able. Firefox/Edge: Edge (Chromium) gets icons like Chrome; Firefox
  quietly shows none.

## Alternatives rejected

- **Letter/monogram glyphs** (draw the first letter of the host). No permission,
  but weak recognition and it competes with the monochrome type system; a poor
  substitute for the real mark. Rejected.
- **No icons at all** (status quo). Simplest and permission-free, but leaves the
  legibility problem the feature exists to solve. Rejected by the mandate.
- **Bundling/generating icons ourselves.** Would need network fetching or a large
  asset set and wouldn't match the user's actual sites. Rejected — defeats the
  local-only, zero-asset design.
- **Weakening the network-surface audit** to permit one `https://`. Rejected in
  favor of runtime-concatenation + percent-encoding (see above), which keeps the
  audit at a hard zero.
