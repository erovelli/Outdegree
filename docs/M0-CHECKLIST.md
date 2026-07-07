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
| 12 | Click **REC** (top-left) to pause, navigate, then click again to resume | **no** new `events` while paused; records **resume** after un-pausing (regression guard: the flag is stored as a string, so resume must actually restart capture) |
| 13 | **Switch tabs** within a window (click another tab) | one `focus` record (`tabId`, `windowId`) per activation, appended in firing order |
| 14 | **Alt-tab away** from Chrome to another app, then back | a `wfocus` with `windowId: -1` on blur, then a `wfocus` with the real window id on return; clicking between two Chrome windows records a `wfocus` per switch (some platforms emit an intermediate `-1`) |
| 15 | **Close the focused tab** | a `close` record, then a `focus` for whichever tab Chrome activates next |
| 16 | Pause (REC), then switch tabs and alt-tab | **no** `focus`/`wfocus` records while paused; they resume after un-pausing |

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

All sixteen rows behave as described; the redirect window and §7.2 table reflect
reality. Then the derived views (graph, tables, sessions) on the dashboard
should match what you browsed.

> Rows 13–16 (foreground attention, §F7) were added with the focus/wfocus
> capture and **have not yet been walked in a real Chrome** — the attribution
> logic is fully covered by `cargo test`, but the capture wiring (event order,
> platform blur behavior, pause gating) needs this manual pass before release.

## Derived-view acceptance (graph/Sankey behavior)

Once capture is correct, confirm the read-side behaviors (these are the failure
modes fixed during real-browser testing — each has a `cargo test`/CI guard, but
verify the end-to-end result once in Chrome). The dashboard folds new events on
open and **live-refreshes** on tab focus/visibility, so just switch back to it.

| # | Action | Expect on the dashboard |
|---|---|---|
| A | Type a URL, then click a link on it; **stay** on the landing page | both hosts **and the edge** appear in the **Graph** (the current page shows via the provisional buffer flush), and in the **Sankey** flow |
| B | From one page, open several links **in new tabs** (e.g. a list → articles) | the **Graph** shows edges `source → each new-tab host` (not isolated dots); the **Sankey** fans out from the source |
| C | Browse a few pages in another tab, then switch back to the dashboard | new nodes/edges appear without reopening; returning **refits** the graph |
| D | **Sankey** tab → pick a session | starting hosts on the **left**, flow → right; toggle **Hostname/Domain** regroups |
| E | **Drag** a node on the Graph | it moves and stays put; reopening preserves the arrangement |
| F | Settings (gear) → **Rebuild from raw events** | the whole graph re-derives from the stored `events` (recovery if the cursor ever drifts) |
| G | Read a page ~1 min in the foreground, leave another tab open in the background, then check **Tables → Top hubs** | "Time spent" shows ≈1 min for the read page (no "≈" prefix — the window has focus data); the background tab's host shows little or none; hover the cell for the explanatory tooltip |
| H | **Upgrade path**: load this build over a profile that captured data with a pre-focus build | on first open the dashboard rebuilds from raw once (brief but automatic); old days show "≈"-prefixed estimates, days after the upgrade show foreground time |

### Accept

All rows behave as described; the Graph and Sankey agree on which hops happened.
