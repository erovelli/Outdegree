# Store screenshots

The Web Store listing needs **≥1 screenshot** (1280×800 or 640×400). The
full-screen graph is the hero shot.

> Current assets live in [`docs/assets/`](assets/): `graph.png` (README hero +
> store hero), `sankey.png`, and `tables.png` — all 1280×800. Re-capture with the
> steps below when the UI changes.

## The look

The dashboard is a Palantir-AIP–style workspace: a pure-black, full-bleed graph
canvas with translucent "glass" control panels floating over it. The single
blue→violet→magenta→red OKLCH spectrum is the only color, and it encodes data
only (node fill = how you arrived; node size = visits; edges colored by kind,
search links dashed, arrowheads show direction).

## How to capture

1. Build and load the extension (`./build.sh`, then load `dist/` unpacked).
2. Browse normally for a while so the graph has real structure (a few dozen
   sites across several sessions makes the best shot).
3. Open the dashboard (toolbar icon), **Graph** tab. Pan/zoom (or hit **fit**,
   bottom-right) to frame a readable cluster; hover a hub to show its inspect
   callout + neighborhood spotlight, or click it to drill into its ego network.
   You can also **drag nodes** to arrange them before capturing.
4. Capture at the store's size:
   - DevTools → device toolbar → set 1280×800, or
   - capture the dashboard tab and crop to 1280×800.
5. Suggested extra shots:
   - **Sankey** tab — a session's left→right flow (start hosts on the left), a
     strong "how I move between sites" story.
   - **Tables** tab — top hubs by weighted degree.
   - The **provenance legend** (right) + **range** control (top) visible to show
     the time-window and color encoding at a glance.

## Tips

- Lead the listing copy with **"100% local · no network · open source."**
- Reinforce the privacy posture: the brand panel's **REC** indicator (top-left,
  click to pause) and the **gear → settings** menu (Export, Forget domain,
  Delete last N days, Rebuild from raw events) all visibly say "your data, your
  device, your control."
- Save final assets under `docs/assets/` if you want them tracked in the repo.
