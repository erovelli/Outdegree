# Store screenshots

The Web Store listing needs **≥1 screenshot** (1280×800 or 640×400). The
dashboard graph is the hero shot.

## How to capture

1. Build and load the extension (`./build.sh`, then load `dist/` unpacked).
2. Browse normally for a while so the graph has real structure (a few dozen
   sites across several sessions makes the best shot).
3. Open the dashboard, **Graph** tab. Pan/zoom to frame a readable cluster;
   optionally click a hub to show its ego network.
4. Capture at the store's size:
   - DevTools → device toolbar → set 1280×800, or
   - capture the dashboard tab and crop to 1280×800.
5. Suggested extra shots: the **Tables** tab (hubs / top edges / origination) and
   a **Sessions** flow.

## Tips

- Lead the listing copy with **"100% local · no network · open source."**
- Show the privacy posture: a screenshot with the **Pause**, **Export**, and
  **Forget domain** controls visible reinforces the local-only story.
- Save final assets under `docs/assets/` if you want them tracked in the repo.
