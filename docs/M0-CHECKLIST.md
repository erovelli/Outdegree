# M0 — Capture verification gate

The one milestone that needs a **real Chrome and some browsing** — it can't be
automated. Work through it once after a build; record what you observe and tune
the few constants noted at the end. (Everything else is covered by `cargo test`
and the CI browser tests.)

## Setup

1. Build the extension:
   ```bash
   ./build.sh        # → dist/
   ```
2. `chrome://extensions` → enable **Developer mode** → **Load unpacked** → select
   `dist/`.
3. Open the service-worker console: on the extension card, click **service
   worker** (or **Inspect views: service worker**).
4. Open the dashboard via the toolbar icon. To inspect raw capture: DevTools →
   **Application** → **IndexedDB** → `browsing_graph` → `events`. (Refresh the
   IndexedDB view after each action.) The dashboard's **Raw** tab shows the same
   stream once you reopen/refresh it.

For each step below, do the action, then confirm the expected `events` record.
Record the **observed** `transitionType` / `qualifiers` — Chrome's exact values
are what the §7.2 mapping must match.

## Scenarios

| # | Action | Expect in `events` (or not) |
|---|---|---|
| 1 | Click a normal link | one `nav`, `transitionType: "link"` |
| 2 | Type a URL in the address bar + Enter | one `nav`, `transitionType: "typed"` |
| 3 | Submit a form (e.g. a search box that POSTs) | one `nav`, `transitionType: "form_submit"` |
| 4 | Search from the omnibox / a results page | one `nav`, `transitionType` in `generated`/`keyword`/`keyword_generated` |
| 5 | Load a page with an **iframe** | **no** extra record for the iframe (frameId ≠ 0 is ignored) |
| 6 | Visit a URL that **client-redirects** (e.g. a shortener) | the landing `nav` carries `client_redirect` in `qualifiers`; note the Δt between hops |
| 7 | **Open link in new tab** (middle-click / Ctrl-click) | a `link` record (`newTabId`, `sourceTabId`), then a `nav` in the new tab |
| 8 | **Back button** to a search-results page | a `nav` with `forward_back` in `qualifiers`; **check its `transitionType`** |
| 9 | **Restart the browser** (quit + reopen) | a `start` record on startup |
| 10 | **Close a tab** | a `close` record (`tabId`) |
| 11 | Open the **dashboard** itself | **no** `nav` recorded for the extension page |
| 12 | Toggle **Pause** in the dashboard, then navigate | **no** new records while paused |

## What to tune from observations

- **`REDIRECT_WINDOW_MS`** (`crates/core/src/rollup.rs`, default 1000): set it
  above the largest hop Δt you saw in step 6, but well below a normal
  inter-navigation gap, so redirect bursts collapse but real navigations don't.
- **§7.2 classification table** (`crates/core/src/interpret.rs`): correct any
  `transitionType` that didn't match what you observed (steps 1–4, 8). In
  particular, confirm **`auto_bookmark`** (marked *verify at M0*) and the
  back-to-results `transitionType` in step 8.
- **Search-link survives the back button**: in step 8, the back-to-results
  `transitionType` must classify as `SearchOrigin` (so `last_prov` becomes
  `SearchOrigin` and subsequent result clicks are colored `SearchLink`). If
  Chrome reports a different type for that nav, extend the §7.2 mapping
  accordingly.

## Accept

All twelve rows behave as described; the redirect window and §7.2 table reflect
reality. Then the derived views (graph, tables, sessions) on the dashboard
should match what you browsed.
