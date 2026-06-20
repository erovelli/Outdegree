# Chrome Web Store submission material (§12)

Everything an agent/maintainer needs to fill the Web Store listing for the
**Browsing Graph** extension. The architecture makes the privacy story
*verifiable*, which de-risks the (stricter-than-usual) review that any
browsing-data extension receives.

## Naming (§12.5)

- **Repo / codename:** `Outdegree` (unique, brandable).
- **Store display title:** **Browsing Graph — a local-only map of how you
  browse.** (Must not contain "Chrome" or imply Google endorsement; the store
  does not enforce title uniqueness.)

## Single purpose (§12.4)

> Visualize your own web-browsing history as an interactive, local-only
> navigation graph.

## Short / detailed description (§12.6)

Lead both with **"100% local · no network · open source."**

> **Browsing Graph** turns your browsing history into an interactive directed
> graph you can explore over time — by session, day, week, month, or year. See
> which sites you move between, which are hubs, and how you got there (link,
> search, typed, form). Everything is processed and stored **locally on your
> device**; the extension makes **no network requests** and cannot read page
> content. Open source.

## Per-permission justifications (§12.3)

- **`webNavigation`** — "Observes page-navigation events to record which sites
  you visit and how you move between them. Reads navigation metadata only (URL,
  navigation type); does not read or access page content."
- **`storage` / `unlimitedStorage`** — "Stores your navigation graph locally on
  your device. `unlimitedStorage` allows a long history (months to a year)
  without hitting the default quota. Nothing is transmitted off-device."
- **No host permissions** — call out as a feature: the extension cannot read
  page content or contact any server.

## Data-practices disclosures (§12.4)

- **Data collected by the developer:** none. Browsing activity is processed
  **locally** and never transmitted to the developer or anyone else.
- Certifications:
  - **Not sold or transferred** to third parties.
  - **Not used** for purposes unrelated to the single purpose.
  - **Not used** for creditworthiness or lending.
  - The "web history / user activity" category is **handled locally**, not
    *collected*.

## Listing logistics (§12.6)

- One-time **$5** developer registration fee.
- Assets: icons (16/32/48/128 — see `extension/icons/`), **≥1 screenshot** (the
  dashboard graph is the hero shot), short + detailed descriptions.
- Expect a **slower, stricter review** due to the browsing-data profile; the
  enforced local-only guarantee is what de-risks it.
- Privacy policy: host `docs/privacy-policy.md` (e.g. GitHub Pages) and link it
  in the listing.

## Pre-submit checklist (§12.7)

- [x] Build the **packed/production** extension (`./build.sh`) — verified the
      dashboard WASM is instantiated **from inlined bytes**, so it works under
      the production CSP (no fetch of the wasm; `connect-src 'none'` cannot block
      instantiation).
- [x] Confirm `connect-src 'none'` breaks nothing — there are no intended
      network calls; export is a local `Blob` download, not a `connect`.
- [x] **Grep the built bundle** for network surface — `fetch`,
      `XMLHttpRequest`, `WebSocket`, `EventSource`, `navigator.sendBeacon`,
      `https://`/`http://`, dynamic `import()`, `importScripts`: **all zero** in
      `dist/` (Vite modulepreload polyfill disabled; wasm-bindgen fetch
      self-loader neutralized).
- [x] Confirm `psl` uses the embedded list (no runtime fetch) and the UI runs in
      CSR (the DOM is built at runtime; no hydration callback).
- [ ] Privacy policy hosted + linked; the form copy above filled in.
- [x] `"incognito": "not_allowed"` present; `host_permissions: []`; CSP includes
      `connect-src 'none'` (verified in `dist/manifest.json`).

Re-run the bundle audit any time with:

```bash
./build.sh
grep -rIoE "fetch\(|XMLHttpRequest|WebSocket|EventSource|sendBeacon|https?://" dist
# (expected: no matches)
```
